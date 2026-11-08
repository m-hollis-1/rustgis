//! Named endpoint definitions, optional remembered passwords, web logins and S3 keys, off-frame
//! web probes, and explicit saved-connection choices for project sources.

use egui::vec2;
use rustgis_engine::connections::{
    Connection, DATABASE_KINDS, LoginReason, LoginRequest, SERVICE_KINDS as WEB_KINDS, WEB, database_key, database_label, login_origin,
};
use rustgis_engine::render::rustgis_tiles::{cache, s3};
use rustgis_engine::s3_connections::STALE;
use rustgis_engine::s3_keys as keys;
use rustgis_engine::service_logins as web;
use rustgis_engine::sources::Source;
use serde_json::{Value, json};

use crate::{RustGisApp, Slot, i18n};

#[derive(Default)]
pub struct State {
    pub id: Option<String>,
    pub kind: String,
    pub name: String,
    pub group: String,
    pub address: String,
    pub region: String,
    pub buckets: String,
    pub credentials: String,
    pub profile: String,
    pub password: String,
    pub auth: String,
    /// The user name the connection logs in with: a web service's basic login, or an enterprise
    /// geodatabase's database user (with SQL Server sign-in).
    pub user: String,
    pub key_name: String,
    pub api_key: String,
    pub allow_insecure: bool,
    pub remember: bool,
    /// The current project's login prompt, never a password.
    pub request: Option<LoginRequest>,
    pub selected: Option<String>,
    pub access_key: String,
    pub secret_key: String,
    pub session_token: String,
    pub test: Option<(Value, Slot<Result<Vec<u8>, String>>)>,
    /// The form was opened to log in (Log In…): Save logs in even with no secret typed,
    /// using the connection's existing session login (or saved DB/S3 credentials).
    pub login: bool,
    /// A login that worked with a warning (translated, e.g. the keys weren't remembered): the
    /// dialog stays open to say so until it is closed or edited again.
    pub warned: Option<String>,
    /// A project's S3 source waiting for this connection (the chooser's), as (source address,
    /// connection id): its choice is made again once that connection has logged in.
    pub resolving: Option<(String, String)>,
    /// Opened from Repair Source… for this layer: once the connection is saved and logged in, the
    /// layer is repaired through it.
    pub continue_repair: Option<u32>,
    /// An enterprise geodatabase's form (platform, server, sign-in, database) in place of the
    /// single connection address; [`definition`] composes the address from it.
    pub form: bool,
    /// The geodatabase's platform; SQL Server is the only one RustGIS reads (`""` is that too).
    pub platform: String,
    pub server: String,
    /// `"sql"` (a database user and password) or `"windows"` (the Windows user); `""` is `"sql"`.
    /// The database user is [`Self::user`], as a web service's basic login is.
    pub sign_in: String,
    pub database: String,
    /// Trust the server's certificate without checking who issued it (SQL Server's own
    /// self-signed certificate): `trust-server-certificate=true` in the address.
    pub trust_cert: bool,
    /// The Name or Group was typed since the form was last filled from a .sde file: only what a
    /// person typed is sent with the next file, never what the engine made from the last one.
    pub typed_name: bool,
    /// The connection the last From .sde file... made, while this dialog is still bound to it: the
    /// next file replaces it, so choosing the right file after a wrong one leaves no orphan.
    pub sde_id: Option<String>,
}

/// What an `mssql://` connection address says, to fill the form with.
#[derive(Debug, PartialEq)]
pub(crate) struct Mssql {
    /// `host`, with `\instance` and `,port` when the address has them (the port 1433 is the default).
    pub server: String,
    pub user: Option<String>,
    pub database: String,
    /// `trust-server-certificate=true` (or `yes`) is in the address.
    pub trust_cert: bool,
}

/// `s` with its `%XX` escapes undone (an escape that isn't valid is kept as it is).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = |at: usize| bytes.get(at).and_then(|c| (*c as char).to_digit(16));
        match (b, hex(i + 1), hex(i + 2)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The server, user and database of an `mssql://user@host:port/database?instance=name` address
/// (no password in it); `None` for any other address.
pub(crate) fn parse_mssql(address: &str) -> Option<Mssql> {
    let rest = address.trim().strip_prefix("mssql://").or_else(|| address.trim().strip_prefix("sqlserver://"))?;
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, database) = rest.split_once('/').unwrap_or((rest, ""));
    let (user, hostport) = match authority.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(percent_decode(userinfo.split(':').next().unwrap_or(""))), hostport),
        None => (None, authority),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => (host, Some(port)),
        _ => (hostport, None),
    };
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| k.to_ascii_lowercase().replace('_', "-") == name)
            .map(|(_, v)| percent_decode(v))
    };
    let instance = param("instance").filter(|v| !v.is_empty());
    let trust_cert = param("trust-server-certificate")
        .or_else(|| param("trustservercertificate"))
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "yes"));
    let mut server = percent_decode(host);
    if let Some(instance) = instance {
        server.push('\\');
        server.push_str(&instance);
    }
    if let Some(port) = port.filter(|p| *p != "1433") {
        server.push(',');
        server.push_str(port);
    }
    Some(Mssql { server, user: user.filter(|u| !u.is_empty()), database: percent_decode(database), trust_cert })
}

/// The address the geodatabase form makes, or why it can't.
fn form_address(st: &State) -> Result<String, String> {
    use rustgis_engine::data::source::sdefile::{SdeConnection, mssql_address};
    if st.server.trim().is_empty() {
        return Err(tl!("Type the server").into());
    }
    let windows = st.sign_in == "windows";
    let address = mssql_address(&SdeConnection {
        platform: if st.platform.is_empty() { "sqlserver".into() } else { st.platform.clone() },
        server: st.server.clone(),
        database: st.database.trim().to_owned(),
        user: (!windows).then(|| st.user.trim().to_owned()),
        os_auth: windows,
        version: None,
    })
    .map_err(|e| e.to_string())?;
    Ok(if st.trust_cert { with_trust(address) } else { address })
}

/// `address` with `trust-server-certificate=true` added to its query.
fn with_trust(address: String) -> String {
    let joiner = if address.contains('?') { '&' } else { '?' };
    format!("{address}{joiner}trust-server-certificate=true")
}

/// Why the geodatabase form can't make an address (`None` outside the form, or when it can).
fn form_error(st: &State) -> Option<String> {
    if st.form { form_address(st).err() } else { None }
}

/// Switches `st` to the geodatabase form for a new SQL Server connection, with these filled in.
pub(crate) fn use_form(st: &mut State, server: Option<&str>, database: Option<&str>) {
    st.form = true;
    st.platform = "sqlserver".into();
    st.sign_in = "sql".into();
    if let Some(server) = server {
        st.server = server.to_owned();
    }
    if let Some(database) = database {
        st.database = database.to_owned();
    }
}

/// The chooser of the saved S3 connection that reads a project's S3 sources (Data Sources ›
/// Choose S3 connection…): what was picked, for the project it is for. Nothing is picked for the
/// user.
#[derive(Default)]
pub struct Chooser {
    /// The candidate picked for each resource waiting, by the source address that names it.
    picked: std::collections::BTreeMap<String, String>,
    /// The project (`Session::opened`) this state is for.
    project: Option<u64>,
    /// Whether the chooser may still open by itself for this project (once).
    auto: bool,
    error: Option<String>,
}

/// A fixed sentence the engine says about saved S3 keys and S3 connection choices, in the
/// current language; any other message as it is.
pub(crate) fn translated(e: &str) -> String {
    match e {
        keys::UNUSABLE => tl!("Saved S3 keys could not be used. Enter the keys again."),
        keys::NONE_SAVED => tl!("No saved S3 keys. Enter accessKeyId and secretAccessKey."),
        keys::UNAVAILABLE => tl!("Saved S3 keys are unavailable. Enter the keys for this session."),
        keys::NOT_REMEMBERED => tl!("Keys are available for this session but were not remembered on this computer."),
        keys::TEMPORARY => tl!("Temporary credentials are session-only. Previously saved keys are unchanged."),
        keys::NOT_FORGOTTEN => tl!("Saved S3 keys could not be forgotten. The connection was not changed."),
        s3::UNRESOLVED => tl!("Choose a saved S3 connection before opening this source."),
        s3::MISMATCH => tl!("The saved S3 connection no longer matches this source."),
        STALE => tl!("This S3 connection choice is no longer available."),
        web::UNUSABLE => tl!("The session login could not be used. Enter it again."),
        web::SESSION_ONLY => tl!("Web-service logins last for this session only."),
        web::INSECURE => tl!("This connection uses unencrypted http://. Tick the confirmation before sending its password or key."),
        web::BAD_SECRET => tl!("The password or key must fit a 4096-byte HTTP header, without control characters."),
        web::HEADER_ASCII => tl!("An API key sent in a header must use plain ASCII characters."),
        web::REQUIRED => tl!("Enter this connection's password or API key to log in."),
        web::UNSAVED => tl!("Save the connection first: a login belongs to a saved connection."),
        web::NO_ORIGIN => tl!("The connection's address has no http:// or https:// origin a login can go to."),
        web::SAVE_BEFORE_TEST => tl!("Save changes before testing this connection."),
        web::NOT_THIS_SESSION => tl!("This request's login does not belong to this session."),
        web::CHOOSE_CONNECTION => tl!("Several saved connections match this address. Choose a connection explicitly."),
        web::NO_SUCH_CONNECTION => tl!("No saved web-service connection with this id."),
        web::CONNECTION_TEXT => tl!("connection must be a saved web-service connection id (text)"),
        other => other,
    }
    .to_string()
}

/// Whether a login failed for want of usable saved keys: typing them is the way on.
pub(crate) fn needs_keys(e: &str) -> bool {
    matches!(e, keys::NONE_SAVED | keys::UNAVAILABLE | keys::UNUSABLE)
}

/// Why the Remember checkbox can't be used, when it can't: no credential store (the browser, or a
/// shell without one), or temporary credentials (a session token), which are never kept.
pub(crate) fn remember_blocked(store: bool, session_token: &str) -> Option<&'static str> {
    if !store {
        Some(tl!("Keys are kept for this session only: there is no credential store to remember them in here."))
    } else if !session_token.trim().is_empty() {
        Some(tl!("Temporary credentials (with a session token) are kept for this session only."))
    } else {
        None
    }
}

/// The checkbox, its reason when it can't be used, and what it does when it can. Unticked when
/// it can't be used.
pub(crate) fn remember_checkbox(ui: &mut egui::Ui, remember: &mut bool, blocked: Option<&str>) -> bool {
    if blocked.is_some() {
        *remember = false;
    }
    let changed = ui.add_enabled(blocked.is_none(), egui::Checkbox::new(remember, tl!("Remember on this computer"))).changed();
    let note = blocked.unwrap_or(tl!(
        "Keeps the access key ID and secret access key in this computer's credential store, never in the connection or the project."
    ));
    ui.label(egui::RichText::new(note).small().weak());
    changed
}

