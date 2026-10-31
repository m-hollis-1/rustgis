//! A layout's surrounds, the elements that read a map frame: legends (`CIMLegend`), scale bars
//! (`CIMDoubleFillScaleBar` and its five siblings), north arrows (`CIMMarkerNorthArrow`) and table
//! frames (`CIMTableFrame`), and the layout's map series (`layoutDefinition.mapSeries`). The walk
//! in [`layout`](super::layout) calls [`read`] per element. The frame a surround reads is a frame's
//! *name* in the file, and the frame may come later in the list, so the element is read bound to
//! [`UNBOUND`] and [`Frames::resolve`](super::frame::Frames::resolve) binds it once the page is read.
//!
//! RustGIS draws a surround with its own settings where the file has none to carry over (a
//! table's rows, a legend's per-layer items): what is dropped or changed is noted.

use serde_json::Value;

use super::layout::{PageBox, geometry_box, shown};
use super::{Document, Report, flag, num, str_of, symbol};
use crate::Rgba;
use crate::layout::{
    ArrowStyle, BarLabels, ElementKind, LegendFit, MAX_LEGEND_COLUMNS, MAX_LEGEND_EXCLUDED, MAX_NORTH_ARROW_SYMBOLS, MAX_SCALE_BEFORE_ZERO,
    MAX_SCALE_DECIMALS, MAX_SCALE_DIVISIONS, MAX_SCALE_SUBDIVISIONS, SCALE_LABEL_SIZE, ScaleBarStyle, ScaleFit, ScaleUnits, UnitLabel,
};

/// The frame a surround holds while the page is being read; the second pass always replaces it
/// (or removes the surround).
pub(crate) const UNBOUND: u32 = u32::MAX;
/// The longest legend title kept (characters).
const MAX_TITLE: usize = 200;
/// The most legend items looked at.
const MAX_ITEMS: usize = 1000;
/// A table frame's rows, which the file doesn't give.
const TABLE_ROWS: usize = 12;
/// What a legend item names rather than sets: the layer it is for.
const ITEM_IDENTITY: [&str; 6] = ["type", "name", "layerURI", "layer", "uRI", "layerName"];

const NULL: Value = Value::Null;

/// What the walk keeps of a surround until the page is read.
pub(crate) struct Pending {
    /// The name of the map frame it reads (empty when it names none).
    pub frame: String,
    /// North arrow markers the engine converts later: the marker's index in the symbol and its
    /// `data:` URL.
    pub pictures: Vec<(usize, String)>,
}

/// The map series, waiting for its frame's name to become an id.
pub(crate) struct PendingSeries {
    /// The layout's name, which its notes are under.
    pub what: String,
    pub frame: String,
    pub layer: String,
    pub name_field: String,
}

/// Reads the surround `e` of CIM type `ty` called `name`: its box, the element (bound to
/// [`UNBOUND`]) and the frame name left to resolve. `None` (and a note) for one with no shape to
/// place, or a table with no layer.
pub(crate) fn read(doc: &Document, e: &Value, ty: &str, name: &str, report: &mut Report) -> Option<(PageBox, ElementKind, Pending)> {
    let Some(b) = e.get("frame").filter(|g| g.is_object()).and_then(geometry_box) else {
        report.skipped(name, "no shape, left out");
        return None;
    };
    let mut pictures = Vec::new();
    let kind = match ty {
        "CIMLegend" => legend(doc, e, name, report),
        "CIMMarkerNorthArrow" => north_arrow(e, name, &mut pictures, report),
        "CIMTableFrame" => table(doc, e, name, report)?,
        _ => scale_bar(ty, e, name, report),
    };
    let frame = str_of(e, "mapFrame").map(str::trim).unwrap_or_default().to_string();
    Some((b, kind, Pending { frame, pictures }))
}

