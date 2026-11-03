//! The maps and feature layers a `.pagx` template embeds, as the layers a RustGIS project opens.
//! A `CIMMap` lists its layers by `CIMPATH=…`; each resolves to a `CIMFeatureLayer` whose data
//! connection names a file (a shapefile, or a table of a file geodatabase) and whose renderer and
//! label classes become a [`Style`] (the carrier the QGIS importer uses, so the caller applies it
//! the same way). Every other kind of layer, and every other kind of connection or renderer, is
//! noted in the [`Report`] and left out, so the map still opens.
//!
//! The input is hostile: layers are capped at [`MAX_LAYERS`], classes at [`MAX_CLASSES`] and
//! [`MAX_LABEL_CLASSES`], groups are followed to a bounded depth with a bounded amount of work,
//! and a key that is missing or of another type reads as absent.

use std::collections::BTreeMap;

use serde_json::Value;

use super::color::symbol_of;
use super::symbol::{self, shown};
use super::units::pt96;
use super::{Document, Report, flag, num, str_of, typed};
use crate::color::{Ramp, Rgba};
use crate::fills::FillLayer;
use crate::layout::TextAlign;
use crate::qgis::symbol::{hidden as hidden_symbol, stacked};
use crate::qgis::{QGIS_SCALE, Style};
use crate::symbology::{
    DEFAULT_LABEL_CLASS, HALO_WIDTH, LABEL_SIZE, LabelClass, LabelPlacement, Labeling, LineStroke, MAX_LABEL_CLASSES, Marker, OTHER_VALUES,
    Renderer, Symbol,
};

/// The most layers read from one map.
const MAX_LAYERS: usize = 500;
/// The most classes read from one unique value renderer.
const MAX_CLASSES: usize = 500;
/// How deep groups are followed, and how many entries (layers and groups) one map may visit: a
/// group that lists itself, or a tree of groups, ends here.
const MAX_GROUP_DEPTH: usize = 8;
const MAX_VISITS: usize = MAX_LAYERS * 8;

const CLEAR: Rgba = Rgba::new(0, 0, 0, 0);
const NULL: Value = Value::Null;

/// A map of a template: its name, spatial reference, default extent and layers.
#[derive(Clone, Debug, PartialEq)]
pub struct MapImport {
    pub name: String,
    /// The map's spatial reference (`latestWkid`, else `wkid`).
    pub wkid: Option<u32>,
    /// `defaultExtent` as `[xmin, ymin, xmax, ymax]`, as given (in the units of `wkid`: degrees
    /// for 4326): the caller converts.
    pub extent: Option<[f64; 4]>,
    /// Top first, as CIMMap.layers lists them.
    pub layers: Vec<LayerImport>,
}

/// One layer of a map.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerImport {
    pub name: String,
    pub visible: bool,
    /// The file to open: a shapefile's, or the `.gdb` folder of a file geodatabase. `None` for a
    /// layer that has no file to open (noted).
    pub path: Option<String>,
    /// The feature class of a file geodatabase (`dataset`).
    pub table: Option<String>,
    /// Where the layer came from, as `<workspaceFactory> <dataset> in <DATABASE>`.
    pub origin: String,
    pub style: Style,
}

/// The map `uri` names, and its layers.
pub fn read_map(doc: &Document, uri: &str, report: &mut Report) -> Result<MapImport, String> {
    let map = doc.resolve(uri).and_then(|m| typed(m, "CIMMap")).ok_or_else(|| format!("{} isn't a map of this document", shown(uri)))?;
    read_map_def(doc, map, report)
}

/// The map `map` (a `CIMMap`, wherever the document found it), and its layers.
pub fn read_map_def(doc: &Document, map: &Value, report: &mut Report) -> Result<MapImport, String> {
    let name = str_of(map, "name").unwrap_or("Map").to_string();
    let wkid = map.get("spatialReference").and_then(|s| {
        ["latestWkid", "wkid"].iter().find_map(|k| s.get(*k).and_then(Value::as_u64).and_then(|w| u32::try_from(w).ok()).filter(|w| *w > 0))
    });
    let extent = map.get("defaultExtent").and_then(|e| Some([num(e, "xmin")?, num(e, "ymin")?, num(e, "xmax")?, num(e, "ymax")?]));
    let mut reader = Reader { doc, report, map: &name, layers: Vec::new(), visits: 0, capped: false, exhausted: false };
    reader.list(map, true, 0);
    let layers = reader.layers;
    Ok(MapImport { name, wkid, extent, layers })
}

