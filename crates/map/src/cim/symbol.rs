//! ArcGIS Pro's symbols (stacks of symbol layers, bottom to top) as RustGIS's [`Symbol`],
//! [`LineStroke`], [`FillLayer`] and [`Marker`], and a text symbol as a [`Text`] style. A vector
//! marker RustGIS has no shape for is drawn from its paths ([`Marker::Path`]), and so is a stack
//! of marker layers; a marker layer of a line symbol becomes markers along the line, and one of a
//! polygon symbol placed inside it a grid of dots. A symbol layer RustGIS has no match for is drawn
//! as near as it can (a gradient by its two ends) or left out, and noted in the [`Report`] under the
//! name of the element it belongs to.
//!
//! CIM counts widths, sizes and offsets in 1/72 inch points, which are scaled to the 96ths the
//! symbol model counts in ([`pt96`]); a text height is in true points ([`text_pt`]). Every list
//! read is bounded, and a key that is missing or of another type reads as absent.
#![allow(dead_code)] // used by the layout and map readers built on this module

use serde_json::Value;

use super::color::{color, symbol_of};
use super::units::{pt96, text_pt};
use super::{Report, flag, num, str_of};
use crate::Rgba;
use crate::fills::{FillLayer, FillPattern, GradientPoint, GradientShape, MAX_FILLS, MAX_STOPS, Spread, Stop, Stops};
use crate::layout::{TextAlign, TextCase, VAlign};
use crate::symbology::{
    Dash, Glyph, ImageRef, LineCap, LineJoin, LineMark, LineMarkers, LineStroke, MAX_DASH_STEPS, MAX_PATH_PARTS, MAX_PATH_POINTS,
    MAX_STROKES, Marker, PathPart, PathRef, PathShape, Placements, Symbol, font_name,
};

/// The most symbol layers, effects and marker graphics read from one symbol.
const MAX_ITEMS: usize = 64;
/// The points of a closed ring that stand for a triangle, a square and a star.
const TRIANGLE_POINTS: usize = 4;
const SQUARE_POINTS: usize = 5;
const STAR_POINTS: usize = 11;
/// How far from an axis a ring's side may lie, as a fraction of the ring's size, to be on it.
const SHAPE_TOLERANCE: f64 = 1e-3;
/// A text's size and a marker's, in points, when the symbol gives none (Pro's own defaults).
const DEFAULT_TEXT_PT: f64 = 10.0;
const DEFAULT_MARKER_PT: f64 = 6.0;
/// A hatch fill's spacing in points when it gives none.
const DEFAULT_HATCH_PT: f64 = 8.0;
/// The spacing of markers inside a polygon, in points, when it gives none (Pro's default).
const DEFAULT_STEP_PT: f64 = 16.0;
/// The points a circle of a marker's drawing is drawn with.
const CIRCLE_POINTS: usize = 32;
/// How far from one circle, as a fraction of its radius, a ring of arcs may stray to be that circle.
const CIRCLE_TOLERANCE: f64 = 1e-3;
/// How far a drawn marker's point may reach from its centre, in its sizes (as [`PathRef`] keeps it).
const PATH_REACH: f64 = 2.0;

const NULL: Value = Value::Null;

// ---- the shared parts -------------------------------------------------------------------------

