//! An ArcGIS Pro layout (`CIMLayoutDocument`, a `.pagx`) as a RustGIS [`Layout`]: the page and its
//! guides, then the elements in Pro's drawing order (the first listed is drawn first, under the
//! rest), each a box on the page and a kind. What doesn't come across, or comes across changed,
//! is said in the [`Report`] under the element's name.
//!
//! CIM measures the page in its own unit (inches, millimetres…) from the bottom-left corner;
//! RustGIS measures it in millimetres from the top-left. Geometry is worked out in page units as a
//! [`PageBox`] (rotation about the element's anchor included) and turned into millimetres last,
//! by [`to_mm`].
//!
//! The walk is bounded: at most [`MAX_ELEMENTS`] elements, groups [`MAX_GROUP_DEPTH`] deep (deeper
//! ones are flattened into the deepest kept), and nothing past [`MAX_WALK_DEPTH`] groups deep.

use serde_json::Value;

use super::color::symbol_of;
use super::dyntext;
use super::frame::{self, Frames};
use super::surround;
use super::symbol::{self, data_url};
use super::units::page_unit_mm;
use super::{Document, Report, flag, num, str_of};
use crate::Rgba;
use crate::layout::{ElementKind, FontFamily, FrameStyle, Layout, LineHead, MAX_ELEMENTS, MAX_GROUP_DEPTH, line_at_angle};

/// Millimetres in an inch, and in a point (1/72 inch).
const INCH_MM: f64 = 25.4;
const PT_MM: f64 = 25.4 / 72.0;
/// How many groups deep the walk goes at all; what is deeper is left out. (The JSON parser's own
/// nesting limit stops a document at about 40.)
const MAX_WALK_DEPTH: usize = 32;
/// The most points read from one shape.
const MAX_POINTS: usize = 100_000;
/// How far from a whole quarter turn (degrees) a rotation may be and still leave a box upright.
const RIGHT_ANGLE_TOLERANCE: f64 = 1e-6;
/// A point-text width estimate: this much of the text size per character.
const CHAR_WIDTH: f64 = 0.55;
/// A text line's height as a multiple of the text size.
const LINE_HEIGHT: f64 = 1.25;
/// Pro's anchor when an element names none.
const DEFAULT_ANCHOR: &str = "BottomLeftCorner";

const NULL: Value = Value::Null;

/// A layout read from a CIM document, with what the caller still has to resolve.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutImport {
    /// Elements in Pro's draw order (first drawn first), groups included.
    pub layout: Layout,
    /// One per map frame element.
    pub frames: Vec<FrameBinding>,
    /// Picture elements whose bytes aren't in the document.
    pub pictures: Vec<(u32, PictureSource)>,
    /// Map URIs the frames refer to, in order of first use.
    pub embedded_maps: Vec<String>,
    /// Text elements and the name of the frame their tokens read from.
    pub text_frames: Vec<(u32, String)>,
    /// Marker pictures of a north arrow's symbol that RustGIS can't keep as they are (GIF, BMP):
    /// the element, the marker's index in its `symbol`, and the picture's `data:` URL, for the
    /// engine to convert to PNG. Empty for every other element.
    pub pending_pictures: Vec<(u32, usize, String)>,
}

/// A map frame element and the map it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameBinding {
    pub element: u32,
    pub name: String,
    pub map_uri: Option<String>,
    /// The saved camera: x, y (in the map's spatial reference) and scale.
    pub saved_camera: Option<(f64, f64, f64)>,
}

/// Where a picture's bytes are to be found when the document doesn't hold them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PictureSource {
    /// A file path, as Pro saved it.
    Path(String),
}

/// A box on the page in page units, measured from the bottom-left corner as CIM does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageBox {
    pub xmin: f64,
    pub ymin: f64,
    pub xmax: f64,
    pub ymax: f64,
}

impl PageBox {
    /// The box round `points` (none when there are none).
    fn around(points: impl IntoIterator<Item = (f64, f64)>) -> Option<PageBox> {
        points.into_iter().fold(None, |b: Option<PageBox>, (x, y)| {
            Some(match b {
                None => PageBox { xmin: x, ymin: y, xmax: x, ymax: y },
                Some(b) => PageBox { xmin: b.xmin.min(x), ymin: b.ymin.min(y), xmax: b.xmax.max(x), ymax: b.ymax.max(y) },
            })
        })
    }

    fn corners(self) -> [(f64, f64); 4] {
        [(self.xmin, self.ymin), (self.xmin, self.ymax), (self.xmax, self.ymax), (self.xmax, self.ymin)]
    }
}

