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
pub mod layout;
pub(crate) mod symbol;
pub(crate) mod units;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use serde_json::Value;

pub use crate::qgis::{Note, NoteKind, Report};
pub use layout::{FrameBinding, LayoutImport, PictureSource, read_layout};

/// The most JSON text a document (or all of a project's entries together) is read from.
pub const MAX_JSON: usize = 64 << 20;

/// The lists of a document's root that hold definitions with a `uRI` to resolve.
const DEFINITION_LISTS: [&str; 3] = ["mapDefinitions", "layerDefinitions", "elevationSurfaceLayerDefinitions"];
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
}

impl Document {
    /// Reads a `.pagx`, `.mapx` or `.lyrx`.
    pub fn from_json(text: &str) -> Result<Document, String> {
        Ok(Self::new(parse(text)?, HashMap::new()))
    }

    /// Reads an `.aprx`: `GISProject.json` is the root, and the other JSON entries are what its
    /// maps, layers and layouts are found in. An entry that isn't JSON is left out, since it
    /// isn't a definition.
    pub fn from_zip(bytes: &[u8]) -> Result<Document, String> {
        let files = rustgis_data::zip::read_matching_within(bytes, |name| name.to_ascii_lowercase().ends_with(".json"), MAX_JSON)?;
        let mut root = None;
        let mut entries = HashMap::new();
        for (name, data) in files {
            let key = path_key(&name);
            let Ok(text) = String::from_utf8(data) else { continue };
            if key == "gisproject.json" {
                root = Some(parse(&text)?);
            } else if let Ok(v) = parse(&text) {
                entries.insert(key, v);
            }
        }
        let root = root.ok_or("the archive has no GISProject.json, so it isn't an ArcGIS Pro project")?;
        Ok(Self::new(root, entries))
    }

    fn new(root: Value, entries: HashMap<String, Value>) -> Document {
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
        Document { root, definitions, binaries, entries }
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

    /// The `data` (base64) of the binary a `CIMPATH=…` names.
    pub fn binary(&self, uri: &str) -> Option<&str> {
        let i = *self.binaries.get(&path_key(uri))?;
        str_of(self.root.get(BINARIES)?.get(i)?, "data")
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
    let p = path.strip_prefix(PATH_PREFIX).unwrap_or(path).replace('\\', "/");
    p.trim_start_matches('/').to_lowercase()
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