/// The enabled layers of the symbol `sym` (a symbol, or a `CIMSymbolReference` holding one),
/// bottom first.
fn layers(sym: &Value) -> impl Iterator<Item = &Value> {
    symbol_of(sym)
        .and_then(|s| s.get("symbolLayers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_ITEMS)
        .filter(|l| flag(l, "enable", true))
}

/// At most 64 characters of `s`, for a note that names something a file said.
pub(super) fn shown(s: &str) -> String {
    s.chars().take(64).collect()
}

/// The colour of a symbol layer, `None` when it has none, when it is fully transparent (the same
/// as none), or when it can't be read (noted).
fn ink(layer: &Value, what: &str, report: &mut Report) -> Option<Rgba> {
    match color(layer.get("color")?) {
        Some(c) if c.a() > 0 => Some(c),
        Some(_) => None,
        None => {
            report.skipped(what, "symbol layers whose colour can't be read, left out");
            None
        }
    }
}

/// A distance in points as 96ths, keeping its side.
fn signed_pt96(v: f64) -> f32 {
    if v < 0.0 { -pt96(-v) } else { pt96(v) }
}

/// The colour a symbol layer is drawn in when it has none to read.
const CLEAR: Rgba = Rgba::new(0, 0, 0, 0);

// ---- polygon and line symbols -----------------------------------------------------------------

/// What a polygon or line symbol draws. `fill` is the topmost solid fill and `stroke` the
/// topmost stroke with its width in 96ths; `strokes` are all the strokes, bottom first (at most
/// [`MAX_STROKES`]) and `fills` the stacked fills (at most [`MAX_FILLS`]), empty when the symbol's
/// fill is one colour alone, as `fill` says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Paint {
    pub fill: Option<Rgba>,
    pub stroke: Option<(Rgba, f32)>,
    pub strokes: Vec<LineStroke>,
    pub fills: Vec<FillLayer>,
}

/// A polygon symbol: its solid, hatch and gradient fills, its markers placed inside it (as dots),
/// and its strokes.
pub(crate) fn polygon(sym: &Value, what: &str, report: &mut Report) -> Paint {
    read(sym, true, true, what, report)
}

/// A line symbol: its strokes and its markers along the line (its other layers are left out and
/// noted).
pub(crate) fn line(sym: &Value, what: &str, report: &mut Report) -> Paint {
    read(sym, false, true, what, report)
}

/// The symbol a marker's graphic (or a character marker's character) is painted with: a polygon
/// symbol whose own marker layers are left out (noted), so a marker can't hold itself.
fn graphic_paint(sym: &Value, what: &str, report: &mut Report) -> Paint {
    read(sym, true, false, what, report)
}

const MARKER_LAYERS: [&str; 3] = ["CIMVectorMarker", "CIMPictureMarker", "CIMCharacterMarker"];

fn read(sym: &Value, with_fills: bool, with_markers: bool, what: &str, report: &mut Report) -> Paint {
    let mut fill = None;
    let mut strokes = Vec::new();
    let mut fills = Vec::new();
    for l in layers(sym) {
        match str_of(l, "type") {
            Some("CIMSolidStroke") => strokes.extend(stroke(l, what, report)),
            Some(m) if with_markers && MARKER_LAYERS.contains(&m) => {
                if !with_fills {
                    strokes.extend(line_markers(l, what, report));
                } else if l.get("markerPlacement").and_then(|p| str_of(p, "type")) == Some("CIMMarkerPlacementInsidePolygon") {
                    fills.extend(inside_polygon(l, what, report));
                } else {
                    report.skipped(what, "markers along polygon outlines, left out");
                }
            }
            Some("CIMSolidFill") if with_fills => {
                if let Some(c) = ink(l, what, report) {
                    fill = Some(c);
                    fills.push(FillLayer { color: Some(c), pattern: FillPattern::Solid });
                }
            }
            Some("CIMHatchFill") if with_fills => fills.extend(hatch(l, what, report)),
            Some("CIMGradientFill") if with_fills => fills.extend(gradient(l, what, report)),
            Some("CIMPictureFill") if with_fills => report.skipped(what, "picture fills, left out"),
            Some(other) => report.skipped(what, format!("{} symbol layers, left out", shown(other))),
            None => {}
        }
    }
    if strokes.len() > MAX_STROKES {
        report.skipped(what, format!("line strokes past the first {MAX_STROKES}"));
        strokes.truncate(MAX_STROKES);
    }
    if fills.len() > MAX_FILLS {
        report.skipped(what, format!("fills past the first {MAX_FILLS}"));
        fills.truncate(MAX_FILLS);
    }
    // One colour and nothing over it is the symbol's fill alone.
    if fills.len() <= 1 && fills.iter().all(|f| f.pattern == FillPattern::Solid) {
        fills.clear();
    }
    // The top line drawn as a line, else the top markers' colour.
    let stroke = strokes.iter().rev().find(|s| s.markers.is_none()).or(strokes.last()).and_then(|s| Some((s.color?, s.width?)));
    Paint { fill, stroke, strokes, fills }
}

/// A `CIMSolidStroke` as a stroke: `None` for one with no colour to draw (transparent).
fn stroke(layer: &Value, what: &str, report: &mut Report) -> Option<LineStroke> {
    let c = ink(layer, what, report)?;
    let mut dash = None;
    let mut offset = 0.0;
    for e in layer.get("effects").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS) {
        match str_of(e, "type") {
            Some("CIMGeometricEffectDashes") => dash = dashes(e, what, report),
            Some("CIMGeometricEffectOffset") => offset = signed_pt96(num(e, "offset").unwrap_or(0.0)),
            Some(other) => report.skipped(what, format!("line effects of type {}, left out", shown(other))),
            None => {}
        }
    }
    Some(
        LineStroke {
            color: Some(c),
            width: Some(pt96(num(layer, "width").unwrap_or(1.0))),
            offset,
            cap: str_of(layer, "capStyle").and_then(LineCap::from_code).unwrap_or(LineCap::Round),
            join: str_of(layer, "joinStyle").and_then(LineJoin::from_code).unwrap_or(LineJoin::Round),
            level: None,
            dash,
            markers: None,
        }
        .clamped(),
    )
}

/// The dash pattern of a `CIMGeometricEffectDashes`: its `dashTemplate` (points, dash and gap in
/// turn). `None` for none, and for one RustGIS can't draw (noted).
fn dashes(effect: &Value, what: &str, report: &mut Report) -> Option<Dash> {
    let template = effect.get("dashTemplate")?.as_array()?;
    if template.is_empty() {
        return None;
    }
    let steps: Vec<f32> = template.iter().take(MAX_DASH_STEPS + 1).map(|s| s.as_f64().map_or(f32::NAN, pt96)).collect();
    let dash = Dash::new(&steps);
    if dash.is_none() {
        report.approximated(what, "dash patterns RustGIS can't draw, drawn solid");
    }
    dash
}