/// Forgets the keys saved on this computer for the S3 connection `id`; its login in this session
/// keeps working. A failure leaves everything as it was.
pub(crate) fn forget_keys(app: &mut RustGisApp, id: &str) -> Result<(), String> {
    app.run("connection.forgetSavedKeys", json!({"id": id})).map(|_| ()).map_err(|e| translated(&e))
}

pub(crate) const KINDS: [(&str, &str); 13] = [
    ("postgres", "PostgreSQL/PostGIS"),
    ("mysql", "MySQL/MariaDB"),
    ("sqlserver", "SQL Server"),
    ("service", "Web service (detected)"),
    ("wms", "WMS"),
    ("wmts", "WMTS"),
    ("wfs", "WFS"),
    ("wcs", "WCS"),
    ("ogcapi", "OGC API"),
    ("arcgis", "ArcGIS REST"),
    ("csw", "Catalogue (CSW)"),
    ("stac", "STAC API"),
    ("s3", "S3"),
];

/// The kinds Add From Service connects to.
pub(crate) const SERVICE_KINDS: &[&str] = &["service", "wms", "wmts", "wfs", "wcs", "ogcapi", "arcgis", "csw"];

/// The connection a dialog was opened on, while its trimmed, redacted address still matches.
/// Never infers identity from an address alone.
pub(crate) fn picked<'a>(list: &'a [Connection], id: Option<&str>, address: &str) -> Option<&'a Connection> {
    let id = id?;
    let c = list.iter().find(|c| c.id == id)?;
    (c.address == rustgis_engine::sources::redact(address.trim())).then_some(c)
}

/// Draws a saved-connection row when these kinds exist; returns only a newly picked connection.
pub(crate) fn picker(ui: &mut egui::Ui, id: &str, list: &[Connection], kinds: &[&str], current: Option<&Connection>) -> Option<Connection> {
    if !list.iter().any(|c| kinds.contains(&c.kind.as_str())) {
        return None;
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Saved connection"));
        picker_combo(ui, id, list, kinds, current)
    })
    .inner
}

/// The picker control without its label, for a dialog's existing form grid.
pub(crate) fn picker_combo(
    ui: &mut egui::Ui,
    id: &str,
    list: &[Connection],
    kinds: &[&str],
    current: Option<&Connection>,
) -> Option<Connection> {
    let label = |c: &Connection| if c.group.is_empty() { c.name.clone() } else { format!("{} › {}", c.group, c.name) };
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .width(280.0)
        .selected_text(current.map(label).unwrap_or_else(|| tl!("Pick one…").to_string()))
        .show_ui(ui, |ui| {
            for c in list.iter().filter(|c| kinds.contains(&c.kind.as_str())) {
                let selected = current.is_some_and(|x| x.id == c.id);
                let hover = if c.kind == "s3" && c.address.is_empty() { tl!("Amazon S3") } else { &c.address };
                if ui.selectable_label(selected, label(c)).on_hover_text(hover).clicked() && !selected {
                    picked = Some(c.clone());
                }
            }
        })
        .response
        .on_hover_text(tl!("Saved connection"));
    picked
}

/// Keeps an endpoint, never its secrets, as a new connection even at an existing address.
pub(crate) fn keep(app: &mut RustGisApp, p: Value) -> Result<String, String> {
    let v = app.run("connection.new", p)?;
    let id = v["id"].as_str().map(str::to_owned).ok_or_else(|| "connection.new returned no id".to_string())?;
    stripped_notice(app, &v);
    app.session.notify(i18n::fmt("Saved as the connection {name}", &[("name", v["name"].as_str().unwrap_or(""))]), false);
    Ok(id)
}

/// Offers to keep the endpoint connected by an add dialog.
pub(crate) fn keep_button(ui: &mut egui::Ui) -> bool {
    ui.button(tl!("Save as Connection"))
        .on_hover_text(tl!("Keep this endpoint under Data Sources › Connections, without its password or keys"))
        .clicked()
}

fn stripped_notice(app: &mut RustGisApp, v: &Value) {
    if v["stripped"] == true {
        app.session.notify(tl!("A password or token in the address was left out of the saved connection definition.").to_string(), false);
    }
}

pub(crate) fn label(kind: &str) -> &'static str {
    KINDS.iter().find(|(k, _)| *k == kind).map(|(_, l)| *l).unwrap_or("Connection")
}

pub fn open_new(app: &mut RustGisApp, kind: &str) -> Result<(), String> {
    if !KINDS.iter().any(|(k, _)| *k == kind) {
        return Err(format!("unknown kind '{kind}'"));
    }
    if cfg!(target_arch = "wasm32") && DATABASE_KINDS.contains(&kind) {
        return Err(tl!("Database connections are used by the desktop app").into());
    }
    app.ui.connection =
        State { kind: kind.into(), credentials: if kind == "s3" { "keys".into() } else { String::new() }, ..Default::default() };
    app.dialogs = crate::dialogs::Dialogs { open: Some("newConnection"), ..Default::default() };
    Ok(())
}

pub fn open_edit(app: &mut RustGisApp, id: &str) -> Result<(), String> {
    let c = app.session.connections.find(id).cloned().ok_or_else(|| format!("no connection '{id}'"))?;
    app.ui.connection = State {
        region: c.options.get("region").cloned().unwrap_or_default(),
        buckets: c.options.get("buckets").cloned().unwrap_or_default(),
        credentials: c.options.get("credentials").cloned().unwrap_or_else(|| "none".into()),
        profile: c.options.get("profile").cloned().unwrap_or_default(),
        auth: c.auth,
        user: c.user,
        key_name: c.key_name,
        id: Some(c.id),
        kind: c.kind,
        name: c.name,
        group: c.group,
        address: c.address,
        ..Default::default()
    };
    app.dialogs = crate::dialogs::Dialogs { open: Some("editConnection"), ..Default::default() };
    Ok(())
}

/// Opens the connection's form to log in (Log In…): Save then logs in. True when it opened.
pub fn log_in(app: &mut RustGisApp, id: &str) -> bool {
    match app.run("ui.connection", json!({"id": id})) {
        Ok(_) => {
            app.ui.connection.login = true;
            let saved = app.ui.connection.kind == "s3"
                && app.ui.connection.credentials == "keys"
                && app.ui.connection.id.as_deref().and_then(|id| app.session.connections.find(id)).is_some_and(|c| c.saved_keys);
            app.dialogs.info = Some(if DATABASE_KINDS.contains(&app.ui.connection.kind.as_str()) {
                tl!("Type the password, then Save. Remember on this computer is optional.").to_string()
            } else if WEB_KINDS.contains(&app.ui.connection.kind.as_str()) {
                tl!("Type the password or key, or leave it blank to use this connection's session login, then Save.").to_string()
            } else if saved {
                tl!("Leave the keys blank to log in with the keys saved on this computer, or type others, then Save.").to_string()
            } else {
                tl!("Type the keys to log in for this session, then Save.").to_string()
            });
            true
        }
        Err(e) => {
            app.session.notify(e, true);
            false
        }
    }
}

fn definition(st: &State) -> Value {
    if st.form {
        // An address the form can't make is left empty: Save and Test say why first.
        let address = form_address(st).unwrap_or_default();
        let name =
            if st.name.trim().is_empty() { i18n::fmt("Geodatabase on {server}", &[("server", st.server.trim())]) } else { st.name.clone() };
        return json!({"kind":st.kind, "name":name, "group":st.group, "address":address});
    }
    let address = st.address.trim();
    let lower = address.to_ascii_lowercase();
    let address = if st.kind == "postgres" && !address.is_empty() && !lower.starts_with("postgres:") && !lower.starts_with("postgresql:") {
        format!("postgres:{address}")
    } else {
        address.to_owned()
    };
    let mut p = json!({"kind":st.kind, "name":st.name, "group":st.group, "address":address});
    if st.kind == "s3" {
        p["options"] = json!({"region":st.region, "buckets":st.buckets, "credentials":st.credentials});
        if st.credentials == "profile" {
            p["options"]["profile"] = json!(st.profile);
        }
    }
    if WEB_KINDS.contains(&st.kind.as_str()) {
        p["auth"] = json!(if st.auth.is_empty() { "none" } else { &st.auth });
        p["user"] = json!(st.user);
        p["keyName"] = json!(st.key_name);
    }
    p
}

fn secrets(st: &State, p: &mut Value) {
    if DATABASE_KINDS.contains(&st.kind.as_str()) && !st.password.is_empty() {
        p["password"] = json!(st.password);
    }
    if st.kind == "s3" && st.credentials == "keys" {
        for (key, value) in [("accessKeyId", &st.access_key), ("secretAccessKey", &st.secret_key), ("sessionToken", &st.session_token)] {
            if !value.is_empty() {
                p[key] = json!(value);
            }
        }
    }
    if WEB_KINDS.contains(&st.kind.as_str()) {
        let (key, value) = if st.auth == "basic" { ("password", &st.password) } else { ("apiKey", &st.api_key) };
        if !st.auth.is_empty() && !value.is_empty() {
            p[key] = json!(value);
            p["allowInsecure"] = json!(st.allow_insecure);
        }
    }
}

/// A saved service Test uses its saved login-relevant definition, never an edited endpoint
/// with an old id. A new anonymous endpoint can still be tested without saving it.
fn test_blocked(app: &RustGisApp) -> Option<&'static str> {
    let st = &app.ui.connection;
    let saved = st.id.as_deref().and_then(|id| app.session.connections.find(id));
    if !WEB_KINDS.contains(&st.kind.as_str()) && !saved.is_some_and(web::is_service) {
        return None;
    }
    let auth = if st.auth == "none" { "" } else { &st.auth };
    if st.id.is_none() && auth.is_empty() {
        return None;
    }
    let matches = saved.is_some_and(|c| {
        c.kind == st.kind
            && c.address == st.address.trim()
            && c.auth == auth
            && c.user == st.user.trim()
            && c.key_name == st.key_name.trim()
    });
    (!matches).then_some(tl!("Save changes before testing this connection."))
}

pub fn start_test(app: &mut RustGisApp) {
    app.dialogs.error = None;
    app.dialogs.info = None;
    app.ui.connection.test = None;
    if let Some(e) = form_error(&app.ui.connection) {
        app.dialogs.error = Some(e);
        return;
    }
    let mut p = definition(&app.ui.connection);
    let service = WEB_KINDS.contains(&app.ui.connection.kind.as_str());
    if let Some(reason) = test_blocked(app) {
        app.dialogs.error = Some(reason.into());
        return;
    }
    if service && typed_secrets(&app.ui.connection) && insecure(&app.ui.connection) && !app.ui.connection.allow_insecure {
        app.dialogs.error = Some(translated(web::INSECURE));
        return;
    }
    if service && let Some(id) = &app.ui.connection.id {
        p["id"] = json!(id);
    }
    secrets(&app.ui.connection, &mut p);
    if DATABASE_KINDS.contains(&app.ui.connection.kind.as_str()) {
        match app.run("connection.test", p) {
            Ok(v) => app.dialogs.info = Some(summary(&app.ui.connection.kind, &v["result"])),
            Err(e) => app.dialogs.error = Some(e),
        }
        return;
    }
    p["probe"] = json!(true);
    let result = app.run("connection.test", p);
    if service {
        clear_secrets(&mut app.ui.connection);
    }
    let v = match result {
        Ok(v) => v,
        Err(e) => {
            app.dialogs.error = Some(translated(&e));
            return;
        }
    };
    let Some(url) = v["fetch"].as_str() else {
        app.dialogs.error = Some(tl!("Nothing to test").into());
        return;
    };
    let mut q = definition(&app.ui.connection);
    if service && let Some(id) = &app.ui.connection.id {
        q["id"] = json!(id);
    }
    let slot: Slot<Result<Vec<u8>, String>> = Slot::default();
    let land = slot.clone();
    cache::download(url, Box::new(move |r| *land.lock().unwrap_or_else(|e| e.into_inner()) = Some(r)));
    app.ui.connection.test = Some((q, slot));
    app.dialogs.info = Some(tl!("Testing…").into());
}