/// Reads a map's layers, groups flattened in place.
struct Reader<'a> {
    doc: &'a Document,
    report: &'a mut Report,
    /// The map's name, for notes about its layer list.
    map: &'a str,
    layers: Vec<LayerImport>,
    visits: usize,
    /// Whether the notes that the layer cap and the visit budget were reached are made.
    capped: bool,
    exhausted: bool,
}

impl<'a> Reader<'a> {
    /// The entries of `parent`'s `layers` (a map's or a group's), shown only where `visible`.
    fn list(&mut self, parent: &'a Value, visible: bool, depth: usize) {
        for entry in parent.get("layers").and_then(Value::as_array).into_iter().flatten().take(MAX_LAYERS + 1) {
            self.layer(entry, visible, depth);
        }
    }

    /// One entry: a `CIMPATH=…` that resolves to a layer, or a layer written out.
    fn layer(&mut self, entry: &'a Value, visible: bool, depth: usize) {
        self.visits += 1;
        if self.layers.len() >= MAX_LAYERS {
            if !std::mem::replace(&mut self.capped, true) {
                self.report.skipped(self.map, format!("layers past the first {MAX_LAYERS}"));
            }
            return;
        }
        if self.visits > MAX_VISITS {
            if !std::mem::replace(&mut self.exhausted, true) {
                self.report.skipped(self.map, "more layers and groups than can be followed");
            }
            return;
        }
        let (def, uri) = match entry {
            Value::String(uri) => (self.doc.resolve(uri), Some(uri.as_str())),
            Value::Object(_) => (Some(entry), None),
            _ => return,
        };
        let Some(def) = def else {
            let name = uri.map(stem).unwrap_or_default();
            self.report.skipped(&name, format!("the layer {} isn't defined in this document, left out", shown(uri.unwrap_or_default())));
            self.layers.push(unread(name, visible, uri.unwrap_or_default().to_string()));
            return;
        };
        let name = str_of(def, "name").map(str::to_string).or_else(|| uri.map(stem)).unwrap_or_else(|| "Layer".into());
        let visible = visible && flag(def, "visibility", true);
        match str_of(def, "type") {
            Some("CIMFeatureLayer") => {
                let layer = feature_layer(def, name, visible, self.report);
                self.layers.push(layer);
            }
            Some("CIMGroupLayer") => {
                self.report.approximated(&name, format!("group {}, its layers listed one by one", shown(&name)));
                if depth >= MAX_GROUP_DEPTH {
                    self.report.skipped(&name, format!("groups inside groups past {MAX_GROUP_DEPTH} levels"));
                } else {
                    self.list(def, visible, depth + 1);
                }
            }
            other => {
                let ty = other.unwrap_or_default();
                self.report.skipped(&name, format!("{} layers, left out", shown(ty)));
                self.layers.push(unread(name, visible, ty.to_string()));
            }
        }
    }
}

/// A layer that has nothing to open.
fn unread(name: String, visible: bool, origin: String) -> LayerImport {
    LayerImport { name, visible, path: None, table: None, origin, style: Style::default() }
}

/// The last part of a `CIMPATH=…`, without `.json`: what a layer is called when it has no name.
fn stem(uri: &str) -> String {
    let last = uri.rsplit(['/', '\\']).next().unwrap_or(uri);
    let lower = last.to_ascii_lowercase();
    last.get(..last.len() - if lower.ends_with(".json") { 5 } else { 0 }).unwrap_or(last).to_string()
}

// ---- a feature layer --------------------------------------------------------------------------

fn feature_layer(def: &Value, name: String, visible: bool, report: &mut Report) -> LayerImport {
    let table = def.get("featureTable");
    let (path, table_name, origin) = match table.and_then(|t| t.get("dataConnection")) {
        Some(c) => connection(c, &name, report),
        None => {
            report.skipped(&name, "a layer with no data connection, nothing to open");
            (None, None, String::new())
        }
    };
    if let Some(q) = table.and_then(|t| str_of(t, "definitionExpression")).map(str::trim).filter(|q| !q.is_empty()) {
        report.skipped(&name, format!("its definition query {}, drawn without it", shown(q)));
    }

    let mut style = Style { transparency: num(def, "transparency").map(|t| (t as f32).clamp(0.0, 100.0)), ..Style::default() };
    // Pro's minScale is the out-beyond limit, as RustGIS's min_scale is; a missing or 0 limit
    // clears it (as the QGIS reader does). Pro works its scales out at 96 dpi as QGIS does, so
    // they take the same factor to RustGIS's 0.28 mm pixels.
    if def.get("minScale").is_some() || def.get("maxScale").is_some() {
        let scale = |k: &str| num(def, k).filter(|s| *s > 0.0).map(|s| s * QGIS_SCALE);
        style.min_scale = Some(scale("minScale"));
        style.max_scale = Some(scale("maxScale"));
    }
    if let Some(r) = def.get("renderer") {
        renderer(r, &name, &mut style, report);
    }
    style.labels = labels(def, &name, report);
    LayerImport { name, visible, path, table: table_name, origin, style }
}

