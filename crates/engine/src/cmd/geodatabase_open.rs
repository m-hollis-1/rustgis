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
pub(crate) fn table_name(dataset: &str) -> String {
    let dataset = dataset.trim();
    match dataset.split_once('.') {
        Some((_, rest)) if rest.contains('.') => rest.to_string(),
        _ => dataset.to_string(),
    }
}

/// Opens `layer.addDatabase {database: c.address, layer: table_name(g.table), name}` after
/// activating the connection's credential, as a new layer: the caller moves or keeps it. A
/// connection with no password it can use without asking is an error (the import never asks).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn add_through(s: &mut Session, c: &Connection, g: &GeodatabaseOrigin, name: &str) -> Result<Value, String> {
    let Some((password, _)) = super::connections::credential(s, c)? else {
        return Err("the connection has no usable password".into());
    };
    super::connections::activate(s, c, password.expose());
    if let Some((kind, key)) = crate::connections::database_key(&c.address) {
        s.database_logins.select(kind, &key, &c.id);
    }
    let params = json!({"database": c.address, "layer": table_name(&g.table), "name": name});
    s.run_part("layer.addDatabase", &params).map_err(|e| if password.expose().is_empty() { e } else { e.replace(password.expose(), "***") })
}
