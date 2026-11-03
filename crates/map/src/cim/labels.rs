//! A feature layer's label classes as RustGIS label classes: the expression as a template, the
//! text symbol and its halo, and where the label goes (placement, callout, stacking).
//!
//! The input is hostile: classes are capped at [`MAX_LABEL_CLASSES`], numbers are clamped to the
//! ranges RustGIS draws, and a key that is missing or of another type reads as absent.

use serde_json::Value;

use super::color::{color, symbol_of};
use super::expr;
use super::symbol::{self, shown};
use super::units::pt_from_96;
use super::{Report, flag, num, str_of};
use crate::layout::TextAlign;
use crate::qgis::QGIS_SCALE;
use crate::symbology::{
    DEFAULT_LABEL_CLASS, HALO_WIDTH, LABEL_OFFSET, LABEL_SIZE, LabelBackground, LabelClass, LabelPlacement, Labeling, MAX_LABEL_CLASSES,
    label_repeat, wrap_char,
};

/// The most stacking separators looked through in one class.
const MAX_SEPARATORS: usize = 64;

const NULL: Value = Value::Null;

/// A layer's `labelClasses` as its labeling, `None` when no class can be carried over.
pub(crate) fn labels(def: &Value, what: &str, report: &mut Report) -> Option<Labeling> {
    let mut classes: Vec<LabelClass> = Vec::new();
    for c in def.get("labelClasses")?.as_array()?.iter().take(MAX_LABEL_CLASSES + 1) {
        if classes.len() >= MAX_LABEL_CLASSES {
            report.skipped(what, format!("label classes past the first {MAX_LABEL_CLASSES}"));
            break;
        }
        classes.extend(label_class(c, what, report));
    }
    let enabled = flag(def, "labelVisibility", false);
    // One class is the Default class; several are classes of their own, with Default (a hidden
    // copy of the first) switched off. (The twin of how `qgis/labels.rs::read` composes them.)
    if classes.len() == 1 {
        let mut default = classes.pop()?;
        default.name = DEFAULT_LABEL_CLASS.into();
        return Some(Labeling { enabled, default, classes: Vec::new() });
    }
    for (i, c) in classes.iter_mut().enumerate() {
        if c.name.is_empty() {
            c.name = format!("Class {}", i + 1);
        }
    }
    let first = classes.first()?.clone();
    let default = LabelClass { name: DEFAULT_LABEL_CLASS.into(), visible: false, where_: None, ..first };
    Some(Labeling { enabled, default, classes })
}

/// A `CIMLabelClass` as a label class, `None` when it has no field to label with (noted).
fn label_class(c: &Value, what: &str, report: &mut Report) -> Option<LabelClass> {
    let expression = str_of(c, "expression").unwrap_or_default();
    let engine = str_of(c, "expressionEngine").unwrap_or("Arcade");
    let Some(t) = expr::template(expression, engine) else {
        report.skipped(what, format!("label expression {}, no field to draw it with, class left out", shown(expression)));
        return None;
    };
    if !t.exact {
        report.approximated(what, format!("label expression {}, drawn as {}", shown(expression), shown(&t.template)));
    }
    for (written, read) in &t.joins {
        report.approximated(what, format!("field {} of a join, read as {}", shown(written), shown(read)));
    }
    if t.tags_dropped {
        report.approximated(what, "text formatting tags, left out");
    }
    let field = t.template;
    // The text symbol, without the halo the label class carries over itself.
    let raw = c.get("textSymbol").and_then(symbol_of).unwrap_or(&NULL);
    let mut plain = raw.clone();
    if let Some(o) = plain.as_object_mut() {
        o.remove("haloSymbol");
    }
    let t = symbol::text(&plain, what, report);
    let halo_symbol = raw.get("haloSymbol").filter(|h| h.is_object());
    let halo_size = num(raw, "haloSize").filter(|s| *s > 0.0);
    let (placement, repeat, offset) = placement(c, what, report);
    let (background, halo_kept) = callout(raw, what, report);
    shadow(raw, what, report);
    // A background callout stands where the halo would.
    let halo = halo_symbol.is_some() && halo_size.is_some() && halo_kept;
    let halo_color = halo_symbol.filter(|_| halo).and_then(|h| symbol::polygon(h, what, report).fill);
    // (Points, as RustGIS keeps them.)
    let halo_width = halo_size.filter(|_| halo).map(|s| s.clamp(*HALO_WIDTH.start(), *HALO_WIDTH.end()) as f32);
    // (Scales at 96 dpi, as the layer's: see `feature_layer`.)
    let scale = |k: &str| num(c, k).filter(|s| *s > 0.0).map(|s| s * QGIS_SCALE);
    Some(LabelClass {
        name: str_of(c, "name").unwrap_or_default().to_string(),
        visible: flag(c, "visibility", true),
        where_: str_of(c, "whereClause").map(str::trim).filter(|w| !w.is_empty()).map(str::to_string),
        field,
        size: t.size.clamp(*LABEL_SIZE.start(), *LABEL_SIZE.end()) as f32,
        color: t.color,
        halo,
        bold: t.bold,
        min_scale: scale("minimumScale"),
        max_scale: scale("maximumScale"),
        closest: None,
        farthest: None,
        placement,
        other_positions: false,
        italic: t.italic,
        font: t.font,
        halo_color,
        halo_width,
        spacing: 0.0,
        offset,
        background,
        wrap_char: stacking(c, what, report),
        wrap_length: 0,
        line_align: TextAlign::Left,
        repeat,
    })
}

