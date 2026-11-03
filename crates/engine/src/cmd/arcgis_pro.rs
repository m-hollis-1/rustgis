//! ArcGIS Pro layouts: `layout.import` hands a `.pagx` (a `CIMLayoutDocument`) to
//! [`import_layout`], which reads it with `rustgis_map::cim`, adds the maps it embeds (or reuses
//! ones an earlier import added), binds each map frame to a project map, reads the pictures it
//! keeps beside it, and adds the layout. `layout.inspect` says which map each frame shows, so the
//! app can ask about the frames the template leaves open before importing.
//!
//! Everything that can fail on the input (the document, the `maps` parameter, the pictures) is
//! read before the project changes. What doesn't come across, or comes across changed, is said in
//! the notes, as the QGIS importer says it.

use std::collections::HashMap;

use rustgis_geom::{Coord, Extent};
use rustgis_map::cim::{
    self, DocKind, Document, Entry, GroupImport, LayerImport, LayerKind, LayerSource, LayoutImport, MapImport, PictureSource, Report,
};
use rustgis_map::layout::{ElementKind, MAX_FRAME_SCALE, MIN_FRAME_SCALE};
use rustgis_map::qgis::{NoteKind, QGIS_SCALE};
use rustgis_map::source::{ServiceOrigin, SourceLayer};
use rustgis_map::symbology::{ImageRef, Marker};
use serde_json::{Value, json};

use super::{ps, qgis};
use crate::{CmdResult, CommandSpec as C, Session};

/// How far into a layout's text its type is looked for.
const TYPE_WITHIN: usize = 200;
const NOT_A_LAYOUT: &str = "not an ArcGIS Pro layout: the document isn't a CIMLayoutDocument";
/// The most of a name or path a note quotes.
const SHOWN: usize = 64;

pub fn commands() -> Vec<C> {
    vec![
        C::new("layout.inspect", "Inspect Layout File", "Home › Project", inspect).view().params(
            r#"{"path"?: "*.pagx", "text"?: a .pagx file's text} → {"frames": [{"name", "map": the embedded map it shows | null (the layout doesn't say)}], "maps": [embedded map names]}"#,
        ),
    ]
}

/// Whether a layout file is an ArcGIS Pro one: a `.pagx` `path`, or text that is a JSON object
/// naming `CIMLayoutDocument` near its start (Pro writes the type first).
pub(crate) fn is_pagx(text: &str, path: Option<&str>) -> bool {
    if path.is_some_and(|p| p.trim().to_ascii_lowercase().ends_with(".pagx")) {
        return true;
    }
    let t = text.trim_start_matches('\u{feff}').trim_start();
    let mut end = t.len().min(TYPE_WITHIN);
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    t.starts_with('{') && t.get(..end).is_some_and(|head| head.contains("\"CIMLayoutDocument\""))
}

/// The folder a layout's relative paths start from: `dir` (a blank one is none), else the folder
/// of `path` (`.` for a file named without one).
pub(crate) fn folder(p: &Value) -> Option<String> {
    ps(p, "dir").filter(|d| !d.trim().is_empty()).map(str::to_string).or_else(|| {
        ps(p, "path").and_then(|path| std::path::Path::new(path).parent()).map(|d| match d.to_string_lossy() {
            d if d.is_empty() => ".".to_string(),
            d => d.into_owned(),
        })
    })
}

/// A `.pagx`'s text as a layout document, or why it isn't one.
fn layout_document(text: &str) -> Result<Document, String> {
    let doc = Document::from_json(text)?;
    match doc.kind() {
        DocKind::Layout => Ok(doc),
        _ => Err(NOT_A_LAYOUT.into()),
    }
}

/// A `uRI` as it is compared: without `CIMPATH=` or a leading slash, `/` for `\`, lower case.
fn uri_key(uri: &str) -> String {
    let lower = uri.trim().replace('\\', "/").to_lowercase();
    lower.strip_prefix("cimpath=").unwrap_or(&lower).trim_start_matches('/').to_string()
}

fn shown(s: &str) -> String {
    s.chars().take(SHOWN).collect()
}

/// The embedded maps the layout's frames show, by `uRI` key: each read, or noted and left out.
fn embedded_maps(doc: &Document, imp: &LayoutImport, report: &mut Report) -> Vec<(String, MapImport)> {
    let mut maps = Vec::new();
    for uri in &imp.embedded_maps {
        match cim::read_map(doc, uri, report) {
            Ok(m) => maps.push((uri_key(uri), m)),
            // (A map the document lacks was noted by the frame that names it.)
            Err(e) if doc.resolve(uri).is_some() => report.skipped(&shown(uri), e),
            Err(_) => {}
        }
    }
    maps
}