/// The file a standard data connection names: `(path, table, origin)`.
fn connection(c: &Value, what: &str, report: &mut Report) -> (Option<String>, Option<String>, String) {
    let ty = str_of(c, "type").unwrap_or_default();
    if ty != "CIMStandardDataConnection" {
        report.skipped(what, format!("data connections of type {}, no file to open", shown(ty)));
        return (None, None, ty.to_string());
    }
    let factory = str_of(c, "workspaceFactory").unwrap_or_default();
    let dataset = str_of(c, "dataset").map(str::trim).unwrap_or_default();
    let database = connection_value(str_of(c, "workspaceConnectionString").unwrap_or_default(), "DATABASE").unwrap_or_default();
    let origin = format!("{factory} {dataset} in {database}");
    if database.is_empty() || dataset.is_empty() {
        report.skipped(what, format!("a {} connection with no folder or no dataset, nothing to open", shown(factory)));
        return (None, None, origin);
    }
    // A dataset is a name inside the folder, never a path out of it.
    if dataset.contains(['/', '\\']) || dataset.contains("..") {
        report.skipped(what, format!("a dataset named {}, which isn't a plain name, nothing to open", shown(dataset)));
        return (None, None, origin);
    }
    if factory.eq_ignore_ascii_case("Shapefile") {
        let folder = database.trim_end_matches(['/', '\\']);
        let sep = if folder.contains('\\') { '\\' } else { '/' };
        // (Pro writes a shapefile's dataset with its .shp; one without is the shapefile's name.)
        let file = if dataset.contains('.') { dataset.to_string() } else { format!("{dataset}.shp") };
        (Some(format!("{folder}{sep}{file}")), None, origin)
    } else if factory.eq_ignore_ascii_case("FileGDB") {
        (Some(database), Some(dataset.to_string()), origin)
    } else {
        report.skipped(what, format!("data in {} workspaces, no file to open", shown(factory)));
        (None, None, origin)
    }
}