// ---- placement --------------------------------------------------------------------------------

/// Which kind of feature a placement block places labels for.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Point,
    Line,
    Polygon,
}

/// Where a class puts its labels: its placement, its repeat distance (points) and its offset
/// (points, right and down). Maplex's block is read when the class has one, else the standard
/// one; a class with neither is placed as Auto. A method RustGIS has no placement for is Auto
/// (noted), and so are the Maplex settings that decide between candidate places.
pub(crate) fn placement(class: &Value, what: &str, report: &mut Report) -> (LabelPlacement, Option<f32>, [f32; 2]) {
    let block = |k: &str| class.get(k).filter(|b| b.is_object());
    let maplex = block("maplexLabelPlacementProperties");
    let Some(props) = maplex.or_else(|| block("standardLabelPlacementProperties")) else {
        return (LabelPlacement::Auto, None, [0.0; 2]);
    };
    let is_maplex = maplex.is_some();
    if is_maplex {
        unread(props, what, report);
    }
    let kind = match str_of(props, "featureType") {
        Some("Point") => Some(Kind::Point),
        Some("Line") => Some(Kind::Line),
        Some("Polygon") => Some(Kind::Polygon),
        // Without one, the method the block has says what it is for.
        _ if props.get("pointPlacementMethod").is_some() => Some(Kind::Point),
        _ if ["linePlacementMethod", "lineLabelPosition"].iter().any(|k| props.get(k).is_some()) => Some(Kind::Line),
        _ if props.get("polygonPlacementMethod").is_some() => Some(Kind::Polygon),
        _ => None,
    };
    let mut repeat = None;
    let mut offset = [0.0; 2];
    // The method as written and the placement it stands for (none for one this can't place).
    let found: Option<(&str, Option<LabelPlacement>)> = match kind {
        Some(Kind::Point) => {
            let found = str_of(props, "pointPlacementMethod").map(|m| (m, point_method(m)));
            if let (true, Some(distance)) = (is_maplex, num(props, "primaryOffset").filter(|d| *d > 0.0)) {
                offset = point_offset(found.and_then(|(_, p)| p).unwrap_or(LabelPlacement::Auto), distance);
            }
            found
        }
        Some(Kind::Line) if is_maplex => str_of(props, "linePlacementMethod").map(|m| (m, line_method(m))),
        Some(Kind::Line) => {
            let position = props.get("lineLabelPosition");
            let along = ["parallel", "horizontal", "perpendicular", "above", "below", "online", "inLine"]
                .iter()
                .any(|k| position.is_some_and(|p| flag(p, k, false)));
            let curved = flag(props, "produceCurvedLabels", false);
            along.then_some(("", Some(if curved { LabelPlacement::Curved } else { LabelPlacement::Parallel })))
        }
        Some(Kind::Polygon) => {
            let method = str_of(props, "polygonPlacementMethod");
            if is_maplex && method == Some("RepeatAlongBoundary") {
                repeat = num(props, "repetitionInterval").and_then(label_repeat);
            }
            method.map(|m| (m, if is_maplex { maplex_polygon_method(m) } else { standard_polygon_method(m) }))
        }
        None => None,
    };
    match found {
        Some((_, Some(p))) => (p, repeat, offset),
        Some((m, None)) => {
            report.approximated(what, format!("label placement {}, drawn as Auto", shown(m)));
            (LabelPlacement::Auto, repeat, offset)
        }
        None => (LabelPlacement::Auto, repeat, offset),
    }
}

