//! ArcGIS Pro's symbols (stacks of symbol layers, bottom to top) as RustGIS's [`Symbol`],
//! [`LineStroke`], [`FillLayer`] and [`Marker`], and a text symbol as a [`Text`] style. A symbol
//! layer RustGIS has no match for is drawn as near as it can (a drawn marker shape as a circle,
//! a gradient by its two ends) or left out, and noted in the [`Report`] under the name of the
//! element it belongs to.
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
use crate::symbology::{Dash, Glyph, ImageRef, LineCap, LineJoin, LineStroke, MAX_DASH_STEPS, MAX_STROKES, Marker, Symbol, font_name};

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
fn shown(s: &str) -> String {
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

/// A polygon symbol: its solid, hatch and gradient fills and its strokes.
pub(crate) fn polygon(sym: &Value, what: &str, report: &mut Report) -> Paint {
    read(sym, true, what, report)
}

/// A line symbol: its strokes (its other layers are left out and noted).
pub(crate) fn line(sym: &Value, what: &str, report: &mut Report) -> Paint {
    read(sym, false, what, report)
}

fn read(sym: &Value, with_fills: bool, what: &str, report: &mut Report) -> Paint {
    let mut fill = None;
    let mut strokes = Vec::new();
    let mut fills = Vec::new();
    for l in layers(sym) {
        match str_of(l, "type") {
            Some("CIMSolidStroke") => strokes.extend(stroke(l, what, report)),
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
    let stroke = strokes.last().and_then(|s| Some((s.color?, s.width?)));
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
/// `pending_pictures` are the pictures RustGIS can't keep as they are (GIF and BMP, which the
/// engine converts to PNG): each is its index in `symbols` (drawn as a circle meanwhile) and its
/// `data:` URL.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PointSymbol {
    pub symbols: Vec<Symbol>,
    pub pending_pictures: Vec<(usize, String)>,
}

/// A point symbol's markers: vector (a shape), picture and character markers.
pub(crate) fn point(sym: &Value, what: &str, report: &mut Report) -> PointSymbol {
    let mut out = PointSymbol { symbols: Vec::new(), pending_pictures: Vec::new() };
    for l in layers(sym) {
        if num(l, "rotation").is_some_and(|r| r != 0.0) {
            report.approximated(what, "marker rotation, left out");
        }
        let size = pt96(num(l, "size").unwrap_or(DEFAULT_MARKER_PT));
        let made = match str_of(l, "type") {
            Some("CIMVectorMarker") => Some(vector_marker(l, size, what, report)),
            Some("CIMPictureMarker") => picture_marker(l, size, out.symbols.len(), &mut out.pending_pictures, what, report),
            Some("CIMCharacterMarker") => character_marker(l, size, what, report),
            Some(other) => {
                report.skipped(what, format!("{} symbol layers, left out", shown(other)));
                None
            }
            None => None,
        };
        out.symbols.extend(made);
    }
    out
}

/// A marker symbol of `marker` painted as `paint` says.
fn marker_symbol(marker: Marker, size: f32, paint: &Paint) -> Symbol {
    let (stroke, stroke_width) = paint.stroke.unwrap_or((CLEAR, 0.0));
    Symbol { fill: paint.fill.unwrap_or(CLEAR), stroke, stroke_width, size, marker }
}

/// A `CIMVectorMarker`: its shape if it is one RustGIS has, painted as its graphic's symbol.
fn vector_marker(l: &Value, size: f32, what: &str, report: &mut Report) -> Symbol {
    let graphic = l.get("markerGraphics").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS).find_map(|g| g.get("symbol"));
    let paint =
        graphic.map(|s| polygon(s, what, report)).unwrap_or(Paint { fill: None, stroke: None, strokes: Vec::new(), fills: Vec::new() });
    let shape = vector_shape(l).unwrap_or_else(|| {
        report.approximated(what, "a drawn marker shape, shown as a circle");
        Marker::Circle
    });
    marker_symbol(shape, size, &paint)
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
        "image/png" | "image/jpeg" | "image/jpg" => ImageRef::new(payload).map(Marker::Image),
        "image/gif" | "image/bmp" if payload.len() <= ImageRef::MAX_BYTES => {
            pending.push((index, url.to_string()));
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

/// The media type (lower case) and base64 text of a `data:<type>;base64,<text>` URL.
fn data_url(url: &str) -> Option<(String, &str)> {
    let (head, payload) = url.get(..5).filter(|p| p.eq_ignore_ascii_case("data:")).and_then(|_| url.get(5..))?.split_once(',')?;
    let mut parts = head.split(';');
    let media = parts.next()?.trim().to_ascii_lowercase();
    parts.any(|p| p.trim().eq_ignore_ascii_case("base64")).then_some((media, payload))
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
        Some(s) => polygon(s, what, report),
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
