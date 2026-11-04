//! The maps a CIM document holds (a `.pagx` template's embedded maps, a `.mapx`'s map, an
//! `.aprx`'s maps) and the layers of a `.lyrx`, as the layer tree a RustGIS project opens. A
//! `CIMMap` lists its layers by `CIMPATH=…`; each resolves to a layer or a group of layers. A layer
//! has a [`LayerSource`] read from its data connection (a file, a feature service, a map, image,
//! WMS or vector tile service, or one RustGIS can't open, kept with its origin so it comes in
//! broken), and its renderer and label classes become a [`Style`] (the carrier the QGIS importer
//! uses, so the caller applies it the same way). What doesn't come across is noted in the
//! [`Report`], and the map still opens.
//!
//! The input is hostile: layers are capped at [`MAX_LAYERS`], classes at [`MAX_CLASSES`] and
//! label classes (see `labels`), groups are followed to a bounded depth with a bounded amount of work,
//! and a key that is missing or of another type reads as absent. A connection string's passwords,
//! tokens and user names never reach an origin or a note (see [`scrub_connection`]).

use std::collections::BTreeMap;

use serde_json::Value;

use super::color::symbol_of;
use super::expr;
use super::labels::labels;
use super::symbol::{self, shown};
use super::units::pt96;
use super::{Document, Report, flag, num, str_of, typed};
use crate::color::{Ramp, Rgba};
use crate::effects::Blend;
use crate::fills::FillLayer;
use crate::qgis::symbol::{hidden as hidden_symbol, stacked};
use crate::qgis::{QGIS_SCALE, Style};
use crate::symbology::{ClassMethod, LineStroke, Marker, OTHER_VALUES, Renderer, Symbol};

/// The most layers read from one map, and the most standalone tables.
const MAX_LAYERS: usize = 500;
/// The most classes read from one unique value renderer.
const MAX_CLASSES: usize = 500;
/// How deep groups are followed, and how many entries (layers and groups) one map may visit: a
/// group that lists itself, or a tree of groups, ends here.
const MAX_GROUP_DEPTH: usize = 8;
const MAX_VISITS: usize = MAX_LAYERS * 8;
/// How many joins deep a joined table is followed to the table it starts from.
const MAX_JOIN_DEPTH: usize = 4;

const CLEAR: Rgba = Rgba::new(0, 0, 0, 0);

/// A map: its name, spatial reference, default extent, reference scale, layers and tables.
#[derive(Clone, Debug, PartialEq)]
pub struct MapImport {
    pub name: String,
    /// The map's spatial reference (`latestWkid`, else `wkid`).
    pub wkid: Option<u32>,
    /// `defaultExtent` as `[xmin, ymin, xmax, ymax]`, as given (in the units of `wkid`: degrees
    /// for 4326): the caller converts.
    pub extent: Option<[f64; 4]>,
    /// The scale symbols are drawn at their size (`referenceScale`), in RustGIS's scale (see
    /// [`QGIS_SCALE`]); `None` when the map has none.
    pub reference_scale: Option<f64>,
    /// Top first, as CIMMap.layers lists them.
    pub entries: Vec<Entry>,
    /// Its standalone tables, in order.
    pub tables: Vec<TableImport>,
}

/// One entry of a map's (or a group's) layer list. (A layer is much larger than a group; a map has
/// at most a few hundred entries, read once, so they are kept unboxed for the callers' sake.)
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Layer(LayerImport),
    Group(GroupImport),
}

/// A group of layers.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupImport {
    pub name: String,
    /// The group's own visibility: a hidden group hides its layers, which keep their own.
    pub visible: bool,
    /// Open in the table of contents.
    pub expanded: bool,
    /// 0 opaque to 100 invisible.
    pub transparency: Option<f32>,
    /// Drawn between these scales (RustGIS's, see [`QGIS_SCALE`]), as a layer's `min_scale` and
    /// `max_scale`: `None` for no limit.
    pub min_scale: Option<f64>,
    pub max_scale: Option<f64>,
    /// Top first.
    pub children: Vec<Entry>,
}

/// What a layer draws, for notes and for the icon of a layer that doesn't open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    /// Features: a feature layer (a file's, a service's, a database's) or KML.
    Feature,
    Raster,
    /// Pictures a service draws: a map, image, WMS or vector tile service.
    Tiles,
    /// A table listed among the layers.
    Table,
    /// Annotation, dimensions, an elevation surface, a knowledge graph or a kind not known.
    Other,
}

/// Where a layer's data is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerSource {
    /// A file, or a table of one: a shapefile, a file geodatabase's feature class or raster (a
    /// feature dataset's too: `table` is the feature class), a raster, a CAD drawing's layer, KML.
    /// A relative `path` is as Pro wrote it, for the caller to resolve.
    File { path: String, table: Option<String> },
    /// A layer (`layer`, its id) of an ArcGIS feature service at `url`.
    FeatureService { url: String, layer: String },
    /// An ArcGIS map service (…/MapServer) or image service (…/ImageServer).
    MapService { url: String },
    /// A WMS layer: the service at `url` and the first of its layers that shows.
    Wms { url: String, layer: String },
    /// A vector tile service (or package).
    VectorTiles { url: String },
    /// Data RustGIS can't open (a database, a spreadsheet, a query layer, a kind of layer it doesn't
    /// draw): the layer comes in broken, with its `origin`.
    Unopenable,
}