/// The placement of a point method (Maplex and the standard engine share them), `None` for one
/// RustGIS has no placement for.
fn point_method(m: &str) -> Option<LabelPlacement> {
    use LabelPlacement::*;
    Some(match m {
        "AroundPoint" => Auto,
        "CenteredOnPoint" => Center,
        "NorthOfPoint" => Above,
        "SouthOfPoint" => Below,
        "EastOfPoint" => Right,
        "WestOfPoint" => Left,
        "NorthEastOfPoint" => AboveRight,
        "NorthWestOfPoint" => AboveLeft,
        "SouthEastOfPoint" => BelowRight,
        "SouthWestOfPoint" => BelowLeft,
        _ => return None,
    })
}

fn line_method(m: &str) -> Option<LabelPlacement> {
    match m {
        "OffsetStraightFromLine" | "CenteredStraightOnLine" | "CenteredHorizontalOnLine" | "OffsetHorizontalFromLine" => {
            Some(LabelPlacement::Parallel)
        }
        "OffsetCurvedFromLine" | "CenteredCurvedOnLine" => Some(LabelPlacement::Curved),
        _ => None,
    }
}

fn maplex_polygon_method(m: &str) -> Option<LabelPlacement> {
    match m {
        "HorizontalInPolygon" | "AlwaysHorizontal" | "BoundaryPlacement" => Some(LabelPlacement::Center),
        "StraightInPolygon" => Some(LabelPlacement::Parallel),
        "CurvedInPolygon" | "RepeatAlongBoundary" => Some(LabelPlacement::Curved),
        _ => None,
    }
}

fn standard_polygon_method(m: &str) -> Option<LabelPlacement> {
    match m {
        "AlwaysHorizontal" | "MixedStrategy" => Some(LabelPlacement::Center),
        "AlwaysStraight" => Some(LabelPlacement::Parallel),
        _ => None,
    }
}

/// How far a label `distance` points from its point is moved: up for the zones above and around
/// (RustGIS puts an Auto label above), down for those below, sideways for those at its side.
fn point_offset(placement: LabelPlacement, distance: f64) -> [f32; 2] {
    use LabelPlacement::*;
    let d = distance.clamp(*LABEL_OFFSET.start(), *LABEL_OFFSET.end()) as f32;
    match placement {
        Auto | Above | AboveLeft | AboveRight => [0.0, -d],
        Below | BelowLeft | BelowRight => [0.0, d],
        Right => [d, 0.0],
        Left => [-d, 0.0],
        _ => [0.0; 2],
    }
}

/// The Maplex settings RustGIS has nothing for, each noted when set off its default.
fn unread(props: &Value, what: &str, report: &mut Report) {
    // (The zone priorities only rank candidate places, which RustGIS has no model of: Pro writes
    // them on every class, and they are not noted.)
    const SETTINGS: [(&str, &str); 6] = [
        ("canOverrunFeature", "labels overrunning their features"),
        ("labelPriority", "label priorities"),
        ("featureWeight", "feature weights"),
        ("keyNumberMethod", "key numbering"),
        ("thinDuplicateLabels", "thinning duplicate labels"),
        ("removeDuplicates", "removing duplicate labels"),
    ];
    for (key, setting) in SETTINGS {
        if props.get(key).is_some_and(is_set) {
            report.skipped(what, format!("{setting}, left out"));
        }
    }
    if props.get("offsetAlongLineProperties").is_some_and(moves_along_line) {
        report.skipped(what, "label offsets along a line, left out");
    }
}