/// The value of `key` (in any case) in a `KEY=VALUE;KEY=VALUE` connection string.
fn connection_value(s: &str, key: &str) -> Option<String> {
    s.split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ---- renderers --------------------------------------------------------------------------------

/// A symbol as the layer's style has it: the symbol, and the stacked strokes and fills that say
/// more than it does.
struct Look {
    symbol: Symbol,
    strokes: Vec<LineStroke>,
    fills: Vec<FillLayer>,
}

/// Nothing drawn.
fn hidden() -> Look {
    Look { symbol: hidden_symbol(), strokes: Vec::new(), fills: Vec::new() }
}

/// A polygon, line or point symbol (or the `CIMSymbolReference` holding one) as a [`Look`].
fn look(v: &Value, what: &str, report: &mut Report) -> Option<Look> {
    let Some(s) = symbol_of(v) else {
        report.skipped(what, "symbols that can't be read, left out");
        return None;
    };
    match str_of(s, "type") {
        Some("CIMPolygonSymbol") => {
            let p = symbol::polygon(s, what, report);
            if p.strokes.iter().any(|s| s.dash.is_some()) {
                report.approximated(what, "dashed polygon outlines, drawn solid");
            }
            let (stroke, stroke_width) = p.stroke.unwrap_or((CLEAR, 0.0));
            let symbol = Symbol { fill: p.fill.unwrap_or(CLEAR), stroke, stroke_width, size: 0.0, marker: Marker::Circle };
            Some(Look { symbol, strokes: Vec::new(), fills: p.fills })
        }
        Some("CIMLineSymbol") => {
            let p = symbol::line(s, what, report);
            let (c, stroke_width) = p.stroke.unwrap_or((CLEAR, 0.0));
            let symbol = Symbol { fill: c, stroke: c, stroke_width, size: 0.0, marker: Marker::Circle };
            let strokes = if stacked(&p.strokes) { p.strokes } else { Vec::new() };
            Some(Look { symbol, strokes, fills: Vec::new() })
        }
        Some("CIMPointSymbol") => {
            let p = symbol::point(s, what, report);
            if !p.pending_pictures.is_empty() {
                report.approximated(what, "a GIF or BMP marker, drawn as a circle");
            }
            if p.symbols.is_empty() {
                report.skipped(what, "point symbols with no marker to draw, left out");
                return None;
            }
            // Several marker layers are drawn as one marker.
            let layers: Vec<_> = p.symbols.into_iter().zip(p.paths).collect();
            Some(Look { symbol: symbol::stacked(&layers, what, report), strokes: Vec::new(), fills: Vec::new() })
        }
        Some(other) => {
            report.skipped(what, format!("{} symbols, left out", shown(other)));
            None
        }
        None => {
            report.skipped(what, "symbols with no type, left out");
            None
        }
    }
}

/// Sets `style`'s renderer (and the strokes and fills that go with it) from a layer's renderer.
fn renderer(r: &Value, what: &str, style: &mut Style, report: &mut Report) {
    let done = match str_of(r, "type") {
        Some("CIMSimpleRenderer") => match r.get("symbol") {
            // (A symbol that can't be read is noted by `look`.)
            Some(s) => match look(s, what, report) {
                Some(l) => {
                    style.line_strokes = Some(l.strokes);
                    style.fill_layers = Some(l.fills);
                    style.renderer = Some(Renderer::Single(l.symbol));
                    true
                }
                None => false,
            },
            None => {
                report.skipped(what, "simple renderers with no symbol, drawn with a default symbol");
                false
            }
        },
        Some("CIMUniqueValueRenderer") => unique(r, what, style, report),
        Some(other) => {
            report.skipped(what, format!("{} renderers, drawn with a default symbol", shown(other)));
            false
        }
        None => return,
    };
    if !done {
        let mut symbol = Symbol::default_for(rustgis_data::GeometryType::Polygon, 0);
        symbol.size = Symbol::default_for(rustgis_data::GeometryType::Point, 0).size;
        style.line_strokes = Some(Vec::new());
        style.fill_layers = Some(Vec::new());
        style.renderer = Some(Renderer::Single(symbol));
    }
}

/// A `CIMUniqueValueRenderer` on one field (the first of several, noted). `false` for one that
/// has no field.
fn unique(r: &Value, what: &str, style: &mut Style, report: &mut Report) -> bool {
    let fields = r.get("fields").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let Some(field) = fields.first().and_then(Value::as_str).filter(|f| !f.is_empty()) else {
        report.skipped(what, "unique value renderers with no field, drawn with a default symbol");
        return false;
    };
    if fields.len() > 1 {
        report.approximated(what, format!("unique values of {} fields, drawn by the first", fields.len()));
    }
    let mut classes: Vec<(String, Symbol)> = Vec::new();
    let (mut class_labels, mut class_strokes, mut class_fills) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    'groups: for group in r.get("groups").and_then(Value::as_array).into_iter().flatten().take(MAX_CLASSES) {
        for class in group.get("classes").and_then(Value::as_array).into_iter().flatten().take(MAX_CLASSES + 1) {
            // A class switched off draws nothing, and its symbol isn't read.
            let l = if flag(class, "visible", true) {
                let Some(l) = class.get("symbol").and_then(|s| look(s, what, report)) else { continue };
                l
            } else {
                hidden()
            };
            let label = str_of(class, "label").filter(|l| !l.is_empty());
            for v in class.get("values").and_then(Value::as_array).into_iter().flatten().take(MAX_CLASSES) {
                let Some(value) = v.get("fieldValues").and_then(Value::as_array).and_then(|f| f.first()).and_then(value_text) else {
                    continue;
                };
                if classes.len() >= MAX_CLASSES {
                    report.skipped(what, format!("classes past the first {MAX_CLASSES}"));
                    break 'groups;
                }
                if let Some(label) = label.filter(|l| *l != value) {
                    class_labels.insert(value.clone(), label.to_string());
                }
                if !l.strokes.is_empty() {
                    class_strokes.insert(value.clone(), l.strokes.clone());
                }
                if !l.fills.is_empty() {
                    class_fills.insert(value.clone(), l.fills.clone());
                }
                classes.push((value, l.symbol));
            }
        }
    }
    // The default symbol draws the rest only where the renderer says to use it.
    let other = if flag(r, "useDefaultSymbol", false) { r.get("defaultSymbol").and_then(|s| look(s, what, report)) } else { None };
    if classes.is_empty() && other.is_none() {
        report.skipped(what, "unique value renderers with no class to read and no default symbol, nothing drawn");
    }
    let other = other.unwrap_or_else(hidden);
    // The default's legend label, unless it is the usual one.
    let other_label = str_of(r, "defaultLabel").map(str::trim).filter(|l| !l.is_empty() && *l != OTHER_VALUES).map(str::to_string);
    style.line_strokes = Some(Vec::new());
    style.fill_layers = Some(Vec::new());
    style.renderer = Some(Renderer::Unique {
        field: field.to_string(),
        classes,
        other: other.symbol,
        ramp: Ramp::default(),
        class_labels,
        other_label,
        class_strokes,
        other_strokes: other.strokes,
        class_fills,
        other_fills: other.fills,
    });
    true
}

