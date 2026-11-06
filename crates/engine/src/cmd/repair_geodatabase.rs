//! Repair Geodatabase Layer: a broken layer ArcGIS Pro read from an Esri enterprise geodatabase
//! (SDE) is read again through a saved SQL Server connection, keeping the layer's place and style.
//! The connection signs in once; `all` repairs every layer of its geodatabase.

use crate::{CmdResult, CommandSpec as C, Session};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

pub fn commands() -> Vec<C> {
    vec![C::new("layer.repairGeodatabase", "Repair Geodatabase Layer", "Data › Data Source", repair_geodatabase).params(
        r#"{"layer"?: name|id (default: the active layer) | "all"?: true (every broken layer read from the connection's geodatabase), "connection": "the saved SQL Server connection's id or name to read the layer's table through; sign in with connection.login first when it has no saved password"} → "repaired" (the names of the layers fixed, each reading the table of the same name through the connection, at the Default version, keeping its style) and "skipped" (each with its "layer" name and the "why" it wasn't fixed); a named layer that can't be repaired is an error, and so is "all" when none could be (the error says why for each)"#,
    )]
}

#[cfg(target_arch = "wasm32")]
fn repair_geodatabase(s: &mut Session, p: &Value) -> CmdResult {
    let _ = (s, p);
    Err(crate::connections::DESKTOP_ONLY.into())
}

/// Why a layer wasn't repaired: the server refused the connection's password (every layer would
/// be refused the same), or anything else about this one layer.
#[cfg(not(target_arch = "wasm32"))]
enum Fail {
    Refused(String),
    Why(String),
}

#[cfg(not(target_arch = "wasm32"))]
fn repair_geodatabase(s: &mut Session, p: &Value) -> CmdResult {
    use crate::geodatabase::{Match, matches};
    let key = super::ps(p, "connection")
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or("connection is required: a saved SQL Server connection")?;
    let c = s.connections.find(key).cloned().ok_or_else(|| format!("no connection '{key}'"))?;
    if c.kind != "sqlserver" {
        return Err(format!("{} isn't a SQL Server connection", c.name));
    }
    let all = super::b(p, "all").unwrap_or(false);
    let origin = |l: &rustgis_map::Layer| l.source.as_ref().filter(|src| src.is_broken()).and_then(|src| src.geodatabase.clone());
    let ids: Vec<u32> = if all {
        let ids: Vec<u32> =
            s.map().layers.iter().filter(|l| origin(l).is_some_and(|g| matches(&g, &c.address) == Match::Exact)).map(|l| l.id).collect();
        if ids.is_empty() {
            return Err(format!("no broken layer of this map was read from the geodatabase {} reaches", c.name));
        }
        ids
    } else {
        let id = s.layer_id(p)?;
        let l = s.map().layer(id).ok_or("the layer no longer exists")?;
        let Some(g) = origin(l) else { return Err(format!("{} isn't an enterprise geodatabase layer", l.name)) };
        if matches(&g, &c.address) == Match::No {
            return Err(format!(
                "{} isn't a connection to the geodatabase {} was read from ({} / {})",
                c.name, l.name, g.server, g.database
            ));
        }
        vec![id]
    };
    // A connection with no password to sign in with fails the same for every layer: said once.
    if super::connections::credential(s, &c)?.is_none() {
        return Err(format!("{} has no password to sign in with: sign in to it first (connection.login)", c.name));
    }
    let (mut repaired, mut skipped) = (Vec::new(), Vec::new());
    for id in ids {
        let Some(name) = s.map().layer(id).map(|l| l.name.clone()) else { continue };
        match repair_layer(s, &c, id) {
            Ok(()) => repaired.push(name),
            // The password is refused: nothing was repaired through it, nor will anything be.
            Err(Fail::Refused(why)) if repaired.is_empty() => return Err(why),
            Err(Fail::Refused(why) | Fail::Why(why)) => {
                if !all {
                    return Err(format!("{name}: {why}"));
                }
                skipped.push(json!({"layer": name, "why": why}));
            }
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

/// Reads the broken layer `id`'s table through `c` as a new layer, and moves what it read into
/// the layer when it fits the layer's style, labels and time slider. The added layer never stays.
#[cfg(not(target_arch = "wasm32"))]
fn repair_layer(s: &mut Session, c: &crate::connections::Connection, id: u32) -> Result<(), Fail> {
    let (name, g) = {
        let l = s.map().layer(id).ok_or_else(|| Fail::Why("the layer no longer exists".into()))?;
        let g = l.source.as_ref().filter(|src| src.is_broken()).and_then(|src| src.geodatabase.clone());
        (l.name.clone(), g.ok_or_else(|| Fail::Why("isn't an enterprise geodatabase layer".into()))?)
    };
    // Adding a layer goes to it and makes it active; a repair leaves the view and the active
    // layer as they were. A registered table that came back stays out of the project's.
    let (camera, active) = (s.map().camera, s.active_layer);
    let tables: Vec<_> = s.project.tables.iter().map(|t| t.id).collect();
    let (had_session, was_rejected) = (s.database_logins.session_password(&c.id).is_some(), s.database_logins.is_rejected(&c.id));
    let r = super::geodatabase_open::add_through(s, c, &g, &name);
    let refused = r.is_err()
        && ((had_session && s.database_logins.session_password(&c.id).is_none())
            || (!was_rejected && s.database_logins.is_rejected(&c.id)));
    let done = finish(s, c, id, &name, &g, r.map_err(|e| super::history::redacted_text(&e)), &tables);
    s.map_mut().camera = camera;
    s.active_layer = active.filter(|a| s.map().layer(*a).is_some());
    done.map_err(|why| if refused { Fail::Refused(why) } else { Fail::Why(why) })
}

/// What follows adding the layer: the fit check, then the move into the broken layer.
#[cfg(not(target_arch = "wasm32"))]
fn finish(
    s: &mut Session,
    c: &crate::connections::Connection,
    id: u32,
    name: &str,
    g: &rustgis_map::source::GeodatabaseOrigin,
    r: Result<Value, String>,
    tables: &[u32],
) -> Result<(), String> {
    let r = r?;
    let mut added: Vec<u32> = r
        .get("layers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|n| u32::try_from(n).ok())
        .collect();
    added.retain(|n| *n != id);
    s.project.tables.retain(|t| tables.contains(&t.id));
    let Some((&new_id, extra)) = added.split_first() else {
        return Err(format!("'{}' came back as no layer of features", g.table));
    };
    // A table of mixed geometry types comes as a layer per type: the first takes the broken
    // layer's place.
    for n in extra {
        super::layer::remove(s, &json!({"layer": n}))?;
    }
    let fit = {
        let m = s.map();
        let (Some(old), Some(new)) = (m.layer(id), m.layer(new_id)) else { return Err("the added layer no longer exists".into()) };
        new.source.as_ref().and_then(|src| src.info()).map_or(Ok(()), |info| super::repair::fits(old, info, &c.name))
    };
    if let Err(why) = fit {
        super::layer::remove(s, &json!({"layer": new_id}))?;
        return Err(why);
    }
    super::retry::adopt(s, id, new_id)?;
    super::retry::check_query(s, id);
    if !g.is_default_version() {
        let version = g.version.as_deref().unwrap_or_default();
        s.notify(format!("{name}: read at the Default version, not {version}"), false);
    }
    Ok(())
}