fn inspect(_: &mut Session, p: &Value) -> CmdResult {
    let doc = layout_document(&super::layout::layout_text(p)?)?;
    let mut report = Report::default();
    let imp = cim::read_layout(&doc, &mut report)?;
    let maps = embedded_maps(&doc, &imp, &mut report);
    let name_of =
        |uri: &Option<String>| uri.as_deref().and_then(|u| maps.iter().find(|(k, _)| *k == uri_key(u))).map(|(_, m)| m.name.clone());
    let frames: Vec<Value> = imp.frames.iter().map(|b| json!({"name": b.name, "map": name_of(&b.map_uri)})).collect();
    Ok(json!({"frames": frames, "maps": maps.iter().map(|(_, m)| m.name.clone()).collect::<Vec<_>>()}))
}

/// The project maps `maps` names for frames (by frame name, in lower case): an object of map
/// names, each a map of the project.
fn chosen_maps(s: &Session, p: &Value) -> Result<HashMap<String, (String, String)>, String> {
    let mut chosen = HashMap::new();
    let given = match p.get("maps") {
        None | Some(Value::Null) => return Ok(chosen),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("maps is {frame name: map name}".into()),
    };
    for (frame, v) in given {
        let name = v.as_str().map(str::trim).ok_or_else(|| format!("maps: the map for {} must be a map's name", shown(frame)))?;
        let m = s
            .project
            .maps
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("maps: no map named {}", shown(name)))?;
        chosen.insert(frame.trim().to_lowercase(), (frame.clone(), m.name.clone()));
    }
    Ok(chosen)
}

/// `path` started from `dir` when it is relative and there is one.
fn resolve(path: &str, dir: Option<&str>) -> String {
    match dir {
        Some(d) if rustgis_map::qgis::is_relative(path) => std::path::Path::new(d).join(path.trim()).display().to_string(),
        _ => path.trim().to_string(),
    }
}

/// `path` as a file to read: started from `dir` when it is relative. A relative path with no
/// folder to start from (text given without `dir`) is refused rather than read against the
/// working directory, which has nothing to do with the layout.
fn local_path(path: &str, dir: Option<&str>) -> Result<String, String> {
    if dir.is_none() && rustgis_map::qgis::is_relative(path) {
        return Err("a relative path, and no folder to find it from".into());
    }
    Ok(resolve(path, dir))
}

pub(crate) fn import_layout(s: &mut Session, text: &str, dir: Option<&str>, p: &Value) -> CmdResult {
    let doc = Document::from_json(text)?;
    if doc.kind() != DocKind::Layout {
        // Text that only names the type near its start (a RustGIS layout called so) is still a
        // RustGIS layout; a .pagx file must be a Pro one.
        let pagx_file = ps(p, "text").is_none() && ps(p, "path").is_some_and(|path| is_pagx("", Some(path)));
        if !pagx_file {
            return super::layout::import(s, text);
        }
        return Err(NOT_A_LAYOUT.into());
    }
    let mut report = Report::default();
    let mut imp = cim::read_layout(&doc, &mut report)?;
    let maps = embedded_maps(&doc, &imp, &mut report);
    let chosen = chosen_maps(s, p)?;
    for (key, (frame, _)) in &chosen {
        if !imp.frames.iter().any(|b| b.name.trim().to_lowercase() == *key) {
            report.skipped("", format!("maps: no frame named {}", shown(frame)));
        }
    }
    read_pictures(&mut imp, dir, &mut report);
    convert_markers(&mut imp, &mut report);

    // The project changes from here.
    let before = (s.project.active_map, s.active_layer, s.table_layer);
    let mut bound: HashMap<String, String> = HashMap::new();
    let mut added: Vec<String> = Vec::new();
    for (key, m) in &maps {
        let name = match reusable(s, m, dir) {
            Some(name) => name,
            None => add_map(s, m, dir, None, &mut report)?.name,
        };
        if !added.contains(&name) {
            added.push(name.clone());
        }
        bound.insert(key.clone(), name);
    }
    (s.project.active_map, s.active_layer, s.table_layer) = before;
    bind_frames(s, &mut imp, &bound, &chosen, &mut report);

    let mut l = imp.layout;
    l.clamp();
    l.name = super::layout::unique_name(s, &l.name);
    let name = l.name.clone();
    s.project.layouts.push(l);
    let i = s.project.layouts.len() - 1;
    s.active_layout = Some(i);
    if !report.notes.is_empty() {
        let left = report.notes.iter().filter(|n| n.kind == NoteKind::Skipped).count();
        s.notify(format!("Imported layout {name}: {} things drawn differently, {left} left out", report.notes.len() - left), false);
    }
    let notes: Vec<Value> = report.notes.iter().map(qgis::note_json).collect();
    Ok(json!({"layout": i, "name": name, "maps": added, "notes": notes}))
}

