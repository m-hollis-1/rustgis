//! Undoable replacement of linked local data without replacing the layer.

use crate::{CmdResult, CommandSpec as C, Session};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

pub fn commands() -> Vec<C> {
    vec![
        C::new("layer.repairSource", "Repair Data Source", "Data › Data Source", repair_source).roles(&[]).effects(crate::policy::Effects::NONE.with(crate::policy::Effect::Files)).params(
            r#"{"layer"?: name|id (default: the active layer), "path": "the file or folder (a Zarr store, a File Geodatabase) the layer's data moved to", "table"?: "the dataset in a GeoPackage, GDAL or DuckDB file, or a raster's subdataset (default: the one the layer reads now)"}"#),
        C::new("layer.repairFolder", "Repair Data Sources in a Folder", "Data › Data Source", repair_folder).roles(&[]).effects(crate::policy::Effects::NONE.with(crate::policy::Effect::Files)).params(
            r#"{"from": "the folder the broken layers' data was in", "to": "the folder it moved to"} → "repaired" (the names of the layers fixed, each to the file of the same name in the new folder) and "skipped" (each with its "layer" name and the "why" it wasn't fixed); fails when no broken layer had its data in the old folder, or when none of them could be repaired (the error says why for each)"#),
        C::new("map.repairTiles", "Repair Tile Archive", "Layers", repair_tiles).roles(&[]).effects(crate::policy::Effects::NONE.with(crate::policy::Effect::Files)).params(
            r#"{"name"?: text | "index"?: number (0 = top) | "id"?: catalog id, "path": "the .pmtiles or .mbtiles file the archive moved to"}"#),
    ]
}

#[cfg(not(target_arch = "wasm32"))]
fn replacement(p: &Value) -> Result<String, String> {
    let path = super::layer::s_req(p, "path")?.trim();
    if !rustgis_map::paths::is_local(path) {
        return Err(format!("{path}: give a file or folder on this computer (web addresses and databases aren't repaired here)"));
    }
    let path = rustgis_map::paths::local_path(path);
    let full = std::path::absolute(path).map_err(|e| format!("{path}: {e}"))?.display().to_string();
    if !std::path::Path::new(&full).exists() {
        return Err(format!("{full}: there's no such file or folder"));
    }
    Ok(full)
}

/// Whether data read as `info` can stand in for the layer `l`'s: the same geometry type, every
/// field its symbology, labels and time slider use, and an expression its classes read on the old
/// fields still reading on the new ones. `what` names the new data in the message (a file's path,
/// a connection's name).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fits(l: &rustgis_map::Layer, info: &rustgis_data::source::SourceInfo, what: &str) -> Result<(), String> {
    if l.dataset.geometry_type != rustgis_data::GeometryType::None && info.geometry_type != l.dataset.geometry_type {
        return Err(format!(
            "{what} holds {} features; {} draws {} features: choose a file of {} features",
            info.geometry_type.label(),
            l.name,
            l.dataset.geometry_type.label(),
            l.dataset.geometry_type.label()
        ));
    }
    let mut required: Vec<&str> = l.renderer.field().into_iter().collect();
    if l.labels.enabled {
        // A template requires each of its fields.
        for c in l.labels.in_order() {
            let fields = rustgis_map::symbology::template_fields(&c.field);
            if fields.is_empty() { required.push(&c.field) } else { required.extend(fields) }
        }
    }
    if let Some(time) = &l.time {
        required.push(&time.field);
    }
    required.retain(|f| l.dataset.fields.iter().any(|old| old.name.eq_ignore_ascii_case(f)));
    required.sort_unstable();
    required.dedup();
    let missing: Vec<&str> = required.into_iter().filter(|f| !info.fields.iter().any(|new| new.name.eq_ignore_ascii_case(f))).collect();
    if !missing.is_empty() {
        return Err(format!(
            "{what} has no field {}, which {}'s symbology, labels or time slider use: choose a file that has them",
            missing.join(", "),
            l.name
        ));
    }
    // Classes by an expression that reads on the old fields must read on the new ones.
    if let Some(e) = l.renderer.expression().filter(|e| rustgis_data::query::parse(e, &l.dataset).is_ok())
        && let Err(why) = rustgis_data::query::parse(e, &rustgis_data::Dataset::new("", info.geometry_type, info.fields.clone()))
    {
        return Err(format!(
            "{what} doesn't fit {}'s symbology, which classes by `{e}` ({why}): choose a file that has its fields",
            l.name
        ));
    }
    Ok(())
}

/// Reopens linked data after compatibility checks; undo restores the source and any reset label moves.
fn repair_source(s: &mut Session, p: &Value) -> CmdResult {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p);
        Err("the web app can't link to files on this computer: add the file again with Add Data".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = s.layer_id(p)?;
        let full = replacement(p)?;
        let table = super::ps(p, "table").map(str::trim).filter(|t| !t.is_empty());
        let l = s.map().layer(id).ok_or("the layer no longer exists")?;
        if l.linked_file().is_none() {
            return Err(format!(
                "{}'s data is kept in the project or read from a database or web address: there's no file to repair",
                l.name
            ));
        }
        if let Some(rl) = &l.raster {
            let new = if rl.time_zarr.is_some() { super::zarr::relink_following(s, rl, &full)? } else { rl.relinked(&full, table)? };
            let l = s.map_mut().layer_mut(id).ok_or("the layer no longer exists")?;
            l.raster = Some(new);
            l.selection.clear();
            l.touch_geometry();
        } else {
            let src = l.source.as_ref().ok_or("the layer has no linked source")?;
            let new = src.relinked(&full, table)?;
            if let Some(info) = new.info() {
                fits(l, info, &full)?;
            }
            // A broken layer's query was kept unchecked while it had no fields to check it against.
            let was_broken = src.is_broken();
            let l = s.map_mut().layer_mut(id).ok_or("the layer no longer exists")?;
            let moved = l.replace_source(new);
            let name = l.name.clone();
            let dropped = l.definition_query.take_if(|q| was_broken && rustgis_data::query::parse(q, &l.dataset).is_err());
            if let Some(q) = dropped {
                s.notify(format!("{name}: its definition query ({q}) doesn't fit {full}'s fields, so it was cleared"), false);
            }
            // A lock's feature ids and callout offsets belong to the old source too. The whole
            // lock goes, not just this layer's entries: while locked only the labels it names are
            // drawn, so dropping this layer's alone would hide its labels until the lock is
            // taken again. Cleared, every label is locked again where it is placed next.
            s.map_mut().label_lock = None;
            s.map_mut().label_lock_line_anchors.clear();
            if moved > 0 {
                s.notify(
                    format!("{moved} saved label move(s) reset in {name}: replacement data may reuse feature ids; undo restores them"),
                    false,
                );
            }
        }
        Ok(json!({"layer": id, "path": full, "table": s.map().layer(id).and_then(|l| l.linked_table())}))
    }
}

/// Repairs each broken file layer whose data was in `from` to the file of the same name in
/// `to`, as one undo step. A layer whose file isn't in `to`, or doesn't fit, is skipped with why;
/// when none is repaired that is an error (so no empty undo step).
fn repair_folder(s: &mut Session, p: &Value) -> CmdResult {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p);
        Err("the web app can't link to files on this computer: add the file again with Add Data".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use super::broken::{file_of, folder_of, same_folder};
        let from = super::layer::s_req(p, "from")?.trim();
        let to = super::layer::s_req(p, "to")?.trim();
        let candidates: Vec<(u32, String, String)> = s
            .map()
            .layers
            .iter()
            .filter_map(|l| {
                let src = l.source.as_ref().filter(|src| src.is_broken())?;
                let path = file_of(src)?;
                if !same_folder(folder_of(path)?, from) {
                    return None;
                }
                let file = path.rsplit(['/', '\\']).next().filter(|f| !f.is_empty())?;
                Some((l.id, l.name.clone(), file.to_string()))
            })
            .collect();
        if candidates.is_empty() {
            return Err(format!("no broken layer had its data in {from}"));
        }
        let (mut repaired, mut skipped) = (Vec::new(), Vec::new());
        for (id, name, file) in candidates {
            let target = std::path::Path::new(to).join(&file);
            if !target.exists() {
                skipped.push(json!({"layer": name, "why": format!("not in {to}")}));
                continue;
            }
            match s.run_part("layer.repairSource", &json!({"layer": id, "path": target.display().to_string()})) {
                Ok(_) => repaired.push(name),
                Err(why) => skipped.push(json!({"layer": name, "why": why})),
            }
        }
        if repaired.is_empty() {
            // Nothing changed: an Ok would leave an empty undo step and mark the project changed.
            let why: Vec<String> = skipped
                .iter()
                .map(|x| format!("{}: {}", x["layer"].as_str().unwrap_or_default(), x["why"].as_str().unwrap_or_default()))
                .collect();
            return Err(format!("nothing was repaired: {}", why.join("; ")));
        }
        Ok(json!({"repaired": repaired, "skipped": skipped}))
    }
}