/// A `CIMHatchFill`: parallel lines in its line symbol's top stroke, `rotation` degrees and
/// `separation` apart.
fn hatch(layer: &Value, what: &str, report: &mut Report) -> Option<FillLayer> {
    let sub = layer.get("lineSymbol")?;
    let line = layers(sub).filter(|l| str_of(l, "type") == Some("CIMSolidStroke")).filter_map(|l| stroke(l, what, report)).last()?;
    if line.dash.is_some() {
        report.approximated(what, "dashed hatch lines, drawn solid");
    }
    let pattern = FillPattern::Hatch {
        angle: num(layer, "rotation").unwrap_or(0.0) as f32,
        spacing: pt96(num(layer, "separation").unwrap_or(DEFAULT_HATCH_PT)),
        width: line.width.unwrap_or(1.0),
        offset: 0.0,
    };
    Some(FillLayer { color: line.color, pattern }.clamped())
}

/// A `CIMGradientFill`: from the ramp's first colour to its last (a multipart ramp's joins as
/// stops, evenly spaced). `None` for a ramp that can't be read (noted).
fn gradient(layer: &Value, what: &str, report: &mut Report) -> Option<FillLayer> {
    let Some((first, last, stops)) = layer.get("colorRamp").and_then(ramp) else {
        report.skipped(what, "gradient fills whose colour ramp can't be read, left out");
        return None;
    };
    let pattern = match str_of(layer, "gradientType") {
        // Shading in from the edge.
        Some("Buffered") => FillPattern::Shapeburst { color2: last, stops, discrete: false, distance: None, blur: 0, ignore_rings: false },
        kind => {
            let (shape, from, to) = match kind {
                Some("Circular") => (GradientShape::Radial, GradientPoint::CENTROID, GradientPoint::At([1.0, 0.5])),
                Some("Rectangular") => {
                    report.approximated(what, "rectangular gradient fills, drawn as circular ones");
                    (GradientShape::Radial, GradientPoint::CENTROID, GradientPoint::At([1.0, 0.5]))
                }
                _ => (GradientShape::Linear, GradientPoint::At([0.0, 0.5]), GradientPoint::At([1.0, 0.5])),
            };
            FillPattern::Gradient {
                shape,
                color2: last,
                stops,
                discrete: false,
                from,
                to,
                spread: Spread::Pad,
                angle: num(layer, "angle").unwrap_or(0.0) as f32,
                in_view: false,
            }
        }
    };
    Some(FillLayer { color: Some(first), pattern }.clamped())
}

/// A colour ramp's first and last colours and the colours between: a linear ramp's two ends, or
/// a multipart ramp's parts end to end (each join a stop, the parts of an even length).
fn ramp(r: &Value) -> Option<(Rgba, Rgba, Stops)> {
    let ends = |r: &Value| Some((color(r.get("fromColor")?)?, color(r.get("toColor")?)?));
    if str_of(r, "type") != Some("CIMMultipartColorRamp") {
        let (a, b) = ends(r)?;
        return Some((a, b, Stops::default()));
    }
    let parts: Vec<(Rgba, Rgba)> = r.get("colorRamps")?.as_array()?.iter().take(MAX_STOPS + 1).map(ends).collect::<Option<_>>()?;
    let (first, last) = (parts.first()?.0, parts.last()?.1);
    let n = parts.len() as f32;
    let stops = Stops::new(parts.iter().enumerate().skip(1).map(|(i, p)| Stop { at: i as f32 / n, color: p.0 }));
    Some((first, last, stops))
}

// ---- point symbols ----------------------------------------------------------------------------

/// The markers of a point symbol, bottom first, one for each enabled marker layer.
/// `paths` holds each marker's drawing as paths beside it (`None` for a picture or a character),
/// so the layers can be drawn as one marker ([`stacked`]). `pending_pictures` are the pictures
/// RustGIS can't keep as they are (GIF and BMP, which the engine converts to PNG): each is its
/// index in `symbols` (drawn as a circle meanwhile) and its `data:` URL.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PointSymbol {
    pub symbols: Vec<Symbol>,
    pub paths: Vec<Option<PathShape>>,
    pub pending_pictures: Vec<(usize, String)>,
}

/// A point symbol's markers: vector (a shape, or drawn), picture and character markers.
pub(crate) fn point(sym: &Value, what: &str, report: &mut Report) -> PointSymbol {
    let mut out = PointSymbol { symbols: Vec::new(), paths: Vec::new(), pending_pictures: Vec::new() };
    for l in layers(sym) {
        if let Some((symbol, path)) = marker_layer(l, out.symbols.len(), &mut out.pending_pictures, what, report) {
            out.symbols.push(symbol);
            out.paths.push(path);
        }
    }
    out
}

/// One marker layer as a marker and, for a vector marker, its drawing as paths. A picture that
/// must be converted goes into `pending` as the `index`th marker.
fn marker_layer(
    l: &Value,
    index: usize,
    pending: &mut Vec<(usize, String)>,
    what: &str,
    report: &mut Report,
) -> Option<(Symbol, Option<PathShape>)> {
    let size_pt = num(l, "size").unwrap_or(DEFAULT_MARKER_PT);
    let size = pt96(size_pt);
    let ty = str_of(l, "type");
    if matches!(ty, Some("CIMPictureMarker" | "CIMCharacterMarker")) && num(l, "rotation").is_some_and(|r| r != 0.0) {
        report.approximated(what, "marker rotation, left out");
    }
    match ty {
        Some("CIMVectorMarker") => Some(vector_marker(l, size_pt, what, report)),
        Some("CIMPictureMarker") => picture_marker(l, size, index, pending, what, report).map(|s| (s, None)),
        Some("CIMCharacterMarker") => character_marker(l, size, what, report).map(|s| (s, None)),
        Some(other) => {
            report.skipped(what, format!("{} symbol layers, left out", shown(other)));
            None
        }
        None => None,
    }
}

