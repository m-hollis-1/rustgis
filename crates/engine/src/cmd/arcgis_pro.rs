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

use rustgis_geom::{Coord, Extent, merc_to_lonlat};
use rustgis_map::cim::{self, DocKind, Document, LayoutImport, MapImport, PictureSource, Report};
use rustgis_map::layout::{ElementKind, MAX_FRAME_SCALE, MIN_FRAME_SCALE};
use rustgis_map::qgis::NoteKind;
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
            None => add_map(s, m, dir, &mut report)?,
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

/// The files of `m`'s layers that are there to open, top first: a path and a table. (A file that
/// isn't there is left out of the map, so it is left out of the comparison too.)
fn layer_files(m: &MapImport, dir: Option<&str>) -> Vec<(String, Option<String>)> {
    m.layers
        .iter()
        .filter_map(|l| {
            let path = local_path(l.path.as_deref()?, dir).ok()?;
            std::path::Path::new(&path).exists().then(|| (absolute(&path), l.table.clone()))
        })
        .collect()
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
            let have: Vec<_> = pm.layers.iter().filter(|l| !l.graphics).collect();
            have.len() == want.len()
                && have.iter().zip(&want).all(|(l, (path, table))| {
                    let src = l.source.as_ref();
                    // A layer read whole keeps its file as its dataset's source.
                    let file = src.and_then(|s| s.path.as_deref()).or(l.dataset.source.as_deref());
                    file.is_some_and(|f| absolute(f) == *path) && src.is_none_or(|s| s.table == *table)
                })
        })
        .map(|pm| pm.name.clone())
}

/// Adds embedded map `m` as a new map, with its coordinate system and its layers, and returns its
/// name. A layer that can't be opened is noted and left out. The new map is active afterwards.
fn add_map(s: &mut Session, m: &MapImport, dir: Option<&str>, report: &mut Report) -> Result<String, String> {
    let added = s.run_part("map.new", &json!({"name": m.name}))?;
    let name = added["name"].as_str().unwrap_or(&m.name).to_string();
    // A template's map draws its layers alone: no basemap, no graticule.
    let map = s.map_mut();
    map.basemap = rustgis_map::Basemap::Blank;
    map.graticule = false;
    if let Some(wkid) = m.wkid {
        let code = match wkid {
            3857 | 102100 => "mercator".to_string(),
            // Pro draws a map in degrees as plate carrée.
            4326 => "equirectangular".to_string(),
            w => format!("EPSG:{w}"),
        };
        if let Err(e) = s.run_part("map.projection", &json!({"type": code})) {
            report.approximated(&name, format!("its coordinate system EPSG:{wkid} ({e}), drawn in Web Mercator"));
        }
    }
    // Bottom first, so each is added on top of the ones below it.
    for layer in m.layers.iter().rev() {
        let Some(path) = layer.path.as_deref() else { continue };
        let path = match local_path(path, dir) {
            Ok(p) => p,
            Err(e) => {
                report.skipped(&layer.name, format!("its data couldn't be opened ({e}), left out"));
                continue;
            }
        };
        let mut q = json!({"path": path, "name": layer.name});
        if let Some(t) = &layer.table {
            q["layer"] = json!(t);
        }
        let opened = s.run_part("layer.addData", &q);
        let id = match opened {
            Ok(r) => r["layer"].as_u64().and_then(|id| u32::try_from(id).ok()),
            Err(e) => {
                report.skipped(&layer.name, format!("its data couldn't be opened ({e}), left out"));
                continue;
            }
        };
        let Some(l) = id.and_then(|id| s.map_mut().layer_mut(id)) else { continue };
        l.visible = layer.visible;
        layer.style.apply(l);
        qgis::check_fields(l, &layer.name, report);
    }
    go_to_extent(s, m);
    Ok(name)
}

/// Shows the new map at its default extent, when it is in a system RustGIS converts and holds some
/// of the data, else at its data.
fn go_to_extent(s: &mut Session, m: &MapImport) {
    let data = s.map().full_extent();
    let saved = m.extent.and_then(|[x0, y0, x1, y1]| {
        let (a, b) = match m.wkid? {
            4326 => (Coord::new(x0, y0), Coord::new(x1, y1)),
            3857 | 102100 => (merc_to_lonlat(Coord::new(x0, y0)), merc_to_lonlat(Coord::new(x1, y1))),
            _ => return None,
        };
        let e = Extent::new(a.x.max(-180.0), a.y.max(-90.0), b.x.min(180.0), b.y.min(90.0));
        (!e.is_empty() && e.min_x.is_finite() && e.max_y.is_finite()).then_some(e)
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
