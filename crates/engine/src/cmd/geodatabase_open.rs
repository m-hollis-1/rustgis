//! Opening a layer ArcGIS Pro read from an Esri enterprise geodatabase (SDE) through a saved SQL
//! Server connection: the Pro import opens one quietly only through the one connection that is an
//! exact match and logs in by itself (a remembered SQL login); otherwise the layer comes in broken
//! with its origin, for Repair Source… to connect.

#[cfg(not(target_arch = "wasm32"))]
use rustgis_map::source::GeodatabaseOrigin;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::{Value, json};

#[cfg(not(target_arch = "wasm32"))]
use crate::Session;
#[cfg(not(target_arch = "wasm32"))]
use crate::connections::Connection;

/// The one saved connection an origin may open through without asking: exactly one exact match,
/// `remembered`, not rejected, and not a Windows sign-in (no user in its address). None otherwise:
/// several are the user's choice, none is Repair Source…'s.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn quiet_connection(s: &Session, g: &GeodatabaseOrigin) -> Option<Connection> {
    let exact = s.connections.matching_geodatabase(g);
    // Two exact matches are two ways in: which one is the user's choice, even if only one would
    // open quietly.
    let [c] = exact.as_slice() else { return None };
    (c.remembered && !s.database_logins.is_rejected(&c.id) && crate::geodatabase::has_user(&c.address)).then(|| (*c).clone())
}

/// The `layer` name `layer.addDatabase` gets for a Pro dataset: a 3-part name loses its first part
/// (the database: "utility.sde.Mains" → "sde.Mains"); a shorter one is kept as it is.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn table_name(dataset: &str) -> String {
    let dataset = dataset.trim();
    match dataset.split_once('.') {
        Some((_, rest)) if rest.contains('.') => rest.to_string(),
        _ => dataset.to_string(),
    }
}

/// Why `add_through` failed: signing in to the connection (every layer through it would fail the
/// same way: no password, an unreachable server, a refused login), or adding this one table.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub(super) enum AddError {
    Connect(String),
    Add(String),
}

#[cfg(not(target_arch = "wasm32"))]
impl AddError {
    pub(super) fn into_message(self) -> String {
        match self {
            AddError::Connect(e) | AddError::Add(e) => e,
        }
    }
}

/// How `add_through` may sign in to a connection whose address names no user.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Windows {
    /// Never as the Windows user: such a connection has no password, so it is an error (the
    /// import and Retry, which never sign in unasked).
    Refuse,
    /// As the Windows user, with no password: the user picked the connection (Repair Source…).
    SignIn,
}

/// Opens `layer.addDatabase {database: c.address, layer: table_name(g.table), name}` after
/// activating the connection's credential and logging in, as a new layer: the caller moves or
/// keeps it. A connection with no password it can use without asking is an error (the import
/// never asks), except that a connection with no user signs in as the Windows user when `windows`
/// allows it. A password the server refuses is handled as Data Sources handles it (a saved one
/// rejected, a session one dropped, neither left active), so it isn't tried again.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn add_through(s: &mut Session, c: &Connection, g: &GeodatabaseOrigin, name: &str, windows: Windows) -> Result<Value, AddError> {
    use rustgis_data::DataError;
    use rustgis_data::source::{duckdb, sqlserver};

    let duckdb::Database::Sqlserver(key) = duckdb::Database::parse(&c.address) else {
        return Err(AddError::Connect("not a SQL Server connection".into()));
    };
    let password = if windows == Windows::SignIn && !crate::geodatabase::has_user(&c.address) {
        None
    } else {
        match super::connections::credential(s, c).map_err(AddError::Connect)? {
            Some(found) => Some(found),
            None => return Err(AddError::Connect("the connection has no usable password".into())),
        }
    };
    let secret = password.as_ref().map_or("", |(p, _)| p.expose());
    let saved = password.as_ref().is_some_and(|(_, saved)| *saved);
    if password.is_some() {
        super::connections::activate(s, c, secret);
    }
    s.database_logins.select("sqlserver", &key, &c.id);
    // Logged in first, so a refusal is told from any other failure. The connection is held until
    // the layer is added, so `layer.addDatabase` reads through it rather than logging in again.
    let _live = match sqlserver::connect(&key, true) {
        Ok(conn) => conn,
        Err(e) => {
            let e = e.redact_secret(secret);
            if password.is_some() && matches!(e, DataError::Login(_)) {
                if saved {
                    s.database_logins.reject(&c.id);
                } else {
                    s.database_logins.reject_session_password(c);
                }
                duckdb::forget_connection_password(&c.id);
                s.database_logins.drop_activation(&key);
            }
            return Err(AddError::Connect(e.to_string()));
        }
    };
    let params = json!({"database": c.address, "layer": table_name(&g.table), "name": name});
    s.run_part("layer.addDatabase", &params).map_err(|e| AddError::Add(if secret.is_empty() { e } else { e.replace(secret, "***") }))
}

