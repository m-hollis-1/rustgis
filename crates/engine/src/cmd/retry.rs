//! Retry: opens a broken layer's source again (a file that has turned up, a service now signed in
//! to), keeping the layer's place, symbology and labels.

use crate::{CmdResult, CommandSpec as C, Session};
use serde_json::{Value, json};

pub fn commands() -> Vec<C> {
    vec![C::new("layer.retry", "Retry", "Layers", retry).params(
        r#"{"layer"?: name|id | "all"?: true (every broken layer of the map)} → {"retried": n, "opened": m, "failed": [{"layer", "error"}]}"#,
    )]
}

/// How one broken layer's retry went.
enum Outcome {
    Opened,
    /// Opening off the frame (a file by its web address): it opens when it lands.
    Opening,
}

fn retry(s: &mut Session, p: &Value) -> CmdResult {
    let ids: Vec<u32> = if super::b(p, "all").unwrap_or(false) {
        s.map().layers.iter().filter(|l| broken(l)).map(|l| l.id).collect()
    } else if p.get("layer").is_some_and(|l| !l.is_null()) {
        vec![s.layer_id(p)?]
    } else {
        return Err("name a layer (\"layer\"), or retry every broken layer (\"all\": true)".into());
    };
    let (mut retried, mut opened, mut failed) = (0, 0, Vec::new());
    for id in ids {
        let Some(l) = s.map().layer(id) else { continue };
        let name = l.name.clone();
        if !broken(l) {
            s.notify(format!("{name} isn't broken: there's nothing to retry"), false);
            continue;
        }
        retried += 1;
        match retry_layer(s, id) {
            Ok(Outcome::Opened) => {
                opened += 1;
                check_query(s, id);
            }
            Ok(Outcome::Opening) => {}
            Err(error) => failed.push(json!({"layer": name, "error": error})),
        }
    }
    Ok(json!({"retried": retried, "opened": opened, "failed": failed}))
}

/// A broken layer's definition query was kept unchecked while it had no fields to check it
/// against: kept now when it fits the fields its source opened with, else cleared with a notice.
fn check_query(s: &mut Session, id: u32) {
    let Some(l) = s.map_mut().layer_mut(id) else { return };
    let dropped = l.definition_query.take_if(|q| rustgis_data::query::parse(q, &l.dataset).is_err());
    if let Some(q) = dropped {
        let name = l.name.clone();
        s.notify(format!("{name}: its definition query ({q}) doesn't fit its data's fields, so it was cleared"), false);
    }
}

fn broken(l: &rustgis_map::Layer) -> bool {
    l.source.as_ref().is_some_and(rustgis_map::source::SourceLayer::is_broken)
}

/// Opens the broken layer `id`'s source again: its service added again, else its file reopened.
fn retry_layer(s: &mut Session, id: u32) -> Result<Outcome, String> {
    let src = s.map().layer(id).and_then(|l| l.source.as_ref()).ok_or("the layer no longer exists")?;
    if let Some(service) = src.service.clone() {
        return retry_service(s, id, &service);
    }
    if src.path.is_none() {
        // (Repair Data Source isn't offered for it either: it has nothing to repair.)
        return Err("there's no file or service to read it from again: remove it".into());
    }
    let l = s.map_mut().layer_mut(id).ok_or("the layer no longer exists")?;
    let src = l.source.as_mut().ok_or("the layer no longer exists")?;
    src.reopen()?;
    if !src.is_open() {
        return Ok(Outcome::Opening);
    }
    // (Opened, it no longer has an origin.)
    let src = l.source.take().ok_or("the layer no longer exists")?;
    l.replace_source(src);
    Ok(Outcome::Opened)
}

/// Adds the service's layer again (`service.add`) and moves what it read into the broken layer,
/// which keeps its place and style.
fn retry_service(s: &mut Session, id: u32, service: &rustgis_map::source::ServiceOrigin) -> Result<Outcome, String> {
    // Adding a layer goes to it and makes it active; a retry leaves the view and the active
    // layer as they were.
    let (camera, active) = (s.map().camera, s.active_layer);
    let r = super::service::add(s, &json!({"url": service.url, "type": service.kind, "layer": service.layer}));
    s.map_mut().camera = camera;
    s.active_layer = active.filter(|a| s.map().layer(*a).is_some());
    let r = r?;
    let added = r.get("layer").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| *n != id);
    let raster = added.and_then(|n| s.map().layer(n)).map(|l| l.raster.is_some());
    let tiles = r.get("index").is_some_and(Value::is_u64);
    if raster == Some(true) || (added.is_none() && tiles) {
        // A web layer drawn from tiles, or a raster: it stays as its own, in place of the broken
        // one, which would add it again at every retry.
        let name = s.map().layer(id).map(|l| l.name.clone()).unwrap_or_default();
        super::layer::remove(s, &json!({"layer": id}))?;
        s.notify(format!("{name}: {} came back as a web layer or a raster, added in its place", service.url), false);
        return Ok(Outcome::Opened);
    }
    let Some(new_id) = added.filter(|_| raster == Some(false)) else {
        // Not a layer (a table, say): the broken layer stays, and a table that came back goes
        // again, so another retry doesn't add another copy.
        let table = r.get("table").is_some();
        if let Some(i) = r.get("id").and_then(Value::as_u64).and_then(|t| u32::try_from(t).ok()).filter(|_| table) {
            s.project.tables.retain(|t| t.id != i);
        }
        let what = if table { "a table" } else { "no layer" };
        return Err(format!("{}: '{}' came back as {what}, not a layer of features", service.url, service.layer));
    };
    // Taken out as Remove takes a layer out: the tile layers over it keep their place, its group
    // and the legends forget it.
    let m = s.map_mut();
    m.release_tiles(&[new_id]);
    m.set_group(new_id, None);
    let new = m.layer_index(new_id).map(|i| m.layers.remove(i)).ok_or("the added layer no longer exists")?;
    super::layer::legends_follow_layer(s, &new.name, None);
    let m = s.map_mut();
    let l = m.layer_mut(id).ok_or("the layer no longer exists")?;
    let name = l.name.clone();
    l.dataset = new.dataset;
    l.dataset.name = name;
    l.source = new.source;
    if let Some(src) = &mut l.source {
        src.origin = None;
    }
    l.selection.clear();
    l.touch_geometry();
    Ok(Outcome::Opened)
}