/// Where across and up the box an anchor is, as fractions of its width and height. A name Pro
/// doesn't write is its default, the bottom-left corner.
fn anchor_fraction(anchor: &str) -> (f64, f64) {
    match anchor {
        "TopLeftCorner" => (0.0, 1.0),
        "TopMidPoint" => (0.5, 1.0),
        "TopRightCorner" => (1.0, 1.0),
        "LeftMidPoint" => (0.0, 0.5),
        "CenterPoint" => (0.5, 0.5),
        "RightMidPoint" => (1.0, 0.5),
        "BottomMidPoint" => (0.5, 0.0),
        "BottomRightCorner" => (1.0, 0.0),
        _ => (0.0, 0.0),
    }
}

/// The point of box `b` an anchor (`TopLeftCorner`, `CenterPoint`…) names.
pub(crate) fn anchor_point(b: PageBox, anchor: &str) -> (f64, f64) {
    let (fx, fy) = anchor_fraction(anchor);
    (b.xmin + fx * (b.xmax - b.xmin), b.ymin + fy * (b.ymax - b.ymin))
}

/// Point `p` turned `degrees` anticlockwise about `about`.
fn rotate(p: (f64, f64), about: (f64, f64), degrees: f64) -> (f64, f64) {
    let (s, c) = degrees.to_radians().sin_cos();
    let (dx, dy) = (p.0 - about.0, p.1 - about.1);
    (about.0 + dx * c - dy * s, about.1 + dx * s + dy * c)
}

/// The box that box `b`, turned `degrees` anticlockwise about `about`, covers.
pub(crate) fn rotated(b: PageBox, about: (f64, f64), degrees: f64) -> PageBox {
    PageBox::around(b.corners().map(|p| rotate(p, about, degrees))).unwrap_or(b)
}

/// Whether a rotation leaves a box upright (a whole number of quarter turns).
pub(crate) fn is_right_angle(degrees: f64) -> bool {
    let off = degrees.rem_euclid(90.0);
    off.min(90.0 - off) <= RIGHT_ANGLE_TOLERANCE
}

/// Box `b` as RustGIS has it: (x, y, width, height) in millimetres from the top-left of a page
/// `page_h` units tall, a unit being `unit_mm` millimetres.
pub(crate) fn to_mm(b: PageBox, page_h: f64, unit_mm: f64) -> (f64, f64, f64, f64) {
    (b.xmin * unit_mm, (page_h - b.ymax) * unit_mm, (b.xmax - b.xmin) * unit_mm, (b.ymax - b.ymin) * unit_mm)
}

/// Box `b` shrunk by `margin` on every side, but never to less than `min` across or up.
fn inset(b: PageBox, margin: f64, min: f64) -> PageBox {
    let m = |lo: f64, hi: f64| if margin.is_finite() { margin.min(((hi - lo) - min).max(0.0) / 2.0) } else { 0.0 };
    let (mx, my) = (m(b.xmin, b.xmax), m(b.ymin, b.ymax));
    PageBox { xmin: b.xmin + mx, ymin: b.ymin + my, xmax: b.xmax - mx, ymax: b.ymax - my }
}

/// A page point in millimetres from the top-left.
fn point_mm(p: (f64, f64), page_h: f64, unit_mm: f64) -> (f64, f64) {
    (p.0 * unit_mm, (page_h - p.1) * unit_mm)
}

/// The box point text of `lines` at `size_pt` points takes, placed so its `anchor` is at `point`
/// (page units of `unit_mm` millimetres). Its width is estimated, [`CHAR_WIDTH`] of the size per
/// character of the longest line, and its height is [`LINE_HEIGHT`] of the size per line: the
/// renderer draws the text at its real width inside, aligned, so an estimate a little out moves
/// the box's far edge, not the ink.
pub(crate) fn text_box(point: (f64, f64), anchor: &str, size_pt: f64, lines: &[&str], unit_mm: f64) -> PageBox {
    let unit = if unit_mm.is_finite() && unit_mm > 0.0 { unit_mm } else { INCH_MM };
    let size = if size_pt.is_finite() { size_pt.max(0.0) } else { 0.0 };
    let chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let w = CHAR_WIDTH * size * chars as f64 * PT_MM / unit;
    let h = lines.len().max(1) as f64 * size * LINE_HEIGHT * PT_MM / unit;
    let (fx, fy) = anchor_fraction(anchor);
    let (xmin, ymin) = (point.0 - fx * w, point.1 - fy * h);
    PageBox { xmin, ymin, xmax: xmin + w, ymax: ymin + h }
}

/// A finite `[x, y, …]` point.
fn xy(p: &Value) -> Option<(f64, f64)> {
    let p = p.as_array()?;
    let at = |i: usize| p.get(i).and_then(Value::as_f64).filter(|v| v.is_finite());
    Some((at(0)?, at(1)?))
}