/// Gives each map frame its project map: the embedded map it shows, else the one `maps` names
/// for it, else the active map (noted when the template doesn't name one). A frame that takes
/// its map's view (scale 0) takes that map's current centre, scale, bearing and pitch.
fn bind_frames(
    s: &Session,
    imp: &mut LayoutImport,
    bound: &HashMap<String, String>,
    chosen: &HashMap<String, (String, String)>,
    report: &mut Report,
) {
    let active = s.map().name.clone();
    for b in &imp.frames {
        let embedded = b.map_uri.as_deref().and_then(|u| bound.get(&uri_key(u)));
        let named = chosen.get(&b.name.trim().to_lowercase()).map(|(_, m)| m);
        let map = embedded.or(named).unwrap_or(&active).clone();
        let Some(el) = imp.layout.element_mut(b.element) else { continue };
        let w = el.w;
        let ElementKind::MapFrame { map: shows, center, scale, bearing, pitch, .. } = &mut el.kind else { continue };
        let mut current = false;
        if *scale == 0.0
            && let Some(m) = s.project.maps.iter().find(|m| m.name == map)
        {
            // As `layout.frameZoom` takes a view: its centre, scale, bearing and pitch.
            (*center, *scale) = current_view(s, m, w);
            (*bearing, *pitch) = (m.camera.bearing, m.camera.pitch);
            current = true;
        }
        *shows = map;
        if b.map_uri.is_none() && named.is_none() {
            let view = if current { " at its current view" } else { "" };
            report.approximated(&b.name, format!("shows {}{view} (the template doesn't say which map)", shown(shows)));
        }
    }
}

/// Map `m`'s view as a frame `w_mm` wide shows it, as `layout.frameZoom` takes it: its centre,
/// and the scale at which the frame is as wide as the map view.
fn current_view(s: &Session, m: &rustgis_map::Map, w_mm: f64) -> (Coord, f64) {
    let scale = s.view_size.0 * m.ground_res() / (w_mm.max(1.0) / 1000.0);
    (m.center_lonlat(), if scale.is_finite() { scale.clamp(MIN_FRAME_SCALE, MAX_FRAME_SCALE) } else { MIN_FRAME_SCALE })
}

/// A path as compared with the one a layer was opened from: absolute where it can be made so,
/// and in lower case on Windows, whose file names are compared regardless of case.
fn absolute(path: &str) -> String {
    let p = std::path::absolute(path).map_or_else(|_| path.to_string(), |a| a.display().to_string());
    if cfg!(windows) { p.to_lowercase() } else { p }
}