/// One layer of a map.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerImport {
    pub name: String,
    /// The layer's own visibility (a group's isn't folded in).
    pub visible: bool,
    /// Open in the table of contents.
    pub expanded: bool,
    pub source: LayerSource,
    /// Where Pro read it, for people and for `SourceLayer.origin`: "<factory> <dataset> in
    /// <DATABASE>", "ArcGIS feature service <url> layer <id>", "an SDE geodatabase on <SERVER>,
    /// database <DATABASE>, table <dataset>" — never a password or user.
    pub origin: String,
    pub style: Style,
    /// The feature table's definition query, as Pro's SQL: the caller checks it.
    pub definition_query: Option<String>,
    /// Whether the layer's symbols take the map's reference scale.
    pub scale_symbols: bool,
    pub kind: LayerKind,
}

/// A standalone table of a map.
#[derive(Clone, Debug, PartialEq)]
pub struct TableImport {
    pub name: String,
    pub source: LayerSource,
    /// As a layer's [`LayerImport::origin`].
    pub origin: String,
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
    let reference_scale = scale(map, "referenceScale");
    let mut reader = Reader::new(doc, report, &name);
    let entries = reader.list(map.get("layers"), 0);
    let tables = reader.tables(map.get("standaloneTables"));
    Ok(MapImport { name, wkid, extent, reference_scale, entries, tables })
}

/// A `.lyrx`'s layers (its `layers` list; groups kept), as a map with no name, projection or extent.
pub fn read_layers(doc: &Document, uris: &[String], report: &mut Report) -> MapImport {
    let list = Value::Array(uris.iter().take(MAX_LAYERS + 1).map(|u| Value::String(u.clone())).collect());
    let entries = Reader::new(doc, report, "Layer file").list(Some(&list), 0);
    MapImport { name: String::new(), wkid: None, extent: None, reference_scale: None, entries, tables: Vec::new() }
}

/// Reads a map's layer tree.
struct Reader<'a> {
    doc: &'a Document,
    report: &'a mut Report,
    /// The map's name, for notes about its layer list.
    map: &'a str,
    /// The layers read so far.
    layers: usize,
    visits: usize,
    /// Whether the notes that the layer cap and the visit budget were reached are made.
    capped: bool,
    exhausted: bool,
}

impl<'a> Reader<'a> {
    fn new(doc: &'a Document, report: &'a mut Report, map: &'a str) -> Self {
        Reader { doc, report, map, layers: 0, visits: 0, capped: false, exhausted: false }
    }

    /// The entries of a `layers` list (a map's or a group's), `depth` groups down.
    fn list(&mut self, list: Option<&Value>, depth: usize) -> Vec<Entry> {
        let mut out = Vec::new();
        for entry in list.and_then(Value::as_array).into_iter().flatten().take(MAX_LAYERS + 1) {
            if let Some(e) = self.entry(entry, depth) {
                out.push(e);
            }
        }
        out
    }

    /// One entry: a `CIMPATH=…` that resolves to a layer or group, or one written out.
    fn entry(&mut self, entry: &Value, depth: usize) -> Option<Entry> {
        self.visits += 1;
        if self.layers >= MAX_LAYERS {
            if !std::mem::replace(&mut self.capped, true) {
                self.report.skipped(self.map, format!("layers past the first {MAX_LAYERS}"));
            }
            return None;
        }
        if self.visits > MAX_VISITS {
            if !std::mem::replace(&mut self.exhausted, true) {
                self.report.skipped(self.map, "more layers and groups than can be followed");
            }
            return None;
        }
        let (def, uri) = definition(self.doc, entry)?;
        let Some(def) = def else {
            let uri = uri.unwrap_or_default();
            let name = stem(uri);
            self.report.skipped(&name, format!("the layer {} isn't defined in this document, left out", shown(uri)));
            self.layers += 1;
            return Some(Entry::Layer(unread(name, uri.to_string())));
        };
        let name = str_of(def, "name").map(str::to_string).or_else(|| uri.map(stem)).unwrap_or_else(|| "Layer".into());
        if str_of(def, "type") == Some("CIMGroupLayer") {
            if depth >= MAX_GROUP_DEPTH {
                self.report.skipped(&name, format!("groups inside groups past {MAX_GROUP_DEPTH} levels"));
                return None;
            }
            let children = self.list(def.get("layers"), depth + 1);
            return Some(Entry::Group(GroupImport {
                visible: flag(def, "visibility", true),
                expanded: flag(def, "expanded", false),
                transparency: transparency(def),
                min_scale: scale(def, "minScale"),
                max_scale: scale(def, "maxScale"),
                children,
                name,
            }));
        }
        self.layers += 1;
        Some(Entry::Layer(layer(def, name, self.report)))
    }

