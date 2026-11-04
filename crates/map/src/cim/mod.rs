//! ArcGIS Pro's CIM documents read as JSON. CIM (the Cartographic Information Model) is how Pro
//! stores a layout (`.pagx`), a map (`.mapx`), a layer (`.lyrx`) and a project (`.aprx`): each is
//! one JSON document of `{"type": "CIM…", …}` objects, and an `.aprx` is a zip of such documents
//! with `GISProject.json` as its root. This module only finds and hands out the JSON; the readers
//! for symbols, layouts and maps are built on it, and say in a [`Report`] what they can't carry
//! over.
//!
//! The input is hostile: text over [`MAX_JSON`] is refused before it is parsed, the parser has its
//! own nesting limit, a zip is read within the same budget, and every lookup is by key, so a
//! document missing what a reader looks for gives `None`, never a panic.

pub(crate) mod color;
pub(crate) mod dyntext;
pub(crate) mod expr;
pub(crate) mod frame;
pub(crate) mod labels;
pub mod layout;
pub mod map;
pub(crate) mod surround;
pub(crate) mod symbol;
pub(crate) mod units;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use serde_json::Value;

pub use crate::qgis::{Note, NoteKind, Report};
pub use layout::{FrameBinding, LayoutImport, PictureSource, read_layout, read_layout_def};
pub use map::{Entry, GroupImport, LayerImport, LayerKind, LayerSource, MapImport, TableImport, read_layers, read_map, read_map_def};
pub use symbol::base64_text;

/// The most JSON text a document (or all of a project's entries together) is read from.
pub const MAX_JSON: usize = 64 << 20;

/// The largest JSON entry of a project read besides its `GISProject.json`; a bigger one (or a
/// damaged one) is skipped, and what needed it says so.
pub const MAX_JSON_ENTRY: usize = 32 << 20;

/// Why a definition that was in a project's zip isn't there: damaged, or over [`MAX_JSON_ENTRY`].
pub const UNREADABLE_JSON: &str = "damaged, or larger than 32 MB";
/// Why a picture that was in a project's zip isn't there: damaged, or over [`MAX_PICTURE_ENTRY`].
pub const UNREADABLE_PICTURE: &str = "damaged, or larger than 16 MB";

/// The largest `.xml` entry of a project read for the CIM JSON Pro may keep under that name (see
/// [`Document::from_zip`]); a bigger one (a genuine ArcMap document, metadata) is skipped.
pub const MAX_XML_ENTRY: usize = 8 << 20;

/// The `.xml` entries of a project read in all, apart from [`MAX_JSON`]'s budget.
pub const MAX_XML_TOTAL: usize = 64 << 20;

/// The most picture bytes a project's zip is read for, all pictures together.
const MAX_PICTURE_TOTAL: usize = 64 << 20;
/// The most bytes of one picture kept.
const MAX_PICTURE_ENTRY: usize = 16 << 20;
/// The extensions of the zip entries kept as pictures.
const PICTURE_EXTENSIONS: [&str; 5] = [".png", ".jpg", ".jpeg", ".gif", ".bmp"];
/// The most project items, views and layer paths listed.
pub const MAX_ITEMS: usize = 1000;

/// The lists of a document's root that hold definitions with a `uRI` to resolve.
const DEFINITION_LISTS: [&str; 4] = ["mapDefinitions", "layerDefinitions", "elevationSurfaceLayerDefinitions", "tableDefinitions"];
/// The list of a document's root that holds its binaries (pictures, fonts).
const BINARIES: &str = "binaryReferences";
/// The prefix of a path to a definition inside the document or project.
const PATH_PREFIX: &str = "CIMPATH=";

/// What a document is, by its root's `type`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocKind {
    /// A `CIMLayoutDocument` (`.pagx`).
    Layout,
    /// A `CIMMapDocument` (`.mapx`).
    Map,
    /// A `CIMLayerDocument` (`.lyrx`).
    Layer,
    /// A `CIMGISProject` (the root of an `.aprx`).
    Project,
    /// Any other `type` (empty when the root has none).
    Other(String),
}

/// What a project item is, of the kinds that can be imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Map,
    Layout,
}

/// A map or layout of a project (an entry of `projectItems`).
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectItem {
    pub name: String,
    pub kind: ItemKind,
    /// Its `catalogPath`: the `CIMPATH=…` of its definition.
    pub uri: String,
    /// Saved by ArcMap, not CIM: `catalogPath` ends in `.xml` and no JSON is found there. (Pro
    /// keeps the `.xml` name of an item it has rewritten as CIM JSON.)
    pub arcmap: bool,
}