/// The files of `m`'s layers, top first, its groups opened up in place: a path (started from
/// `dir`) and a table. A layer on a file comes in opened, or broken with that file, so each is
/// compared, there or not. (A table listed among the layers becomes a project table.)
fn layer_files(m: &MapImport, dir: Option<&str>) -> Vec<(String, Option<String>)> {
    fn walk(entries: &[Entry], dir: Option<&str>, out: &mut Vec<(String, Option<String>)>) {
        for e in entries {
            match e {
                Entry::Group(g) => walk(&g.children, dir, out),
                Entry::Layer(l) => {
                    if let (false, LayerSource::File { path, table }) = (l.kind == LayerKind::Table, &l.source) {
                        out.push((absolute(&resolve(path, dir)), table.clone()));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&m.entries, dir, &mut out);
    out
}

/// The file a project layer reads: its source's (a broken layer's too), its raster's, or, for a
/// layer read whole, its dataset's.
fn layer_file(l: &rustgis_map::Layer) -> Option<&str> {
    l.source
        .as_ref()
        .and_then(|s| s.path.as_deref())
        .or_else(|| l.raster.as_ref().and_then(|r| r.path.as_deref()))
        .or(l.dataset.source.as_deref())
}

/// Whether a project map's name is `base`, or `base` numbered as `map.new` numbers a name that
/// is taken (`base2`, `base3`…): the map an earlier import added when another had the name.
fn named_as(name: &str, base: &str) -> bool {
    if name.eq_ignore_ascii_case(base) {
        return true;
    }
    let (Some(head), Some(tail)) = (name.get(..base.len()), name.get(base.len()..)) else { return false };
    head.eq_ignore_ascii_case(base) && tail.parse::<u32>().is_ok_and(|n| n >= 2) && tail.bytes().all(|b| b.is_ascii_digit())
}

/// A project map an earlier import added for `m`: of its name (or that name numbered), with its
/// layers' files in order.
fn reusable(s: &Session, m: &MapImport, dir: Option<&str>) -> Option<String> {
    let want = layer_files(m, dir);
    s.project
        .maps
        .iter()
        .find(|pm| {
            if !named_as(&pm.name, &m.name) {
                return false;
            }
            let have: Vec<_> = pm.layers.iter().filter(|l| !l.graphics && layer_file(l).is_some()).collect();
            have.len() == want.len()
                && have.iter().zip(&want).all(|(l, (path, table))| {
                    let file = layer_file(l);
                    file.is_some_and(|f| absolute(f) == *path) && l.source.as_ref().is_none_or(|s| s.table == *table)
                })
        })
        .map(|pm| pm.name.clone())
}

// ---- adding a map ------------------------------------------------------------------------------

/// What adding a map, or layers to one, brought in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Added {
    /// The project map's name.
    pub name: String,
    /// The layers added: opened, broken, and web layers drawn as tiles.
    pub layers: usize,
    /// Of those, the ones added broken.
    pub broken: usize,
}

/// How a map's coordinates (its camera's, its default extent's) become longitude and latitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Units {
    /// Longitude and latitude already.
    Degrees,
    /// In the project map's projection, which is the Pro map's.
    Projected,
    /// In a system RustGIS doesn't project: the view is left to the data.
    Unknown,
}

/// What an entry became in the project map.
enum Placed {
    Layer(u32),
    Group(u32),
    /// Nothing a group can hold: a web layer drawn as tiles, a table, a group that couldn't be made.
    Outside,
}

/// Where a broken layer's data was, for Repair and Retry.
#[derive(Default)]
struct Origin {
    path: Option<String>,
    table: Option<String>,
    service: Option<ServiceOrigin>,
}

/// An import's state while it adds entries.
struct Adding<'a> {
    dir: Option<&'a str>,
    /// The map's reference scale, which the layers whose symbols take it are given.
    reference_scale: Option<f64>,
    added: Added,
}

/// Adds `m` as a new map (its projection, groups, layers top to bottom, tables, view) and
/// returns what it added, the map's name first; the new map is active afterwards. `camera` is
/// the view Pro saved (`Document::view_camera`): `(x, y, scale)` in the map's coordinates.
pub(crate) fn add_map(
    s: &mut Session,
    m: &MapImport,
    dir: Option<&str>,
    camera: Option<(f64, f64, f64)>,
    report: &mut Report,
) -> Result<Added, String> {
    let made = s.run_part("map.new", &json!({"name": m.name}))?;
    let name = made["name"].as_str().unwrap_or(&m.name).to_string();
    // A Pro map draws its layers alone: no basemap, no graticule.
    let map = s.map_mut();
    map.basemap = rustgis_map::Basemap::Blank;
    map.graticule = false;
    let units = projection(s, m, &name, report);
    let mut added = add_entries(s, m, dir, None, report);
    added.name = name;
    go_to_view(s, m, units, camera);
    Ok(added)
}

/// Gives the active map `m`'s coordinate system, and says how `m`'s coordinates become
/// longitude and latitude. One RustGIS doesn't project is noted, and the map stays in Web Mercator.
fn projection(s: &mut Session, m: &MapImport, name: &str, report: &mut Report) -> Units {
    let Some(wkid) = m.wkid else { return Units::Unknown };
    if matches!(wkid, 3857 | 102100) {
        return match s.run_part("map.projection", &json!({"type": "mercator"})) {
            Ok(_) => Units::Projected,
            Err(_) => Units::Unknown,
        };
    }
    if let Some(g) = qgis::Geographic::of(&format!("EPSG:{wkid}")) {
        // Pro draws a map in degrees as plate carrée.
        match s.run_part("map.projection", &json!({"type": "equirectangular"})) {
            Err(e) => report.approximated(name, format!("its coordinate system EPSG:{wkid} ({e}), drawn in Web Mercator")),
            Ok(_) if !g.wgs84 || !g.greenwich => {
                let meridian = if g.greenwich { "" } else { ", its longitudes taken as from Greenwich" };
                report.approximated(name, format!("its coordinate system EPSG:{wkid}, drawn in Equirectangular on WGS 84{meridian}"));
            }
            Ok(_) => {}
        }
        // (A view in degrees from another meridian is left to the data.)
        return if g.greenwich { Units::Degrees } else { Units::Unknown };
    }
    // Pro writes Esri's own codes (a state plane in feet, say) with the EPSG ones.
    let mut first = None;
    for code in [format!("EPSG:{wkid}"), format!("ESRI:{wkid}")] {
        match s.run_part("map.projection", &json!({"type": code})) {
            Ok(_) => return Units::Projected,
            Err(e) => {
                first.get_or_insert(e);
            }
        }
    }
    let why = first.unwrap_or_default();
    report.approximated(name, format!("its coordinate system EPSG:{wkid} ({why}), drawn in Web Mercator"));
    Units::Unknown
}

/// Adds `m`'s entries to the active map (a `.lyrx`, or a `.mapx`'s layers into an existing map),
/// in `group` when it is one of the map's (above its layers), and its standalone tables.
pub(crate) fn add_entries(s: &mut Session, m: &MapImport, dir: Option<&str>, group: Option<u32>, report: &mut Report) -> Added {
    let mut a = Adding { dir, reference_scale: m.reference_scale, added: Added { name: s.map().name.clone(), ..Added::default() } };
    let into = group.and_then(|g| s.map().group(g).map(|x| (g, x.name.clone())));
    // The group's top layer before anything is added: what the entries go above.
    let top = into.as_ref().and_then(|(g, _)| s.map().group_layers(*g).first().copied());
    let placed = add_list(s, &m.entries, into.as_ref().map(|(_, n)| n.as_str()), &mut a, report);
    if let Some((g, name)) = &into {
        attach(s, *g, name, &placed, top, report);
    }
    add_tables(s, m, dir, report);
    a.added
}

/// Adds `entries` (top first) bottom first, so each lands on top of the ones below it, and says
/// what each became, top first. `within` names the group they are listed in, for notes.
fn add_list(s: &mut Session, entries: &[Entry], within: Option<&str>, a: &mut Adding, report: &mut Report) -> Vec<Placed> {
    let mut placed: Vec<Placed> = entries
        .iter()
        .rev()
        .map(|e| match e {
            Entry::Layer(l) => add_layer(s, l, within, a, report),
            Entry::Group(g) => add_group(s, g, a, report),
        })
        .collect();
    placed.reverse();
    placed
}

/// Runs one of the import's commands; a failure is noted against `what`, and the import goes on.
fn step(s: &mut Session, id: &str, p: &Value, what: &str, report: &mut Report) {
    if let Err(e) = s.run_part(id, p) {
        report.approximated(what, format!("{id} didn't apply ({e})"));
    }
}

/// Adds a group: its children first, then the group holding them, as Pro shows it.
fn add_group(s: &mut Session, g: &GroupImport, a: &mut Adding, report: &mut Report) -> Placed {
    let children = add_list(s, &g.children, Some(&g.name), a, report);
    let layers: Vec<u32> = children.iter().filter_map(|c| if let Placed::Layer(id) = c { Some(*id) } else { None }).collect();
    let name = if g.name.trim().is_empty() { "Group" } else { g.name.as_str() };
    let id = match s.run_part("group.add", &json!({"name": name, "layers": layers})) {
        Ok(r) => r["group"].as_u64().and_then(|id| u32::try_from(id).ok()),
        Err(e) => {
            report.skipped(name, format!("the group itself ({e}): its layers are listed outside it"));
            None
        }
    };
    let Some(id) = id else { return Placed::Outside };
    attach(s, id, name, &children, None, report);
    let mut set = json!({"group": id, "visible": g.visible});
    if let Some(t) = g.transparency {
        set["transparency"] = json!(t);
    }
    step(s, "group.set", &set, name, report);
    step(s, "group.expand", &json!({"group": id, "on": g.expanded}), name, report);
    if g.min_scale.is_some() || g.max_scale.is_some() {
        step(s, "group.scaleRange", &json!({"group": id, "min": g.min_scale, "max": g.max_scale}), name, report);
    }
    Placed::Group(id)
}

/// Puts what `children` (top first) became in `group`, in their order, above the layer `below`
/// (none: at the group's end). What can't join it stays outside, noted where it was added.
fn attach(s: &mut Session, group: u32, name: &str, children: &[Placed], below: Option<u32>, report: &mut Report) {
    let mut below = below;
    for c in children.iter().rev() {
        let before = below.map_or(Value::Null, |b| json!(b));
        match c {
            Placed::Layer(id) => {
                // (Moving a layer needs one active.)
                s.active_layer = Some(*id);
                match s.run_part("layer.group", &json!({"layer": id, "group": group, "before": before})) {
                    Ok(_) => below = Some(*id),
                    Err(e) => {
                        let what = s.map().layer(*id).map(|l| l.name.clone()).unwrap_or_default();
                        report.approximated(&what, format!("listed outside its group {} ({e})", shown(name)));
                    }
                }
            }
            Placed::Group(sub) => match s.run_part("group.setParent", &json!({"group": sub, "parent": group, "before": before})) {
                Ok(_) => {
                    if let Some(top) = s.map().group_layers(*sub).first() {
                        below = Some(*top);
                    }
                }
                Err(e) => {
                    let what = s.map().group(*sub).map(|g| g.name.clone()).unwrap_or_default();
                    report.approximated(&what, format!("listed outside its group {} ({e})", shown(name)));
                }
            },
            Placed::Outside => {}
        }
    }
}

/// Adds a layer from its source: opened, or broken with its origin when it can't be.
fn add_layer(s: &mut Session, l: &LayerImport, within: Option<&str>, a: &mut Adding, report: &mut Report) -> Placed {
    let name = l.name.as_str();
    let before = s.active_layer;
    let service = |url: &str, kind: &str, layer: &str| ServiceOrigin { url: url.to_string(), kind: kind.into(), layer: layer.to_string() };
    let (added, from) = match &l.source {
        LayerSource::File { path, table } => {
            let from = Origin { path: Some(resolve(path, a.dir)), table: table.clone(), service: None };
            let opened = local_path(path, a.dir).and_then(|path| {
                let mut q = json!({"path": path, "name": name});
                if let Some(t) = table {
                    q["layer"] = json!(t);
                }
                s.run_part("layer.addData", &q)
            });
            (opened, from)
        }
        LayerSource::FeatureService { url, layer } => (
            s.run_part("service.add", &json!({"url": url, "type": "arcgis", "layer": layer, "name": name})),
            Origin { service: Some(service(url, "arcgis", layer)), ..Origin::default() },
        ),
        LayerSource::MapService { url } => {
            // A map service's whole map; an image service's one picture.
            let image = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/').to_ascii_lowercase().ends_with("/imageserver");
            let layer = if image { "image" } else { "map" };
            (
                s.run_part("service.add", &json!({"url": url, "type": "arcgis", "layer": layer, "name": name})),
                Origin { service: Some(service(url, "arcgis", layer)), ..Origin::default() },
            )
        }
        LayerSource::Wms { url, layer } => (
            s.run_part("service.add", &json!({"url": url, "type": "wms", "layer": layer, "name": name})),
            Origin { service: Some(service(url, "wms", layer)), ..Origin::default() },
        ),
        LayerSource::VectorTiles { url } => {
            (s.run_part("map.addTiles", &json!({"url": style_url(url), "kind": "style", "name": name, "overlay": true})), Origin::default())
        }
        // (The reader said why.)
        LayerSource::Unopenable => return broken_layer(s, l, Origin::default(), "RustGIS can't open this kind of data".into(), a),
    };
    match added {
        Ok(r) => opened(s, l, &r, before, within, a, report),
        Err(e) => {
            report.skipped(name, format!("its data couldn't be opened ({e}); added broken, to repair or retry"));
            broken_layer(s, l, from, e, a)
        }
    }
}

/// The style a vector tile layer draws with: an ArcGIS vector tile service's default style, else
/// the address as it is (a style already).
fn style_url(url: &str) -> String {
    let (path, query) = url.split_once('?').map_or((url, None), |(p, q)| (p, Some(q)));
    let path = path.trim_end_matches('/');
    if !path.to_ascii_lowercase().ends_with("/vectortileserver") {
        return url.to_string();
    }
    match query {
        Some(q) => format!("{path}/resources/styles/root.json?{q}"),
        None => format!("{path}/resources/styles/root.json"),
    }
}

/// Settles what a layer's command answered `r`: a tile layer (outside any group), a project
/// table, or a map layer, given what Pro kept with the layer.
fn opened(
    s: &mut Session,
    l: &LayerImport,
    r: &Value,
    before: Option<u32>,
    within: Option<&str>,
    a: &mut Adding,
    report: &mut Report,
) -> Placed {
    if let Some(index) = r.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()) {
        tile_settings(s, index, l, report);
        if let Some(g) = within {
            report.approximated(&l.name, format!("a web layer, listed outside its group {}", shown(g)));
        }
        a.added.layers += 1;
        return Placed::Outside;
    }
    if r.get("table").is_some() {
        report.approximated(&l.name, "a table, added to the project's tables");
        return Placed::Outside;
    }
    let id =
        r.get("layer").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()).or(s.active_layer.filter(|id| Some(*id) != before));
    let Some(id) = id.filter(|id| s.map().layer(*id).is_some()) else {
        report.skipped(&l.name, "opened, but nothing was added to the map");
        return Placed::Outside;
    };
    settle(s, id, l, a, report);
    a.added.layers += 1;
    Placed::Layer(id)
}