    /// The standalone tables a map lists, at most [`MAX_LAYERS`].
    fn tables(&mut self, list: Option<&Value>) -> Vec<TableImport> {
        let list = list.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        if list.len() > MAX_LAYERS {
            self.report.skipped(self.map, format!("tables past the first {MAX_LAYERS}"));
        }
        let mut out = Vec::new();
        for entry in list.iter().take(MAX_LAYERS) {
            let Some((def, uri)) = definition(self.doc, entry) else { continue };
            let Some(def) = def else {
                let uri = uri.unwrap_or_default();
                self.report.skipped(&stem(uri), format!("the table {} isn't defined in this document, left out", shown(uri)));
                continue;
            };
            let name = str_of(def, "name").map(str::to_string).or_else(|| uri.map(stem)).unwrap_or_else(|| "Table".into());
            let (source, origin) = match data_connection(def) {
                Some(c) => source_of(c, def, &name, self.report),
                None => {
                    self.report.skipped(&name, "a table with no data connection, nothing to open");
                    (LayerSource::Unopenable, String::new())
                }
            };
            out.push(TableImport { name, source, origin });
        }
        out
    }
}

/// A list entry's definition and `CIMPATH=…`: a path resolves in `doc` (to `None` when it doesn't),
/// an object is its own definition. `None` for anything else.
fn definition<'v>(doc: &'v Document, entry: &'v Value) -> Option<(Option<&'v Value>, Option<&'v str>)> {
    match entry {
        Value::String(uri) => Some((doc.resolve(uri), Some(uri.as_str()))),
        Value::Object(_) => Some((Some(entry), None)),
        _ => None,
    }
}

/// A layer that isn't there to read.
fn unread(name: String, origin: String) -> LayerImport {
    LayerImport {
        name,
        visible: true,
        expanded: false,
        source: LayerSource::Unopenable,
        origin,
        style: Style::default(),
        definition_query: None,
        scale_symbols: true,
        kind: LayerKind::Other,
    }
}

/// The last part of a `CIMPATH=…`, without `.json`: what a layer is called when it has no name.
fn stem(uri: &str) -> String {
    let last = uri.rsplit(['/', '\\']).next().unwrap_or(uri);
    let lower = last.to_ascii_lowercase();
    last.get(..last.len() - if lower.ends_with(".json") { 5 } else { 0 }).unwrap_or(last).to_string()
}

/// A scale at `k` in RustGIS's scale; `None` when missing or 0 (no limit). Pro works its scales
/// out at 96 dpi as QGIS does, so they take the same factor to RustGIS's 0.28 mm pixels.
fn scale(v: &Value, k: &str) -> Option<f64> {
    num(v, k).filter(|s| *s > 0.0).map(|s| s * QGIS_SCALE)
}

/// `transparency`, 0 opaque to 100 invisible.
fn transparency(v: &Value) -> Option<f32> {
    num(v, "transparency").map(|t| (t as f32).clamp(0.0, 100.0))
}

// ---- a layer ----------------------------------------------------------------------------------

/// A layer's data connection: its feature table's, else its own, else its service's.
fn data_connection(def: &Value) -> Option<&Value> {
    let table = def.get("featureTable").and_then(|t| t.get("dataConnection"));
    [table, def.get("dataConnection"), def.get("serviceConnection")].into_iter().flatten().find(|c| c.is_object())
}

/// A layer (anything but a group) of type `def.type`.
fn layer(def: &Value, name: String, report: &mut Report) -> LayerImport {
    let ty = str_of(def, "type").unwrap_or_default();
    let kind = match ty {
        "CIMFeatureLayer" | "CIMKMLLayer" => Some(LayerKind::Feature),
        "CIMRasterLayer" => Some(LayerKind::Raster),
        "CIMTiledServiceLayer" | "CIMMapImageLayer" | "CIMImageServiceLayer" | "CIMWMSLayer" | "CIMVectorTileLayer" => {
            Some(LayerKind::Tiles)
        }
        "CIMStandaloneTable" => Some(LayerKind::Table),
        _ => None,
    };
    let connection = data_connection(def);
    let mut style = Style { transparency: transparency(def), ..Style::default() };
    // Pro's minScale is the out-beyond limit, as RustGIS's min_scale is; a missing or 0 limit
    // clears it (as the QGIS reader does).
    if def.get("minScale").is_some() || def.get("maxScale").is_some() {
        style.min_scale = Some(scale(def, "minScale"));
        style.max_scale = Some(scale(def, "maxScale"));
    }
    let (source, origin) = match (kind, connection) {
        (Some(_), Some(c)) => source_of(c, def, &name, report),
        (Some(_), None) => {
            report.skipped(&name, "a layer with no data connection, nothing to open");
            (LayerSource::Unopenable, String::new())
        }
        (None, c) => {
            // One note says it all: nothing else about a layer RustGIS doesn't draw is read.
            report.skipped(&name, format!("{} layers, which RustGIS doesn't draw", shown(ty)));
            // Where it was, for people; what reading the connection notes is beside the point.
            let origin = c.map(|c| source_of(c, def, &name, &mut Report::default()).1).unwrap_or_else(|| shown(ty));
            return LayerImport {
                visible: flag(def, "visibility", true),
                expanded: flag(def, "expanded", false),
                scale_symbols: flag(def, "scaleSymbols", true),
                kind: LayerKind::Other,
                name,
                source: LayerSource::Unopenable,
                origin,
                style,
                definition_query: None,
            };
        }
    };

    style.blend = blend(def, &name, report);
    if let Some(r) = def.get("renderer") {
        renderer(r, &name, &mut style, report);
    }
    style.labels = labels(def, &name, report);
    unsupported_properties(def, &name, report);
    let definition_query = def
        .get("featureTable")
        .and_then(|t| str_of(t, "definitionExpression"))
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_string);
    LayerImport {
        visible: flag(def, "visibility", true),
        expanded: flag(def, "expanded", false),
        scale_symbols: flag(def, "scaleSymbols", true),
        kind: kind.unwrap_or(LayerKind::Other),
        name,
        source,
        origin,
        style,
        definition_query,
    }
}