/// A marker symbol of `marker` painted as `paint` says.
fn marker_symbol(marker: Marker, size: f32, paint: &Paint) -> Symbol {
    let (stroke, stroke_width) = paint.stroke.unwrap_or((CLEAR, 0.0));
    Symbol { fill: paint.fill.unwrap_or(CLEAR), stroke, stroke_width, size, marker }
}

/// A `CIMVectorMarker` of `size_pt` points: its shape if it is one RustGIS has and it isn't turned
/// or moved, else its drawing as paths, painted as its first graphic's symbol; and its drawing.
fn vector_marker(l: &Value, size_pt: f64, what: &str, report: &mut Report) -> (Symbol, Option<PathShape>) {
    let graphic = l.get("markerGraphics").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS).find_map(|g| g.get("symbol"));
    // (Its notes come from `path_marker`, which reads every graphic.)
    let paint = graphic.map(|s| graphic_paint(s, what, &mut Report::default())).unwrap_or(Paint {
        fill: None,
        stroke: None,
        strokes: Vec::new(),
        fills: Vec::new(),
    });
    let path = path_marker(l, size_pt, what, report);
    let placed = ["rotation", "offsetX", "offsetY"].iter().any(|k| num(l, k).is_some_and(|v| v != 0.0));
    let marker = match vector_shape(l).filter(|_| !placed) {
        Some(shape) => shape,
        None => path.clone().and_then(PathRef::new).map(Marker::Path).unwrap_or_else(|| {
            report.approximated(what, "a drawn marker shape, shown as a circle");
            Marker::Circle
        }),
    };
    (marker_symbol(marker, pt96(size_pt), &paint), path)
}

/// A `CIMVectorMarker`'s graphics as paths in unit space: each graphic's `rings`, `paths` and
/// `curveRings` (a ring of arcs round one centre is a circle of [`CIRCLE_POINTS`]; other curves
/// are drawn through their control points, noted), scaled so the height of the marker's `frame`
/// (else of its graphics' box) is 1 (Pro sizes a marker by its frame's height), centred on the
/// frame's centre, turned by `rotation` (anticlockwise, as Pro) and moved by `offsetX`/`offsetY`
/// (points, as a fraction of `size_pt`). Each part is filled with its graphic's top solid fill and
/// stroked with its top stroke; a graphic with neither is left out. `None` for a marker with no
/// drawable graphic. At most [`MAX_PATH_PARTS`] parts of [`MAX_PATH_POINTS`] points are read.
pub(crate) fn path_marker(l: &Value, size_pt: f64, what: &str, report: &mut Report) -> Option<PathShape> {
    struct Raw {
        points: Vec<[f64; 2]>,
        closed: bool,
        fill: Option<Rgba>,
        stroke: Option<(Rgba, f32)>,
    }
    let graphics = l.get("markerGraphics")?.as_array()?;
    let (mut raw, mut cut, mut curved) = (Vec::new(), graphics.len() > MAX_PATH_PARTS, false);
    'graphics: for g in graphics.iter().take(MAX_PATH_PARTS) {
        let Some(p) = g.get("symbol").map(|s| graphic_paint(s, what, report)) else { continue };
        if p.fill.is_none() && p.stroke.is_none() {
            continue;
        }
        let geometry = g.get("geometry").unwrap_or(&NULL);
        for (key, closed) in [("rings", true), ("paths", false), ("curveRings", true)] {
            for part in geometry.get(key).and_then(Value::as_array).into_iter().flatten().take(MAX_PATH_PARTS) {
                if raw.len() >= MAX_PATH_PARTS {
                    cut = true;
                    break 'graphics;
                }
                let mut points = if key == "curveRings" {
                    let (points, approximated) = curve_ring(part);
                    curved |= approximated;
                    points
                } else {
                    part.as_array().into_iter().flatten().take(MAX_PATH_POINTS + 1).filter_map(xy).collect()
                };
                cut |= points.len() > MAX_PATH_POINTS;
                points.truncate(MAX_PATH_POINTS);
                if points.len() >= 2 {
                    raw.push(Raw { points, closed, fill: p.fill, stroke: p.stroke });
                }
            }
        }
    }
    if cut {
        report.approximated(what, format!("marker drawings past {MAX_PATH_PARTS} parts of {MAX_PATH_POINTS} points, cut short"));
    }
    if curved {
        report.approximated(what, "curves in marker drawings, drawn through their control points");
    }
    // The frame, else the box round the drawing.
    let frame = l.get("frame").and_then(|f| Some([num(f, "xmin")?, num(f, "ymin")?, num(f, "xmax")?, num(f, "ymax")?]));
    let [x0, y0, x1, y1] = frame.filter(|[x0, y0, x1, y1]| x1 > x0 || y1 > y0).or_else(|| {
        raw.iter().flat_map(|r| r.points.iter()).fold(None, |b: Option<[f64; 4]>, &[x, y]| {
            Some(b.map_or([x, y, x, y], |[a, b, c, d]| [a.min(x), b.min(y), c.max(x), d.max(y)]))
        })
    })?;
    let unit = if y1 > y0 { y1 - y0 } else { x1 - x0 };
    if !(unit.is_finite() && unit > 0.0) {
        return None;
    }
    let (cx, cy) = (x0 / 2.0 + x1 / 2.0, y0 / 2.0 + y1 / 2.0);
    let (sin, cos) = num(l, "rotation").unwrap_or(0.0).to_radians().sin_cos();
    let along = |k: &str| if size_pt > 0.0 { num(l, k).unwrap_or(0.0) / size_pt } else { 0.0 };
    let (dx, dy) = (along("offsetX"), along("offsetY"));
    let size96 = pt96(size_pt);
    let parts = raw
        .into_iter()
        .map(|r| {
            let points = r
                .points
                .iter()
                .map(|&[x, y]| {
                    let (ux, uy) = ((x - cx) / unit, (y - cy) / unit);
                    let fit = |v: f64| if v.is_finite() { v.clamp(-PATH_REACH, PATH_REACH) as f32 } else { 0.0 };
                    [fit(ux * cos - uy * sin + dx), fit(ux * sin + uy * cos + dy)]
                })
                .collect();
            let (stroke, width) = match r.stroke {
                Some((c, w)) if size96 > 0.0 => (Some(c), (w / size96).clamp(0.0, 0.5)),
                Some((c, _)) => (Some(c), 0.0),
                None => (None, 0.0),
            };
            PathPart { points, closed: r.closed, fill: r.fill, stroke, width }
        })
        .collect();
    Some(PathShape { parts })
}