/// Gives an opened layer what Pro kept with it: its visibility, style and expansion; a feature
/// layer its definition query (one that doesn't fit is noted and dropped, as the QGIS importer
/// does), the fields its style names checked, and the map's reference scale when its symbols
/// take it.
fn settle(s: &mut Session, id: u32, l: &LayerImport, a: &Adding, report: &mut Report) {
    let name = l.name.as_str();
    s.active_layer = Some(id);
    let Some(layer) = s.map_mut().layer_mut(id) else { return };
    layer.visible = l.visible;
    let raster = layer.is_raster();
    if raster {
        if let Some(why) = layer.raster.as_ref().and_then(|r| l.style.raster_misfit(r)) {
            report.skipped(name, why);
        }
        l.style.apply(layer);
    } else {
        l.style.apply(layer);
        if let Some(q) = &l.definition_query {
            match rustgis_data::query::parse(q, &layer.dataset) {
                Ok(_) => layer.definition_query = Some(q.clone()),
                Err(e) => report.skipped(name, format!("its definition query `{}` ({e}), dropped", shown(q))),
            }
        }
        qgis::check_fields(layer, name, report);
    }
    step(s, "layer.expand", &json!({"layer": id, "on": l.expanded}), name, report);
    if !raster
        && l.scale_symbols
        && let Some(scale) = a.reference_scale
    {
        step(s, "layer.referenceScale", &json!({"layer": id, "scale": scale}), name, report);
    }
}