/// A layer's `blendingMode` as a blend; another mode is noted and drawn normally.
fn blend(def: &Value, what: &str, report: &mut Report) -> Option<Blend> {
    Some(match str_of(def, "blendingMode")? {
        "Alpha" | "Normal" => Blend::Normal,
        "Multiply" => Blend::Multiply,
        "Screen" => Blend::Screen,
        "Overlay" => Blend::Overlay,
        "Darken" => Blend::Darken,
        "Lighten" => Blend::Lighten,
        "Difference" => Blend::Difference,
        other => {
            report.approximated(what, format!("its blending mode {}, drawn as normal", shown(other)));
            return None;
        }
    })
}

/// Notes, once each, the layer properties RustGIS has no counterpart for.
fn unsupported_properties(def: &Value, what: &str, report: &mut Report) {
    if flag(def, "enableDisplayFilters", false) && set(def.get("displayFilters")) {
        report.skipped(what, "display filters, every feature drawn at every scale");
    }
    if set(def.get("layerMasks")) {
        report.skipped(what, "layer masks, drawn without them");
    }
    if set(def.get("featureReduction")) {
        report.skipped(what, "feature reduction (clusters or bins), every feature drawn");
    }
    if def.get("symbolLayerDrawing").is_some_and(|d| flag(d, "useSymbolLayerDrawing", false)) {
        report.approximated(what, "symbol layer drawing, each feature drawn whole in turn");
    }
    if set(def.get("charts")) {
        report.skipped(what, "charts, left out");
    }
    if set(def.get("popupInfo")) {
        report.skipped(what, "its pop-up, left out");
    }
    if set(def.get("timeFilter")) {
        report.skipped(what, "its time filter, every feature drawn");
    }
    if set(def.get("selectionSymbol")) {
        report.approximated(what, "its selection symbol, selections drawn as RustGIS draws them");
    }
}

/// Whether a property is there to say something: not missing, null, false or empty, and not an
/// object that says it is off (`"enabled": false`).
fn set(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(o @ Value::Object(m)) => !m.is_empty() && flag(o, "enabled", true),
        Some(_) => true,
    }
}

// ---- sources ----------------------------------------------------------------------------------

/// The workspace factories whose data is a file (or a table of one) RustGIS may open.
const FILE_FACTORIES: [&str; 4] = ["Shapefile", "FileGDB", "Raster", "Cad"];

/// The source a data connection `c` names, and where it is for people. `owner` is the layer or
/// table it belongs to (a WMS layer lists its sublayers there); `what` names it in notes.
fn source_of(c: &Value, owner: &Value, what: &str, report: &mut Report) -> (LayerSource, String) {
    source_at(c, owner, what, report, 0)
}