/// A CIM document's JSON, with its embedded definitions and binaries findable by `uRI`.
#[derive(Clone, Debug)]
pub struct Document {
    root: Value,
    /// Where each definition is in the root, by its lower-cased `uRI`: its list in
    /// [`DEFINITION_LISTS`] and its place in that list. (Indices, not copies: a document can be
    /// tens of megabytes.)
    definitions: HashMap<String, (usize, usize)>,
    /// The same for the entries of [`BINARIES`].
    binaries: HashMap<String, usize>,
    /// The other JSON entries of a project's zip, by lower-cased path.
    entries: HashMap<String, Value>,
    /// The pictures of a project's zip as base64 text, by lower-cased path.
    pictures: HashMap<String, String>,
    /// The JSON entries and pictures of a project's zip that couldn't be read (damaged, or over
    /// their cap), by lower-cased path: what needs one says why it isn't there.
    unreadable: HashSet<String>,
}

impl Document {
    /// Reads a `.pagx`, `.mapx` or `.lyrx`.
    pub fn from_json(text: &str) -> Result<Document, String> {
        Ok(Self::new(parse(text)?, HashMap::new(), HashMap::new()))
    }

    /// Reads an `.aprx`: `GISProject.json` is the root, read strictly (a damaged one is no
    /// project), and the other JSON entries are what its maps, layers and layouts are found in:
    /// each at most [`MAX_JSON_ENTRY`], within [`MAX_JSON`] with the root, and one that can't be
    /// read is skipped and remembered (see [`Document::unreadable`]), never failing the project.
    /// `.xml` ones are read too, as Pro keeps the `.xml` name of what came over from ArcMap after
    /// rewriting it as JSON: apart, within their own budget ([`MAX_XML_TOTAL`], [`MAX_XML_ENTRY`]
    /// each), and skipped the same way. An entry that isn't JSON is left out, since it isn't a
    /// definition.
    pub fn from_zip(bytes: &[u8]) -> Result<Document, String> {
        let ends = |name: &str, ext: &str| name.to_ascii_lowercase().ends_with(ext);
        let is_root = |name: &str| path_key(name) == "gisproject.json";
        let mut root = None;
        let mut used = 0usize;
        for (_, data) in rustgis_data::zip::read_matching_within(bytes, is_root, MAX_JSON)? {
            used = used.saturating_add(data.len());
            let Ok(text) = String::from_utf8(data) else { continue };
            root = Some(parse(&text)?);
        }
        let root = root.ok_or("the archive has no GISProject.json, so it isn't an ArcGIS Pro project")?;
        let other_json = |n: &str| ends(n, ".json") && !is_root(n);
        let (files, skipped) =
            rustgis_data::zip::read_matching_lenient_noting(bytes, other_json, MAX_JSON_ENTRY, MAX_JSON.saturating_sub(used))?;
        let xml = rustgis_data::zip::read_matching_lenient(bytes, |n| ends(n, ".xml"), MAX_XML_ENTRY, MAX_XML_TOTAL).unwrap_or_default();
        let mut unreadable: HashSet<String> = skipped.iter().map(|n| path_key(n)).collect();
        let mut entries = HashMap::new();
        for (name, data) in files {
            let Ok(text) = String::from_utf8(data) else { continue };
            if let Ok(v) = parse(&text) {
                entries.insert(path_key(&name), v);
            }
        }
        for (name, data) in xml {
            // (Only text that starts as JSON is parsed: most `.xml` entries are XML.)
            let Ok(text) = String::from_utf8(data) else { continue };
            if text.trim_start_matches('\u{feff}').trim_start().starts_with('{')
                && let Ok(v) = parse(&text)
            {
                entries.entry(path_key(&name)).or_insert(v);
            }
        }
        let (pictures, skipped) = pictures(bytes);
        unreadable.extend(skipped);
        let mut doc = Self::new(root, entries, pictures);
        doc.unreadable = unreadable;
        Ok(doc)
    }