fn repair_tiles(s: &mut Session, p: &Value) -> CmdResult {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p);
        Err("the web app can't link to files on this computer: add the file again with Add Data".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rustgis_render::rustgis_tiles::{cache, mbtiles, pmtiles};
        let i = super::tiles::find(s, p)?.ok_or("name, index or id is required")?;
        let t = s.map().tiles.get(i).ok_or("the tile layer no longer exists")?;
        if t.archive_file().is_none() {
            return Err(format!("{}: only a tile archive read from a file (PMTiles or MBTiles) can be repaired", t.name));
        }
        let full = replacement(p)?;
        let mut new = t.clone();
        new.url = full.clone();
        new.error = None;
        new.repair()?;
        let vector = match new.kind {
            rustgis_map::TileKind::MbTiles => mbtiles::info(&full)?.format.is_vector(),
            _ => pmtiles::info_blocking(&full, web_time::Duration::from_secs(10))?.tile_type.is_vector(),
        };
        if (!t.layers.is_empty() || !t.style_layers.is_empty()) && !vector {
            return Err(format!(
                "{full} holds raster tiles; {} draws vector tiles with its own layers or style: choose a vector tile archive",
                t.name
            ));
        }
        cache::forget(&t.url);
        let slot = s.map_mut().tiles.get_mut(i).ok_or("the tile layer no longer exists")?;
        *slot = new;
        Ok(json!({"index": i, "path": full}))
    }
}