fn source_at(c: &Value, owner: &Value, what: &str, report: &mut Report, joins: usize) -> (LayerSource, String) {
    let ty = str_of(c, "type").unwrap_or_default();
    match ty {
        "CIMStandardDataConnection" | "CIMFeatureDatasetDataConnection" => workspace(c, what, report),
        "CIMSqlQueryDataConnection" => {
            let conn = scrub_connection(str_of(c, "workspaceConnectionString").unwrap_or_default());
            let factory = str_of(c, "workspaceFactory").map(str::trim).unwrap_or_default();
            let place = if factory.eq_ignore_ascii_case("SDE") {
                sde_place(&conn)
            } else {
                format!("{factory} {}", connection_value(&conn, "DATABASE").unwrap_or_default())
            };
            report.skipped(what, "SQL query layers, which RustGIS can't open");
            (LayerSource::Unopenable, format!("SQL query {} on {place}", dataset_of(c)))
        }
        "CIMRelQueryTableDataConnection" => {
            let Some(source) = c.get("sourceTable").filter(|s| s.is_object() && joins < MAX_JOIN_DEPTH) else {
                report.skipped(what, "a join whose table can't be read, nothing to open");
                return (LayerSource::Unopenable, String::new());
            };
            let joined = c.get("destinationTable").map(dataset_of).unwrap_or_default();
            report.approximated(what, format!("a join to {}, drawn without it", shown(joined)));
            source_at(source, owner, what, report, joins + 1)
        }
        "CIMKMLDataConnection" => match str_of(c, "uRI").map(str::trim).filter(|u| !u.is_empty()) {
            Some(path) => {
                // (A KML file may be on the web, with a token in its address.)
                let path = scrub_url(path);
                let origin = format!("KML {path}");
                (LayerSource::File { path, table: None }, origin)
            }
            None => {
                report.skipped(what, "a KML connection with no file, nothing to open");
                (LayerSource::Unopenable, String::new())
            }
        },
        "CIMAGSServiceConnection" => {
            let named = || {
                let server = c.get("serverConnection").and_then(|s| str_of(s, "url"))?.trim().trim_end_matches('/');
                let name = str_of(c, "objectName")?.trim();
                Some(format!("{server}/{name}/{}", str_of(c, "objectType").unwrap_or("MapServer").trim()))
            };
            let Some(url) = str_of(c, "url").map(str::trim).filter(|u| !u.is_empty()).map(str::to_string).or_else(named) else {
                report.skipped(what, "an ArcGIS service connection with no address, nothing to open");
                return (LayerSource::Unopenable, String::new());
            };
            let url = scrub_url(&url);
            let image = str_of(c, "objectType") == Some("ImageServer") || url.to_ascii_lowercase().ends_with("/imageserver");
            let origin = format!("ArcGIS {} service {url}", if image { "image" } else { "map" });
            (LayerSource::MapService { url }, origin)
        }
        "CIMWMSServiceConnection" => {
            let url =
                c.get("serverConnection").and_then(|s| str_of(s, "url")).or_else(|| str_of(c, "url")).map(str::trim).unwrap_or_default();
            if url.is_empty() {
                report.skipped(what, "a WMS connection with no address, nothing to open");
                return (LayerSource::Unopenable, String::new());
            }
            let url = scrub_url(url);
            let Some(layer) = wms_layer(owner) else {
                report.skipped(what, "a WMS layer that names no visible sublayer, nothing to draw");
                return (LayerSource::Unopenable, format!("WMS {url}"));
            };
            let origin = format!("WMS {url} layer {}", shown(&layer));
            (LayerSource::Wms { url, layer }, origin)
        }
        "CIMVectorTileDataConnection" => match str_of(c, "uRI").map(str::trim).filter(|u| !u.is_empty()) {
            Some(url) => {
                let url = scrub_url(url);
                let origin = format!("vector tiles {url}");
                (LayerSource::VectorTiles { url }, origin)
            }
            None => {
                report.skipped(what, "a vector tile connection with no address, nothing to open");
                (LayerSource::Unopenable, String::new())
            }
        },
        other => {
            report.skipped(what, format!("data connections of type {}, which RustGIS can't open", shown(other)));
            (LayerSource::Unopenable, shown(other))
        }
    }
}

/// The first sublayer of a WMS layer that shows, depth first, by its WMS name: Pro writes that as
/// `subLayerID` (its `name` is the title people see), and `name` is read when there's none. A
/// sublayer holding others (Pro's `CIMServiceCompositeSubLayer`, the service's root) is looked
/// into, as is one with no name. A hidden sublayer hides the ones in it. At most [`MAX_LAYERS`]
/// sublayers are looked at, [`MAX_GROUP_DEPTH`] deep.
fn wms_layer(owner: &Value) -> Option<String> {
    fn subs(v: &Value) -> &[Value] {
        v.get("subLayers").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
    }
    // Each level's sublayers still to look at, top of the stack first.
    let mut stack = vec![subs(owner).iter()];
    let mut seen = 0;
    while let Some(level) = stack.last_mut() {
        let Some(s) = level.next() else {
            stack.pop();
            continue;
        };
        seen += 1;
        if seen > MAX_LAYERS {
            return None;
        }
        if !flag(s, "visibility", true) {
            continue;
        }
        let named = |k: &str| str_of(s, k).map(str::trim).filter(|n| !n.is_empty());
        if subs(s).is_empty()
            && let Some(name) = named("subLayerID").or_else(|| named("name"))
        {
            return Some(name.to_string());
        }
        if stack.len() < MAX_GROUP_DEPTH {
            stack.push(subs(s).iter());
        }
    }
    None
}

/// A connection's `dataset`, trimmed.
fn dataset_of(c: &Value) -> &str {
    str_of(c, "dataset").map(str::trim).unwrap_or_default()
}