/// The points of a ring or path, in order, with the end of each curve segment among them.
pub(crate) fn part_points(part: &Value, budget: &mut usize) -> Vec<(f64, f64)> {
    part.as_array()
        .into_iter()
        .flatten()
        .take(*budget)
        .filter_map(|seg| xy(seg).or_else(|| curve_end(seg)))
        .inspect(|_| *budget -= 1)
        .collect()
}

/// A curve segment's end point (`{"a": [end, …]}`, `{"b": …}`, `{"c": …}`).
fn curve_end(seg: &Value) -> Option<(f64, f64)> {
    ["a", "b", "c"].iter().find_map(|k| seg.get(*k)?.as_array()?.first().and_then(xy))
}

/// The box round a shape: an envelope (`xmin`…), a point (`x`, `y`), or `rings`, `paths`,
/// `curveRings` and `curvePaths`. A curve segment counts its control points, and an arc the
/// whole circle or ellipse it is part of, so the box holds the curve (if a little generously
/// for part of an arc).
pub(crate) fn geometry_box(v: &Value) -> Option<PageBox> {
    if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (num(v, "xmin"), num(v, "ymin"), num(v, "xmax"), num(v, "ymax")) {
        return PageBox::around([(x0, y0), (x1, y1)]);
    }
    if let (Some(x), Some(y)) = (num(v, "x"), num(v, "y")) {
        return PageBox::around([(x, y)]);
    }
    let mut points = Vec::new();
    let mut budget = MAX_POINTS;
    for key in ["rings", "paths", "curveRings", "curvePaths"] {
        for part in v.get(key).and_then(Value::as_array).into_iter().flatten() {
            let mut prev = None;
            for seg in part.as_array().into_iter().flatten() {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                if let Some(p) = xy(seg) {
                    points.push(p);
                    prev = Some(p);
                } else if let Some(end) = curve_end(seg) {
                    curve_points(seg, prev.unwrap_or(end), &mut points);
                    points.push(end);
                    prev = Some(end);
                }
            }
        }
    }
    PageBox::around(points)
}

/// The points that bound a curve segment starting at `start`: a Bézier's control points, a
/// three-point arc's middle point, and the box of the circle or ellipse an arc is part of.
fn curve_points(seg: &Value, start: (f64, f64), out: &mut Vec<(f64, f64)>) {
    if let Some(b) = seg.get("b").and_then(Value::as_array) {
        out.extend(b.iter().skip(1).take(2).filter_map(xy));
    } else if let Some(c) = seg.get("c").and_then(Value::as_array) {
        out.extend(c.get(1).and_then(xy));
    } else if let Some(a) = seg.get("a").and_then(Value::as_array)
        && let Some((cx, cy)) = a.get(1).and_then(xy)
    {
        let at = |i: usize| a.get(i).and_then(Value::as_f64).filter(|v| v.is_finite());
        // `[end, centre, minor, clockwise, rotation, semi-major axis, ratio]` for an ellipse;
        // a circle stops after `clockwise`, its radius the distance from the centre.
        let (hx, hy) = match (at(4), at(5), at(6)) {
            (Some(rot), Some(major), Some(ratio)) => {
                let (major, minor) = (major.abs(), (major * ratio).abs());
                let (s, c) = rot.sin_cos();
                ((major * c).hypot(minor * s), (major * s).hypot(minor * c))
            }
            _ => {
                let r = (start.0 - cx).hypot(start.1 - cy);
                (r, r)
            }
        };
        out.extend([(cx - hx, cy - hy), (cx + hx, cy + hy)]);
    }
}

/// Whether a shape is one ring of four corners along the axes: a rectangle as Pro writes one.
pub(crate) fn is_four_corner_ring(v: &Value) -> bool {
    let Some([ring]) = v.get("rings").and_then(Value::as_array).map(Vec::as_slice) else { return false };
    let mut budget = 6;
    let pts = part_points(ring, &mut budget);
    let pts = match pts.as_slice() {
        [a, .., z] if pts.len() == 5 && a == z => pts.get(..4).unwrap_or_default(),
        _ if pts.len() == 4 => pts.as_slice(),
        _ => return false,
    };
    let Some(b) = PageBox::around(pts.iter().copied()) else { return false };
    let tol = 1e-9 * (b.xmax - b.xmin).abs().max((b.ymax - b.ymin).abs()).max(1.0);
    let same = |p: &(f64, f64), q: &(f64, f64)| (p.0 - q.0).abs() <= tol && (p.1 - q.1).abs() <= tol;
    b.corners().iter().all(|c| pts.iter().any(|p| same(p, c))) && pts.iter().all(|p| b.corners().iter().any(|c| same(p, c)))
}