    fn new(root: Value, entries: HashMap<String, Value>, pictures: HashMap<String, String>) -> Document {
        let mut definitions = HashMap::new();
        for (list, key) in DEFINITION_LISTS.iter().enumerate() {
            for (i, def) in root.get(key).and_then(Value::as_array).into_iter().flatten().enumerate() {
                if let Some(uri) = str_of(def, "uRI") {
                    // Of two with the same path, the first is the one found.
                    definitions.entry(path_key(uri)).or_insert((list, i));
                }
            }
        }
        let mut binaries = HashMap::new();
        for (i, b) in root.get(BINARIES).and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(uri) = str_of(b, "uRI") {
                binaries.entry(path_key(uri)).or_insert(i);
            }
        }
        Document { root, definitions, binaries, entries, pictures, unreadable: HashSet::new() }
    }

    /// Whether the entry a `CIMPATH=…` names was in the project's zip but couldn't be read from
    /// it (damaged, or over its cap), so it doesn't resolve.
    pub fn unreadable(&self, uri: &str) -> bool {
        self.unreadable.contains(&path_key(uri))
    }

    /// What the root's `type` says this is.
    pub fn kind(&self) -> DocKind {
        match str_of(&self.root, "type") {
            Some("CIMLayoutDocument") => DocKind::Layout,
            Some("CIMMapDocument") => DocKind::Map,
            Some("CIMLayerDocument") => DocKind::Layer,
            Some("CIMGISProject") => DocKind::Project,
            other => DocKind::Other(other.unwrap_or_default().to_string()),
        }
    }

    pub fn root(&self) -> &Value {
        &self.root
    }

    /// The definition a `CIMPATH=…` names (in any case): one of the root's own, or else a zip
    /// entry of that path.
    pub fn resolve(&self, uri: &str) -> Option<&Value> {
        let key = path_key(uri);
        if let Some(&(list, i)) = self.definitions.get(&key) {
            return self.root.get(DEFINITION_LISTS.get(list)?)?.get(i);
        }
        self.entries.get(&key)
    }

    /// The `data` (base64) of the binary a `CIMPATH=…` names: one of the root's own, or else a
    /// picture of the project's zip.
    pub fn binary(&self, uri: &str) -> Option<&str> {
        let key = path_key(uri);
        if let Some(&i) = self.binaries.get(&key) {
            return str_of(self.root.get(BINARIES)?.get(i)?, "data");
        }
        self.pictures.get(&key).map(String::as_str)
    }

    /// A project's maps and layouts (`projectItems` of `itemType` Map or Layout), in file order, at
    /// most [`MAX_ITEMS`]. Empty for a document that isn't a project.
    pub fn items(&self) -> Vec<ProjectItem> {
        let list = self.root.get("projectItems").and_then(Value::as_array).into_iter().flatten();
        list.filter_map(|item| {
            let kind = match str_of(item, "itemType")? {
                "Map" => ItemKind::Map,
                "Layout" => ItemKind::Layout,
                _ => return None,
            };
            let uri = str_of(item, "catalogPath")?.to_string();
            let arcmap = uri.trim().to_ascii_lowercase().ends_with(".xml") && self.resolve(&uri).is_none();
            Some(ProjectItem { name: str_of(item, "name").unwrap_or_default().to_string(), kind, uri, arcmap })
        })
        .take(MAX_ITEMS)
        .collect()
    }

    /// The `CIMMapView`s of a project's `views`, bounded.
    fn map_views(&self) -> impl Iterator<Item = &Value> {
        let views = self.root.get("views").and_then(Value::as_array).into_iter().flatten();
        views.take(MAX_ITEMS).filter(|v| str_of(v, "type") == Some("CIMMapView"))
    }

    /// The URIs of the maps whose panes were open when the project was saved, in order.
    pub fn open_maps(&self) -> Vec<String> {
        self.map_views().filter_map(|v| str_of(v, "viewableObjectPath")).map(str::to_string).collect()
    }

    /// The saved camera of the view on `map_uri`: `(x, y, scale)` in the map's spatial reference.
    pub fn view_camera(&self, map_uri: &str) -> Option<(f64, f64, f64)> {
        let camera = self.view_of(map_uri)?.get("camera")?;
        Some((num(camera, "x")?, num(camera, "y")?, num(camera, "scale")?))
    }

    /// The bearing of the saved view on `map_uri`, its camera's `heading` (see [`bearing_of`]);
    /// `what` names the map in a note.
    pub fn view_bearing(&self, map_uri: &str, what: &str, report: &mut Report) -> Option<f64> {
        bearing_of(self.view_of(map_uri)?.get("camera")?, "heading", false, what, report)
    }

    /// The `CIMMapView` on `map_uri`.
    fn view_of(&self, map_uri: &str) -> Option<&Value> {
        let key = path_key(map_uri);
        self.map_views().find(|v| str_of(v, "viewableObjectPath").is_some_and(|p| path_key(p) == key))
    }

    /// A `.mapx`'s own map: its `mapDefinition`.
    pub fn map_definition(&self) -> Option<&Value> {
        self.root.get("mapDefinition").filter(|d| d.is_object())
    }

    /// A `.lyrx`'s `layers` (a `CIMLayerDocument`), else empty.
    pub fn layer_uris(&self) -> Vec<String> {
        if !matches!(self.kind(), DocKind::Layer) {
            return Vec::new();
        }
        let layers = self.root.get("layers").and_then(Value::as_array).into_iter().flatten();
        layers.filter_map(Value::as_str).take(MAX_ITEMS).map(str::to_string).collect()
    }
}