/// Whether the data a layer's table was read as can stand in for the broken layer.
#[cfg(not(target_arch = "wasm32"))]
pub(super) type Fit<'a> = &'a dyn Fn(&rustgis_map::Layer, &rustgis_data::source::SourceInfo) -> Result<(), String>;

/// Reads the broken layer `id`'s table through `c` (`add_through`) and moves what it read into the
/// layer, which keeps its place and style (`retry::adopt`). `fit` is asked first whether the new
/// data suits the layer: when it doesn't, the added layer is taken out again and its reason is the
/// error. A table of mixed geometry types comes as a layer per type: the first takes the layer's
/// place. The view and the active layer are as they were, whatever the outcome.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn open_into(
    s: &mut Session,
    id: u32,
    c: &Connection,
    g: &GeodatabaseOrigin,
    fit: Option<Fit>,
    windows: Windows,
) -> Result<(), AddError> {
    let name = s.map().layer(id).map(|l| l.name.clone()).ok_or_else(|| AddError::Add("the layer no longer exists".into()))?;
    // Adding a layer goes to it and makes it active. A registered table that came back stays out
    // of the project's.
    let (camera, active) = (s.map().camera, s.active_layer);
    let tables: Vec<_> = s.project.tables.iter().map(|t| t.id).collect();
    let done = match add_through(s, c, g, &name, windows) {
        Ok(r) => take_in(s, id, &name, g, &r, &tables, fit).map_err(AddError::Add),
        Err(e) => Err(e),
    };
    s.map_mut().camera = camera;
    s.active_layer = active.filter(|a| s.map().layer(*a).is_some());
    done
}

/// What follows adding the layer `r` answered: the fit check, then the move into layer `id`.
#[cfg(not(target_arch = "wasm32"))]
fn take_in(s: &mut Session, id: u32, name: &str, g: &GeodatabaseOrigin, r: &Value, tables: &[u32], fit: Option<Fit>) -> Result<(), String> {
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
        return Err(format!("{}: '{}' came back as no layer of features", g.server, g.table));
    };
    for n in extra {
        super::layer::remove(s, &json!({"layer": n}))?;
    }
    if let Some(fit) = fit {
        let checked = {
            let m = s.map();
            let (Some(old), Some(new)) = (m.layer(id), m.layer(new_id)) else { return Err("the added layer no longer exists".into()) };
            new.source.as_ref().and_then(|src| src.info()).map_or(Ok(()), |info| fit(old, info))
        };
        if let Err(why) = checked {
            super::layer::remove(s, &json!({"layer": new_id}))?;
            return Err(why);
        }
    }
    super::retry::adopt(s, id, new_id)?;
    if !g.is_default_version() {
        let version = g.version.as_deref().unwrap_or_default();
        s.notify(format!("{name}: read at the Default version, not {version}"), false);
    }
    Ok(())
}