/// A request remains authorized only for its original project and exact saved definitions.
fn candidates(app: &RustGisApp, request: &LoginRequest) -> Vec<Connection> {
    if request.opened != app.session.opened {
        return Vec::new();
    }
    request
        .connections
        .iter()
        .filter_map(|id| {
            let c = app.session.connections.find(id)?;
            if request.kind == WEB {
                return (web::is_service(c) && login_origin(c).as_deref() == Some(&request.key)).then(|| c.clone());
            }
            let (kind, key) = database_key(&c.address)?;
            (kind == request.kind && key == request.key).then(|| c.clone())
        })
        .collect()
}

fn password_request(app: &RustGisApp, id: &str) -> Option<LoginRequest> {
    app.session
        .database_logins
        .requests(app.session.opened)
        .find(|r| r.reason == LoginReason::Password && r.connections.iter().any(|c| c == id))
        .cloned()
}

fn password_prompt(app: &mut RustGisApp, id: &str, request: Option<LoginRequest>, error: Option<String>) {
    match open_edit(app, id) {
        Ok(()) => {
            app.ui.connection.request = request;
            if WEB_KINDS.contains(&app.ui.connection.kind.as_str()) {
                app.ui.connection.login = true;
                app.dialogs.info =
                    Some(tl!("Enter the password or key for this connection. Web-service logins last for this session only.").into());
                app.dialogs.error = error.map(|e| translated(&e));
                return;
            }
            app.dialogs.info = Some(tl!("The saved password could not be used. Type a password to try again; check Remember on this computer to replace the saved password.").into());
            app.dialogs.error = error;
        }
        Err(e) => app.session.notify(e, true),
    }
}

pub(crate) fn choose(app: &mut RustGisApp, request: LoginRequest) {
    if candidates(app, &request).is_empty() {
        return;
    }
    app.ui.connection = State { request: Some(request), ..Default::default() };
    app.dialogs = crate::dialogs::Dialogs { open: Some("chooseConnection"), ..Default::default() };
}

pub(crate) fn chooser(app: &mut RustGisApp, ui: &mut egui::Ui) -> bool {
    let Some(request) = app.ui.connection.request.clone() else { return true };
    if request.kind == WEB && request.reason == LoginReason::Connection {
        ui.label(i18n::fmt("This project uses a connection for {origin} that is not saved on this computer.", &[("origin", &request.key)]));
        return ui
            .horizontal(|ui| {
                if ui.button(tl!("New Connection…")).clicked() {
                    match open_new(app, "service") {
                        Ok(()) => {
                            app.ui.connection.address = request.key.clone();
                            // Saving it logs in, so the layers waiting for a connection take it.
                            app.ui.connection.request = Some(request.clone());
                        }
                        Err(e) => app.dialogs.error = Some(e),
                    }
                    return false;
                }
                ui.button(tl!("Cancel")).clicked()
            })
            .inner;
    }
    let list = candidates(app, &request);
    if list.is_empty() {
        return true;
    }
    if request.kind == WEB {
        ui.label(tl!("Choose which saved connection to use for this address. This choice lasts for this session."));
        ui.label(&request.key);
    } else {
        ui.label(tl!("Choose which saved connection to use for this database. This choice lasts for this session."));
        ui.label(database_label(request.kind, &request.key));
    }
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        for c in &list {
            ui.group(|ui| {
                ui.selectable_value(&mut app.ui.connection.selected, Some(c.id.clone()), &c.name);
                if !c.group.is_empty() {
                    ui.label(egui::RichText::new(&c.group).weak());
                }
                ui.label(egui::RichText::new(&c.address).small().weak());
            });
        }
    });
    let selected = app.ui.connection.selected.clone().filter(|id| list.iter().any(|c| &c.id == id));
    ui.horizontal(|ui| {
        if ui.add_enabled(selected.is_some(), egui::Button::new(tl!("Use Connection"))).clicked()
            && let Some(id) = selected
        {
            let result = app.run("connection.login", json!({"id":id}));
            crate::connectiontree::refresh(app, Some(&id));
            let refused = password_request(app, &id);
            if result.is_err() || refused.is_some() {
                password_prompt(app, &id, refused.or(Some(request)), result.err());
                return false;
            }
            return true;
        }
        ui.button(tl!("Cancel")).clicked()
    })
    .inner
}

fn poll_requests(app: &mut RustGisApp) {
    if let Some(request) = &app.ui.connection.request
        && matches!(app.dialogs.open, Some("chooseConnection" | "editConnection"))
        && (request.opened != app.session.opened || request.reason != LoginReason::Connection && candidates(app, request).is_empty())
    {
        app.dialogs.open = None;
        app.ui.connection = State::default();
    }
    // A pending request never displaces an unrelated dialog, including a form being edited.
    if app.dialogs.open.is_some() {
        return;
    }
    while let Some(request) = app.session.database_logins.next_unprompted(app.session.opened) {
        if app.ui.connection.request.as_ref().is_some_and(|shown| {
            shown.opened == request.opened
                && shown.kind == request.kind
                && shown.key == request.key
                && shown.reason == request.reason
                && shown.connections == request.connections
        }) {
            // A layer-menu prompt may have shown this request before automatic polling reached
            // it. In particular, cancelling it must not immediately open it again.
            continue;
        }
        if request.kind == WEB && request.reason == LoginReason::Connection {
            app.ui.connection = State { request: Some(request), ..Default::default() };
            app.dialogs = crate::dialogs::Dialogs { open: Some("chooseConnection"), ..Default::default() };
            break;
        }
        let list = candidates(app, &request);
        if list.is_empty() {
            continue;
        }
        if request.reason == LoginReason::Choose || list.len() > 1 {
            choose(app, request);
        } else {
            password_prompt(app, &list[0].id, Some(request), None);
        }
        break;
    }
}

pub fn poll(app: &mut RustGisApp, ctx: &egui::Context) {
    poll_requests(app);
    if !matches!(app.dialogs.open, Some("newConnection" | "editConnection")) {
        app.ui.connection.test = None;
        return;
    }
    let Some((q, slot)) = app.ui.connection.test.as_mut() else { return };
    let result = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(result) = result else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        return;
    };
    let mut q = q.take();
    app.ui.connection.test = None;
    app.dialogs.info = None;
    match result {
        Ok(bytes) => {
            q["answer"] = json!(String::from_utf8_lossy(&bytes));
            let kind = q["kind"].as_str().unwrap_or("").to_owned();
            match app.run("connection.test", q) {
                Ok(v) => app.dialogs.info = Some(summary(&kind, &v["result"])),
                Err(e) => app.dialogs.error = Some(e),
            }
        }
        Err(e) => app.dialogs.error = Some(e),
    }
}

/// How many layers a service's listing has: all of them when the list stops short (`total`),
/// with a `+` when the document has more than were read.
pub(crate) fn layer_count(r: &Value) -> String {
    let count = r["total"].as_u64().unwrap_or(r["layers"].as_array().map_or(0, Vec::len) as u64);
    if r["atLeast"] == true { format!("{count}+") } else { count.to_string() }
}

fn summary(kind: &str, r: &Value) -> String {
    let count = r["count"].as_u64().unwrap_or(0).to_string();
    match kind {
        "postgres" | "mysql" | "sqlserver" => {
            let tables = i18n::fmt("{n} tables with geometry", &[("n", &count)]);
            let mut said = match r.get("geodatabase").filter(|g| g.is_object()) {
                Some(g) => {
                    let release = g.get("release").and_then(Value::as_str).filter(|v| !v.is_empty());
                    let label = release.map_or_else(
                        || tl!("Enterprise geodatabase").to_string(),
                        |release| i18n::fmt("Enterprise geodatabase {release}", &[("release", release)]),
                    );
                    format!("{label}, {tables}")
                }
                None => tables,
            };
            if let Some(why) = r.get("geodatabaseProblem").and_then(Value::as_str) {
                said.push_str(" · ");
                said.push_str(&i18n::fmt("the geodatabase catalog couldn't be read: {why}", &[("why", why)]));
            }
            said
        }
        "csw" => {
            let n = r["matched"].as_u64().map(|n| n.to_string()).unwrap_or(count);
            i18n::fmt("{n} records in the catalogue", &[("n", &n)])
        }
        "stac" => i18n::fmt("{n} collections", &[("n", &count)]),
        "s3" => {
            if let Some(bucket) = r["bucket"].as_str() {
                let n = if r["truncated"] == true { format!("{count}+") } else { count };
                i18n::fmt("{n} items in {bucket}", &[("n", &n), ("bucket", bucket)])
            } else {
                i18n::fmt("{n} buckets", &[("n", &r["buckets"].as_array().map_or(0, Vec::len).to_string())])
            }
        }
        _ => {
            let kind = r["kind"].as_str().unwrap_or("");
            let title = r["title"].as_str().unwrap_or("");
            let count = layer_count(r);
            if r["layers"].as_array().is_none_or(Vec::is_empty) && r["next"].is_string() {
                i18n::fmt("Connected to {kind} {title}", &[("kind", kind), ("title", title)])
            } else {
                i18n::fmt("{kind} {title}: {count} layers", &[("kind", kind), ("title", title), ("count", &count)])
            }
        }
    }
}

fn insecure(st: &State) -> bool {
    !st.auth.is_empty() && st.address.trim().get(..7).is_some_and(|s| s.eq_ignore_ascii_case("http://"))
}

/// Whether a password or any key is typed in the form.
fn typed_secrets(st: &State) -> bool {
    (DATABASE_KINDS.contains(&st.kind.as_str()) && !st.password.is_empty())
        || (WEB_KINDS.contains(&st.kind.as_str())
            && match st.auth.as_str() {
                "basic" => !st.password.is_empty(),
                "header" => !st.api_key.is_empty(),
                _ => false,
            })
        || (st.kind == "s3"
            && st.credentials == "keys"
            && [&st.access_key, &st.secret_key, &st.session_token].iter().any(|v| !v.is_empty()))
}

fn clear_secrets(st: &mut State) {
    st.password.clear();
    st.api_key.clear();
    st.allow_insecure = false;
    st.access_key.clear();
    st.secret_key.clear();
    st.session_token.clear();
    st.remember = false;
}