/// The pictures of a project's zip as base64 text, by lower-cased path, and the lower-cased paths
/// of those that couldn't be read: one over [`MAX_PICTURE_ENTRY`], past what is left of
/// [`MAX_PICTURE_TOTAL`], or damaged is skipped alone (the layout that shows it says so).
fn pictures(zip: &[u8]) -> (HashMap<String, String>, Vec<String>) {
    use base64::Engine;
    let is_picture = |name: &str| {
        let lower = name.to_ascii_lowercase();
        PICTURE_EXTENSIONS.iter().any(|e| lower.ends_with(e))
    };
    let Ok((files, skipped)) = rustgis_data::zip::read_matching_lenient_noting(zip, is_picture, MAX_PICTURE_ENTRY, MAX_PICTURE_TOTAL)
    else {
        return (HashMap::new(), Vec::new());
    };
    let read = files.into_iter().map(|(name, data)| (path_key(&name), base64::engine::general_purpose::STANDARD.encode(data))).collect();
    (read, skipped.iter().map(|n| path_key(n)).collect())
}

/// The most a rotation may be, either way, and be believed (degrees).
const MAX_ROTATION: f64 = 360.0;

/// The bearing (degrees clockwise from north, in (-180, 180]) that the rotation at `k` of `v`
/// gives: as it is (a camera's `heading`), or turned the other way when `negate` (a map's
/// `defaultRotation`). `None` when there is none; one that isn't a number of at most
/// [`MAX_ROTATION`] degrees either way is noted (`what` names it) and drawn north up.
///
/// The sign: Pro's `CIMViewCamera.heading` is its camera's heading, the compass direction the
/// camera faces, clockwise from north (as the ArcGIS JavaScript API's `Camera.heading`, which
/// grows as the view turns clockwise). Looking straight down, that is the direction the top of
/// the view faces, which is what RustGIS's camera bearing is. A map's rotation turns the other
/// way (as the JavaScript API's `MapView.rotation` is the heading's negative): Pro writes a map's
/// `defaultRotation` as -13.8 where its view's camera `heading` is 13.8.
pub fn bearing_of(v: &Value, k: &str, negate: bool, what: &str, report: &mut Report) -> Option<f64> {
    let raw = v.get(k).filter(|r| !r.is_null())?;
    match raw.as_f64().filter(|d| d.is_finite() && d.abs() <= MAX_ROTATION) {
        Some(d) => Some(crate::camera::normalize_bearing(if negate { -d } else { d })),
        None => {
            let text: String = raw.to_string().chars().take(32).collect();
            report.approximated(what, format!("its rotation ({text}), which isn't an angle, drawn north up"));
            None
        }
    }
}

/// Parses `text` as a JSON object, once it is known to be of a size that can be read.
fn parse(text: &str) -> Result<Value, String> {
    if text.len() > MAX_JSON {
        return Err(format!("the document is {} MB; at most {} MB can be read", text.len() >> 20, MAX_JSON >> 20));
    }
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not a CIM document: {e}"))?;
    if v.is_object() { Ok(v) } else { Err("not a CIM document: its JSON isn't an object".into()) }
}

/// A path as it is compared: without `CIMPATH=` or a leading slash, with `/` for `\`, lower-cased.
/// (A `uRI` and the zip entry it names can differ in case.)
fn path_key(path: &str) -> String {
    // Lower-cased first, so `cimpath=` comes off in any case (as the engine's `uri_key`).
    let lower = path.trim().replace('\\', "/").to_lowercase();
    lower.strip_prefix(&PATH_PREFIX.to_lowercase()).unwrap_or(&lower).trim_start_matches('/').to_string()
}

// What the symbol, layout and map readers share: a key that is missing or of another type gives
// `None` (or the default) here, in one place.

/// The string at `k`.
#[allow(dead_code)] // used by the readers built on this module
pub(crate) fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)?.as_str()
}

/// The finite number at `k`.
#[allow(dead_code)]
pub(crate) fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k)?.as_f64().filter(|n| n.is_finite())
}

/// The boolean at `k`, or `default` when it is missing or not a boolean.
#[allow(dead_code)]
pub(crate) fn flag(v: &Value, k: &str, default: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(default)
}

/// `v` when its `type` is `ty`.
#[allow(dead_code)]
pub(crate) fn typed<'a>(v: &'a Value, ty: &str) -> Option<&'a Value> {
    (str_of(v, "type") == Some(ty)).then_some(v)
}
