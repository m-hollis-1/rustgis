//! Repair Geodatabase Layer: a broken layer ArcGIS Pro read from an Esri enterprise geodatabase
//! (SDE) is read again through a saved SQL Server connection, keeping the layer's place and style.
//! The connection signs in once; `all` repairs every layer of its geodatabase.

use crate::{CmdResult, CommandSpec as C, Session};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

pub fn commands() -> Vec<C> {
    vec![C::new("layer.repairGeodatabase", "Repair Geodatabase Layer", "Data › Data Source", repair_geodatabase).params(
        r#"{"layer"?: name|id (default: the active layer) | "all"?: true (every broken layer read from the connection's geodatabase), "connection": "the saved SQL Server connection's id or name to read the layer's table through; sign in with connection.login first when it has a user and no saved password; a Windows sign-in connection (no user) connects as you"} → "repaired" (the names of the layers fixed, each reading the table of the same name through the connection, at the Default version, keeping its style) and "skipped" (each with its "layer" name and the "why" it wasn't fixed); a named layer that can't be repaired is an error, and so is "all" when none could be (the error says why for each)"#,
    )]
}

#[cfg(target_arch = "wasm32")]
fn repair_geodatabase(s: &mut Session, p: &Value) -> CmdResult {
    let _ = (s, p);
    Err(crate::connections::DESKTOP_ONLY.into())
}

/// Why a layer wasn't repaired: the server refused the connection's password, signing in to the
/// connection failed otherwise (every layer through it would fail the same way), or anything
/// else about this one layer.
#[cfg(not(target_arch = "wasm32"))]
enum Fail {
    Refused(String),
    Connect(String),
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
    // A connection with no password to sign in with fails the same for every layer: said once. One
    // whose address names no user signs in as the Windows user (the user picked it), with none.
    if crate::geodatabase::has_user(&c.address) && super::connections::credential(s, &c)?.is_none() {
        return Err(format!("{} has no password to sign in with: sign in to it first (connection.login)", c.name));
    }
    let (mut repaired, mut skipped) = (Vec::new(), Vec::new());
    // Once signing in to the connection has failed it isn't tried (and waited for) again.
    let mut dead = false;
    for id in ids {
        let Some(name) = s.map().layer(id).map(|l| l.name.clone()) else { continue };
        let tried = if dead { Err(Fail::Why(format!("not tried again after {} failed", shown(&c.name)))) } else { repair_layer(s, &c, id) };
        match tried {
            Ok(()) => repaired.push(name),
            // The password is refused: nothing was repaired through it, nor will anything be.
            Err(Fail::Refused(why)) if repaired.is_empty() => return Err(why),
            Err(Fail::Refused(why) | Fail::Connect(why) | Fail::Why(why)) if !all => return Err(format!("{name}: {why}")),
            Err(fail) => {
                let why = match fail {
                    Fail::Refused(why) | Fail::Connect(why) => {
                        dead = true;
                        why
                    }
                    Fail::Why(why) => why,
                };
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

/// Reads the broken layer `id`'s table through `c` and moves what it read into the layer when it
/// fits the layer's style, labels and time slider.
#[cfg(not(target_arch = "wasm32"))]
fn repair_layer(s: &mut Session, c: &crate::connections::Connection, id: u32) -> Result<(), Fail> {
    use super::geodatabase_open::{AddError, Windows};
    let g = s.map().layer(id).and_then(|l| l.source.as_ref()).filter(|src| src.is_broken()).and_then(|src| src.geodatabase.clone());
    let g = g.ok_or_else(|| Fail::Why("isn't an enterprise geodatabase layer".into()))?;
    let (had_session, was_rejected) = (s.database_logins.session_password(&c.id).is_some(), s.database_logins.is_rejected(&c.id));
    let fit = |old: &rustgis_map::Layer, info: &rustgis_data::source::SourceInfo| super::repair::fits(old, info, &c.name);
    match super::geodatabase_open::open_into(s, id, c, &g, Some(&fit), Windows::SignIn) {
        Ok(()) => {
            // A query kept unchecked while the layer had no fields must fit the new table's.
            super::retry::check_query(s, id);
            Ok(())
        }
        Err(AddError::Add(why)) => Err(Fail::Why(super::history::redacted_text(&why))),
        Err(AddError::Connect(why)) => {
            let why = super::history::redacted_text(&why);
            // A refusal drops the session password or rejects the saved one.
            let refused = (had_session && s.database_logins.session_password(&c.id).is_none())
                || (!was_rejected && s.database_logins.is_rejected(&c.id));
            Err(if refused { Fail::Refused(why) } else { Fail::Connect(why) })
        }
    }
}

/// A connection's name as the first 60 characters of it.
#[cfg(not(target_arch = "wasm32"))]
fn shown(name: &str) -> String {
    name.chars().take(60).collect()
}