/// Saves the definition, then logs in when a secret is typed, or the form was opened to log in
/// (using this connection's session login or saved DB/S3 credentials). Only DB/S3 logins remember
/// secrets. A login that worked with a warning keeps the dialog open to say so.
fn save(app: &mut RustGisApp) -> Result<(), String> {
    let store = app.session.database_logins.has_store();
    let st = &app.ui.connection;
    if let Some(e) = form_error(st) {
        return Err(e);
    }
    let service = WEB_KINDS.contains(&st.kind.as_str());
    if service && typed_secrets(st) {
        let (connection, _) = rustgis_engine::connections::clean(Connection {
            kind: st.kind.clone(),
            address: st.address.clone(),
            auth: st.auth.clone(),
            user: st.user.clone(),
            key_name: st.key_name.clone(),
            ..Default::default()
        })?;
        let secret = if st.auth == "basic" { &st.password } else { &st.api_key };
        web::check_login_secret(&connection, secret).map_err(|e| translated(&e))?;
    }
    if service && typed_secrets(st) && insecure(st) && !st.allow_insecure {
        return Err(translated(web::INSECURE));
    }
    let keys_mode = st.kind == "s3" && st.credentials == "keys";
    let remember = keys_mode && st.remember && remember_blocked(store, &st.session_token).is_none();
    if remember && (st.access_key.trim().is_empty() || st.secret_key.trim().is_empty()) {
        return Err(tl!("Type the access key ID and the secret access key to remember them on this computer.").into());
    }
    if DATABASE_KINDS.contains(&st.kind.as_str()) && st.remember && st.password.is_empty() {
        return Err(tl!("Type the password to remember it on this computer.").into());
    }
    let adopts = service && st.request.as_ref().is_some_and(|r| r.kind == WEB && r.reason == LoginReason::Connection);
    let wants_login = typed_secrets(st) || (st.login && (keys_mode || service) && st.id.is_some()) || adopts;
    let mut p = definition(st);
    let command = if let Some(id) = &st.id {
        p["id"] = json!(id);
        if st.kind != "s3"
            && let Some(object) = p.as_object_mut()
        {
            object.remove("options");
        }
        "connection.edit"
    } else {
        // The form names a connection after its server; an address is named by the engine.
        if st.name.trim().is_empty()
            && !st.form
            && let Some(object) = p.as_object_mut()
        {
            object.remove("name");
        }
        "connection.new"
    };
    let v = app.run(command, p)?;
    stripped_notice(app, &v);
    let id = v["id"].as_str().map(str::to_owned).ok_or_else(|| "the connection has no id".to_string())?;
    // Kept at once: a Save tried again edits this connection rather than adding another.
    let created = app.ui.connection.id.is_none();
    app.ui.connection.id = Some(id.clone());
    if !wants_login {
        clear_secrets(&mut app.ui.connection);
        crate::connectiontree::refresh(app, Some(&id));
        // Repair Source… is waiting for this connection: with Windows sign-in there is no
        // password to wait for, and with a database user it can't go on without one.
        if app.ui.connection.continue_repair.is_some() && app.ui.connection.kind == "sqlserver" {
            let st = &app.ui.connection;
            let windows = if st.form {
                st.sign_in == "windows"
            } else {
                definition(st)["address"].as_str().and_then(parse_mssql).is_some_and(|m| m.user.is_none())
            };
            if !windows {
                return Err(tl!("Type the password to connect").into());
            }
            if let Some(layer) = app.ui.connection.continue_repair.take() {
                crate::repairbroken::continue_repair(app, layer, id);
            }
        }
        return Ok(());
    }
    let mut login = json!({"id": id});
    secrets(&app.ui.connection, &mut login);
    let database = DATABASE_KINDS.contains(&app.ui.connection.kind.as_str());
    if database {
        login["remember"] = json!(app.ui.connection.remember);
    } else if remember {
        login["remember"] = json!(true);
    }
    let result = app.run("connection.login", login);
    if service {
        // A web login is not kept in the form once sent, whether it worked or not.
        clear_secrets(&mut app.ui.connection);
        crate::connectiontree::refresh(app, Some(&id));
    }
    if database {
        crate::connectiontree::refresh(app, Some(&id));
        let refused = password_request(app, &id);
        if result.is_err() || refused.is_some() {
            app.ui.connection.password.clear();
            app.ui.connection.remember = false;
            app.ui.connection.request = refused;
            app.dialogs.open = Some("editConnection");
            return Err(result.err().unwrap_or_else(|| tl!("The password was refused. Type a password to try again.").into()));
        }
    }
    let v = match result {
        Ok(v) => v,
        Err(e) => {
            if created {
                app.dialogs.open = Some("editConnection");
            }
            return Err(translated(&e));
        }
    };
    clear_secrets(&mut app.ui.connection);
    app.ui.connection.login = false;
    if let Some(warning) = v["warning"].as_str() {
        if database {
            // Database login retains its established notice-and-close behavior.
            app.session.notify(warning.to_string(), true);
        } else {
            app.ui.connection.warned = Some(translated(warning));
            app.dialogs.open = Some("editConnection");
        }
    }
    resolve_waiting(app, &id);
    if let Some(layer) = app.ui.connection.continue_repair.take() {
        crate::repairbroken::continue_repair(app, layer, id);
    }
    Ok(())
}

/// "Enter an address instead" / "Use the form instead": whatever the form composed stays in the
/// address box, and the form's fields keep what they had; nothing is parsed back from the address.
fn swap_form(st: &mut State) {
    if st.form {
        if let Ok(address) = form_address(st) {
            st.address = address;
        }
        st.form = false;
    } else {
        use_form(st, None, None);
    }
}

/// The enterprise geodatabase form's rows in the dialog's grid: platform, server, how to sign in,
/// the user and password (for a database user), database, name and group. True when edited.
fn geodatabase_rows(ui: &mut egui::Ui, st: &mut State) -> bool {
    let mut changed = false;
    if st.sign_in.is_empty() {
        st.sign_in = "sql".into();
    }
    let input_label = ui.label(tl!("Platform"));
    // SQL Server is the only platform RustGIS reads, so the choice is shown but not offered.
    ui.add_enabled_ui(false, |ui| {
        egui::ComboBox::from_id_salt("connection_platform")
            .selected_text(tl!("SQL Server"))
            .width(360.0)
            .show_ui(ui, |ui| {
                let _ = ui.selectable_label(true, tl!("SQL Server"));
            })
            .response
            .labelled_by(input_label.id)
    });
    ui.end_row();
    let input_label = ui.label(tl!("Server"));
    changed |= ui
        .add(egui::TextEdit::singleline(&mut st.server).hint_text(tl!("Server or server\\instance")).desired_width(360.0))
        .labelled_by(input_label.id)
        .changed();
    ui.end_row();
    let input_label = ui.label(tl!("Sign in"));
    let shown = if st.sign_in == "windows" { tl!("Windows") } else { tl!("Database user") };
    egui::ComboBox::from_id_salt("connection_sign_in")
        .selected_text(shown)
        .width(360.0)
        .show_ui(ui, |ui| {
            changed |= ui.selectable_value(&mut st.sign_in, "sql".to_owned(), tl!("Database user")).changed();
            changed |= ui.selectable_value(&mut st.sign_in, "windows".to_owned(), tl!("Windows")).changed();
        })
        .response
        .labelled_by(input_label.id);
    ui.end_row();
    if st.sign_in == "windows" {
        // The Windows user signs in: there is no user name or password to type or keep.
        st.password.clear();
        st.remember = false;
    } else {
        let input_label = ui.label(tl!("User"));
        changed |= ui.add(egui::TextEdit::singleline(&mut st.user).desired_width(360.0)).labelled_by(input_label.id).changed();
        ui.end_row();
        let input_label = ui.label(tl!("Password"));
        changed |= ui
            .add(egui::TextEdit::singleline(&mut st.password).password(true).hint_text(tl!("Optional")).desired_width(360.0))
            .labelled_by(input_label.id)
            .changed();
        ui.end_row();
        ui.label("");
        changed |= ui.checkbox(&mut st.remember, tl!("Remember on this computer")).changed();
        ui.end_row();
    }
    let input_label = ui.label(tl!("Database"));
    changed |= ui.add(egui::TextEdit::singleline(&mut st.database).desired_width(360.0)).labelled_by(input_label.id).changed();
    ui.end_row();
    ui.label("");
    changed |= ui
        .checkbox(&mut st.trust_cert, tl!("Trust the server's certificate"))
        .on_hover_text(tl!(
            "Connect even though the server's certificate isn't from an authority this computer trusts, as Pro and SQL Server Management Studio do with \"Trust server certificate\". SQL Server's own self-signed certificate needs it. Tick it only for your organisation's server."
        ))
        .changed();
    ui.end_row();
    let input_label = ui.label(tl!("Name"));
    let default = i18n::fmt("Geodatabase on {server}", &[("server", st.server.trim())]);
    let edited =
        ui.add(egui::TextEdit::singleline(&mut st.name).hint_text(default).desired_width(360.0)).labelled_by(input_label.id).changed();
    st.typed_name |= edited;
    changed |= edited;
    ui.end_row();
    let input_label = ui.label(tl!("Group"));
    let edited =
        ui.add(egui::TextEdit::singleline(&mut st.group).hint_text(tl!("None")).desired_width(360.0)).labelled_by(input_label.id).changed();
    st.typed_name |= edited;
    changed |= edited;
    ui.end_row();
    changed
}

/// From .sde file…: the engine reads the Pro connection file and keeps the connection it
/// describes (no password: a .sde file's is never read); the form is filled from the result and
/// belongs to that saved connection, so Save logs it in.
pub(crate) fn fill_from_sde(app: &mut RustGisApp, path: &str) -> Result<(), String> {
    let st = &app.ui.connection;
    let mut p = json!({"kind": "sqlserver", "sdeFile": path});
    if st.typed_name {
        for (key, value) in [("name", st.name.trim()), ("group", st.group.trim())] {
            if !value.is_empty() {
                p[key] = json!(value);
            }
        }
    }
    // The connection an earlier file made in this dialog gives way to this one, so a wrong file
    // followed by the right one leaves a single connection (and its name is free again).
    if let Some(old) = app.ui.connection.sde_id.take()
        && app.ui.connection.id.as_deref() == Some(old.as_str())
    {
        app.run("connection.remove", json!({"id": old}))?;
        app.ui.connection.id = None;
        crate::connectiontree::refresh(app, None);
    }
    let v = app.run("connection.new", p)?;
    let id = v["id"].as_str().map(str::to_owned).ok_or_else(|| "connection.new returned no id".to_string())?;
    let address = v["address"].as_str().unwrap_or_default().to_owned();
    let parsed = parse_mssql(&address);
    let windows = v["osAuth"] == true;
    let st = &mut app.ui.connection;
    use_form(st, None, None);
    st.kind = "sqlserver".into();
    st.id = Some(id.clone());
    st.sde_id = Some(id.clone());
    st.typed_name = false;
    st.name = v["name"].as_str().unwrap_or_default().to_owned();
    st.group = v["group"].as_str().unwrap_or_default().to_owned();
    st.server = parsed.as_ref().map(|m| m.server.clone()).unwrap_or_default();
    st.database = parsed.as_ref().map(|m| m.database.clone()).unwrap_or_default();
    st.sign_in = if windows { "windows" } else { "sql" }.into();
    st.trust_cert = parsed.as_ref().is_some_and(|m| m.trust_cert);
    st.user = v["user"].as_str().map(str::to_owned).or_else(|| parsed.and_then(|m| m.user)).unwrap_or_default();
    st.address = address;
    st.password.clear();
    st.remember = false;
    st.test = None;
    app.dialogs.error = None;
    app.dialogs.info = Some(
        if v["needsPassword"] == true {
            tl!("Type the password, then Save.")
        } else {
            tl!("This file signs in as the Windows user: Save, or Test to check the connection.")
        }
        .to_string(),
    );
    crate::connectiontree::refresh(app, Some(&id));
    Ok(())
}