/// The name of the layer a `CIMPATH=` names in the document.
fn layer_name(doc: &Document, uri: &str) -> Option<String> {
    let def = doc.resolve(uri).filter(|d| str_of(d, "type") != Some("CIMMap"))?;
    str_of(def, "name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string)
}

/// The whole number at `key`, within `lo..=hi`.
fn count(v: &Value, key: &str, lo: usize, hi: usize) -> Option<usize> {
    num(v, key).map(|n| n.round().clamp(lo as f64, hi as f64) as usize)
}

fn present(v: &Value, key: &str) -> bool {
    v.get(key).is_some_and(|x| !x.is_null())
}

// ---- legends ----------------------------------------------------------------------------------

fn legend(doc: &Document, e: &Value, name: &str, report: &mut Report) -> ElementKind {
    let title =
        if flag(e, "showTitle", true) { str_of(e, "title").unwrap_or("Legend").chars().take(MAX_TITLE).collect() } else { String::new() };
    let fit = match str_of(e, "fittingStrategy") {
        None | Some("Manual" | "ManualColumns") => LegendFit::Manual,
        Some("AdjustSize" | "AdjustFontSize") => LegendFit::AdjustSize,
        Some("AdjustColumns") => LegendFit::AdjustColumns,
        Some("AdjustColumnsAndSize") => LegendFit::AdjustColumnsAndSize,
        Some("AdjustFrame") => LegendFit::AdjustFrame,
        Some(other) => {
            report.approximated(name, format!("fitting strategy {}, drawn as {}", shown(other), LegendFit::Manual.label().to_lowercase()));
            LegendFit::Manual
        }
    };
    // Layers of other maps don't resolve here, and belong to those maps.
    let exclude = e
        .get("excludedLayers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_LEGEND_EXCLUDED)
        .filter_map(Value::as_str)
        .filter_map(|uri| layer_name(doc, uri))
        .collect();
    let default_item = e.get("defaultLegendItem").filter(|d| d.is_object());
    let items = e.get("items").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    if items.iter().take(MAX_ITEMS).any(|i| item_differs(i, default_item)) {
        report.approximated(name, "per-layer legend item settings, drawn with the defaults");
    }
    ElementKind::Legend {
        frame: UNBOUND,
        title,
        all_classes: false,
        hide_other: false,
        exclude,
        columns: count(e, "columns", 1, MAX_LEGEND_COLUMNS).unwrap_or(1),
        fit,
        show_layer_names: flag(default_item.unwrap_or(&NULL), "showLayerName", true),
    }
}

/// Whether a legend item sets anything the default item doesn't have the same.
fn item_differs(item: &Value, default: Option<&Value>) -> bool {
    let Some(settings) = item.as_object() else { return false };
    settings.iter().filter(|(k, _)| !ITEM_IDENTITY.contains(&k.as_str())).any(|(k, v)| default.and_then(|d| d.get(k)) != Some(v))
}

// ---- scale bars -------------------------------------------------------------------------------

fn scale_bar(ty: &str, e: &Value, name: &str, report: &mut Report) -> ElementKind {
    let mut kind = ElementKind::scale_bar(UNBOUND);
    let ElementKind::ScaleBar {
        units,
        divisions,
        style: bar_style,
        subdivisions,
        divisions_before_zero,
        label_position,
        unit_label,
        unit_label_position,
        decimals,
        label_size,
        fit,
        ..
    } = &mut kind
    else {
        return kind;
    };
    *bar_style = match ty {
        "CIMLineScaleBar" => ScaleBarStyle::Line,
        "CIMSteppedLineScaleBar" => ScaleBarStyle::Stepped,
        "CIMHollowScaleBar" => {
            report.approximated(name, "a hollow bar, drawn as an alternating bar");
            ScaleBarStyle::Alternating
        }
        "CIMSingleFillScaleBar" => {
            report.approximated(name, "a single-fill bar, drawn as an alternating bar");
            ScaleBarStyle::Alternating
        }
        _ => ScaleBarStyle::Alternating,
    };
    match e.get("units").and_then(|u| u.get("uwkid")).and_then(Value::as_i64) {
        None => {}
        Some(9002 | 9003) => *units = ScaleUnits::Feet,
        Some(9001) => *units = ScaleUnits::Meters,
        Some(9036) => *units = ScaleUnits::Kilometers,
        Some(9093) => *units = ScaleUnits::Miles,
        Some(other) => report.approximated(name, format!("units {other}, drawn in automatic units")),
    }
    *divisions = count(e, "divisions", 1, MAX_SCALE_DIVISIONS).unwrap_or(*divisions);
    *subdivisions = count(e, "subdivisions", 0, MAX_SCALE_SUBDIVISIONS).unwrap_or(0);
    *divisions_before_zero = count(e, "divisionsBeforeZero", 0, MAX_SCALE_BEFORE_ZERO).unwrap_or(0);
    if let Some(s) = str_of(e, "labelPosition") {
        match BarLabels::from_name(s) {
            Some(p) => *label_position = p,
            None => report.approximated(name, format!("label position {}, drawn {}", shown(s), label_position.label().to_lowercase())),
        }
    }
    *unit_label = str_of(e, "unitLabel").map(str::to_string);
    if let Some(s) = str_of(e, "unitLabelPosition") {
        match UnitLabel::from_name(s) {
            Some(p) => *unit_label_position = p,
            None => {
                report.approximated(name, format!("unit label position {}, drawn {}", shown(s), unit_label_position.label().to_lowercase()))
            }
        }
    }
    // Pro's 0 decimals is whole numbers; RustGIS's is "as many as each division needs", the same
    // for the whole division values scale bars usually have.
    if let Some(format) = e.get("numberFormat").filter(|f| str_of(f, "roundingOption") == Some("esriRoundNumberOfDecimals")) {
        *decimals = count(format, "roundingValue", 0, MAX_SCALE_DECIMALS).unwrap_or(*decimals);
    }
    if let Some(sym) = e.get("labelSymbol").filter(|s| s.is_object()) {
        *label_size = symbol::text(sym, name, report).size.clamp(*SCALE_LABEL_SIZE.start(), *SCALE_LABEL_SIZE.end());
    }
    if let Some(s) = str_of(e, "fittingStrategy") {
        match s {
            "AdjustDivision" => *fit = ScaleFit::AdjustDivision,
            "AdjustDivisionValue" => *fit = ScaleFit::AdjustDivisionValue,
            "AdjustWidth" | "AdjustFrame" => *fit = ScaleFit::AdjustWidth,
            other => report.approximated(name, format!("fitting strategy {}, drawn as {}", shown(other), fit.label().to_lowercase())),
        }
    }
    if let (ScaleFit::AdjustWidth | ScaleFit::AdjustDivision, Some(v)) = (*fit, num(e, "divisionValue")) {
        report.approximated(name, format!("division value {v}, drawn as RustGIS fits it"));
    }
    if ["markFrequency", "markHeight", "subdivisionMarkHeight"].iter().any(|k| present(e, k)) {
        report.approximated(name, "mark heights and frequency, drawn as RustGIS's");
    }
    kind
}

// ---- north arrows -----------------------------------------------------------------------------

fn north_arrow(e: &Value, name: &str, pictures: &mut Vec<(usize, String)>, report: &mut Report) -> ElementKind {
    let point = e.get("pointSymbol").filter(|s| s.is_object()).map(|s| symbol::point(s, name, report));
    let (mut symbols, mut pending) = point.map_or((Vec::new(), Vec::new()), |p| (p.symbols, p.pending_pictures));
    if symbols.len() > MAX_NORTH_ARROW_SYMBOLS {
        report.approximated(name, format!("{} marker layers, the first {MAX_NORTH_ARROW_SYMBOLS} kept", symbols.len()));
        symbols.truncate(MAX_NORTH_ARROW_SYMBOLS);
        pending.retain(|(i, _)| *i < MAX_NORTH_ARROW_SYMBOLS);
    }
    // The arrow's colour is the top drawn marker's fill; a picture has none of its own.
    let ink = symbols
        .iter()
        .enumerate()
        .rev()
        .filter(|(i, s)| !matches!(s.marker, crate::symbology::Marker::Image(_)) && !pending.iter().any(|(p, _)| p == i))
        .map(|(_, s)| s.fill)
        .find(|c| c.0[3] > 0)
        .unwrap_or(Rgba::rgb(0, 0, 0));
    if let Some(t) = str_of(e, "northType").filter(|t| *t != "TrueNorth") {
        let kind = t.strip_suffix("North").filter(|k| !k.is_empty()).unwrap_or(t);
        report.approximated(name, format!("{} north, drawn as true north", shown(kind)));
    }
    if let Some(a) = num(e, "calibrationAngle").filter(|a| *a != 0.0) {
        report.approximated(name, format!("a calibration angle of {a} degrees, left out"));
    }
    *pictures = pending;
    ElementKind::NorthArrow { frame: UNBOUND, style: ArrowStyle::Classic, fill: ink, outline: ink, symbol: symbols }
}

// ---- tables -----------------------------------------------------------------------------------

fn table(doc: &Document, e: &Value, name: &str, report: &mut Report) -> Option<ElementKind> {
    let layer = ["tableFrameURI", "layerURI"].iter().filter_map(|k| str_of(e, k)).find_map(|uri| layer_name(doc, uri));
    let Some(layer) = layer else {
        report.skipped(name, "its layer isn't in the document, left out");
        return None;
    };
    report.approximated(name, "table settings, drawn as RustGIS's");
    Some(ElementKind::Table { frame: UNBOUND, layer, rows: TABLE_ROWS })
}

// ---- the map series ---------------------------------------------------------------------------

/// The layout's map series (`mapSeries`, a `CIMSpatialMapSeries`) when its index layer is in the
/// document; the frame it follows is found once the page is read.
pub(crate) fn series(doc: &Document, layout: &Value, what: &str, report: &mut Report) -> Option<PendingSeries> {
    let s = layout.get("mapSeries").filter(|s| s.is_object())?;
    let Some(layer) = str_of(s, "indexLayerURI").and_then(|uri| layer_name(doc, uri)) else {
        report.skipped(what, "the map series' index layer isn't in the document, left out");
        return None;
    };
    if str_of(s, "sortField").is_some_and(|f| !f.trim().is_empty()) || present(s, "extentOptions") {
        report.approximated(what, "map series options, drawn as RustGIS's");
    }
    Some(PendingSeries {
        what: what.to_string(),
        frame: str_of(s, "mapFrameName").map(str::trim).unwrap_or_default().to_string(),
        layer,
        name_field: str_of(s, "nameField").unwrap_or_default().to_string(),
    })
}