/// A point `[x, y, …]` whose coordinates are finite numbers.
fn xy(p: &Value) -> Option<[f64; 2]> {
    let p = p.as_array()?;
    let [x, y] = [p.first()?.as_f64()?, p.get(1)?.as_f64()?];
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

/// A `curveRings` ring's points: a ring of arcs round one centre (as Pro writes a circle) as a
/// circle of [`CIRCLE_POINTS`] points; any other as its points with the curves' control points,
/// and `true` when a curve was drawn that way.
fn curve_ring(ring: &Value) -> (Vec<[f64; 2]>, bool) {
    let mut points: Vec<[f64; 2]> = Vec::new();
    let mut centres = Vec::new();
    // Whether anything but arcs follows the start, and whether any curve is read.
    let (mut lines, mut curved) = (false, false);
    for s in ring.as_array().into_iter().flatten().take(MAX_PATH_POINTS) {
        let curve = |k: &str| s.get(k).and_then(Value::as_array);
        if let Some(p) = xy(s) {
            lines |= !points.is_empty();
            points.push(p);
        } else if let Some(a) = curve("a") {
            // An elliptic arc: its end, then its centre.
            curved = true;
            match (a.first().and_then(xy), a.get(1).and_then(xy)) {
                (Some(end), Some(centre)) => {
                    centres.push(centre);
                    points.push(end);
                }
                (end, _) => {
                    lines = true;
                    points.extend(end);
                }
            }
        } else if let Some(b) = curve("b") {
            // A Bézier curve: its end, then its two control points.
            (curved, lines) = (true, true);
            points.extend(b.get(1..).into_iter().flatten().filter_map(xy).chain(b.first().and_then(xy)));
        } else if let Some(c) = curve("c") {
            // A circular arc: its end, then a point it passes through.
            (curved, lines) = (true, true);
            points.extend(c.get(1).and_then(xy).into_iter().chain(c.first().and_then(xy)));
        }
    }
    if let (false, Some(&[sx, sy]), Some(&[cx, cy])) = (lines, points.first(), centres.first()) {
        let r = (sx - cx).hypot(sy - cy);
        let near = |[x, y]: [f64; 2], to: f64| ((x - cx).hypot(y - cy) - to).abs() <= r * CIRCLE_TOLERANCE;
        if r.is_finite() && r > 0.0 && centres.iter().all(|c| near(*c, 0.0)) && points.iter().all(|p| near(*p, r)) {
            let step = std::f64::consts::TAU / CIRCLE_POINTS as f64;
            return ((0..CIRCLE_POINTS).map(|i| [cx + r * (i as f64 * step).cos(), cy + r * (i as f64 * step).sin()]).collect(), false);
        }
    }
    (points, curved)
}

/// Several marker layers (bottom first, each with its drawing) as one marker: each layer's paths
/// scaled by its size over the largest, the marker the largest size and painted as the top layer
/// (for the legend). One layer is itself. When a layer has no drawing (a picture, a character, or
/// a vector marker with nothing to draw) or the paths can't be kept, the first marker, noted.
pub(crate) fn stacked(symbols: &[(Symbol, Option<PathShape>)], what: &str, report: &mut Report) -> Symbol {
    let none = Paint { fill: None, stroke: None, strokes: Vec::new(), fills: Vec::new() };
    let first = symbols.first().map_or_else(|| marker_symbol(Marker::Circle, 0.0, &none), |(s, _)| *s);
    let (Some((top, _)), true) = (symbols.last(), symbols.len() > 1) else { return first };
    if symbols.iter().any(|(_, p)| p.is_none()) {
        let why = "a picture, a character or a marker with nothing to draw among them";
        report.approximated(what, format!("point symbols of several marker layers with {why}, drawn as the first marker"));
        return first;
    }
    let largest = symbols.iter().map(|(s, _)| s.size).fold(0.0, f32::max);
    let parts: Vec<PathPart> = symbols
        .iter()
        .filter_map(|(s, p)| Some((if largest > 0.0 { s.size / largest } else { 1.0 }, p.as_ref()?)))
        .flat_map(|(k, p)| {
            p.parts.iter().map(move |part| PathPart {
                points: part.points.iter().map(|[x, y]| [x * k, y * k]).collect(),
                width: part.width * k,
                ..part.clone()
            })
        })
        .collect();
    if parts.len() > MAX_PATH_PARTS {
        report.approximated(what, format!("stacked markers of more than {MAX_PATH_PARTS} parts, the parts past them left out"));
    }
    match PathRef::new(PathShape { parts }) {
        Some(p) => Symbol { size: largest, marker: Marker::Path(p), ..*top },
        None => {
            report.approximated(what, "point symbols of several marker layers, drawn as the first marker");
            first
        }
    }
}

/// A line symbol's marker layer as a stroke of markers along the line, in the marker's fill with
/// its outline. `CIMMarkerPlacementAlongLineSameSize`/`AlongLineVariableSize` place them at every
/// interval (`placementTemplate`'s first step, points); `AtExtremities` at the first and last
/// vertices (`extremityPlacement`); `AtRatioPositions` halfway along (`[0.5]`; any other ratios at
/// every interval, noted); `AtMeasuredUnits` at every interval (noted); `OnVertices` on the
/// vertices. `angleToLine` turns them with the line, and `offset` (points) moves them sideways.
/// `None` for a placement RustGIS has no match for (noted) or a marker it can't read.
fn line_markers(l: &Value, what: &str, report: &mut Report) -> Option<LineStroke> {
    let p = l.get("markerPlacement").unwrap_or(&NULL);
    let mut interval = None;
    let at = match str_of(p, "type") {
        Some(ty @ ("CIMMarkerPlacementAlongLineSameSize" | "CIMMarkerPlacementAlongLineVariableSize")) => {
            let template = p.get("placementTemplate").and_then(Value::as_array);
            interval = template.and_then(|t| t.first()).and_then(Value::as_f64).filter(|v| *v > 0.0).map(pt96);
            if template.is_some_and(|t| t.len() > 1) {
                report.approximated(what, "markers along lines spaced unevenly, spaced evenly");
            }
            if ty == "CIMMarkerPlacementAlongLineVariableSize" {
                report.approximated(what, "markers along lines that vary in size, drawn at one size");
            }
            Placements::INTERVAL
        }
        Some("CIMMarkerPlacementAtExtremities") => match str_of(p, "extremityPlacement") {
            Some("JustBegin") => Placements::FIRST,
            Some("JustEnd") => Placements::LAST,
            Some("None") => return None,
            _ => Placements::FIRST.with(Placements::LAST),
        },
        Some("CIMMarkerPlacementAtRatioPositions") => {
            let ratios = p.get("positionArray").and_then(Value::as_array).map(Vec::as_slice);
            if matches!(ratios, Some([r]) if r.as_f64() == Some(0.5)) {
                Placements::MIDDLE
            } else {
                report.approximated(what, "markers at ratio positions along lines, drawn at intervals");
                Placements::INTERVAL
            }
        }
        Some("CIMMarkerPlacementAtMeasuredUnits") => {
            report.approximated(what, "markers at measured units along lines, drawn at intervals");
            Placements::INTERVAL
        }
        Some("CIMMarkerPlacementOnVertices") => Placements::VERTICES,
        Some(other) => {
            report.skipped(what, format!("markers along lines placed by {}, left out", shown(other)));
            return None;
        }
        None => {
            report.skipped(what, "markers along lines with no placement, left out");
            return None;
        }
    };
    let mut pending = Vec::new();
    let (symbol, _) = marker_layer(l, 0, &mut pending, what, report)?;
    if !pending.is_empty() {
        report.approximated(what, "GIF or BMP markers along lines, drawn as circles");
    }
    let markers = LineMarkers {
        mark: LineMark::Marker(symbol.marker),
        size: symbol.size,
        at,
        interval: interval.unwrap_or(LineMarkers::default().interval),
        rotate: flag(p, "angleToLine", false),
        outline: Some(symbol.stroke).filter(|c| c.a() > 0 && symbol.stroke_width > 0.0),
        ..LineMarkers::default()
    };
    let offset = signed_pt96(num(p, "offset").unwrap_or(0.0));
    Some(
        LineStroke { color: Some(symbol.fill), width: Some(symbol.stroke_width), offset, markers: Some(markers), ..LineStroke::default() }
            .clamped(),
    )
}

/// A polygon symbol's marker layer placed by `CIMMarkerPlacementInsidePolygon` as a fill of
/// markers on a grid `stepX` by `stepY` points (every other row shifted half a step across with
/// `shiftOddRows`), shifted `offsetX`/`offsetY` points; a turned grid (`gridAngle`) or one placed
/// at random is drawn square and fixed, noted. `None` for a marker it can't read.
fn inside_polygon(l: &Value, what: &str, report: &mut Report) -> Option<FillLayer> {
    let p = l.get("markerPlacement").unwrap_or(&NULL);
    let mut pending = Vec::new();
    let (symbol, _) = marker_layer(l, 0, &mut pending, what, report)?;
    if !pending.is_empty() {
        report.approximated(what, "GIF or BMP markers in polygons, drawn as circles");
    }
    if num(p, "gridAngle").is_some_and(|a| a.rem_euclid(360.0) != 0.0) {
        report.approximated(what, "turned grids of markers in polygons, drawn square");
    }
    if str_of(p, "gridType").is_some_and(|t| t != "Fixed") {
        report.approximated(what, "markers scattered in polygons, drawn on a grid");
    }
    let step = |k: &str| pt96(num(p, k).filter(|v| *v > 0.0).unwrap_or(DEFAULT_STEP_PT));
    let [step_x, step_y] = [step("stepX"), step("stepY")];
    let shift = if flag(p, "shiftOddRows", false) { step_x / 2.0 } else { 0.0 };
    let pattern = FillPattern::Dots {
        marker: symbol.marker,
        size: symbol.size,
        outline: Some(symbol.stroke).filter(|c| c.a() > 0 && symbol.stroke_width > 0.0),
        outline_width: symbol.stroke_width,
        spacing: [step_x, step_y],
        displacement: [shift, 0.0],
        // (Pro's y runs up, the grid's down.)
        offset: [signed_pt96(num(p, "offsetX").unwrap_or(0.0)), -signed_pt96(num(p, "offsetY").unwrap_or(0.0))],
    };
    Some(FillLayer { color: Some(symbol.fill), pattern }.clamped())
}

/// The shape a `CIMVectorMarker`'s drawing stands for: a single graphic whose geometry is a
/// circle (Pro writes circles as `curveRings`), a closed square (axis-aligned) or diamond (a
/// square turned 45 degrees), a triangle or a five-pointed star. `None` for any other drawing.
pub(crate) fn vector_shape(marker: &Value) -> Option<Marker> {
    let graphics = marker.get("markerGraphics")?.as_array()?;
    let [graphic] = graphics.as_slice() else { return None };
    let geometry = graphic.get("geometry")?;
    if geometry.get("curveRings").is_some_and(Value::is_array) {
        return Some(Marker::Circle);
    }
    let [ring] = geometry.get("rings")?.as_array()?.as_slice() else { return None };
    let ring = ring.as_array()?;
    match ring.len() {
        TRIANGLE_POINTS => Some(Marker::Triangle),
        STAR_POINTS => Some(Marker::Star),
        SQUARE_POINTS => {
            let pts: Vec<(f64, f64)> = ring
                .iter()
                .map(|p| {
                    let p = p.as_array()?;
                    Some((p.first()?.as_f64().filter(|v| v.is_finite())?, p.get(1)?.as_f64().filter(|v| v.is_finite())?))
                })
                .collect::<Option<_>>()?;
            square_or_diamond(&pts)
        }
        _ => None,
    }
}

/// A closed ring of five points as a square (every side along an axis) or a diamond (a point
/// at the middle of each side of the box round it).
fn square_or_diamond(pts: &[(f64, f64)]) -> Option<Marker> {
    let fold = |f: fn(f64, f64) -> f64, i: usize| pts.iter().map(|p| if i == 0 { p.0 } else { p.1 }).reduce(f);
    let (x0, x1, y0, y1) = (fold(f64::min, 0)?, fold(f64::max, 0)?, fold(f64::min, 1)?, fold(f64::max, 1)?);
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (tx, ty) = (w * SHAPE_TOLERANCE, h * SHAPE_TOLERANCE);
    let axis_aligned = pts.windows(2).all(|s| match s {
        [a, b] => (a.0 - b.0).abs() <= tx || (a.1 - b.1).abs() <= ty,
        _ => false,
    });
    if axis_aligned {
        return Some(Marker::Square);
    }
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let touches = |on_x: bool, v: f64| {
        pts.iter()
            .any(|p| if on_x { (p.0 - v).abs() <= tx && (p.1 - cy).abs() <= ty } else { (p.1 - v).abs() <= ty && (p.0 - cx).abs() <= tx })
    };
    (touches(false, y0) && touches(false, y1) && touches(true, x0) && touches(true, x1)).then_some(Marker::Diamond)
}

/// A `CIMPictureMarker`: a PNG or JPEG data URL is kept; a GIF or BMP is a circle for now and
/// goes into `pending` (with its place in the symbols) to be converted; any other picture is a
/// noted circle, and one that isn't embedded is left out.
fn picture_marker(
    l: &Value,
    size: f32,
    index: usize,
    pending: &mut Vec<(usize, String)>,
    what: &str,
    report: &mut Report,
) -> Option<Symbol> {
    let url = str_of(l, "url").unwrap_or_default();
    let Some((media, payload)) = data_url(url) else {
        report.skipped(what, "picture markers whose picture isn't embedded, left out");
        return None;
    };
    let none = Paint { fill: None, stroke: None, strokes: Vec::new(), fills: Vec::new() };
    let kept = match media.as_str() {
        "image/png" | "image/jpeg" | "image/jpg" => ImageRef::new(&payload).map(Marker::Image),
        "image/gif" | "image/bmp" if payload.len() <= ImageRef::MAX_BYTES => {
            pending.push((index, format!("data:{media};base64,{payload}")));
            return Some(marker_symbol(Marker::Circle, size, &none));
        }
        _ => None,
    };
    let marker = kept.unwrap_or_else(|| {
        report.approximated(what, "picture markers RustGIS can't keep, shown as circles");
        Marker::Circle
    });
    Some(marker_symbol(marker, size, &none))
}

/// The media type (lower case) and base64 text (see [`base64_text`]) of a
/// `data:<type>;base64,<text>` URL.
pub(crate) fn data_url(url: &str) -> Option<(String, String)> {
    let (head, payload) = url.get(..5).filter(|p| p.eq_ignore_ascii_case("data:")).and_then(|_| url.get(5..))?.split_once(',')?;
    let mut parts = head.split(';');
    let media = parts.next()?.trim().to_ascii_lowercase();
    parts.any(|p| p.trim().eq_ignore_ascii_case("base64")).then(|| (media, base64_text(payload)))
}

/// Base64 text without the whitespace it may carry: Pro wraps the base64 it writes with
/// newlines, which decoders reject.
pub fn base64_text(payload: &str) -> String {
    payload.chars().filter(|c| !c.is_ascii_whitespace()).collect()
}

/// A `CIMCharacterMarker`: a character of a font, in the colours of its symbol (black without one).
fn character_marker(l: &Value, size: f32, what: &str, report: &mut Report) -> Option<Symbol> {
    let ch = l
        .get("characterIndex")
        .and_then(Value::as_u64)
        .and_then(|i| u32::try_from(i).ok())
        .and_then(char::from_u32)
        .filter(|c| !c.is_control());
    let Some(ch) = ch else {
        report.skipped(what, "character markers with no character to draw, left out");
        return None;
    };
    let paint = match l.get("symbol") {
        Some(s) => graphic_paint(s, what, report),
        None => Paint { fill: Some(Rgba::rgb(0, 0, 0)), stroke: None, strokes: Vec::new(), fills: Vec::new() },
    };
    let glyph = Glyph::new(ch, str_of(l, "fontFamilyName").unwrap_or_default());
    Some(marker_symbol(Marker::Char(glyph), size, &paint))
}

// ---- text symbols -----------------------------------------------------------------------------

/// A `CIMTextSymbol`'s style. `size` is in points, `letter_spacing` in points added after each
/// letter, and `line_spacing` a multiple of the single line spacing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Text {
    pub font: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub size: f64,
    pub color: Rgba,
    pub align: TextAlign,
    pub valign: VAlign,
    pub case: TextCase,
    pub letter_spacing: f64,
    pub line_spacing: f64,
}