/// The source of a workspace connection (a standard one, or a feature dataset's).
fn workspace(c: &Value, what: &str, report: &mut Report) -> (LayerSource, String) {
    let factory = str_of(c, "workspaceFactory").map(str::trim).unwrap_or_default();
    let conn = scrub_connection(str_of(c, "workspaceConnectionString").unwrap_or_default());
    let dataset = dataset_of(c);
    if factory.eq_ignore_ascii_case("FeatureService") {
        let url = connection_value(&conn, "URL").map(|u| scrub_url(&u)).unwrap_or_default();
        let origin = format!("ArcGIS feature service {url} layer {dataset}");
        if url.is_empty() || dataset.is_empty() {
            report.skipped(what, "a feature service connection with no address or no layer, nothing to open");
            return (LayerSource::Unopenable, origin);
        }
        return (LayerSource::FeatureService { url, layer: dataset.to_string() }, origin);
    }
    if factory.eq_ignore_ascii_case("SDE") {
        report.skipped(what, "data in SDE workspaces, which RustGIS can't open");
        return (LayerSource::Unopenable, format!("{}, table {dataset}", sde_place(&conn)));
    }
    // A CAD drawing is the feature dataset; its layer is the dataset (Pro may write it as
    // `drawing.dwg:Polyline`).
    let cad = factory.eq_ignore_ascii_case("Cad");
    let feature_dataset = str_of(c, "featureDataset").map(str::trim).unwrap_or_default();
    let (file, table) = match (cad, dataset.split_once(':')) {
        (true, Some((file, table))) => (file.trim(), Some(table.trim())),
        (true, None) if !feature_dataset.is_empty() => (feature_dataset, Some(dataset)),
        _ => (dataset, None),
    };
    let database = connection_value(&conn, "DATABASE").unwrap_or_default();
    let shown_dataset = match table {
        Some(t) if cad => format!("{file}:{t}"),
        _ => dataset.to_string(),
    };
    let origin = format!("{factory} {shown_dataset} in {database}");
    if !FILE_FACTORIES.iter().any(|f| f.eq_ignore_ascii_case(factory)) {
        report.skipped(what, format!("data in {} workspaces, which RustGIS can't open", shown(factory)));
        return (LayerSource::Unopenable, origin);
    }
    if database.is_empty() || file.is_empty() || table.is_some_and(str::is_empty) {
        report.skipped(what, format!("a {} connection with no folder or no dataset, nothing to open", shown(factory)));
        return (LayerSource::Unopenable, origin);
    }
    // A dataset is a name inside the folder, never a path out of it.
    if [file, table.unwrap_or_default()].iter().any(|d| d.contains(['/', '\\']) || d.contains("..")) {
        report.skipped(what, format!("a dataset named {}, which isn't a plain name, nothing to open", shown(&shown_dataset)));
        return (LayerSource::Unopenable, origin);
    }
    let source = if factory.eq_ignore_ascii_case("FileGDB") {
        LayerSource::File { path: database, table: Some(file.to_string()) }
    } else {
        // Pro writes a shapefile's dataset with its .shp; one without is the shapefile's name.
        let shp = factory.eq_ignore_ascii_case("Shapefile") && !file.contains('.');
        let file = if shp { format!("{file}.shp") } else { file.to_string() };
        LayerSource::File { path: joined(&database, &file), table: table.map(str::to_string) }
    };
    (source, origin)
}

/// `file` in `folder`, with the folder's own separator.
fn joined(folder: &str, file: &str) -> String {
    let folder = folder.trim_end_matches(['/', '\\']);
    let sep = if folder.contains('\\') { '\\' } else { '/' };
    format!("{folder}{sep}{file}")
}

/// An SDE connection's place, for people: "an SDE geodatabase on <SERVER>, database <DATABASE>".
fn sde_place(conn: &str) -> String {
    let server = ["SERVER", "DB_CONNECTION_PROPERTIES", "INSTANCE"].iter().find_map(|k| connection_value(conn, k));
    let mut place = format!("an SDE geodatabase on {}", server.as_deref().unwrap_or("a server not named"));
    if let Some(db) = connection_value(conn, "DATABASE") {
        place.push_str(&format!(", database {db}"));
    }
    place
}