/// The element's own frame style (`graphicFrame`), or for a graphic its graphic's (`frame`).
fn graphic_frame(e: &Value) -> Option<&Value> {
    e.get("graphicFrame").or_else(|| e.get("graphic").and_then(|g| g.get("frame")).filter(|f| str_of(f, "type") == Some("CIMGraphicFrame")))
}

/// A `CIMGraphicFrame` as a [`FrameStyle`]: the border the line symbol's top stroke draws, the
/// background the polygon symbol's top fill, and the gap; `None` when it draws neither.
pub(crate) fn frame_style(frame: &Value, what: &str, report: &mut Report) -> Option<FrameStyle> {
    let border = frame.get("borderSymbol").and_then(|s| symbol::line(s, what, report).stroke);
    let background = frame.get("backgroundSymbol").and_then(|s| symbol::polygon(s, what, report).fill);
    let shadow = frame.get("shadowSymbol").and_then(symbol_of).and_then(|s| s.get("symbolLayers")).and_then(Value::as_array);
    if shadow.is_some_and(|l| !l.is_empty()) {
        report.skipped(what, "a shadow, left out");
    }
    if border.is_none() && background.is_none() {
        return None;
    }
    let mut style = FrameStyle {
        border: border.map(|(c, _)| c),
        border_width: border.map_or(1.0, |(_, w)| f64::from(w)),
        background,
        gap: num(frame, "borderGapX").unwrap_or(0.0) * PT_MM,
    };
    style.clamp();
    Some(style)
}

/// At most 64 characters of `s`, for a note that names something a file said.
pub(crate) fn shown(s: &str) -> String {
    s.chars().take(64).collect()
}

/// Reads the layout of a `CIMLayoutDocument`: the page, then each element as a RustGIS element.
pub fn read_layout(doc: &Document, report: &mut Report) -> Result<LayoutImport, String> {
    let def = doc
        .root()
        .get("layoutDefinition")
        .filter(|d| d.is_object())
        .ok_or("not an ArcGIS Pro layout: the document has no layoutDefinition")?;
    let name = str_of(def, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Layout").to_string();
    let page = def.get("page").unwrap_or(&NULL);
    let unit_mm = page_unit_mm(page).unwrap_or_else(|| {
        report.approximated(&name, "page units RustGIS doesn't know, read as inches");
        INCH_MM
    });
    let size = |k: &str| num(page, k).filter(|v| *v > 0.0);
    let (Some(w), Some(h)) = (size("width"), size("height")) else {
        return Err("the layout's page has no size".into());
    };
    let mut layout = Layout::new(name.clone(), w * unit_mm, h * unit_mm);
    for g in page.get("guides").and_then(Value::as_array).into_iter().flatten() {
        let Some(at) = num(g, "position") else { continue };
        match str_of(g, "orientation") {
            Some("Vertical") => layout.guides.x.push(at * unit_mm),
            Some("Horizontal") => layout.guides.y.push((h - at) * unit_mm),
            _ => {}
        }
    }
    let mut reader = Reader {
        doc,
        report,
        name,
        layout,
        pictures: Vec::new(),
        text_frames: Vec::new(),
        pending_pictures: Vec::new(),
        frames: Frames::default(),
        page_h: h,
        unit_mm,
        full: false,
    };
    reader.walk(def.get("elements").unwrap_or(&NULL), 0, 0);
    let Reader { mut layout, pictures, text_frames, mut frames, mut pending_pictures, name, .. } = reader;
    frames.set_series(surround::series(doc, def, &name, report));
    frames.resolve(&mut layout, &text_frames, report);
    layout.clamp();
    frames.restore_unset_scales(&mut layout);
    // A north arrow dropped for want of a frame takes its pictures with it.
    pending_pictures.retain(|(id, _, _)| layout.element(*id).is_some());
    Ok(LayoutImport { layout, frames: frames.bindings, pictures, embedded_maps: frames.embedded, text_frames, pending_pictures })
}

/// The walk over a layout's elements, and what it has built so far.
struct Reader<'a> {
    doc: &'a Document,
    report: &'a mut Report,
    /// The layout's name, for notes about the whole layout.
    name: String,
    layout: Layout,
    pictures: Vec<(u32, PictureSource)>,
    text_frames: Vec<(u32, String)>,
    pending_pictures: Vec<(u32, usize, String)>,
    frames: Frames,
    /// The page's height in its units, and how many millimetres a unit is.
    page_h: f64,
    unit_mm: f64,
    /// [`MAX_ELEMENTS`] reached (and said).
    full: bool,
}

