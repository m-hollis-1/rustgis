//! Retry: opens a broken layer's source again (a file that has turned up, a service now signed in
//! to), keeping the layer's place, symbology and labels.

use crate::{CmdResult, CommandSpec as C, Session};
use serde_json::{Value, json};

pub fn commands() -> Vec<C> {
    vec![
        C::new("layer.retry", "Retry", "Layers", retry).params(
            r#"{"layer"?: name|id | "all"?: true (every broken layer of the map)} → {"retried": n, "opened": m, "failed": [{"layer", "error"}]}"#,
        ),
        C::new("layer.relinkService", "Use Another Address", "Layers", relink).params(
            r#"{"layer": name|id of a broken service layer, "url": "the service's new address"} → the layer's id and whether it opened; the layer stays broken, at its old address, when the new one fails (the error says why)"#,
        ),
    ]
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
    // The saved connections (id -> name) that failed in this run: not tried again for the next layer.
    let mut dead = std::collections::HashMap::new();
    for id in ids {
        let Some(l) = s.map().layer(id) else { continue };
        let name = l.name.clone();
        if !broken(l) {
            s.notify(format!("{name} isn't broken: there's nothing to retry"), false);
            continue;
        }
        retried += 1;
        match retry_layer(s, id, &mut dead) {
            Ok(Outcome::Opened) => {
                opened += 1;
                check_query(s, id);
            }
            Ok(Outcome::Opening) => {}
            Err(error) => {
                // What the next diagnosis (Repair Source) goes by. The project keeps it, so
                // without the keys, tokens and passwords an error may quote.
                let error = super::history::redacted_text(&error);
                if let Some(src) = s.map_mut().layer_mut(id).and_then(|l| l.source.as_mut()) {
                    src.error = Some(error.clone());
                    src.reason = Some(error.clone());
                }
                failed.push(json!({"layer": name, "error": error}));
            }
        }
    }
    Ok(json!({"retried": retried, "opened": opened, "failed": failed}))
}

/// `layer.relinkService`: a broken service layer opened from another address (the same kind of
/// service and layer, moved), keeping its place and style. When the new address fails, the layer
/// stays broken at its old one, with the error as its reason.
fn relink(s: &mut Session, p: &Value) -> CmdResult {
    let id = s.layer_id(p)?;
    let url = super::ps(p, "url").map(str::trim).filter(|u| !u.is_empty()).ok_or("url is required: the service's new address")?;
    let l = s.map().layer(id).ok_or("the layer no longer exists")?;
    let old = l.source.as_ref().filter(|src| src.is_broken()).and_then(|src| src.service.clone());
    let Some(old) = old else { return Err(format!("{} isn't a broken service layer", l.name)) };
    let service = rustgis_map::source::ServiceOrigin { url: url.to_string(), kind: old.kind, layer: old.layer };
    match retry_service(s, id, &service) {
        Ok(outcome) => {
            let layer = s.map().layer(id).map(|_| id);
            if layer.is_some() {
                check_query(s, id);
            }
            Ok(json!({"layer": layer, "opened": matches!(outcome, Outcome::Opened)}))
        }
        Err(error) => {
            // As a failed retry: what the next diagnosis goes by, without the keys, tokens and
            // passwords the error may quote.
            let error = super::history::redacted_text(&error);
            if let Some(src) = s.map_mut().layer_mut(id).and_then(|l| l.source.as_mut()) {
                src.error = Some(error.clone());
                src.reason = Some(error.clone());
            }
            Err(error)
        }
    }
}

/// A broken layer's definition query was kept unchecked while it had no fields to check it
/// against: kept now when it fits the fields its source opened with, else cleared with a notice.
pub(super) fn check_query(s: &mut Session, id: u32) {
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

/// Opens the broken layer `id`'s source again: its service added again, its geodatabase table
/// through the one connection that opens quietly, else its file reopened.
fn retry_layer(s: &mut Session, id: u32, dead: &mut std::collections::HashMap<String, String>) -> Result<Outcome, String> {
    let src = s.map().layer(id).and_then(|l| l.source.as_ref()).ok_or("the layer no longer exists")?;
    if let Some(service) = src.service.clone() {
        return retry_service(s, id, &service);
    }
    if let Some(g) = src.geodatabase.clone() {
        return retry_geodatabase(s, id, &g, dead);
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
    adopt(s, id, new_id)?;
    Ok(Outcome::Opened)
}

/// Moves what the newly added layer `new_id` read into the broken layer `id`, which keeps its
/// place and style; the added layer is taken out as Remove takes a layer out.
pub(super) fn adopt(s: &mut Session, id: u32, new_id: u32) -> Result<(), String> {
    // The tile layers over it keep their place, its group and the legends forget it.
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
    Ok(())
}

/// Reads the broken geodatabase layer `id`'s table again through the one saved connection that
/// opens without asking; with none, the layer stays broken and Repair Source… is the way in.
#[cfg(not(target_arch = "wasm32"))]
fn retry_geodatabase(
    s: &mut Session,
    id: u32,
    g: &rustgis_map::source::GeodatabaseOrigin,
    dead: &mut std::collections::HashMap<String, String>,
) -> Result<Outcome, String> {
    use super::geodatabase_open::AddError;
    let Some(c) = super::geodatabase_open::quiet_connection(s, g) else {
        return Err(format!("needs a connection to the geodatabase on {}: use Repair Source…", g.server));
    };
    // A connection that failed for an earlier layer of this run isn't tried (and waited for) again.
    if let Some(named) = dead.get(&c.id) {
        return Err(format!("not tried again after {} failed", named.chars().take(60).collect::<String>()));
    }
    match super::geodatabase_open::open_into(s, id, &c, g, None, super::geodatabase_open::Windows::Refuse) {
        Ok(()) => Ok(Outcome::Opened),
        Err(AddError::Connect(e)) => {
            // Not tried (and waited for) again for the next layer of this run.
            dead.insert(c.id.clone(), c.name.clone());
            Err(e)
        }
        Err(AddError::Add(e)) => Err(e),
    }
}

#[cfg(target_arch = "wasm32")]
fn retry_geodatabase(
    _: &mut Session,
    _: u32,
    g: &rustgis_map::source::GeodatabaseOrigin,
    _: &mut std::collections::HashMap<String, String>,
) -> Result<Outcome, String> {
    Err(format!("needs a connection to the geodatabase on {}: use Repair Source…", g.server))
}
