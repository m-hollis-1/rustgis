//! Units and sizes in a CIM document: a layout page's unit, and the point sizes symbols and text
//! are given in.

use serde_json::Value;

use crate::graphics::FONT_SIZE;

/// The largest size, in pixels, [`pt96`] gives: past it a width is a mistake, not a design.
const MAX_PX: f32 = 1000.0;

/// Millimetres per page unit, by the `uwkid` of the page's `units`. These are Esri's linear-unit
/// codes (the `LinearUnit.CreateLinearUnit` table of the ArcGIS Pro SDK): the International
/// metre, foot and inch, and the centimetre, millimetre and typographic point (1/72 inch). A code
/// not listed here isn't guessed at.
const PAGE_UNITS_MM: [(i64, f64); 6] =
    [(9001, 1000.0), (9002, 304.8), (109006, 10.0), (109007, 1.0), (109008, 25.4), (109016, 25.4 / 72.0)];

/// How many millimetres the page's unit is, or `None` for a unit this doesn't know.
#[allow(dead_code)] // used by the layout reader built on this module
pub(crate) fn page_unit_mm(page: &Value) -> Option<f64> {
    let wkid = page.get("units")?.get("uwkid")?.as_i64()?;
    PAGE_UNITS_MM.iter().find(|(id, _)| *id == wkid).map(|&(_, mm)| mm)
}

/// A size in points (CIM's unit for widths and offsets) as pixels at 96 per inch, which is what
/// the symbol model counts in. A size that isn't a finite number is 0.
#[allow(dead_code)]
pub(crate) fn pt96(v: f64) -> f32 {
    if v.is_finite() { ((v * 96.0 / 72.0) as f32).clamp(0.0, MAX_PX) } else { 0.0 }
}

/// A size in 96ths of an inch (what [`pt96`] gives) back in points.
pub(crate) fn pt_from_96(v: f32) -> f32 {
    v * 72.0 / 96.0
}

/// A text size in points, within what text can be drawn at (a non-finite size is the smallest).
#[allow(dead_code)]
pub(crate) fn text_pt(v: f64) -> f64 {
    if v.is_finite() { v.clamp(*FONT_SIZE.start(), *FONT_SIZE.end()) } else { *FONT_SIZE.start() }
}