/// Adds a layer that can't be opened as a broken one: its origin (never a password or user: the
/// reader scrubbed it), where its data was (for Repair and Retry), why, and its style, visibility,
/// expansion and definition query (unchecked: there are no fields yet; Retry and Repair check it).
fn broken_layer(s: &mut Session, l: &LayerImport, from: Origin, error: String, a: &mut Adding) -> Placed {
    let origin = if l.origin.trim().is_empty() { "an ArcGIS Pro layer whose source isn't known".to_string() } else { l.origin.clone() };
    let src = SourceLayer::broken(origin, from.path, from.table, from.service, error);
    let id = s.map_mut().add_streamed(l.name.clone(), src);
    // Active, as a layer just added is (the layer commands that follow work on it).
    s.active_layer = Some(id);
    let (lo, hi) = rustgis_map::REFERENCE_SCALES;
    if let Some(layer) = s.map_mut().layer_mut(id) {
        l.style.apply_all(layer);
        layer.visible = l.visible;
        layer.expanded = l.expanded;
        layer.definition_query = l.definition_query.clone();
        if l.scale_symbols
            && l.kind == LayerKind::Feature
            && let Some(scale) = a.reference_scale.filter(|r| (lo..=hi).contains(r))
        {
            layer.reference_scale = Some(scale);
        }
    }
    a.added.layers += 1;
    a.added.broken += 1;
    Placed::Layer(id)
}