/// The value of `key` (in any case) in a `KEY=VALUE;KEY=VALUE` connection string.
fn connection_value(s: &str, key: &str) -> Option<String> {
    s.split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// A connection string's `KEY=VALUE` pairs without the ones that sign in: `PASSWORD`,
/// `ENCRYPTED_PASSWORD…`, `TOKEN` and `USER` (in any case). A part that isn't `KEY=VALUE` (the
/// rest of a value with a `;` in it) is left out too.
pub(crate) fn scrub_connection(s: &str) -> String {
    let secret = |key: &str| {
        let key = key.trim().to_ascii_uppercase();
        key.starts_with("ENCRYPTED_PASSWORD") || ["PASSWORD", "TOKEN", "USER"].contains(&key.as_str())
    };
    let pairs = s.split(';').filter(|pair| pair.split_once('=').is_some_and(|(k, _)| !k.trim().is_empty() && !secret(k)));
    pairs.map(str::trim).collect::<Vec<_>>().join(";")
}

/// A service address without a user and password before its host or a `token` in its query.
fn scrub_url(url: &str) -> String {
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let base = match base.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = rest.split_once('/').map_or((rest, None), |(h, p)| (h, Some(p)));
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            match path {
                Some(p) => format!("{scheme}://{host}/{p}"),
                None => format!("{scheme}://{host}"),
            }
        }
        None => base.to_string(),
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.split_once('=').map_or(*p, |(k, _)| k).trim().eq_ignore_ascii_case("token"))
        .collect();
    if kept.is_empty() { base } else { format!("{base}?{}", kept.join("&")) }
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
    let Some(kind) = str_of(r, "type") else { return };
    let done = match kind {
        "CIMSimpleRenderer" => simple(r, what, style, report),
        "CIMUniqueValueRenderer" => unique(r, what, style, report),
        "CIMClassBreaksRenderer" => class_breaks(r, what, style, report),
        "CIMProportionalRenderer" => proportional(r, what, style, report),
        other => first_symbol(r, other, what, style, report),
    };
    if done {
        size_variable(r, what, style, report);
    } else {
        let mut symbol = Symbol::default_for(rustgis_data::GeometryType::Polygon, 0);
        symbol.size = Symbol::default_for(rustgis_data::GeometryType::Point, 0).size;
        style.line_strokes = Some(Vec::new());
        style.fill_layers = Some(Vec::new());
        style.renderer = Some(Renderer::Single(symbol));
    }
}

/// Makes `l` the layer's single symbol.
fn set_single(l: Look, style: &mut Style) {
    style.line_strokes = Some(l.strokes);
    style.fill_layers = Some(l.fills);
    style.renderer = Some(Renderer::Single(l.symbol));
}

/// A `CIMSimpleRenderer`. `false` for one whose symbol can't be read (noted by `look`).
fn simple(r: &Value, what: &str, style: &mut Style, report: &mut Report) -> bool {
    let Some(s) = r.get("symbol") else {
        report.skipped(what, "simple renderers with no symbol, drawn with a default symbol");
        return false;
    };
    let Some(l) = look(s, what, report) else { return false };
    set_single(l, style);
    true
}

/// A `CIMClassBreaksRenderer` as graduated classes, sorted by their upper bounds. `false` for one
/// with no field or no class to read.
fn class_breaks(r: &Value, what: &str, style: &mut Style, report: &mut Report) -> bool {
    let Some(field) = str_of(r, "field").map(str::trim).filter(|f| !f.is_empty()) else {
        report.skipped(what, "class breaks renderers with no field, drawn with a default symbol");
        return false;
    };
    if str_of(r, "classBreakType") == Some("UnclassedColor") {
        report.approximated(what, "an unclassed colour ramp, drawn as its classes");
    }
    if let Some(n) = str_of(r, "normalizationType").filter(|n| *n != "Nothing") {
        report.approximated(what, format!("values normalized ({}), drawn as they are", shown(n)));
    }
    let mut classes: Vec<(f64, Look, Option<String>)> = Vec::new();
    for b in r.get("breaks").and_then(Value::as_array).into_iter().flatten().take(MAX_CLASSES + 1) {
        if classes.len() >= MAX_CLASSES {
            report.skipped(what, format!("classes past the first {MAX_CLASSES}"));
            break;
        }
        let Some(upper) = num(b, "upperBound") else {
            report.skipped(what, "class breaks with no upper bound, left out");
            continue;
        };
        // A class switched off draws nothing, and its symbol isn't read.
        let l = if flag(b, "visible", true) {
            let Some(l) = b.get("symbol").and_then(|s| look(s, what, report)) else { continue };
            l
        } else {
            hidden()
        };
        let label = str_of(b, "label").map(str::trim).filter(|l| !l.is_empty()).map(str::to_string);
        classes.push((upper, l, label));
    }
    classes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(first) = classes.first().map(|c| c.0) else {
        report.skipped(what, "class breaks renderers with no class to read, drawn with a default symbol");
        return false;
    };
    let min = num(r, "minimumBreak").unwrap_or(first);
    let method = str_of(r, "classificationMethod").and_then(ClassMethod::from_name).unwrap_or(ClassMethod::Manual);
    let count = classes.len();
    let (mut breaks, mut symbols) = (Vec::new(), Vec::new());
    let (mut class_labels, mut class_strokes, mut class_fills) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for (i, (upper, l, label)) in classes.into_iter().enumerate() {
        if let Some(label) = label {
            class_labels.insert(i, label);
        }
        if !l.strokes.is_empty() {
            class_strokes.insert(i, l.strokes);
        }
        if !l.fills.is_empty() {
            class_fills.insert(i, l.fills);
        }
        breaks.push(upper);
        symbols.push(l.symbol);
    }
    style.line_strokes = Some(Vec::new());
    style.fill_layers = Some(Vec::new());
    style.renderer = Some(Renderer::Graduated {
        field: field.to_string(),
        breaks,
        symbols,
        ramp: Ramp::default(),
        method,
        min,
        classes: count,
        class_labels,
        class_strokes,
        class_fills,
    });
    true
}