pub fn dialog(app: &mut RustGisApp, ui: &mut egui::Ui) -> bool {
    let intro = if DATABASE_KINDS.contains(&app.ui.connection.kind.as_str()) {
        tl!(
            "A named endpoint you can reuse. Passwords last for this session unless you choose to remember them in this computer's password store."
        )
    } else if app.ui.connection.kind == "s3" {
        tl!(
            "A named endpoint you can reuse. Keys are kept until RustGIS closes unless you choose to remember them on this computer; the connection itself never keeps them."
        )
    } else {
        tl!("Web-service logins last for this session only.")
    };
    ui.label(egui::RichText::new(intro).weak());
    let store = app.session.database_logins.has_store();
    // Status metadata only: rendering must never read the credential store.
    let saved_keys =
        app.ui.connection.id.as_deref().and_then(|id| app.session.connections.find(id)).is_some_and(|c| c.kind == "s3" && c.saved_keys);
    let current = app.ui.connection.id.as_deref().and_then(|id| app.session.connections.find(id));
    let logged_in = current.is_some_and(|c| web::logged_in(&app.session, c));
    let st = &mut app.ui.connection;
    let database = DATABASE_KINDS.contains(&st.kind.as_str());
    let service = WEB_KINDS.contains(&st.kind.as_str());
    let mut changed = false;
    let mut source_changed = false;
    let mut forget = false;
    let form_mode = st.form && st.kind == "sqlserver";
    let mut toggle_form = false;
    let mut from_sde = false;
    let mut logout = false;
    egui::Grid::new("connection").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
        if form_mode {
            changed |= geodatabase_rows(ui, st);
            return;
        }
        ui.label(tl!("Type"));
        ui.label(tl!(label(&st.kind)));
        ui.end_row();
        let input_label = ui.label(tl!("Name"));
        let edited = ui
            .add(egui::TextEdit::singleline(&mut st.name).hint_text(tl!("From the address")).desired_width(360.0))
            .labelled_by(input_label.id)
            .changed();
        st.typed_name |= edited;
        changed |= edited;
        ui.end_row();
        let input_label = ui.label(tl!("Group"));
        let edited = ui
            .add(egui::TextEdit::singleline(&mut st.group).hint_text(tl!("None")).desired_width(360.0))
            .labelled_by(input_label.id)
            .changed();
        st.typed_name |= edited;
        changed |= edited;
        ui.end_row();
        if database {
            let input_label = ui.label(tl!("Connection"));
            let hint = match st.kind.as_str() {
                "mysql" => "mysql://user@host:3306/db",
                "sqlserver" => "mssql://user@host:1433/db",
                _ => "host=… dbname=… user=…",
            };
            changed |= ui
                .add(egui::TextEdit::singleline(&mut st.address).hint_text(hint).desired_width(360.0))
                .labelled_by(input_label.id)
                .changed();
            ui.end_row();
            if st.kind == "sqlserver" {
                ui.label("");
                toggle_form = ui.link(tl!("Use the form instead")).clicked();
                ui.end_row();
            }
            ui.label(tl!("Password"));
            changed |= ui
                .add(egui::TextEdit::singleline(&mut st.password).password(true).hint_text(tl!("Optional")).desired_width(360.0))
                .changed();
            ui.end_row();
            ui.label("");
            changed |= ui.checkbox(&mut st.remember, tl!("Remember on this computer")).changed();
            ui.end_row();
        } else if st.kind == "s3" {
            for (label, field, hint) in [
                (tl!("Endpoint"), &mut st.address, tl!("Blank for Amazon S3")),
                (tl!("Region"), &mut st.region, s3::DEFAULT_REGION),
                (tl!("Buckets"), &mut st.buckets, tl!("Comma-separated (optional)")),
            ] {
                let input_label = ui.label(label);
                changed |=
                    ui.add(egui::TextEdit::singleline(field).hint_text(hint).desired_width(360.0)).labelled_by(input_label.id).changed();
                ui.end_row();
            }
            let input_label = ui.label(tl!("Keys from"));
            let shown = crate::s3ui::SOURCES.iter().find(|(c, _)| *c == st.credentials).map(|(_, l)| tl!(*l)).unwrap_or_default();
            egui::ComboBox::from_id_salt("connection_credentials")
                .selected_text(shown)
                .width(360.0)
                .show_ui(ui, |ui| {
                    for (code, label) in crate::s3ui::SOURCES {
                        if cfg!(target_arch = "wasm32") && matches!(code, "profile" | "environment") {
                            continue;
                        }
                        source_changed |= ui.selectable_value(&mut st.credentials, code.to_owned(), tl!(label)).changed();
                        changed |= source_changed;
                    }
                })
                .response
                .labelled_by(input_label.id);
            ui.end_row();
            if st.credentials == "profile" {
                let input_label = ui.label(tl!("Profile"));
                changed |= ui.add(egui::TextEdit::singleline(&mut st.profile).desired_width(360.0)).labelled_by(input_label.id).changed();
                ui.end_row();
            }
            if st.credentials == "keys" {
                for (label, field, secret) in [
                    (tl!("Access key ID"), &mut st.access_key, false),
                    (tl!("Secret access key"), &mut st.secret_key, true),
                    (tl!("Session token"), &mut st.session_token, true),
                ] {
                    let input_label = ui.label(label);
                    changed |= ui
                        .add(egui::TextEdit::singleline(field).password(secret).desired_width(360.0))
                        .labelled_by(input_label.id)
                        .changed();
                    ui.end_row();
                }
                ui.label("");
                let blocked = remember_blocked(store, &st.session_token);
                ui.vertical(|ui| changed |= remember_checkbox(ui, &mut st.remember, blocked));
                ui.end_row();
                if saved_keys {
                    ui.label(tl!("Saved keys"));
                    ui.vertical(|ui| {
                        let note = if store {
                            tl!("This computer keeps keys for this connection: leave the keys blank to log in with them.")
                        } else {
                            tl!("Keys were saved for this connection, but there is no credential store here to read them from.")
                        };
                        ui.label(egui::RichText::new(note).small().weak());
                        forget = ui.button(tl!("Forget Saved Keys")).clicked();
                    });
                    ui.end_row();
                }
            }
        } else {
            let input_label = ui.label(tl!("Address"));
            let hint = match st.kind.as_str() {
                "stac" => "https://…/stac/v1",
                "arcgis" => "https://…/rest/services",
                "csw" => "https://…/csw",
                _ => "https://…",
            };
            changed |= ui
                .add(egui::TextEdit::singleline(&mut st.address).hint_text(hint).desired_width(360.0))
                .labelled_by(input_label.id)
                .changed();
            ui.end_row();
            if service {
                ui.label(egui::RichText::new(tl!("Login")).strong());
                ui.label("");
                ui.end_row();
                let input_label = ui.label(tl!("Method"));
                let methods = [("", tl!("None")), ("basic", tl!("Basic authentication")), ("header", tl!("API key in a header"))];
                let shown = methods.iter().find(|(code, _)| *code == st.auth).map_or(tl!("None"), |(_, name)| *name);
                let mut method_changed = false;
                egui::ComboBox::from_id_salt("connection_auth")
                    .selected_text(shown)
                    .width(360.0)
                    .show_ui(ui, |ui| {
                        for (code, name) in methods {
                            method_changed |= ui.selectable_value(&mut st.auth, code.to_string(), name).changed();
                        }
                    })
                    .response
                    .labelled_by(input_label.id);
                ui.end_row();
                if method_changed {
                    clear_secrets(st);
                    changed = true;
                }
                if st.auth == "basic" {
                    let input_label = ui.label(tl!("User"));
                    changed |= ui.add(egui::TextEdit::singleline(&mut st.user).desired_width(360.0)).labelled_by(input_label.id).changed();
                    ui.end_row();
                } else if st.auth == "header" {
                    let input_label = ui.label(tl!("Header name"));
                    changed |=
                        ui.add(egui::TextEdit::singleline(&mut st.key_name).desired_width(360.0)).labelled_by(input_label.id).changed();
                    ui.end_row();
                }
                if !st.auth.is_empty() {
                    let basic = st.auth == "basic";
                    let input_label = ui.label(if basic { tl!("Password") } else { tl!("Key") });
                    let field = if basic { &mut st.password } else { &mut st.api_key };
                    changed |=
                        ui.add(egui::TextEdit::singleline(field).password(true).desired_width(360.0)).labelled_by(input_label.id).changed();
                    ui.end_row();
                    if insecure(st) {
                        ui.label("");
                        changed |= ui.checkbox(&mut st.allow_insecure, tl!("Send the login over unencrypted http://")).changed();
                        ui.end_row();
                    } else {
                        st.allow_insecure = false;
                    }
                    if logged_in {
                        ui.label(tl!("Status"));
                        ui.vertical(|ui| {
                            ui.label(tl!("Logged in"));
                            logout = ui.button(tl!("Log Out")).clicked();
                        });
                        ui.end_row();
                    }
                }
            }
        }
    });
    if form_mode {
        ui.horizontal(|ui| {
            if !cfg!(target_arch = "wasm32")
                && ui.button(tl!("From .sde file…")).on_hover_text(tl!("Fill the form from an ArcGIS Pro connection file")).clicked()
            {
                from_sde = true;
            }
            toggle_form = ui.link(tl!("Enter an address instead")).clicked();
        });
    }
    changed |= toggle_form;
    // A credential-source switch never carries opt-in to the newly chosen source.
    if source_changed || !(database || st.kind == "s3" && st.credentials == "keys") {
        st.remember = false;
    }
    if changed {
        // A late download may finish, but its detached slot cannot report on the edited form.
        st.test = None;
        // Edited again: back to saving.
        st.warned = None;
        app.dialogs.info = None;
        app.dialogs.error = None;
    }
    if forget && let Some(id) = app.ui.connection.id.clone() {
        match forget_keys(app, &id) {
            Ok(()) => {
                app.dialogs.error = None;
                app.dialogs.info =
                    Some(tl!("The keys saved on this computer were forgotten; a login in this session keeps working.").to_string());
            }
            Err(e) => {
                app.dialogs.info = None;
                app.dialogs.error = Some(e);
            }
        }
    }
    if toggle_form {
        swap_form(&mut app.ui.connection);
    }
    if from_sde
        && let Some(path) = crate::dialogs::pick_open(&["sde"], tl!("ArcGIS connection file"))
        && let Err(e) = fill_from_sde(app, &path)
    {
        app.dialogs.info = None;
        app.dialogs.error = Some(e);
    }
    if logout && let Some(id) = app.ui.connection.id.clone() {
        match app.run("connection.logout", json!({"id":id})) {
            Ok(_) => app.dialogs.info = Some(tl!("Logged out. Enter the password or key to log in again.").into()),
            Err(e) => app.dialogs.error = Some(translated(&e)),
        }
    }
    // A login that worked but couldn't do all it was asked: said until closed (or edited again).
    if let Some(warning) = app.ui.connection.warned.clone() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!("Logged in for this session")).strong());
        ui.label(egui::RichText::new(warning).color(ui.visuals().warn_fg_color));
        return ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let accent = crate::theme::Tokens::get(ui.ctx()).accent;
                crate::widgets::dialog_button(ui, tl!("Close"), Some(accent), vec2(84.0, 28.0)).clicked()
            })
            .inner;
    }
    let st = &app.ui.connection;
    let testing = st.test.is_some();
    let unavailable = database && cfg!(target_arch = "wasm32");
    let blocked = test_blocked(app);
    let clicked = ui
        .horizontal(|ui| {
            let response = ui
                .add_enabled_ui(!testing && !unavailable && blocked.is_none(), |ui| {
                    crate::widgets::dialog_button(ui, tl!("Test"), None, vec2(84.0, 28.0))
                })
                .inner;
            let response = if unavailable {
                response.on_disabled_hover_text(tl!("Database connections are used by the desktop app"))
            } else if let Some(reason) = blocked {
                response.on_disabled_hover_text(reason)
            } else {
                response
            };
            if testing {
                ui.spinner();
            }
            response.clicked()
        })
        .inner;
    if clicked {
        start_test(app);
    }
    let close = crate::dialogs::buttons(app, ui, "Save", save);
    // A warning to read keeps it open.
    close && app.ui.connection.warned.is_none()
}