/// Whether a setting is off its default: true, a number but 0 (and -1, Pro's no priority), a
/// name but "None" or "NoKey".
fn is_set(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && n != -1.0),
        Value::String(s) => !matches!(s.as_str(), "" | "None" | "NoKey"),
        _ => false,
    }
}

/// Whether `offsetAlongLineProperties` moves the label: a distance, or a place along the line
/// other than the best one (which is where RustGIS puts it).
fn moves_along_line(v: &Value) -> bool {
    num(v, "distance").is_some_and(|d| d != 0.0) || str_of(v, "placementMethod").is_some_and(|m| m != "BestPositionAlongLine")
}

// ---- callouts, shadows and stacking -----------------------------------------------------------

/// The background a text symbol's callout draws, and whether the halo is kept (not when a
/// background stands in its place). A leader line has no counterpart: the label is drawn plain.
pub(crate) fn callout(text: &Value, what: &str, report: &mut Report) -> (Option<LabelBackground>, bool) {
    let Some(c) = text.get("callout").filter(|c| c.is_object()) else {
        return (None, true);
    };
    match str_of(c, "type") {
        Some("CIMBackgroundCallout") => background(c, what, report),
        Some("CIMSimpleLineCallout" | "CIMCompositeCallout") => {
            report.skipped(what, "a leader line, left out");
            (None, true)
        }
        Some(other) => {
            report.skipped(what, format!("callouts of type {}, left out", shown(other)));
            (None, true)
        }
        None => (None, true),
    }
}

/// A `CIMBackgroundCallout` as a label background (points throughout, as RustGIS keeps them); the
/// padding is the wider margin of each pair. Nothing to draw is no background.
fn background(c: &Value, what: &str, report: &mut Report) -> (Option<LabelBackground>, bool) {
    let Some(paint) = c.get("backgroundSymbol").map(|s| symbol::polygon(s, what, report)) else {
        return (None, true);
    };
    // (The outline is read in 96ths: back to points. A zero-width one is no outline.)
    let outline = paint.stroke.map(|(color, w)| (color, pt_from_96(w))).filter(|(_, w)| *w > 0.0);
    if paint.fill.is_none() && outline.is_none() {
        return (None, true);
    }
    let side = |k: &str| c.get("margin").and_then(|m| num(m, k)).unwrap_or(0.0).max(0.0);
    let background = LabelBackground {
        fill: paint.fill,
        stroke: outline.map(|(color, _)| color),
        stroke_width: outline.map_or(0.0, |(_, w)| w),
        padding: [side("left").max(side("right")) as f32, side("top").max(side("bottom")) as f32],
        radius: num(c, "cornerRadius").unwrap_or(0.0) as f32,
    };
    (Some(background.checked()), false)
}

/// Notes a text shadow (a colour that shows, or an offset), which RustGIS can't draw.
fn shadow(text: &Value, what: &str, report: &mut Report) {
    let colored = text.get("shadowColor").and_then(color).is_some_and(|c| c.a() > 0);
    let offset = ["shadowX", "shadowY"].iter().any(|k| num(text, k).is_some_and(|v| v != 0.0));
    if colored || offset {
        report.skipped(what, "a text shadow, left out");
    }
}

/// The wrap character a class that stacks its labels breaks at: the first separator Pro drops
/// from the label (`visible` false). One it keeps is noted, and the next dropped one is looked
/// for. Pro writes the settings in the Maplex block; the class itself is read too.
pub(crate) fn stacking(class: &Value, what: &str, report: &mut Report) -> Option<String> {
    let maplex = class.get("maplexLabelPlacementProperties").filter(|b| b.is_object());
    let find = |k: &str| maplex.and_then(|m| m.get(k)).or_else(|| class.get(k));
    if !find("canStackLabel").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    for s in find("labelStackingProperties")?.get("separators")?.as_array()?.iter().take(MAX_SEPARATORS) {
        let Some(sep) = str_of(s, "separator").and_then(wrap_char) else { continue };
        if flag(s, "visible", false) {
            report.approximated(what, format!("stacking at {} keeping it, drawn without", shown(&sep)));
        } else {
            return Some(sep);
        }
    }
    None
}