/// A text symbol (or the `CIMSymbolReference` holding one) as a style. A text halo is left out
/// and noted.
pub(crate) fn text(sym: &Value, what: &str, report: &mut Report) -> Text {
    let t = if str_of(sym, "type") == Some("CIMSymbolReference") { sym.get("symbol").unwrap_or(&NULL) } else { sym };
    let style = str_of(t, "fontStyleName").unwrap_or_default().to_ascii_lowercase();
    let size = text_pt(num(t, "height").unwrap_or(DEFAULT_TEXT_PT));
    let color = t.get("symbol").and_then(|s| polygon(s, what, report).fill).unwrap_or(Rgba::rgb(0, 0, 0));
    let align = match str_of(t, "horizontalAlignment") {
        Some("Center") => TextAlign::Center,
        Some("Right") => TextAlign::Right,
        Some("Justify") => {
            report.approximated(what, "justified text, aligned left");
            TextAlign::Left
        }
        _ => TextAlign::Left,
    };
    let valign = match str_of(t, "verticalAlignment") {
        Some("Center") => VAlign::Middle,
        Some("Bottom" | "Baseline") => VAlign::Bottom,
        _ => VAlign::Top,
    };
    let mut case = match str_of(t, "textCase") {
        Some("Allcaps") => TextCase::Upper,
        Some("Lowercase") => TextCase::Lower,
        _ => TextCase::None,
    };
    if flag(t, "smallCaps", false) {
        report.approximated(what, "small caps, shown as upper case");
        case = TextCase::Upper;
    }
    // A halo size alone is no halo: it needs the symbol that draws it.
    if t.get("haloSymbol").is_some_and(Value::is_object) && num(t, "haloSize").is_none_or(|h| h > 0.0) {
        report.skipped(what, "a text halo, left out");
    }
    Text {
        font: str_of(t, "fontFamilyName").and_then(font_name),
        bold: style.contains("bold"),
        italic: style.contains("italic") || style.contains("oblique"),
        size,
        color,
        align,
        valign,
        case,
        letter_spacing: size * num(t, "letterSpacing").unwrap_or(0.0) / 100.0,
        line_spacing: num(t, "lineSpacing").filter(|s| *s > 0.0).unwrap_or(1.0),
    }
}
