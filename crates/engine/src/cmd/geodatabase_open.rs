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
    (c.remembered && !s.database_logins.is_rejected(&c.id) && has_user(&c.address)).then(|| (*c).clone())
}

/// Whether a `mssql://` address names a user (a SQL login, or a `DOMAIN\user` one): without one it
/// signs in as the Windows user, which the import never does unasked.
#[cfg(not(target_arch = "wasm32"))]
fn has_user(address: &str) -> bool {
    use rustgis_data::source::{msconn, pgconn};
    let Some((public, _)) = msconn::split_password(address) else { return false };
    let Some((_, rest)) = public.split_once("://") else { return false };
    pgconn::userinfo_end(rest).is_some_and(|at| at > 0)
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

/// Opens `layer.addDatabase {database: c.address, layer: table_name(g.table), name}` after
/// activating the connection's credential and logging in, as a new layer: the caller moves or
/// keeps it. A connection with no password it can use without asking is an error (the import
/// never asks). A password the server refuses is handled as Data Sources handles it (a saved one
/// rejected, a session one dropped, neither left active), so it isn't tried again.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn add_through(s: &mut Session, c: &Connection, g: &GeodatabaseOrigin, name: &str) -> Result<Value, String> {
    use rustgis_data::DataError;
    use rustgis_data::source::{duckdb, sqlserver};

    let duckdb::Database::Sqlserver(key) = duckdb::Database::parse(&c.address) else {
        return Err("not a SQL Server connection".into());
    };
    let Some((password, saved)) = super::connections::credential(s, c)? else {
        return Err("the connection has no usable password".into());
    };
    let secret = password.expose();
    super::connections::activate(s, c, secret);
    s.database_logins.select("sqlserver", &key, &c.id);
    // Logged in first, so a refusal is told from any other failure; the layer then reads through
    // the live connection this leaves.
    if let Err(e) = sqlserver::connect(&key, true) {
        let e = e.redact_secret(secret);
        if matches!(e, DataError::Login(_)) {
            if saved {
                s.database_logins.reject(&c.id);
            } else {
                s.database_logins.reject_session_password(c);
            }
            duckdb::forget_connection_password(&c.id);
            s.database_logins.drop_activation(&key);
        }
        return Err(e.to_string());
    }
    let params = json!({"database": c.address, "layer": table_name(&g.table), "name": name});
    s.run_part("layer.addDatabase", &params).map_err(|e| if secret.is_empty() { e } else { e.replace(secret, "***") })
}