/// A `CIMProportionalRenderer`: marker sizes from its smallest and largest symbols over its data
/// range. `false` for one with no field, no data range or no symbol.
fn proportional(r: &Value, what: &str, style: &mut Style, report: &mut Report) -> bool {
    let Some(field) = str_of(r, "field").map(str::trim).filter(|f| !f.is_empty()) else {
        report.skipped(what, "proportional renderers with no field, drawn with a default symbol");
        return false;
    };
    let (Some(min), Some(max)) = (num(r, "minDataValue"), num(r, "maxDataValue")) else {
        report.skipped(what, "proportional renderers with no data range, drawn with a default symbol");
        return false;
    };
    let mut read = |key: &str| r.get(key).and_then(|s| look(s, what, report));
    let (small, large, own) = (read("minSymbol"), read("maxSymbol"), read("symbol"));
    let (small_size, large_size) = (small.as_ref().map(|l| l.symbol.size), large.as_ref().map(|l| l.symbol.size));
    let Some(base) = own.or(large).or(small) else {
        report.skipped(what, "proportional renderers with no symbol to read, drawn with a default symbol");
        return false;
    };
    let (min_size, max_size) = (small_size.unwrap_or(base.symbol.size), large_size.unwrap_or(base.symbol.size));
    let symbol = base.symbol;
    style.line_strokes = Some(base.strokes);
    style.fill_layers = Some(base.fills);
    style.renderer = Some(Renderer::Proportional { field: field.to_string(), base: symbol, min_size, max_size, min, max });
    true
}

/// The most visual variables looked through on one renderer.
const MAX_VARIABLES: usize = 64;

/// A `CIMSizeVisualVariable` of the renderer `r` (already read into `style`). On a simple renderer
/// whose symbol is a marker it makes the sizes proportional; any other size variable is noted and
/// the symbols stay at one size.
fn size_variable(r: &Value, what: &str, style: &mut Style, report: &mut Report) {
    let Some(v) = r
        .get("visualVariables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_VARIABLES)
        .find(|v| typed(v, "CIMSizeVisualVariable").is_some())
    else {
        return;
    };
    let expressions: Vec<&str> = v
        .get("valueExpressionInfo")
        .and_then(|e| str_of(e, "expression"))
        .into_iter()
        .chain(str_of(v, "expression"))
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .collect();
    let field = expressions.iter().copied().find_map(expr::field);
    let sizes = num(v, "minSize").zip(num(v, "maxSize"));
    let values = num(v, "minValue").zip(num(v, "maxValue"));
    // A simple renderer's marker grows with the field (only a marker has a size to grow).
    let simple = str_of(r, "type") == Some("CIMSimpleRenderer");
    let grown = match (&field, sizes, values, style.renderer.as_ref()) {
        (Some(field), Some((min_size, max_size)), Some((min, max)), Some(Renderer::Single(base))) if simple && base.size > 0.0 => {
            Some(Renderer::Proportional { field: field.clone(), base: *base, min_size: pt96(min_size), max_size: pt96(max_size), min, max })
        }
        _ => None,
    };
    if grown.is_some() {
        style.renderer = grown;
        return;
    }
    let by = match (&field, expressions.first()) {
        (Some(field), _) => format!("by {}", shown(field)),
        (None, Some(e)) if *e == "$view.scale" => "with the scale".to_string(),
        (None, Some(e)) => format!("by {}", shown(e)),
        (None, None) => "by an expression".to_string(),
    };
    report.approximated(what, format!("symbol sizes that change {by}, drawn at one size"));
}

/// How many nodes of a renderer [`first_symbol`] looks at.
const MAX_SYMBOL_SEARCH: usize = 5000;

/// A renderer of a kind that isn't read (a heat map, dot density, chart...) drawn with the first
/// symbol in it (depth first). `false` when it holds none.
fn first_symbol(r: &Value, kind: &str, what: &str, style: &mut Style, report: &mut Report) -> bool {
    let mut stack = vec![r];
    let mut budget = MAX_SYMBOL_SEARCH;
    while let Some(node) = stack.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        // (A symbol that can't be read is only noted when it is the one drawn.)
        if symbol_of(node).is_some() && look(node, what, &mut Report::default()).is_some() {
            let Some(l) = look(node, what, report) else { break };
            report.approximated(what, format!("{} renderers, drawn with its first symbol", shown(kind)));
            set_single(l, style);
            return true;
        }
        // Pushed last-first, so the first child is looked at first.
        match node {
            Value::Array(items) => stack.extend(items.iter().rev()),
            Value::Object(map) => stack.extend(map.values().rev()),
            _ => {}
        }
    }
    report.skipped(what, format!("{} renderers, drawn with a default symbol", shown(kind)));
    false
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