/// Gives a web layer drawn as tiles the layer's visibility, opacity and blending mode, as
/// `qgis::web_settings` does. A tile layer has no scale range, so one is noted.
fn tile_settings(s: &mut Session, index: usize, l: &LayerImport, report: &mut Report) {
    let st = &l.style;
    if let Some(t) = s.map_mut().tiles.get_mut(index) {
        t.visible = l.visible;
        if let Some(blend) = st.blend {
            t.blend = blend;
        }
        if let Some(tr) = st.transparency.filter(|t| t.is_finite() && *t > 0.0) {
            t.opacity = (1.0 - tr / 100.0).clamp(0.0, 1.0);
        }
    }
    if st.min_scale.flatten().is_some() || st.max_scale.flatten().is_some() {
        report.approximated(&l.name, "its scale range: a web layer draws at every scale");
    }
}

/// Adds `m`'s standalone tables on files (a dataset with no geometry becomes a project table);
/// one that can't be opened, or isn't on a file, is noted.
fn add_tables(s: &mut Session, m: &MapImport, dir: Option<&str>, report: &mut Report) {
    for t in &m.tables {
        match &t.source {
            LayerSource::File { path, table } => {
                let opened = local_path(path, dir).and_then(|path| {
                    let mut q = json!({"path": path, "name": t.name});
                    if let Some(tb) = table {
                        q["layer"] = json!(tb);
                    }
                    s.run_part("layer.addData", &q)
                });
                if let Err(e) = opened {
                    report.skipped(&t.name, format!("the table couldn't be opened ({e}), left out"));
                }
            }
            // (The reader said why it can't be opened.)
            LayerSource::Unopenable => {}
            _ => report.skipped(&t.name, format!("a table on a service ({}), which RustGIS doesn't add", shown(&t.origin))),
        }
    }
}

/// Shows the new map at the view Pro saved (its centre and scale), else at its default extent
/// when that holds some of the data, else at its data.
fn go_to_view(s: &mut Session, m: &MapImport, units: Units, camera: Option<(f64, f64, f64)>) {
    let lonlat = |s: &Session, x: f64, y: f64| -> Option<Coord> {
        let c = match units {
            Units::Degrees => Coord::new(x.clamp(-180.0, 180.0), y.clamp(-90.0, 90.0)),
            Units::Projected => s.map().to_lonlat(Coord::new(x, y))?,
            Units::Unknown => return None,
        };
        (c.x.is_finite() && c.y.is_finite() && c.x.abs() <= 180.0 && c.y.abs() <= 90.0).then_some(c)
    };
    if let Some((x, y, scale)) = camera
        && let Some(c) = lonlat(s, x, y)
    {
        let mut q = json!({"lon": c.x, "lat": c.y});
        // Pro's scale is at 96 dots to the inch, as QGIS's is.
        if scale.is_finite() && scale > 0.0 {
            q["scale"] = json!(scale * QGIS_SCALE);
        }
        if s.run_part("map.goToXY", &q).is_ok() {
            return;
        }
    }
    let data = s.map().full_extent();
    let saved = m.extent.and_then(|[x0, y0, x1, y1]| {
        let mut e = Extent::EMPTY;
        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let c = lonlat(s, x, y)?;
            e = e.union(&Extent::new(c.x, c.y, c.x, c.y));
        }
        (!e.is_empty()).then_some(e)
    });
    let target = match saved {
        Some(e) if data.is_empty() || e.intersects(&data) => e,
        _ => data,
    };
    if !target.is_empty() {
        s.zoom_to(&target);
    }
}

// ---- pictures ----------------------------------------------------------------------------------

/// Reads the pictures into their elements, those the layout keeps beside it and those it embeds,
/// each checked and converted as `layout.addElement` would take it (a PNG or JPEG of at most
/// [`MAX_PICTURE`](super::graphics::MAX_PICTURE), a GIF or BMP converted to PNG); one that can't
/// be is noted and left empty with its `source`, to be relinked.
fn read_pictures(imp: &mut LayoutImport, dir: Option<&str>, report: &mut Report) {
    for (id, src) in &imp.pictures {
        let Some(el) = imp.layout.element_mut(*id) else { continue };
        let what = el.name.clone();
        let ElementKind::Picture { image, .. } = &mut el.kind else { continue };
        match src {
            PictureSource::Path(src) => match local_path(src, dir).and_then(|path| read_file(&path)).and_then(|b| as_png_or_jpeg(&b)) {
                Ok(b64) => *image = b64,
                Err(e) => report.skipped(&what, format!("its picture {} couldn't be read ({e}); relink it", shown(src))),
            },
            PictureSource::Embedded(b64) => match embedded_bytes(b64).and_then(|b| as_png_or_jpeg(&b)) {
                Ok(b64) => *image = b64,
                Err(e) => report.skipped(&what, format!("its embedded picture couldn't be used ({e}); relink it")),
            },
        }
    }
}