/// A field value as the text the layer's attributes are compared with.
fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ---- labels -----------------------------------------------------------------------------------

/// A layer's `labelClasses` as its labeling, `None` when no class can be carried over.
fn labels(def: &Value, what: &str, report: &mut Report) -> Option<Labeling> {
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
    let Some((field, exact)) = expression_field(expression) else {
        report.skipped(what, format!("label expression {}, no field to draw it with, class left out", shown(expression)));
        return None;
    };
    if !exact {
        report.approximated(what, format!("label expression {}, drawn as {}", shown(expression), shown(&field)));
    }
    // The text symbol, without the halo the label class carries over itself.
    let raw = c.get("textSymbol").and_then(symbol_of).unwrap_or(&NULL);
    let mut plain = raw.clone();
    if let Some(o) = plain.as_object_mut() {
        o.remove("haloSymbol");
    }
    let t = symbol::text(&plain, what, report);
    let halo_symbol = raw.get("haloSymbol").filter(|h| h.is_object());
    let halo_size = num(raw, "haloSize").filter(|s| *s > 0.0);
    let halo = halo_symbol.is_some() && halo_size.is_some();
    let halo_color = halo_symbol.filter(|_| halo).and_then(|h| symbol::polygon(h, what, report).fill);
    let halo_width = halo_size.filter(|_| halo).map(|s| f64::from(pt96(s)).clamp(*HALO_WIDTH.start(), *HALO_WIDTH.end()) as f32);
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
        placement: LabelPlacement::Auto,
        other_positions: false,
        italic: t.italic,
        font: t.font,
        halo_color,
        halo_width,
        spacing: 0.0,
        offset: [0.0; 2],
        background: None,
        wrap_char: None,
        wrap_length: 0,
        line_align: TextAlign::Left,
        repeat: None,
    })
}

/// The field a label expression stands for, and whether the expression is only that field
/// (`$feature.X`, `$feature["X"]`, `$feature['X']`, `[X]`, `!X!` or `X`). Of any other expression,
/// the first field it refers to.
fn expression_field(e: &str) -> Option<(String, bool)> {
    let e = e.trim();
    if is_name(e) {
        return Some((e.to_string(), true));
    }
    let (field, start, end) = first_reference(e)?;
    Some((field, start == 0 && end == e.len()))
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The first field reference in `e`, with where it starts and ends (bytes).
fn first_reference(e: &str) -> Option<(String, usize, usize)> {
    const FEATURE: &str = "$feature";
    for (i, ch) in e.char_indices() {
        let rest = e.get(i..)?;
        if let Some(after) = rest.strip_prefix(FEATURE) {
            if let Some(name) = after.strip_prefix('.') {
                let n: String = name.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !n.is_empty() {
                    return Some((n.clone(), i, i + FEATURE.len() + 1 + n.len()));
                }
            } else if let Some(inner) = after.strip_prefix('[') {
                let quote = inner.chars().next().filter(|q| *q == '"' || *q == '\'');
                let quoted = quote.and_then(|q| inner.get(1..)?.split_once(q));
                if let Some((n, _)) = quoted.filter(|(n, tail)| !n.is_empty() && tail.starts_with(']')) {
                    return Some((n.to_string(), i, i + FEATURE.len() + 1 + 1 + n.len() + 1 + 1));
                }
            }
        } else if ch == '[' || ch == '!' {
            let close = if ch == '[' { ']' } else { '!' };
            if let Some((n, _)) = rest.get(1..).and_then(|r| r.split_once(close)).filter(|(n, _)| is_name(n)) {
                return Some((n.to_string(), i, i + 1 + n.len() + 1));
            }
        }
    }
    None
}