impl Reader<'_> {
    /// Adds the elements of the list `elements`, `depth` groups deep in the layout (`walked` deep
    /// in the document), and returns their ids in order.
    fn walk(&mut self, elements: &Value, depth: usize, walked: usize) -> Vec<u32> {
        let mut ids = Vec::new();
        for e in elements.as_array().into_iter().flatten() {
            if !self.room() {
                break;
            }
            let name = str_of(e, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Element").to_string();
            match str_of(e, "type").unwrap_or_default() {
                "CIMGroupElement" => ids.extend(self.group(e, &name, depth, walked)),
                "CIMGraphicElement" => ids.extend(self.graphic(e, &name)),
                "CIMMapFrame" => ids.extend(self.map_frame(e, &name)),
                ty @ ("CIMLegend"
                | "CIMMarkerNorthArrow"
                | "CIMTableFrame"
                | "CIMDoubleFillScaleBar"
                | "CIMSingleFillScaleBar"
                | "CIMLineScaleBar"
                | "CIMSteppedLineScaleBar"
                | "CIMHollowScaleBar"
                | "CIMAlternatingScaleBar") => ids.extend(self.surround(e, ty, &name)),
                "" => self.report.skipped(&name, "an element with no type, left out"),
                other => self.report.skipped(&name, format!("a {} element, left out", shown(other))),
            }
        }
        ids
    }

    /// Whether another element fits; once it doesn't, says so (once).
    fn room(&mut self) -> bool {
        if self.layout.elements.len() < MAX_ELEMENTS {
            return true;
        }
        if !self.full {
            self.full = true;
            self.report.skipped(&self.name, format!("elements past the first {MAX_ELEMENTS}, left out"));
        }
        false
    }

    /// A `CIMGroupElement`: its elements, then the group of them. One nested past
    /// [`MAX_GROUP_DEPTH`] gives its elements to the group round it.
    fn group(&mut self, e: &Value, name: &str, depth: usize, walked: usize) -> Vec<u32> {
        let elements = e.get("elements").unwrap_or(&NULL);
        if walked >= MAX_WALK_DEPTH {
            self.report.skipped(name, format!("groups nested past {MAX_WALK_DEPTH} deep, left out"));
            return Vec::new();
        }
        if depth >= MAX_GROUP_DEPTH {
            self.report.approximated(name, format!("groups nested past {MAX_GROUP_DEPTH} deep, flattened into the group round them"));
            return self.walk(elements, depth, walked + 1);
        }
        let children = self.walk(elements, depth + 1, walked + 1);
        if children.is_empty() || !self.room() {
            return children;
        }
        // Its box is its children's (`Layout::group_box`).
        let boxes = children.iter().filter_map(|c| self.layout.element_box(*c));
        let corners = boxes.flat_map(|(x, y, w, h)| [(x, y), (x + w, y + h)]);
        let (x, y, w, h) = PageBox::around(corners).map_or((0.0, 0.0, 0.0, 0.0), |b| (b.xmin, b.ymin, b.xmax - b.xmin, b.ymax - b.ymin));
        let id = self.layout.add(name, (x, y, w, h), ElementKind::Group { children });
        self.finish(id, e, name);
        vec![id]
    }

    /// A `CIMMapFrame` (see [`frame`]); the names it holds are resolved after the walk.
    fn map_frame(&mut self, e: &Value, name: &str) -> Option<u32> {
        let (b, kind, pending) = frame::read(self.doc, e, name, self.report)?;
        let id = self.add(e, b, name, kind)?;
        self.frames.add(id, pending);
        Some(id)
    }

    /// A legend, scale bar, north arrow or table frame (see [`surround`]); the frame it reads is
    /// resolved after the walk.
    fn surround(&mut self, e: &Value, ty: &str, name: &str) -> Option<u32> {
        let (b, kind, pending) = surround::read(self.doc, e, ty, name, self.report)?;
        let id = self.add(e, b, name, kind)?;
        self.frames.add_surround(id, pending.frame);
        self.pending_pictures.extend(pending.pictures.into_iter().map(|(i, url)| (id, i, url)));
        Some(id)
    }

    /// A `CIMGraphicElement`, by the kind of its `graphic`.
    fn graphic(&mut self, e: &Value, name: &str) -> Option<u32> {
        let g = e.get("graphic").unwrap_or(&NULL);
        match str_of(g, "type").unwrap_or_default() {
            "CIMPolygonGraphic" => self.polygon(e, g, name),
            "CIMLineGraphic" => self.line(e, g, name),
            "CIMPictureGraphic" => self.picture(e, g, name),
            "CIMTextGraphic" | "CIMParagraphTextGraphic" => self.text(e, g, name),
            other => {
                self.report.skipped(name, format!("a {} graphic, left out", shown(other)));
                None
            }
        }
    }

    /// The box of an element whose shape (unrotated) covers `b`: turned about its anchor, in
    /// millimetres. A turn that leaves it slanted keeps the box round it, and is noted.
    fn place(&mut self, e: &Value, b: PageBox, name: &str) -> (f64, f64, f64, f64) {
        let degrees = num(e, "rotation").unwrap_or(0.0);
        let b = if degrees == 0.0 {
            b
        } else {
            if !is_right_angle(degrees) {
                self.report.approximated(
                    name,
                    format!("rotated {:.1}°; RustGIS elements don't turn, so it fills the upright box round it", degrees),
                );
            }
            rotated(b, anchor_point(b, str_of(e, "anchor").unwrap_or(DEFAULT_ANCHOR)), degrees)
        };
        to_mm(b, self.page_h, self.unit_mm)
    }

    /// Adds an element of `kind` in box `b` (page units, unrotated).
    fn add(&mut self, e: &Value, b: PageBox, name: &str, kind: ElementKind) -> Option<u32> {
        let rect = self.place(e, b, name);
        let id = self.layout.add(name, rect, kind);
        self.finish(id, e, name);
        Some(id)
    }

    /// What every element carries: shown or hidden, locked, and its frame style.
    fn finish(&mut self, id: u32, e: &Value, name: &str) {
        let style = graphic_frame(e).and_then(|f| frame_style(f, name, self.report));
        if let Some(el) = self.layout.element_mut(id) {
            el.visible = flag(e, "visible", true);
            el.locked = flag(e, "locked", false);
            el.frame_style = style;
        }
    }

    /// A `CIMPolygonGraphic`: a rectangle (four corners, rounded or not), an ellipse (one ring of
    /// arcs about one centre), or the rectangle round any other shape.
    fn polygon(&mut self, e: &Value, g: &Value, name: &str) -> Option<u32> {
        let shape = shape_of(g);
        let Some(b) = geometry_box(shape) else {
            self.report.skipped(name, "a shape with no points, left out");
            return None;
        };
        let (sym, effect_rounding) = without_rounding(g.get("symbol").unwrap_or(&NULL));
        let rounding = effect_rounding.or_else(|| attribute(g, "Rounding")).filter(|r| *r > 0.0);
        let paint = symbol::polygon(&sym, name, self.report);
        if !paint.fills.is_empty() {
            self.report.approximated(name, "fills other than one solid colour, drawn in the top solid one");
        }
        let fill = paint.fill.unwrap_or(Rgba::TRANSPARENT);
        let (outline, width) = paint.stroke.map_or((Rgba::TRANSPARENT, 0.0), |(c, w)| (c, f64::from(w)));
        let rect = self.place(e, b, name);
        let kind = if is_ellipse(shape) {
            ElementKind::Ellipse { fill, outline, width }
        } else {
            if !is_four_corner_ring(shape) {
                self.report.approximated(name, "a polygon other than a rectangle, drawn as the rectangle round it");
            }
            let radius = rounding.map_or(0.0, |pct| {
                self.report.approximated(name, "rounded corners, approximated");
                corner_radius(pct, rect.2, rect.3)
            });
            ElementKind::Rectangle { fill, outline, width, radius }
        };
        let id = self.layout.add(name, rect, kind);
        self.finish(id, e, name);
        Some(id)
    }

    /// A `CIMLineGraphic`: a straight line from its first point to its last, with arrowheads
    /// where its symbol puts markers at its ends.
    fn line(&mut self, e: &Value, g: &Value, name: &str) -> Option<u32> {
        let shape = shape_of(g);
        let mut budget = MAX_POINTS;
        let paths: Vec<Vec<(f64, f64)>> = ["paths", "curvePaths"]
            .iter()
            .flat_map(|k| shape.get(*k).and_then(Value::as_array).into_iter().flatten())
            .map(|p| part_points(p, &mut budget))
            .filter(|p| !p.is_empty())
            .collect();
        let (Some(&first), Some(&last)) = (paths.first().and_then(|p| p.first()), paths.last().and_then(|p| p.last())) else {
            self.report.skipped(name, "a line with no points, left out");
            return None;
        };
        if paths.len() > 1 || paths.first().is_some_and(|p| p.len() > 2) || shape.get("curvePaths").is_some() {
            self.report.approximated(name, "a line of more than one straight segment, drawn straight from its first point to its last");
        }
        // Turned about its anchor, as a box is; a line can run at any angle, so nothing is lost.
        let degrees = num(e, "rotation").unwrap_or(0.0);
        let (a, b) = match PageBox::around([first, last]) {
            Some(bx) if degrees != 0.0 => {
                let about = anchor_point(bx, str_of(e, "anchor").unwrap_or(DEFAULT_ANCHOR));
                (rotate(first, about, degrees), rotate(last, about, degrees))
            }
            _ => (first, last),
        };
        let sym = g.get("symbol").unwrap_or(&NULL);
        let (strokes, head) = line_heads(sym, name, self.report);
        let paint = symbol::line(&strokes, name, self.report);
        let (color, width) = paint.stroke.map_or((Rgba::TRANSPARENT, 0.0), |(c, w)| (c, f64::from(w)));
        let (a, b) = (point_mm(a, self.page_h, self.unit_mm), point_mm(b, self.page_h, self.unit_mm));
        // From tail to tip: a head at the start alone points back along the line.
        let (tail, tip, head) = if head == LineHead::Start { (b, a, LineHead::End) } else { (a, b, head) };
        let angle = (tail.1 - tip.1).atan2(tip.0 - tail.0).to_degrees();
        let centre = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let (rect, dir, head) = line_at_angle(centre, (b.0 - a.0).hypot(b.1 - a.1), angle, head);
        let id = self.layout.add(name, rect, ElementKind::Line { color, width, dir, head });
        self.finish(id, e, name);
        Some(id)
    }

    /// A `CIMTextGraphic` (point text, one line or more, no wrapping, its box the text's own) or
    /// a `CIMParagraphTextGraphic` (wrapped in its shape, less its `margin`). Pro's markup is
    /// translated to tokens; the name of the frame the tokens read is kept in `text_frames`.
    fn text(&mut self, e: &Value, g: &Value, name: &str) -> Option<u32> {
        let Some(b) = geometry_box(shape_of(g)) else {
            self.report.skipped(name, "text with no position, left out");
            return None;
        };
        let style = symbol::text(g.get("symbol").unwrap_or(&NULL), name, self.report);
        let (text, frame) = dyntext::translate(str_of(g, "text").unwrap_or_default(), self.report, name);
        let paragraph = str_of(g, "type") == Some("CIMParagraphTextGraphic");
        let b = if paragraph {
            let margin = num(g, "margin").or_else(|| num(e, "margin")).unwrap_or(0.0).max(0.0) * PT_MM / self.unit_mm;
            inset(b, margin, 1.0 / self.unit_mm)
        } else {
            let lines: Vec<&str> = text.split('\n').collect();
            text_box((b.xmin, b.ymin), str_of(e, "anchor").unwrap_or(DEFAULT_ANCHOR), style.size, &lines, self.unit_mm)
        };
        let kind = ElementKind::Text {
            text,
            size: style.size,
            color: style.color,
            bold: style.bold,
            italic: style.italic,
            align: style.align,
            letter_spacing: style.letter_spacing,
            font: FontFamily::Sans,
            font_name: style.font,
            frame: None,
            wrap: paragraph,
            valign: style.valign,
            line_spacing: style.line_spacing,
            case: style.case,
        };
        let id = self.add(e, b, name, kind)?;
        if let Some(f) = frame {
            self.text_frames.push((id, f));
        }
        Some(id)
    }

    /// A `CIMPictureGraphic`: the picture embedded in the document (`referenceURI`), in a `data:`
    /// URL, or, kept to be read later, the file it came from.
    fn picture(&mut self, e: &Value, g: &Value, name: &str) -> Option<u32> {
        let Some(b) = geometry_box(shape_of(g)) else {
            self.report.skipped(name, "a picture with no box, left out");
            return None;
        };
        let url = str_of(g, "sourceURL").map(str::trim).filter(|u| !u.is_empty());
        let is_data = url.is_some_and(|u| u.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")));
        let inline = url.and_then(data_url).map(|(_, payload)| payload);
        if is_data && inline.is_none() {
            self.report.skipped(name, "a data: URL that isn't base64, left out");
        }
        let path = url.filter(|_| !is_data);
        let embedded = str_of(g, "referenceURI").and_then(|u| self.doc.binary(u)).map(|d| data_url(d).map_or(d, |(_, payload)| payload));
        let image = embedded.or(inline).unwrap_or_default().to_string();
        if image.is_empty() && path.is_none() {
            self.report.skipped(name, "a picture with no image or file, left empty");
        }
        let id = self.add(e, b, name, ElementKind::Picture { image: image.clone(), source: path.map(str::to_string) })?;
        if let (true, Some(p)) = (image.is_empty(), path) {
            self.pictures.push((id, PictureSource::Path(p.to_string())));
        }
        Some(id)
    }
}

/// The largest corner radius a rectangle takes (mm).
const MAX_CORNER_RADIUS: f64 = 50.0;

/// The corner radius (mm) of a `w` × `h` mm rectangle rounded by `percent`: a percentage of half
/// its shorter side, as Pro's rounding is.
pub(crate) fn corner_radius(percent: f64, w: f64, h: f64) -> f64 {
    let r = percent.clamp(0.0, 100.0) / 100.0 * w.min(h).max(0.0) / 2.0;
    if r.is_finite() { r.clamp(0.0, MAX_CORNER_RADIUS) } else { 0.0 }
}

/// The number a graphic's `attributes` property set gives `key` (`["Rounding", 0, …]`).
fn attribute(g: &Value, key: &str) -> Option<f64> {
    let items = g.get("attributes")?.get("propertySetItems")?.as_array()?;
    let i = items.iter().take(MAX_POINTS).step_by(2).position(|k| k.as_str() == Some(key))?;
    items.get(i * 2 + 1)?.as_f64().filter(|v| v.is_finite())
}

/// A polygon symbol without its rounded-corner effects (`CIMGeometricEffectRounding`, which the
/// rectangle's radius stands for), and the largest `rounding` they gave.
fn without_rounding(sym: &Value) -> (Value, Option<f64>) {
    let Some(mut s) = symbol_of(sym).cloned() else { return (NULL, None) };
    let mut rounding: Option<f64> = None;
    for layer in s.get_mut("symbolLayers").and_then(Value::as_array_mut).into_iter().flatten() {
        if let Some(effects) = layer.get_mut("effects").and_then(Value::as_array_mut) {
            effects.retain(|fx| {
                if str_of(fx, "type") != Some("CIMGeometricEffectRounding") {
                    return true;
                }
                if let Some(r) = num(fx, "rounding") {
                    rounding = Some(rounding.map_or(r, |m| m.max(r)));
                }
                false
            });
        }
    }
    (s, rounding)
}

/// Whether a shape is an ellipse or circle: one ring that starts at a point, runs only along
/// arcs about one centre, and closes.
pub(crate) fn is_ellipse(v: &Value) -> bool {
    if v.get("rings").is_some() {
        return false;
    }
    let Some([ring]) = v.get("curveRings").and_then(Value::as_array).map(Vec::as_slice) else { return false };
    let Some((Some(start), segs)) = ring.as_array().and_then(|r| r.split_first()).map(|(s, rest)| (xy(s), rest)) else {
        return false;
    };
    let arcs: Option<Vec<((f64, f64), (f64, f64))>> = segs
        .iter()
        .take(MAX_POINTS)
        .map(|seg| {
            let a = seg.get("a")?.as_array()?;
            Some((a.first().and_then(xy)?, a.get(1).and_then(xy)?))
        })
        .collect();
    let Some(arcs) = arcs.filter(|a| !a.is_empty()) else { return false };
    let Some(&(end, centre)) = arcs.last() else { return false };
    let size = arcs.iter().map(|(p, c)| (p.0 - c.0).hypot(p.1 - c.1)).fold(1.0, f64::max);
    let tol = 1e-9 * size;
    let same = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() <= tol && (p.1 - q.1).abs() <= tol;
    same(end, start) && arcs.iter().all(|(_, c)| same(*c, centre))
}

/// A graphic's geometry: `shape`, or what its kind calls it (`polygon`, `line`, `box`).
fn shape_of(g: &Value) -> &Value {
    ["shape", "polygon", "line", "box"].iter().find_map(|k| g.get(*k).filter(|v| v.is_object())).unwrap_or(&NULL)
}

/// The arrowheads a line symbol's markers stand for, and the symbol without those markers (its
/// strokes). A marker at the line's beginning is a head at its start, at the end one at its end;
/// markers placed along the line are left out and noted.
fn line_heads(sym: &Value, what: &str, report: &mut Report) -> (Value, LineHead) {
    let Some(mut s) = symbol_of(sym).cloned() else { return (NULL, LineHead::None) };
    let (mut start, mut end) = (false, false);
    if let Some(layers) = s.get_mut("symbolLayers").and_then(Value::as_array_mut) {
        layers.retain(|l| {
            if !matches!(str_of(l, "type"), Some("CIMVectorMarker" | "CIMCharacterMarker" | "CIMPictureMarker")) {
                return true;
            }
            if !flag(l, "enable", true) {
                return false;
            }
            let placement = l.get("markerPlacement").unwrap_or(&NULL);
            let at = match str_of(placement, "type") {
                Some("CIMMarkerPlacementAtExtremities") => match str_of(placement, "extremityPlacement").unwrap_or("Both") {
                    "JustBegin" => Some((true, false)),
                    "JustEnd" => Some((false, true)),
                    "None" => Some((false, false)),
                    _ => Some((true, true)),
                },
                Some("CIMMarkerPlacementOnLine") => match str_of(placement, "relativeTo") {
                    Some("LineBeginning") => Some((true, false)),
                    Some("LineEnd") => Some((false, true)),
                    _ => None,
                },
                _ => None,
            };
            match at {
                Some((s, e)) => {
                    start |= s;
                    end |= e;
                }
                None => report.skipped(what, "markers along a line, left out"),
            }
            false
        });
    }
    let head = match (start, end) {
        (true, true) => LineHead::Both,
        (true, false) => LineHead::Start,
        (false, true) => LineHead::End,
        (false, false) => LineHead::None,
    };
    (s, head)
}