/// The bytes of embedded base64 `b64`, refused past [`MAX_PICTURE`](super::graphics::MAX_PICTURE).
fn embedded_bytes(b64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let limit = super::graphics::MAX_PICTURE;
    if b64.len() > limit / 3 * 4 + 4 {
        return Err(format!("larger than {} MB", limit >> 20));
    }
    base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| "not base64".to_string())
}

/// Converts the GIF and BMP markers of north arrows to PNG; one that can't be is noted and drawn
/// as the circle the reader left.
fn convert_markers(imp: &mut LayoutImport, report: &mut Report) {
    for (id, index, url) in &imp.pending_pictures {
        let Some(el) = imp.layout.element_mut(*id) else { continue };
        let what = el.name.clone();
        let ElementKind::NorthArrow { symbol, .. } = &mut el.kind else { continue };
        let Some(sym) = symbol.get_mut(*index) else { continue };
        let converted = data_bytes(url).and_then(|bytes| to_png(&bytes)).and_then(|png| {
            use base64::Engine;
            ImageRef::new(&base64::engine::general_purpose::STANDARD.encode(png)).ok_or_else(|| "too large once converted".to_string())
        });
        match converted {
            Ok(image) => sym.marker = Marker::Image(image),
            Err(e) => report.approximated(&what, format!("a GIF or BMP marker that couldn't be converted ({e}), drawn as a circle")),
        }
    }
}

/// The bytes of a `data:…;base64,…` URL (whitespace in its base64, as Pro writes it, is dropped).
fn data_bytes(url: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let (head, payload) = url.split_once(',').ok_or("not a data URL")?;
    if !head.to_ascii_lowercase().contains(";base64") {
        return Err("not base64".into());
    }
    base64::engine::general_purpose::STANDARD.decode(cim::base64_text(payload)).map_err(|_| "not base64".to_string())
}

/// A picture file's bytes as base64 PNG or JPEG: a PNG or JPEG as it is, a GIF or BMP converted.
fn as_png_or_jpeg(bytes: &[u8]) -> Result<String, String> {
    use base64::Engine;
    // Kept as it is only when it is a PNG or JPEG: the renderer may read other formats where the
    // `image` crate has them (natively), but a layout must draw the same everywhere.
    let png_or_jpeg = bytes.starts_with(b"\x89PNG") || bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    if png_or_jpeg && rustgis_render::picture_size(&b64).is_some() {
        return Ok(b64);
    }
    if bytes.starts_with(b"GIF8") || bytes.starts_with(b"BM") {
        let png = to_png(bytes)?;
        let limit = super::graphics::MAX_PICTURE;
        if png.len() > limit {
            return Err(format!("larger than {} MB once converted to PNG", limit >> 20));
        }
        return Ok(base64::engine::general_purpose::STANDARD.encode(png));
    }
    Err("not a PNG, JPEG, GIF or BMP picture".into())
}

/// Reads at most [`MAX_PICTURE`](super::graphics::MAX_PICTURE) bytes of `path`.
#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let limit = super::graphics::MAX_PICTURE;
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("larger than {} MB", limit >> 20));
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
fn read_file(_: &str) -> Result<Vec<u8>, String> {
    Err("the browser can't read files beside a layout; relink the picture".into())
}

/// A GIF or BMP picture as PNG: at most 4096 pixels each way.
#[cfg(not(target_arch = "wasm32"))]
fn to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Cursor;
    /// The widest and tallest a GIF or BMP picture may be to be converted.
    const MAX_SIDE: u32 = 4096;
    let format = match image::guess_format(bytes) {
        Ok(f @ (image::ImageFormat::Gif | image::ImageFormat::Bmp)) => f,
        _ => return Err("not a GIF or BMP picture".into()),
    };
    let (w, h) = image::ImageReader::with_format(Cursor::new(bytes), format).into_dimensions().map_err(|e| e.to_string())?;
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!("{w} × {h} pixels; at most {MAX_SIDE} × {MAX_SIDE} are converted"));
    }
    let img = image::ImageReader::with_format(Cursor::new(bytes), format).decode().map_err(|e| e.to_string())?;
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(png)
}

#[cfg(target_arch = "wasm32")]
fn to_png(_: &[u8]) -> Result<Vec<u8>, String> {
    Err("the browser can't convert GIF and BMP pictures".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1×1 GIF.
    const GIF: &str = "R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";

    #[test]
    fn data_urls_decode_base64_wrapped_by_newlines() {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(GIF).unwrap();
        let (a, b) = GIF.split_at(20);
        // Pro wraps its base64 with newlines; a URL the reader didn't clean is still read.
        let url = format!("data:image/gif;base64,{a}\n{b}\r\n");
        assert_eq!(data_bytes(&url).unwrap(), bytes);
        #[cfg(not(target_arch = "wasm32"))]
        assert!(to_png(&data_bytes(&url).unwrap()).is_ok());
        assert!(data_bytes("data:image/gif;base64,R0l GOD!").is_err());
        assert!(data_bytes("data:image/gif,R0lGOD").is_err());
    }
}