pub fn open_connection(app: &mut RustGisApp, c: &Connection) -> Result<(), String> {
    if web::is_service(c)
        && !c.auth.is_empty()
        && !web::logged_in(&app.session, c)
        && let Err(e) = app.run("connection.login", json!({"id":c.id}))
    {
        password_prompt(app, &c.id, password_request(app, &c.id), Some(e));
        return Ok(());
    }
    let (kind, extra) = match c.kind.as_str() {
        "postgres" | "mysql" | "sqlserver" => {
            if cfg!(target_arch = "wasm32") {
                return Err(tl!("Database connections are used by the desktop app").into());
            }
            if app.session.database_logins.has_session_password(&c.id) || c.remembered {
                let result = app.run("connection.login", json!({"id":c.id}));
                crate::connectiontree::refresh(app, Some(&c.id));
                let refused = password_request(app, &c.id);
                if result.is_err() || refused.is_some() {
                    password_prompt(app, &c.id, refused, result.err());
                    return Ok(());
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            if !app.session.database_logins.has_session_password(&c.id)
                && !c.remembered
                && let Some((_, key)) = database_key(&c.address)
            {
                // Picking an ID without credentials must not inherit another ID's activation.
                // An unowned, explicitly typed address password remains highest priority.
                rustgis_engine::data::source::duckdb::set_connection_password(&key, &c.id, None);
                app.session.database_logins.drop_activation(&key);
            }
            ("database", Value::Null)
        }
        "service" => ("service", json!({"type":"auto"})),
        "wms" | "wmts" | "wfs" | "wcs" | "ogcapi" | "arcgis" | "csw" => ("service", json!({"type":c.kind})),
        "stac" => ("stac", json!({"api":"true"})),
        "s3" => {
            if !app.session.has_s3_login(&c.id) {
                let keys = c.options.get("credentials").is_some_and(|v| v == "keys");
                // Keys typed for the session: the form, to type them. Keys saved on this computer
                // are read by this click's login (the system may ask to unlock its store).
                if keys && !c.saved_keys {
                    open_edit(app, &c.id)?;
                    app.ui.connection.login = true;
                    app.dialogs.info = Some(tl!("Type the keys to log in for this session, then Save.").into());
                    return Ok(());
                }
                if let Err(e) = app.run("connection.login", json!({"id":c.id})) {
                    if keys && needs_keys(&e) {
                        open_edit(app, &c.id)?;
                        app.ui.connection.login = true;
                        app.dialogs.error = Some(translated(&e));
                        app.dialogs.info = Some(tl!("Type the keys to log in for this session, then Save.").into());
                        return Ok(());
                    }
                    return Err(translated(&e));
                }
            }
            crate::s3ui::fill(app, c);
            let r = crate::sourcespane::open(
                app,
                &Source { kind: "s3".into(), name: c.name.clone(), address: c.name.clone(), extra: Value::Null },
            );
            if r.is_ok() {
                app.ui.s3.connection = Some(c.id.clone());
            }
            return r;
        }
        _ => return Err(format!("unknown kind '{}'", c.kind)),
    };
    if kind == "service" {
        return app.run("ui.addService", json!({"url":c.address, "type":extra["type"], "connection":c.id})).map(|_| ());
    }
    let r = crate::sourcespane::open(app, &Source { kind: kind.into(), name: c.name.clone(), address: c.address.clone(), extra });
    if r.is_ok() {
        if kind == "stac" {
            app.ui.stac.connection = Some(c.id.clone());
        } else {
            app.dialogs.connection = Some(c.id.clone());
        }
    }
    r
}

/// After a login from the form: the project's S3 sources that waited for that connection (the
/// chooser asked for its login) are given to it now.
fn resolve_waiting(app: &mut RustGisApp, id: &str) {
    let Some((source, waiting)) = app.ui.connection.resolving.take() else { return };
    if waiting == id
        && let Err(e) = choose_s3(app, &source, id)
    {
        app.session.notify(translated(&e), true);
    }
}

/// The user's choice of the saved connection `id` for the project's S3 sources waiting under
/// `source`: the engine logs it in if needed and opens them; nothing is read or signed here.
fn choose_s3(app: &mut RustGisApp, source: &str, id: &str) -> Result<(), String> {
    let v = app.run("connection.resolveS3", json!({"source": source, "id": id}))?;
    let name = app.session.connections.find(id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_string());
    let opened = v["reopened"].as_u64().unwrap_or(0);
    let failed = v["failed"].as_u64().unwrap_or(0);
    // Rasters read in place whose overview is on its way: the connection reads them, but they
    // aren't in yet (the desktop doesn't wait; they land as the map draws).
    let reading = v["reading"].as_u64().unwrap_or(0);
    let n = opened.saturating_add(reading).to_string();
    let (failed_n, reading_n) = (failed.to_string(), reading.to_string());
    let args: &[(&str, &str)] = &[("name", &name), ("n", &n), ("failed", &failed_n), ("reading", &reading_n)];
    let text = match (failed, reading) {
        (0, 0) => i18n::fmt("{name} now reads {n} project sources", args),
        (0, _) => i18n::fmt("{name} now reads {n} project sources; {reading} still being read", args),
        (_, 0) => i18n::fmt("{name} now reads {n} project sources; {failed} could not be read", args),
        _ => i18n::fmt("{name} now reads {n} project sources; {reading} still being read and {failed} could not be read", args),
    };
    app.session.notify(text, failed > 0);
    Ok(())
}

/// Opens the chooser of the saved S3 connections that read the project's waiting S3 sources
/// (`ui.s3Connections`), with nothing picked.
pub fn open_chooser(app: &mut RustGisApp) -> Result<(), String> {
    if app.session.s3_pending.is_empty() {
        return Err(tl!("No project sources are waiting for an S3 connection.").into());
    }
    let ch = &mut app.ui.s3_chooser;
    ch.picked.clear();
    ch.error = None;
    ch.auto = false;
    app.dialogs = crate::dialogs::Dialogs { open: Some("s3Connections"), ..Default::default() };
    Ok(())
}

/// Once a frame: a project replacing the open one drops what was picked for the old one (and a
/// source waiting for a login), and the chooser opens by itself once for a project with S3
/// sources waiting for a choice (not again after Cancel).
pub fn follow_project(app: &mut RustGisApp) {
    let opened = app.session.opened;
    if app.ui.s3_chooser.project != Some(opened) {
        app.ui.s3_chooser = Chooser { project: Some(opened), auto: true, ..Default::default() };
        app.ui.connection.resolving = None;
        if app.dialogs.open == Some("s3Connections") {
            app.dialogs.open = None;
        }
    }
    if app.ui.s3_chooser.auto && app.dialogs.open.is_none() {
        app.ui.s3_chooser.auto = false;
        if !app.session.s3_pending.is_empty() {
            let _ = open_chooser(app);
        }
    }
}

/// Where a waiting resource's bucket is, as the chooser says it.
fn endpoint_label(endpoint: Option<&str>) -> String {
    match endpoint {
        None => tl!("unknown (an older project)").to_string(),
        Some("") => tl!("Amazon S3").to_string(),
        Some(e) => e.to_string(),
    }
}

/// The chooser (dialog `s3Connections`): each bucket the project's waiting sources read, and the
/// saved connections that could read it. True when it should close.
pub fn s3_chooser(app: &mut RustGisApp, ui: &mut egui::Ui) -> bool {
    let pending = app.session.s3_pending.clone();
    app.ui.s3_chooser.picked.retain(|source, _| pending.iter().any(|p| &p.source == source));
    ui.label(
        egui::RichText::new(tl!(
            "More than one saved connection could read these buckets. Choose the one that reads each; the project's sources that read it stay closed until you do."
        ))
        .weak(),
    );
    if pending.is_empty() {
        ui.label(tl!("No project sources are waiting for an S3 connection."));
    }
    for p in &pending {
        ui.group(|ui| {
            ui.label(egui::RichText::new(i18n::fmt(
                "Bucket {bucket} at {endpoint}",
                &[("bucket", &p.bucket), ("endpoint", &endpoint_label(p.endpoint.as_deref()))],
            )));
            for id in &p.candidates {
                let Some(c) = app.session.connections.connections.iter().find(|c| &c.id == id) else { continue };
                let name = if c.group.is_empty() { c.name.clone() } else { format!("{} › {}", c.group, c.name) };
                let at = if c.address.is_empty() { tl!("Amazon S3") } else { c.address.as_str() };
                let selected = app.ui.s3_chooser.picked.get(&p.source) == Some(id);
                if ui.radio(selected, format!("{name} — {at}")).clicked() {
                    app.ui.s3_chooser.picked.insert(p.source.clone(), id.clone());
                }
            }
        });
    }
    if let Some(e) = &app.ui.s3_chooser.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    let mut close = false;
    let mut open = false;
    ui.add_space(4.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let accent = crate::theme::Tokens::get(ui.ctx()).accent;
        let any = !app.ui.s3_chooser.picked.is_empty();
        open = ui.add_enabled_ui(any, |ui| crate::widgets::dialog_button(ui, tl!("Open"), Some(accent), vec2(84.0, 28.0))).inner.clicked();
        // The sources stay closed.
        close = crate::widgets::dialog_button(ui, tl!("Cancel"), None, vec2(84.0, 28.0)).clicked();
    });
    if open {
        close = open_chosen(app);
    }
    close
}

/// Gives each resource picked to its connection. One whose keys must be typed opens that
/// connection's Log In, and the choice is made again once it has logged in. True when nothing
/// waits any more.
fn open_chosen(app: &mut RustGisApp) -> bool {
    app.ui.s3_chooser.error = None;
    let picks: Vec<(String, String)> = app.ui.s3_chooser.picked.iter().map(|(s, id)| (s.clone(), id.clone())).collect();
    for (source, id) in picks {
        match choose_s3(app, &source, &id) {
            Ok(()) => {
                app.ui.s3_chooser.picked.remove(&source);
            }
            Err(e) if needs_keys(&e) => {
                app.ui.s3_chooser.picked.remove(&source);
                if log_in(app, &id) {
                    app.ui.connection.resolving = Some((source, id));
                    app.dialogs.error = Some(translated(&e));
                }
                // Log In is open in its place.
                return false;
            }
            Err(e) => {
                app.ui.s3_chooser.error = Some(translated(&e));
                return false;
            }
        }
    }
    app.session.s3_pending.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edited_service_test_is_blocked_without_sending_or_saving_the_key() {
        let mut app = RustGisApp::with_sample();
        let c = app
            .run(
                "connection.new",
                json!({
                    "kind":"wms", "address":"https://test-form.example/wms", "auth":"header", "keyName":"X-Api-Key"
                }),
            )
            .unwrap();
        let id = c["id"].as_str().unwrap();
        let saved = app.session.connections.find(id).unwrap().clone();
        for field in ["address", "auth", "user", "keyName", "kind", "non-web kind"] {
            open_edit(&mut app, id).unwrap();
            let st = &mut app.ui.connection;
            st.api_key = "never-sent-form-key".into();
            match field {
                "address" => st.address = "https://other-test-form.example/wms".into(),
                "auth" => st.auth = "basic".into(),
                "user" => st.user = "unsaved-user".into(),
                "keyName" => st.key_name = "X-Other-Key".into(),
                "kind" => st.kind = "wmts".into(),
                "non-web kind" => st.kind = "postgres".into(),
                _ => {}
            }
            assert!(test_blocked(&app).is_some(), "{field}");
            start_test(&mut app);
            assert_eq!(app.dialogs.error.as_deref(), Some(web::SAVE_BEFORE_TEST), "{field}");
            assert!(app.ui.connection.test.is_none());
            assert_eq!(app.ui.connection.api_key, "never-sent-form-key");
            assert!(!app.session.database_logins.has_session_password(id));
            assert!(!web::logged_in(&app.session, &saved));
            assert_eq!(app.session.connections.find(id), Some(&saved));
        }
        open_edit(&mut app, id).unwrap();
        app.ui.connection.name = "An unsaved display name".into();
        assert!(test_blocked(&app).is_none(), "display-only edits don't change the login");
    }

    #[test]
    fn invalid_service_replacement_preserves_session_login_and_definition() {
        let _turn = crate::fetching::turn();
        let mut app = RustGisApp::with_sample();
        let c = app
            .run(
                "connection.new",
                json!({
                    "kind":"wms", "address":"https://replacement-form.example/wms", "auth":"basic", "user":"reader"
                }),
            )
            .unwrap();
        let id = c["id"].as_str().unwrap();
        app.run("connection.login", json!({"id":id, "password":"original-password"})).unwrap();
        let saved = app.session.connections.find(id).unwrap().clone();
        for password in ["bad\npassword".to_string(), "x".repeat(4097), "x".repeat(3070)] {
            open_edit(&mut app, id).unwrap();
            app.ui.connection.address = "https://unsaved-replacement-form.example/wms".into();
            app.ui.connection.password = password;
            assert!(save(&mut app).is_err());
            assert_eq!(app.session.connections.find(id), Some(&saved));
            assert_eq!(web::known_secret(&app.session, &saved).unwrap().expose(), "original-password");
        }
    }

    #[test]
    fn opening_a_connection_without_a_login_method_logs_nothing_in() {
        let mut app = RustGisApp::with_sample();
        let made = app.run("connection.new", json!({"kind":"wms", "address":"http://127.0.0.1:9/wms"})).unwrap();
        let c = app.session.connections.find(made["id"].as_str().unwrap()).unwrap().clone();
        let origin = login_origin(&c).unwrap();
        let _ = open_connection(&mut app, &c);
        // A login would choose this connection for its origin (and drop the tile cache).
        assert_eq!(app.session.database_logins.selection(WEB, &origin), None);
    }

    #[test]
    fn a_refused_s3_login_keeps_the_typed_keys_in_the_form() {
        let mut app = RustGisApp::with_sample();
        open_new(&mut app, "s3").unwrap();
        app.ui.connection.name = "Refused keys".into();
        app.ui.connection.access_key = "AKIAEXAMPLE".into();
        assert!(save(&mut app).is_err(), "a key ID without its secret is refused");
        assert_eq!(app.ui.connection.access_key, "AKIAEXAMPLE");
    }

    #[test]
    fn picked_requires_identity_and_the_unchanged_public_address() {
        let list = vec![
            Connection { id: "c1".into(), address: "https://example.com/wms".into(), ..Default::default() },
            Connection { id: "c2".into(), address: "https://example.com/wms".into(), ..Default::default() },
        ];
        assert!(picked(&list, None, &list[0].address).is_none());
        assert_eq!(picked(&list, Some("c2"), " https://example.com/wms ").map(|c| c.id.as_str()), Some("c2"));
        assert!(picked(&list, Some("removed"), &list[0].address).is_none());
        assert!(picked(&list, Some("c1"), "https://example.com/other").is_none());
        assert_eq!(picked(&list, Some("c1"), "https://example.com/wms?token=secret").map(|c| c.id.as_str()), Some("c1"));
    }

    fn form(sign_in: &str) -> State {
        State {
            form: true,
            server: "gis-db\\GIS".into(),
            user: "sde".into(),
            database: "utility".into(),
            sign_in: sign_in.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_form_composes_the_address() {
        let p = definition(&form("sql"));
        assert_eq!(p["address"], "mssql://sde@gis-db/utility?instance=GIS");
        assert_eq!(p["name"], "Geodatabase on gis-db\\GIS");
        // Windows sign-in names no user, whatever was typed in the User box.
        let windows = State { server: "gis-db".into(), ..form("windows") };
        assert_eq!(definition(&windows)["address"], "mssql://gis-db/utility");
        assert_eq!(definition(&form("windows"))["address"], "mssql://gis-db/utility?instance=GIS");
        // A name typed wins over the default.
        let named = State { name: "Utility GDB".into(), ..form("sql") };
        assert_eq!(definition(&named)["name"], "Utility GDB");
    }

    #[test]
    fn a_form_that_cannot_make_an_address_says_why() {
        let mut st = form("sql");
        st.server = "bad host!".into();
        assert!(form_error(&st).is_some());
        assert_eq!(definition(&st)["address"], "");
        st.server = "  ".into();
        assert!(form_error(&st).is_some());
        assert!(form_error(&form("sql")).is_none());
        // Address mode is never checked against the form.
        st.form = false;
        assert!(form_error(&st).is_none());
    }

    #[test]
    fn trusting_the_certificate_goes_into_the_address_and_back() {
        let mut st = form("sql");
        assert!(!form_address(&st).unwrap().contains("trust"));
        st.trust_cert = true;
        let address = form_address(&st).unwrap();
        assert!(address.ends_with("trust-server-certificate=true"), "{address}");
        assert_eq!(address.matches('?').count(), 1, "{address}");
        assert!(parse_mssql(&address).unwrap().trust_cert);
        // After an instance, the option joins the query.
        st.server = r"gis-db\GIS".into();
        let address = form_address(&st).unwrap();
        assert!(address.contains("?instance=GIS&trust-server-certificate=true"), "{address}");
        let back = parse_mssql(&address).unwrap();
        assert!(back.trust_cert);
        assert_eq!(back.server, r"gis-db\GIS");
        assert!(parse_mssql("mssql://h/d?TrustServerCertificate=yes").unwrap().trust_cert);
        assert!(!parse_mssql("mssql://h/d?trust-server-certificate=false").unwrap().trust_cert);
    }

    #[test]
    fn an_address_fills_the_form_back_in() {
        let f = |a: &str| parse_mssql(a).map(|p| (p.server, p.user, p.database));
        assert_eq!(f("mssql://sde@gis-db/utility?instance=GIS"), Some(("gis-db\\GIS".into(), Some("sde".into()), "utility".into())));
        assert_eq!(f("mssql://gis-db:1444/util%20ity"), Some(("gis-db,1444".into(), None, "util ity".into())));
        assert_eq!(f("mssql://gis-db:1433/u"), Some(("gis-db".into(), None, "u".into())));
        assert_eq!(f("postgres:host=x"), None);
        // What the form composes comes back as it was typed.
        let composed = form_address(&form("sql")).unwrap();
        let back = parse_mssql(&composed).unwrap();
        assert_eq!((back.server.as_str(), back.user.as_deref(), back.database.as_str()), ("gis-db\\GIS", Some("sde"), "utility"));
    }

    #[test]
    fn a_geodatabase_test_says_so() {
        let r = json!({"count": 12, "layers": [], "geodatabase": {"owner": "sde", "release": "10.9"}});
        assert_eq!(summary("sqlserver", &r), "Enterprise geodatabase 10.9, 12 tables with geometry");
        let r = json!({"count": 3, "geodatabase": {"owner": "sde", "release": null}});
        assert_eq!(summary("sqlserver", &r), "Enterprise geodatabase, 3 tables with geometry");
        let r = json!({"count": 3, "geodatabaseProblem": "no access"});
        assert_eq!(summary("sqlserver", &r), "3 tables with geometry · the geodatabase catalog couldn't be read: no access");
        assert_eq!(summary("sqlserver", &json!({"count": 3})), "3 tables with geometry");
    }

    fn sde_file(name: &str, text: &[u8]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        let path = path.display().to_string();
        (dir, path)
    }

    #[test]
    fn a_sde_file_fills_the_form_and_keeps_the_connection() {
        let (_dir, path) = sde_file(
            "Utility.sde",
            b"SERVER=gis-db;DBCLIENT=sqlserver;DATABASE=utility;USER=editor;AUTHENTICATION_MODE=DBMS;VERSION=sde.DEFAULT;INSTANCE=sde:sqlserver:gis-db",
        );
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "sdeFile": path})).unwrap();
        let st = &app.ui.connection;
        assert!(st.form);
        assert_eq!(
            (st.server.as_str(), st.database.as_str(), st.user.as_str(), st.sign_in.as_str()),
            ("gis-db", "utility", "editor", "sql")
        );
        assert_eq!(st.name, "Utility");
        let id = st.id.clone().expect("the connection is saved at once");
        assert_eq!(app.session.connections.find(&id).map(|c| c.address.as_str()), Some("mssql://editor@gis-db/utility"));
        assert_eq!(app.dialogs.info.as_deref(), Some("Type the password, then Save."));
        // Save is now an edit: the same connection, not a second one.
        save(&mut app).unwrap();
        assert_eq!(app.session.connections.connections.iter().filter(|c| c.kind == "sqlserver").count(), 1);
    }

    #[test]
    fn a_second_sde_file_replaces_the_first_and_is_named_from_itself() {
        let (_a, first) = sde_file("First.sde", b"SERVER=gis-db;DBCLIENT=sqlserver;DATABASE=one;USER=editor;");
        let (_b, second) = sde_file("Second.sde", b"SERVER=gis-db2;DBCLIENT=sqlserver;DATABASE=two;USER=reader;");
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "sdeFile": first})).unwrap();
        let old = app.ui.connection.id.clone().unwrap();
        fill_from_sde(&mut app, &second).unwrap();
        let st = &app.ui.connection;
        assert_eq!((st.name.as_str(), st.database.as_str(), st.user.as_str()), ("Second", "two", "reader"));
        let list = &app.session.connections.connections;
        assert_eq!(list.len(), 1, "no orphan from the first pick");
        assert_ne!(st.id.as_deref(), Some(old.as_str()));
        assert_eq!(list.first().map(|c| c.name.as_str()), Some("Second"));
        // The same file again is no duplicate of itself, and a name the person typed is kept.
        fill_from_sde(&mut app, &second).unwrap();
        assert_eq!(app.session.connections.connections.len(), 1);
        app.ui.connection.name = "Mine".into();
        app.ui.connection.typed_name = true;
        fill_from_sde(&mut app, &first).unwrap();
        assert_eq!(app.session.connections.connections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Mine"]);
        // Once filled, that name is the engine's again: the next file is named from itself.
        fill_from_sde(&mut app, &second).unwrap();
        assert_eq!(app.session.connections.connections.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Second"]);
    }

    #[test]
    fn a_sql_login_with_no_user_does_not_continue_a_repair() {
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "server": "gis-db", "database": "utility"})).unwrap();
        app.ui.connection.continue_repair = Some(7);
        // The form composes an address with no user: that is not Windows sign-in.
        assert_eq!(save(&mut app).unwrap_err(), "Type the password to connect");
        assert_eq!(app.ui.connection.continue_repair, Some(7));
    }

    #[test]
    fn a_windows_sde_file_selects_windows_sign_in() {
        let (_dir, path) = sde_file("os.sde", b"SERVER=gis-db\\GIS;DBCLIENT=sqlserver;DATABASE=utility;AUTHENTICATION_MODE=OSA;");
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "sdeFile": path})).unwrap();
        let st = &app.ui.connection;
        assert_eq!((st.server.as_str(), st.sign_in.as_str(), st.user.as_str()), ("gis-db\\GIS", "windows", ""));
        assert!(app.dialogs.info.as_deref().is_some_and(|i| i.contains("Windows user")));
    }

    #[test]
    fn a_sde_file_that_is_refused_says_why_and_leaves_no_connection() {
        let (_dir, path) = sde_file("pg.sde", b"SERVER=gis-db;DBCLIENT=postgresql;DATABASE=utility;USER=editor;");
        let mut app = RustGisApp::with_sample();
        // The form still opens, to enter the server by hand, with the reason in it.
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "sdeFile": path})).unwrap();
        assert_eq!(app.dialogs.open, Some("newConnection"));
        let e = app.dialogs.error.clone().unwrap();
        assert!(e.contains("a geodatabase on PostgreSQL; RustGIS reads SQL Server geodatabases"), "{e}");
        assert!(app.session.connections.connections.is_empty());
    }

    #[test]
    fn the_from_file_button_picks_a_file_and_cancelling_changes_nothing() {
        let (_dir, path) = sde_file("Utility.sde", b"SERVER=gis-db;DBCLIENT=sqlserver;DATABASE=utility;USER=editor;");
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true})).unwrap();
        assert!(crate::dialogs::pick_open(&["sde"], "x").is_none());
        crate::dialogs::PICKED.with(|p| *p.borrow_mut() = Some(path.clone()));
        let picked = crate::dialogs::pick_open(&["sde"], "x").unwrap();
        fill_from_sde(&mut app, &picked).unwrap();
        assert_eq!(app.ui.connection.database, "utility");
    }

    #[test]
    fn a_form_save_makes_the_connection_and_a_bad_form_makes_none() {
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "server": "gis-db\\GIS", "database": "utility"})).unwrap();
        assert_eq!((app.ui.connection.server.as_str(), app.ui.connection.database.as_str()), ("gis-db\\GIS", "utility"));
        app.ui.connection.user = "sde".into();
        app.ui.connection.server = "bad host!".into();
        assert!(save(&mut app).is_err());
        assert!(app.session.connections.connections.is_empty());
        app.ui.connection.server = "gis-db\\GIS".into();
        save(&mut app).unwrap();
        let list = app.session.connections.connections;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].address, "mssql://sde@gis-db/utility?instance=GIS");
        assert_eq!(list[0].name, "Geodatabase on gis-db\\GIS");
    }

    #[test]
    fn repair_continues_after_a_windows_save_and_waits_for_a_password_otherwise() {
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "server": "gis-db", "database": "utility"})).unwrap();
        app.ui.connection.continue_repair = Some(7);
        app.ui.connection.user = "sde".into();
        // A database user with no password: saved, but the dialog says what it waits for.
        let e = save(&mut app).unwrap_err();
        assert_eq!(e, "Type the password to connect");
        assert_eq!(app.ui.connection.continue_repair, Some(7));
        assert_eq!(app.session.connections.connections.len(), 1);
        // Windows sign-in has no password to wait for.
        app.ui.connection.sign_in = "windows".into();
        save(&mut app).unwrap();
        assert_eq!(app.ui.connection.continue_repair, None);
        assert_eq!(app.session.connections.connections.len(), 1);
    }

    #[test]
    fn the_form_and_the_address_swap_without_losing_what_was_typed() {
        let mut st = State { kind: "sqlserver".into(), address: "mssql://old@elsewhere/db".into(), ..form("sql") };
        swap_form(&mut st);
        assert!(!st.form);
        assert_eq!(definition(&st)["address"], "mssql://sde@gis-db/utility?instance=GIS");
        // A form that can't make an address leaves the address as it was.
        st.form = true;
        st.server = "bad host!".into();
        swap_form(&mut st);
        assert_eq!(st.address, "mssql://sde@gis-db/utility?instance=GIS");
        // Back to the form: its fields are as they were.
        swap_form(&mut st);
        assert!(st.form);
        assert_eq!((st.server.as_str(), st.user.as_str(), st.database.as_str()), ("bad host!", "sde", "utility"));
    }

    #[test]
    fn the_form_draws_its_rows_and_swaps_with_the_address() {
        use egui_kittest::kittest::Queryable;
        let mut app = RustGisApp::with_sample();
        app.run("ui.connection", json!({"kind": "sqlserver", "form": true})).unwrap();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 700.0)).build_ui_state(
            move |ui, app: &mut RustGisApp| {
                if !fonts {
                    crate::theme::install_fonts(ui.ctx(), &[], None);
                    fonts = true;
                    return;
                }
                dialog(app, ui);
            },
            app,
        );
        h.run_steps(2);
        for label in ["Platform", "Server", "Sign in", "User", "Password", "Database", "Name", "Group", "From .sde file…"] {
            assert!(h.query_by_label(label).is_some(), "{label}");
        }
        assert!(h.query_by_label("Connection").is_none());
        h.get_by_label("Enter an address instead").click();
        h.run_steps(2);
        assert!(!h.state().ui.connection.form);
        assert!(h.query_by_label("Connection").is_some());
        assert!(h.query_by_label("Server").is_none());
        h.get_by_label("Use the form instead").click();
        h.run_steps(2);
        assert!(h.state().ui.connection.form);
        // Windows sign-in has no user or password to type.
        h.state_mut().ui.connection.sign_in = "windows".into();
        h.run_steps(2);
        assert!(h.query_by_label("User").is_none());
        assert!(h.query_by_label("Password").is_none());
    }

    #[test]
    fn a_sde_file_dropped_or_added_opens_the_connection_form() {
        let (_dir, path) = sde_file("Utility.sde", b"SERVER=gis-db;DBCLIENT=sqlserver;DATABASE=utility;USER=editor;");
        let mut app = RustGisApp::with_sample();
        app.open_dropped(&path);
        assert_eq!(app.dialogs.open, Some("newConnection"));
        assert!(app.ui.connection.form);
        assert_eq!(app.ui.connection.server, "gis-db");
        let mut app = RustGisApp::with_sample();
        let layers = app.session.project.maps.first().map_or(0, |m| m.layers.len());
        crate::layerchooser::add_file(&mut app, &path).unwrap();
        assert_eq!(app.dialogs.open, Some("newConnection"));
        assert_eq!(app.session.project.maps.first().map_or(0, |m| m.layers.len()), layers);
    }

    #[test]
    fn remember_needs_a_store_and_no_session_token() {
        assert!(remember_blocked(true, "").is_none());
        assert!(remember_blocked(true, "  ").is_none());
        assert!(remember_blocked(false, "").is_some());
        assert!(remember_blocked(true, "token").is_some());
    }

    fn new_s3_form(app: &mut RustGisApp, name: &str) {
        app.run("ui.connection", json!({"kind": "s3"})).unwrap();
        let st = &mut app.ui.connection;
        assert!(!st.remember, "Remember starts off");
        st.name = name.into();
        st.access_key = "AKIDREMEMBERUI".into();
        st.secret_key = "remember-ui-secret".into();
        st.remember = true;
    }

    #[test]
    fn remember_without_a_database_password_changes_nothing() {
        let mut app = RustGisApp::with_sample();
        app.session.database_logins.set_store(Some(std::sync::Arc::new(rustgis_engine::data::secrets::MemorySecrets::new())));
        app.run("ui.connection", json!({"kind": "postgres"})).unwrap();
        app.ui.connection.remember = true;
        assert!(save(&mut app).is_err());
        assert!(app.session.connections.connections.is_empty());
    }

    #[test]
    fn remember_without_a_typed_pair_changes_nothing() {
        let mut app = RustGisApp::with_sample();
        app.session.database_logins.set_store(Some(std::sync::Arc::new(rustgis_engine::data::secrets::MemorySecrets::new())));
        new_s3_form(&mut app, "remember-ui-partial-test");
        app.ui.connection.secret_key.clear();
        assert!(save(&mut app).is_err());
        assert!(app.session.connections.connections.is_empty(), "nothing saved");
    }
}
