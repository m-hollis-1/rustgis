//! Repair Source…: a broken feature layer explained (where its data was, why it doesn't open)
//! with the fix that fits what went wrong (`layer.diagnose`): signing in to ArcGIS Online,
//! another address for a service, the file found elsewhere (and the other broken layers from its
//! folder), trying again, or removing the layer. A fix that works closes the dialog with a notice;
//! one that fails keeps it open, diagnosed again.

use crate::RustGisApp;
use crate::theme::Tokens;
use rustgis_engine::Session;
use rustgis_engine::connections::Connection;
use rustgis_engine::sources::redact_text;
use serde_json::{Value, json};

/// The dialog's name for `ui.dialog`.
pub const NAME: &str = "repairBroken";

/// What stops the layer opening, as `layer.diagnose` sorts it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    SignIn,
    Certificate,
    Unreachable,
    ServiceMissing,
    FileMissing,
    /// A layer ArcGIS Pro read from an enterprise geodatabase.
    Geodatabase,
    Other,
}

impl Kind {
    fn parse(kind: &str) -> Self {
        match kind {
            "signIn" => Kind::SignIn,
            "certificate" => Kind::Certificate,
            "unreachable" => Kind::Unreachable,
            "serviceMissing" => Kind::ServiceMissing,
            "fileMissing" => Kind::FileMissing,
            "geodatabase" => Kind::Geodatabase,
            _ => Kind::Other,
        }
    }
}

/// What `layer.diagnose` said, its reason without the keys or tokens an address in it may carry.
struct Diagnosis {
    kind: Kind,
    reason: String,
    server: Option<String>,
    /// The file it read, on this computer.
    path: Option<String>,
    /// (The web app has no files to browse for, so never asks about the folder.)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    folder: Option<String>,
    /// How many other broken layers read their data from the same folder (or, for a geodatabase
    /// layer, from the same geodatabase).
    siblings: usize,
    /// An enterprise geodatabase layer's database, table, platform (as Pro wrote it, maybe empty)
    /// and version.
    database: String,
    table: String,
    platform: String,
    #[allow(dead_code)]
    version: Option<String>,
    /// The saved connections that fit it, as (id, name, exact), exact ones first.
    connections: Vec<(String, String, bool)>,
}

/// What the dialog is asking about, after the layer was repaired.
enum Ask {
    /// The file was found in another folder; the folder's other broken layers can be fixed too.
    /// Only Browse for the file… builds it, and that's desktop-only.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Folder { from: String, to: String, count: usize },
    /// The layer was repaired through a connection; the geodatabase's other broken layers can be
    /// too.
    Geodatabase { connection: String, name: String, count: usize },
}

impl Ask {
    fn text(&self) -> String {
        match self {
            Ask::Folder { count, .. } => ask_text(*count),
            Ask::Geodatabase { count, .. } => geodatabase_ask_text(*count),
        }
    }
}

/// What the dialog was asked to do this frame.
enum Action {
    Retry,
    SignIn,
    Relink(String),
    Browse,
    FixSiblings(bool),
    /// Repair the layer through the saved connection (its id).
    ConnectAndRepair(String),
    /// Add a SQL Server connection to the layer's geodatabase.
    AddConnection,
    /// Type the password of the saved connection (its id), then repair.
    LogIn(String),
    Remove,
}

/// The broken layer the dialog explains, captured with its project and map (layer ids are
/// map-local).
pub struct Form {
    layer: u32,
    opened: u64,
    map: (usize, String),
    name: String,
    /// Where its data was (redacted).
    was: String,
    diagnosis: Diagnosis,
    /// Use another address…: the address being typed, once asked for.
    address: Option<String>,
    /// Asking whether to fix the folder's other broken layers too.
    ask: Option<Ask>,
    /// The last action's error, when the reason doesn't already say it.
    error: Option<String>,
    /// The saved connection (id) the exact ones' list has picked; the first when none.
    chosen: Option<String>,
    /// The connection (id) whose failed repair was about its password: Log in… offers to type it.
    login: Option<String>,
    /// A sign-in started here is under way.
    signing_in: bool,
}

impl Form {
    /// Opens on broken feature layer `layer`; an error for a raster, or a layer that isn't broken.
    pub fn new(app: &mut RustGisApp, layer: u32) -> Result<Self, String> {
        let s = &mut app.session;
        let l = s.map().layer(layer).ok_or_else(|| tl!("Layer not found").to_string())?;
        if l.is_raster() {
            return Err(tl!("Repair Source… is for feature layers").to_string());
        }
        let name = l.name.clone();
        let was = l
            .source
            .as_ref()
            .and_then(|src| {
                let origin = src.origin.clone().filter(|o| !o.trim().is_empty());
                origin.or_else(|| src.path.clone()).or_else(|| src.service.as_ref().map(|v| v.url.clone()))
            })
            .unwrap_or_default();
        let diagnosis = diagnose(s, layer)?;
        Ok(Self {
            layer,
            opened: s.opened,
            map: (s.project.active_map, s.map().name.clone()),
            name,
            was: redact_text(&was),
            diagnosis,
            address: None,
            ask: None,
            error: None,
            chosen: None,
            login: None,
            signing_in: false,
        })
    }
}

fn diagnose(s: &mut Session, layer: u32) -> Result<Diagnosis, String> {
    let r = s.run("layer.diagnose", &json!({"layer": layer}))?;
    let text = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(Diagnosis {
        kind: Kind::parse(r.get("kind").and_then(Value::as_str).unwrap_or("")),
        reason: redact_text(&text("reason").unwrap_or_default()),
        server: text("server"),
        path: text("path"),
        folder: text("folder"),
        siblings: r.get("siblings").and_then(Value::as_array).map_or(0, Vec::len),
        database: text("database").unwrap_or_default(),
        table: text("table").unwrap_or_default(),
        platform: text("platform").unwrap_or_default(),
        version: text("version"),
        connections: r
            .get("connections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let id = c.get("id").and_then(Value::as_str)?;
                let name = c.get("name").and_then(Value::as_str)?;
                Some((id.to_string(), name.to_string(), c.get("exact").and_then(Value::as_bool).unwrap_or(false)))
            })
            .collect(),
    })
}

/// The folder a file is in (either separator); a drive's or the file system's root keeps its
/// separator (`C:\`, `/`).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn folder_of(path: &str) -> Option<&str> {
    let i = path.rfind(['/', '\\'])?;
    let folder = path.get(..i)?;
    if folder.is_empty() || folder.ends_with(':') { path.get(..=i) } else { Some(folder) }
}

/// The saved connections that fit a geodatabase layer, as they are now (one may have been
/// removed since the diagnosis).
struct Choices {
    /// The same server and database.
    exact: Vec<Connection>,
    /// The same database on another host, as (id, name, the host its address names).
    near: Vec<(String, String, String)>,
}

fn choices(s: &Session, d: &Diagnosis) -> Choices {
    let mut out = Choices { exact: Vec::new(), near: Vec::new() };
    for (id, _, exact) in &d.connections {
        let Some(c) = s.connections.find(id) else { continue };
        if *exact {
            out.exact.push(c.clone());
        } else {
            let host = rustgis_engine::geodatabase::address_key(&c.address).map(|(server, _)| server.host).unwrap_or_default();
            out.near.push((c.id.clone(), c.name.clone(), host));
        }
    }
    out
}

/// Whether RustGIS can connect to the geodatabase's platform (Pro's text; blank is taken as SQL
/// Server, the one the saved connections reach).
fn sql_server(platform: &str) -> bool {
    platform.trim().is_empty() || platform.trim().eq_ignore_ascii_case("sqlserver")
}

/// A platform as people write it (`PostgreSQL` for Pro's `postgresql`); another as Pro wrote it.
fn platform_name(platform: &str) -> &str {
    let platform = platform.trim();
    match platform.to_ascii_lowercase().as_str() {
        "postgresql" => "PostgreSQL",
        "oracle" => "Oracle",
        "db2" => "Db2",
        "informix" => "Informix",
        "sqlserver" => "SQL Server",
        _ => platform,
    }
}

/// What a geodatabase layer's diagnosis means, and which connection (if any) serves it.
fn geodatabase_text(d: &Diagnosis, c: &Choices) -> String {
    if !sql_server(&d.platform) {
        // "on {platform}", so no article depends on the platform's name ("a oracle").
        return crate::i18n::fmt(
            "This layer is in a geodatabase on {platform}, which RustGIS can't connect to yet.",
            &[("platform", platform_name(&d.platform))],
        );
    }
    let server = d.server.as_deref().unwrap_or(tl_raw!("its server"));
    let args = [("server", server), ("database", d.database.as_str()), ("table", d.table.as_str())];
    if !c.exact.is_empty() {
        let names: Vec<&str> = c.exact.iter().map(|x| x.name.as_str()).collect();
        let name = names.join(", ");
        let mut args = args.to_vec();
        args.push(("name", name.as_str()));
        crate::i18n::fmt(
            "This layer is in an enterprise geodatabase on {server}, database {database} (table {table}). You have a connection to it: {name}.",
            &args,
        )
    } else if let Some((_, name, host)) = c.near.first() {
        let mut args = args.to_vec();
        args.extend([("name", name.as_str()), ("host", host.as_str())]);
        crate::i18n::fmt(
            "This layer is in an enterprise geodatabase on {server}, database {database} (table {table}). {name} is a connection to a database called {database} on {host}.",
            &args,
        )
    } else {
        crate::i18n::fmt(
            "This layer is in an enterprise geodatabase on {server}, database {database}. RustGIS needs a connection to it.",
            &[("server", server), ("database", d.database.as_str())],
        )
    }
}

/// What the diagnosis means, in words.
fn explain(d: &Diagnosis, choices: &Choices) -> String {
    let server = d.server.as_deref().unwrap_or(tl_raw!("its server"));
    match d.kind {
        Kind::SignIn => crate::i18n::fmt("This layer is on {server}, which needs you to sign in.", &[("server", server)]),
        Kind::Certificate => crate::i18n::fmt(
            "{server}'s security certificate isn't trusted on this computer, so RustGIS can't connect safely. ArcGIS Pro may have been set to accept it anyway. Ask whoever runs that server to install a certificate from a trusted authority, or check you're on the network it's meant for.",
            &[("server", server)],
        ),
        Kind::Unreachable => crate::i18n::fmt("{server} didn't answer.", &[("server", server)]),
        Kind::ServiceMissing => crate::i18n::fmt("{server} answered, but the service isn't there any more.", &[("server", server)]),
        Kind::FileMissing => crate::i18n::fmt("The file isn't at {path}.", &[("path", d.path.as_deref().unwrap_or(""))]),
        Kind::Geodatabase => geodatabase_text(d, choices),
        Kind::Other => d.reason.clone(),
    }
}

/// The question after the file was found elsewhere.
fn ask_text(count: usize) -> String {
    if count == 1 {
        tl!("1 other broken layer had data in the same folder. Fix it too?").to_string()
    } else {
        crate::i18n::fmt("{n} other broken layers had data in the same folder. Fix them too?", &[("n", &count.to_string())])
    }
}

/// The question after a geodatabase layer was repaired.
fn geodatabase_ask_text(count: usize) -> String {
    if count == 1 {
        tl!("1 other broken layer is in the same geodatabase. Repair it too?").to_string()
    } else {
        crate::i18n::fmt("{n} other broken layers are in the same geodatabase. Repair them too?", &[("n", &count.to_string())])
    }
}

/// Whether `layer` is still a broken layer of the map.
fn still_broken(s: &Session, layer: u32) -> bool {
    s.map().layer(layer).and_then(|l| l.source.as_ref()).is_some_and(|src| src.is_broken())
}

/// Shows the diagnosis and its fixes, and runs the one picked.
pub fn show(app: &mut RustGisApp, ctx: &egui::Context) {
    let Some(mut f) = app.dialogs.broken.take() else {
        app.dialogs.open = None;
        return;
    };
    let s = &app.session;
    let moved = s.opened != f.opened || s.project.active_map != f.map.0 || s.map().name != f.map.1;
    // Fixed meanwhile (a sign-in's retry, say) or gone: nothing left to explain. Not while asking
    // about the folder's other layers, after this one was fixed.
    if moved || (f.ask.is_none() && !still_broken(s, f.layer)) {
        app.dialogs.open = None;
        return;
    }
    let pending = crate::arcgisui::signing_in(app);
    if f.signing_in && !pending {
        // Back from the browser: signed in (the broken layers were retried) or not.
        f.signing_in = false;
        let failure = crate::arcgisui::sign_in_failure(app);
        refresh(&mut app.session, &mut f, failure);
    }
    let choices = choices(&app.session, &f.diagnosis);
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut action = None;
    let mut close = false;
    egui::Window::new(tl!("Repair Source"))
        .id(egui::Id::unique("repair_broken"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center() + egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            ui.set_width(460.0);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            egui::Grid::new("repair_broken_details").num_columns(2).spacing(egui::vec2(8.0, 8.0)).show(ui, |ui| {
                ui.label(tl!("Layer"));
                ui.add(egui::Label::new(&f.name).wrap());
                ui.end_row();
                ui.label(tl!("Its data was"));
                ui.add(egui::Label::new(egui::RichText::new(&f.was).monospace()).wrap().selectable(true));
                ui.end_row();
                ui.label(tl!("Reason"));
                ui.add(egui::Label::new(egui::RichText::new(&f.diagnosis.reason).color(t.red)).wrap().selectable(true));
                ui.end_row();
            });
            ui.separator();
            if let Some(ask) = &f.ask {
                ui.add(egui::Label::new(ask.text()).wrap());
                ui.horizontal(|ui| {
                    if crate::widgets::dialog_button(ui, tl!("Yes"), Some(t.accent), egui::vec2(84.0, 28.0)).clicked() {
                        action = Some(Action::FixSiblings(true));
                    }
                    if crate::widgets::dialog_button(ui, tl!("No"), None, egui::vec2(84.0, 28.0)).clicked() {
                        action = Some(Action::FixSiblings(false));
                    }
                });
                return;
            }
            ui.add(egui::Label::new(egui::RichText::new(explain(&f.diagnosis, &choices)).strong()).wrap());
            let geodatabase = f.diagnosis.kind == Kind::Geodatabase && sql_server(&f.diagnosis.platform);
            // Which exact connection repairs it, when several fit.
            let exact = choices.exact.iter().find(|c| f.chosen.as_ref() == Some(&c.id)).or(choices.exact.first());
            if geodatabase && !cfg!(target_arch = "wasm32") {
                if choices.exact.is_empty() {
                    for (_, name, host) in choices.near.iter().skip(1) {
                        let args = [("name", name.as_str()), ("database", f.diagnosis.database.as_str()), ("host", host.as_str())];
                        let text = crate::i18n::fmt("{name} is a connection to a database called {database} on {host}.", &args);
                        ui.add(egui::Label::new(text).wrap());
                    }
                } else if choices.exact.len() > 1
                    && let Some(c) =
                        crate::connectionui::picker_combo(ui, "repair_geodatabase_connection", &choices.exact, &["sqlserver"], exact)
                {
                    f.chosen = Some(c.id);
                }
            }
            if f.diagnosis.kind == Kind::Geodatabase && cfg!(target_arch = "wasm32") {
                ui.label(tl!("Database connections are used by the desktop app"));
            }
            if let Some(address) = &mut f.address {
                let label = ui.label(tl!("The service's new address"));
                ui.add(egui::TextEdit::singleline(address).hint_text("https://…/FeatureServer/0").desired_width(f32::INFINITY))
                    .labelled_by(label.id);
                let ready = !address.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new(tl!("Open From This Address"))).clicked() {
                    action = Some(Action::Relink(address.trim().to_string()));
                }
            }
            if pending && f.signing_in {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if cfg!(target_arch = "wasm32") {
                        tl!("Finish signing in in the pop-up window…")
                    } else {
                        tl!("Finish signing in in the browser…")
                    });
                });
            }
            if let Some(e) = &f.error {
                ui.add(egui::Label::new(egui::RichText::new(e).color(t.red)).wrap());
            }
            ui.horizontal_wrapped(|ui| {
                let size = egui::vec2(84.0, 28.0);
                let kind = f.diagnosis.kind;
                if kind == Kind::SignIn {
                    let sign_in = ui.add_enabled_ui(!pending, |ui| {
                        crate::widgets::dialog_button(ui, tl!("Sign in to ArcGIS Online"), Some(t.accent), size)
                    });
                    if sign_in.inner.clicked() {
                        action = Some(Action::SignIn);
                    }
                }
                if geodatabase && !cfg!(target_arch = "wasm32") {
                    if let Some(c) = exact {
                        if crate::widgets::dialog_button(ui, tl!("Connect and repair"), Some(t.accent), size).clicked() {
                            action = Some(Action::ConnectAndRepair(c.id.clone()));
                        }
                    } else {
                        for (id, name, _) in &choices.near {
                            let label = crate::i18n::fmt("Use {name}", &[("name", name)]);
                            if crate::widgets::dialog_button(ui, &label, Some(t.accent), size).clicked() {
                                action = Some(Action::ConnectAndRepair(id.clone()));
                            }
                        }
                        if crate::widgets::dialog_button(ui, tl!("Add connection…"), None, size).clicked() {
                            action = Some(Action::AddConnection);
                        }
                    }
                    if let Some(id) = &f.login
                        && crate::widgets::dialog_button(ui, tl!("Log in…"), Some(t.accent), size).clicked()
                    {
                        action = Some(Action::LogIn(id.clone()));
                    }
                }
                if f.diagnosis.path.is_some() {
                    let fill = (kind == Kind::FileMissing).then_some(t.accent);
                    if crate::widgets::dialog_button(ui, tl!("Browse for the file…"), fill, size).clicked() {
                        action = Some(Action::Browse);
                    }
                }
                if !matches!(kind, Kind::FileMissing | Kind::Geodatabase)
                    && crate::widgets::dialog_button(ui, tl!("Retry"), None, size).clicked()
                {
                    action = Some(Action::Retry);
                }
                if matches!(kind, Kind::Unreachable | Kind::ServiceMissing)
                    && f.address.is_none()
                    && crate::widgets::dialog_button(ui, tl!("Use another address…"), None, size).clicked()
                {
                    f.address = Some(String::new());
                }
                if crate::widgets::dialog_button(ui, tl!("Remove"), None, size).clicked() {
                    action = Some(Action::Remove);
                }
                if crate::widgets::dialog_button(ui, tl!("Close"), None, size).clicked() {
                    close = true;
                }
            });
        });
    if !open || close {
        app.dialogs.open = None;
        return;
    }
    let layer = f.layer;
    let name = f.name.clone();
    // What Add connection… fills the form with: the layer's server and database.
    let (server, database) = (f.diagnosis.server.clone(), f.diagnosis.database.clone());
    app.dialogs.broken = Some(f);
    match action {
        None => {}
        // A service can take a while to answer, or not to: the working window says so meanwhile.
        Some(Action::Retry) => {
            let message = crate::i18n::fmt("Trying {name} again…", &[("name", &name)]);
            crate::busy::queue_short(app, message, move |app| retry(app, layer, &name));
        }
        Some(Action::Relink(url)) => {
            let message = crate::i18n::fmt("Opening {name} from its new address…", &[("name", &name)]);
            crate::busy::queue_short(app, message, move |app| relink(app, layer, &name, &url));
        }
        Some(Action::SignIn) => {
            let started = crate::arcgisui::sign_in_online(app);
            if let Some(f) = app.dialogs.broken.as_mut() {
                f.signing_in = started.is_ok();
                f.error = started.err();
            }
        }
        Some(Action::Browse) => browse(app, layer, name),
        Some(Action::FixSiblings(yes)) => fix_siblings(app, yes),
        // A connection that signs in with a password it doesn't have asks for it first, then
        // comes back here to repair.
        Some(Action::ConnectAndRepair(connection)) if needs_password(&app.session, &connection) => {
            if crate::connectionui::log_in(app, &connection) {
                app.ui.connection.continue_repair = Some(layer);
            }
        }
        Some(Action::ConnectAndRepair(connection)) => {
            if let Some(f) = app.dialogs.broken.as_mut() {
                f.login = None;
            }
            let shown = app.session.connections.find(&connection).map(|c| c.name.clone()).unwrap_or_default();
            let message = crate::i18n::fmt("Repairing {name} through {connection}…", &[("name", &name), ("connection", &shown)]);
            crate::busy::queue_short(app, message, move |app| repair_geodatabase(app, layer, &name, &connection));
        }
        Some(Action::AddConnection) => {
            match app.run("ui.connection", json!({"kind": "sqlserver", "form": true, "server": server, "database": database})) {
                // Back here, repairing, once the connection is saved and logged in.
                Ok(_) => app.ui.connection.continue_repair = Some(layer),
                Err(e) => failed(app, layer, e),
            }
        }
        Some(Action::LogIn(connection)) => {
            if crate::connectionui::log_in(app, &connection) {
                app.ui.connection.continue_repair = Some(layer);
            }
        }
        Some(Action::Remove) => match app.run("layer.remove", json!({"layer": layer})) {
            Ok(_) => done(app, crate::i18n::fmt("Removed {name}", &[("name", &name)])),
            Err(e) => failed(app, layer, e),
        },
    }
}

/// The form, while the dialog is still open on `layer` (an action queued behind the working
/// window runs a frame or more later).
fn form(app: &mut RustGisApp, layer: u32) -> Option<&mut Form> {
    if app.dialogs.open != Some(NAME) {
        return None;
    }
    app.dialogs.broken.as_mut().filter(|f| f.layer == layer)
}

/// The layer fixed (or removed): the dialog closes, `notice` says so.
fn done(app: &mut RustGisApp, notice: String) {
    if app.dialogs.open == Some(NAME) {
        app.dialogs.open = None;
        app.dialogs.broken = None;
    }
    app.session.notify(notice, false);
}

/// Diagnosed again after a failed action, showing `error` unless the new reason says it.
fn refresh(s: &mut Session, f: &mut Form, error: Option<String>) {
    let error = error.map(|e| redact_text(&e));
    match diagnose(s, f.layer) {
        Ok(d) => {
            f.error = error.filter(|e| *e != d.reason);
            f.diagnosis = d;
        }
        // No longer broken (opening off the frame, say): the next frame closes the dialog.
        Err(e) => f.error = error.or(Some(e)),
    }
}

/// An action failed on `layer`: the dialog stays, diagnosed again (or, closed meanwhile, a
/// notice says why).
fn failed(app: &mut RustGisApp, layer: u32, error: String) {
    if form(app, layer).is_none() {
        app.session.notify(redact_text(&error), true);
        return;
    }
    let Some(mut f) = app.dialogs.broken.take() else { return };
    refresh(&mut app.session, &mut f, Some(error));
    app.dialogs.broken = Some(f);
}

/// Layer `name` opens again: the dialog closes with a notice.
fn repaired(app: &mut RustGisApp, name: &str) {
    done(app, crate::i18n::fmt("Repaired {name}", &[("name", name)]));
}

/// The notice after `layer.retry`'s result `r` with no failure: opened, or (read off the frame,
/// a file by its web address) still loading.
fn retried(r: &Value, name: &str) -> String {
    if r.get("opened").and_then(Value::as_u64).unwrap_or(0) > 0 {
        crate::i18n::fmt("Repaired {name}", &[("name", name)])
    } else {
        crate::i18n::fmt("{name} is opening; it shows when it has loaded", &[("name", name)])
    }
}

fn retry(app: &mut RustGisApp, layer: u32, name: &str) {
    match app.run("layer.retry", json!({"layer": layer})) {
        Ok(r) => {
            let error = r.get("failed").and_then(|f| f.get(0)).and_then(|f| f.get("error")).and_then(Value::as_str);
            match error {
                Some(e) => failed(app, layer, e.to_string()),
                None => done(app, retried(&r, name)),
            }
        }
        Err(e) => failed(app, layer, e),
    }
}

fn relink(app: &mut RustGisApp, layer: u32, name: &str, url: &str) {
    match app.run("layer.relinkService", json!({"layer": layer, "url": url})) {
        Ok(_) => repaired(app, name),
        Err(e) => failed(app, layer, e),
    }
}

/// Browse for the file…: the layer reads the file picked; when its old folder held other broken
/// layers' data, the dialog asks whether to fix those too.
#[cfg(not(target_arch = "wasm32"))]
fn browse(app: &mut RustGisApp, layer: u32, name: String) {
    let Some(path) = crate::dialogs::pick_open(&[], tl!("Data")) else { return };
    let message = crate::i18n::fmt("Opening {name} from {file}…", &[("name", &name), ("file", crate::busy::file_name(&path))]);
    crate::busy::queue_short(app, message, move |app| match app.run("layer.repairSource", json!({"layer": layer, "path": path})) {
        Ok(_) => {
            let Some(f) = form(app, layer) else {
                repaired(app, &name);
                return;
            };
            match (f.diagnosis.folder.clone(), folder_of(&path)) {
                (Some(from), Some(to)) if f.diagnosis.siblings > 0 => {
                    f.error = None;
                    f.ask = Some(Ask::Folder { from, to: to.to_string(), count: f.diagnosis.siblings });
                }
                _ => repaired(app, &name),
            }
        }
        Err(e) => failed(app, layer, e),
    });
}

/// The web app can't read files on this computer by their path.
#[cfg(target_arch = "wasm32")]
fn browse(app: &mut RustGisApp, layer: u32, _name: String) {
    if let Some(f) = form(app, layer) {
        f.error = Some(tl!("The web app can't link to files on this computer: add the file again with Add Data.").to_string());
    }
}

/// The answer to "Fix them too?" / "Repair them too?": Yes repairs the other broken layers (of the
/// old folder, from the new one; of the geodatabase, through the connection), behind the working
/// window.
fn fix_siblings(app: &mut RustGisApp, yes: bool) {
    let Some(f) = app.dialogs.broken.as_ref() else { return };
    let name = f.name.clone();
    match f.ask.as_ref().filter(|_| yes) {
        None => repaired(app, &name),
        Some(Ask::Folder { from, to, .. }) => {
            let (from, to) = (from.clone(), to.clone());
            let message = crate::i18n::fmt("Repairing the other layers from {folder}…", &[("folder", &from)]);
            crate::busy::queue_short(app, message, move |app| repair_folder(app, &name, &from, &to));
        }
        Some(Ask::Geodatabase { connection, name: shown, .. }) => {
            let connection = connection.clone();
            let message = crate::i18n::fmt("Repairing the other layers through {connection}…", &[("connection", shown)]);
            crate::busy::queue_short(app, message, move |app| repair_geodatabase_rest(app, &name, &connection));
        }
    }
}

/// The notices after `name` and the other layers `r` (`repaired` and `skipped`) were repaired:
/// the dialog closes saying which were, and which weren't.
fn notify_repaired(app: &mut RustGisApp, name: &str, r: &Value) {
    let mut all = vec![name.to_string()];
    all.extend(r.get("repaired").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
    done(app, crate::i18n::fmt("Repaired {names}", &[("names", &all.join(", "))]));
    let skipped: Vec<String> = r
        .get("skipped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|s| {
            let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("");
            format!("{} ({})", text("layer"), redact_text(text("why")))
        })
        .collect();
    if !skipped.is_empty() {
        app.session.notify(crate::i18n::fmt("Not repaired: {layers}", &[("layers", &skipped.join("; "))]), true);
    }
}

/// The old folder's other broken layers repaired from `to`; the notices list which were and
/// which weren't.
fn repair_folder(app: &mut RustGisApp, name: &str, from: &str, to: &str) {
    match app.run("layer.repairFolder", json!({"from": from, "to": to})) {
        Ok(r) => notify_repaired(app, name, &r),
        Err(e) => {
            repaired(app, name);
            app.session.notify(redact_text(&e), true);
        }
    }
}

/// Whether repairing through connection `id` needs a password typed first: its address names a
/// user (not a Windows sign-in) and no password is known for it this session or remembered.
fn needs_password(s: &Session, id: &str) -> bool {
    let Some(c) = s.connections.find(id) else { return false };
    rustgis_engine::geodatabase::has_user(&c.address)
        && s.database_logins.known_password(id).is_none()
        && !(c.remembered && !s.database_logins.is_rejected(id))
}

/// Whether a failed repair through connection `id` was about its password: the connection has
/// none to sign in with (the server refused it, or none was given), or the error says so.
fn login_problem(s: &Session, id: &str, error: &str) -> bool {
    let said = error.to_lowercase();
    s.database_logins.known_password(id).is_none() || ["password", "login", "log in", "sign in"].iter().any(|w| said.contains(w))
}

/// Repair through the saved connection `connection`: the layer reads its table again. When the
/// geodatabase has other broken layers (and this connection fits them exactly), the dialog asks
/// whether to repair them too; a password problem keeps the dialog open with Log in….
fn repair_geodatabase(app: &mut RustGisApp, layer: u32, name: &str, connection: &str) {
    match app.run("layer.repairGeodatabase", json!({"layer": layer, "connection": connection})) {
        Ok(_) => {
            let Some(f) = form(app, layer) else {
                repaired(app, name);
                return;
            };
            let exact = f.diagnosis.connections.iter().any(|(id, _, exact)| *exact && id == connection);
            let count = f.diagnosis.siblings;
            match app.session.connections.find(connection).map(|c| c.name.clone()).filter(|_| exact && count > 0) {
                Some(shown) => {
                    if let Some(f) = form(app, layer) {
                        f.error = None;
                        f.ask = Some(Ask::Geodatabase { connection: connection.to_string(), name: shown, count });
                    }
                }
                None => repaired(app, name),
            }
        }
        Err(e) => {
            let login = login_problem(&app.session, connection, &e);
            failed(app, layer, e);
            if let Some(f) = form(app, layer) {
                f.login = login.then(|| connection.to_string());
            }
        }
    }
}

/// The geodatabase's other broken layers repaired through `connection`; the notices list which
/// were and which weren't.
fn repair_geodatabase_rest(app: &mut RustGisApp, name: &str, connection: &str) {
    match app.run("layer.repairGeodatabase", json!({"connection": connection, "all": true})) {
        Ok(r) => notify_repaired(app, name, &r),
        Err(e) => {
            repaired(app, name);
            app.session.notify(redact_text(&e), true);
        }
    }
}

/// Back from adding or logging in to a connection (Add connection…, Log in…): the Repair Source…
/// dialog opens again on `layer` and repairs it through `connection`, behind the working window.
pub(crate) fn continue_repair(app: &mut RustGisApp, layer: u32, connection: String) {
    let name = app.session.map().layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let shown = app.session.connections.find(&connection).map(|c| c.name.clone()).unwrap_or_default();
    let message = crate::i18n::fmt("Repairing {name} through {connection}…", &[("name", &name), ("connection", &shown)]);
    crate::busy::queue_short(app, message, move |app| match app.run("ui.dialog", json!({"name": NAME, "layer": layer})) {
        Ok(_) => repair_geodatabase(app, layer, &name, &connection),
        Err(e) => app.session.notify(redact_text(&e), true),
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use rustgis_engine::map::source::{ServiceOrigin, SourceLayer};
    use serde_json::json;

    const URL: &str = "https://127.0.0.1:9/x/FeatureServer";

    /// A layer read from a service that didn't open, for `reason`.
    fn broken_service(app: &mut RustGisApp, reason: &str) -> u32 {
        let service = ServiceOrigin { url: URL.into(), kind: "arcgis".into(), layer: "0".into() };
        let src = SourceLayer::broken(format!("ArcGIS Pro read it from {URL}"), None, None, Some(service), reason.into());
        app.session.map_mut().add_streamed("Parcels", src)
    }

    /// A layer read from a shapefile that isn't at `path`.
    fn broken_file(app: &mut RustGisApp, name: &str, path: &str) -> u32 {
        let src = SourceLayer::broken(format!("a shapefile at {path}"), Some(path.into()), None, None, "the file isn't there".into());
        app.session.map_mut().add_streamed(name, src)
    }

    /// A layer Pro read from the enterprise geodatabase `utility` on `gis-db`.
    fn broken_geodatabase(app: &mut RustGisApp, name: &str, platform: &str) -> u32 {
        let mut src =
            SourceLayer::broken("ArcGIS Pro read it from an enterprise geodatabase".into(), None, None, None, "needs a connection".into());
        src.geodatabase = Some(rustgis_engine::map::source::GeodatabaseOrigin {
            platform: platform.into(),
            server: "gis-db".into(),
            database: "utility".into(),
            table: "utility.sde.Mains".into(),
            version: None,
        });
        app.session.map_mut().add_streamed(name, src)
    }

    fn connect(app: &mut RustGisApp, name: &str, address: &str) -> String {
        let r = app.run("connection.new", json!({"kind": "sqlserver", "name": name, "address": address})).unwrap();
        r["id"].as_str().unwrap().to_string()
    }

    #[test]
    fn a_geodatabase_layer_with_its_connection_offers_to_connect_and_repair() {
        let mut app = RustGisApp::with_sample();
        let id = broken_geodatabase(&mut app, "Mains", "sqlserver");
        connect(&mut app, "Utility GIS", "mssql://reader@gis-db/utility");
        let mut h = harness(app, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("You have a connection to it: Utility GIS.").is_some());
        assert!(h.query_by_label_contains("enterprise geodatabase on gis-db, database utility (table utility.sde.Mains)").is_some());
        h.get_by_label("Remove");
        assert!(h.query_by_label("Retry").is_none());
        // No password to sign in with: the connection's form opens to type it, then comes back
        // to repair.
        h.get_by_label("Connect and repair").click();
        h.run_steps(3);
        assert_eq!(h.state().dialogs.open, Some("editConnection"));
        assert!(h.state().ui.connection.login);
        assert_eq!(h.state().ui.connection.continue_repair, Some(id));
    }

    #[test]
    fn a_windows_sign_in_connection_repairs_without_asking_for_a_password() {
        let mut app = RustGisApp::with_sample();
        let c = connect(&mut app, "Utility (Windows)", "mssql://gis-db/utility");
        assert!(!needs_password(&app.session, &c));
        let user = connect(&mut app, "Utility GIS", "mssql://reader@gis-db/utility");
        assert!(needs_password(&app.session, &user));
        app.session.database_logins.set_session_password(&user, "made-up-pw");
        assert!(!needs_password(&app.session, &user));
    }

    #[test]
    fn a_near_connection_is_offered_by_name_and_host() {
        let mut app = RustGisApp::with_sample();
        let id = broken_geodatabase(&mut app, "Mains", "sqlserver");
        connect(&mut app, "Utility copy", "mssql://reader@gis-db2/utility");
        let mut h = harness(app, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("Utility copy is a connection to a database called utility on gis-db2.").is_some());
        h.get_by_label("Use Utility copy");
        h.get_by_label("Add connection…");
        h.get_by_label("Remove");
    }

    #[test]
    fn with_no_connection_the_helper_asks_for_one() {
        let mut app = RustGisApp::with_sample();
        let id = broken_geodatabase(&mut app, "Mains", "");
        let mut h = harness(app, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("RustGIS needs a connection to it.").is_some());
        h.get_by_label("Add connection…").click();
        h.run_steps(3);
        assert_eq!(h.state().dialogs.open, Some("newConnection"));
        assert_eq!(h.state().ui.connection.continue_repair, Some(id));
        assert_eq!(h.state().ui.connection.kind, "sqlserver");
        // The geodatabase form, with the layer's database filled in.
        assert!(h.state().ui.connection.form);
        assert_eq!(h.state().ui.connection.database, "utility");
    }

    #[test]
    fn another_platform_can_only_be_removed() {
        let mut app = RustGisApp::with_sample();
        let id = broken_geodatabase(&mut app, "Mains", "oracle");
        let mut h = harness(app, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("This layer is in a geodatabase on Oracle, which RustGIS can't connect to yet.").is_some());
        assert!(h.query_by_label("Add connection…").is_none());
        assert!(h.query_by_label("Connect and repair").is_none());
        h.get_by_label("Remove");
    }

    /// The app's per-frame work, the working window and the dialogs, on `app` with the dialog
    /// open on `layer`.
    fn harness(mut app: RustGisApp, layer: u32) -> egui_kittest::Harness<'static, RustGisApp> {
        app.run("ui.dialog", json!({"name": NAME, "layer": layer})).unwrap();
        let mut fonts = false;
        egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
            move |ui, app: &mut RustGisApp| {
                if !fonts {
                    crate::theme::install_fonts(ui.ctx(), &[], None);
                    fonts = true;
                    return;
                }
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                crate::busy::show(app, &ctx);
                crate::dialogs::show(app, &ctx);
            },
            app,
        )
    }

    #[test]
    fn the_helper_explains_a_sign_in_layer() {
        let mut app = RustGisApp::with_sample();
        let id = broken_service(&mut app, "Token Required (499)");
        let mut h = harness(app, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("needs you to sign in").is_some());
        assert!(h.query_all_by_label_contains("is on https://127.0.0.1:9").count() == 1, "the server");
        h.get_by_label("Sign in to ArcGIS Online");
        h.get_by_label("Retry");
        h.get_by_label("Remove");
    }

    #[test]
    fn the_helper_explains_a_certificate_failure() {
        let mut app = RustGisApp::with_sample();
        let id = broken_service(&mut app, "rustls: invalid peer certificate: UnknownIssuer");
        let mut h = harness(app, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("isn't trusted on this computer").is_some());
        assert!(h.query_by_label("Sign in to ArcGIS Online").is_none());
        assert!(h.query_by_label("Use another address…").is_none());
        h.get_by_label("Retry");
    }

    /// The address is shown without the token it was saved with.
    #[test]
    fn where_the_data_was_keeps_its_token_out() {
        let mut app = RustGisApp::with_sample();
        let service = ServiceOrigin { url: format!("{URL}?token=SECRET123"), kind: "arcgis".into(), layer: "0".into() };
        let src = SourceLayer::broken(format!("ArcGIS Pro read it from {URL}?token=SECRET123"), None, None, Some(service), "x".into());
        let id = app.session.map_mut().add_streamed("Parcels", src);
        let mut h = harness(app, id);
        h.run_steps(3);
        assert!(h.query_by_label_contains("127.0.0.1").is_some());
        assert!(h.query_by_label_contains("SECRET123").is_none());
    }

    /// Retry that fails keeps the dialog open and diagnoses the layer again: the new reason (no
    /// answer from the server) replaces the sign-in it said before.
    #[test]
    fn a_failed_retry_shows_the_new_reason() {
        let mut app = RustGisApp::with_sample();
        let id = broken_service(&mut app, "Token Required (499)");
        let mut h = harness(app, id);
        h.run_steps(3);
        h.get_by_label("Retry").click();
        h.run_steps(8);
        assert_eq!(h.state().dialogs.open, Some(NAME));
        assert!(h.query_by_label_contains("didn't answer").is_some());
        assert!(h.query_by_label_contains("needs you to sign in").is_none());
        h.get_by_label("Use another address…");
    }

    #[test]
    fn remove_takes_the_layer_out() {
        let mut app = RustGisApp::with_sample();
        let id = broken_service(&mut app, "Token Required (499)");
        let mut h = harness(app, id);
        h.run_steps(3);
        h.get_by_label("Remove").click();
        h.run_steps(3);
        assert!(h.state().session.map().layer(id).is_none());
        assert!(h.state().dialogs.open.is_none());
    }

    /// A file at a drive's or the file system's root: the folder keeps its separator.
    #[test]
    fn folder_of_keeps_a_roots_separator() {
        assert_eq!(folder_of("C:\\x.shp"), Some("C:\\"));
        assert_eq!(folder_of("C:/x.shp"), Some("C:/"));
        assert_eq!(folder_of("/x.shp"), Some("/"));
        assert_eq!(folder_of("C:\\data\\x.shp"), Some("C:\\data"));
        assert_eq!(folder_of("/data/x.shp"), Some("/data"));
        assert_eq!(folder_of("x.shp"), None);
    }

    /// A retry that only started reading (off the frame) doesn't claim the layer was repaired.
    #[test]
    fn a_retry_still_loading_says_so() {
        assert_eq!(retried(&json!({"retried": 1, "opened": 1, "failed": []}), "Roads"), "Repaired Roads");
        assert_eq!(retried(&json!({"retried": 1, "opened": 0, "failed": []}), "Roads"), "Roads is opening; it shows when it has loaded");
    }

    /// The notice names the layer even when the dialog closed before the queued call ran.
    #[test]
    fn a_queued_success_names_the_layer_after_the_dialog_closed() {
        let mut app = RustGisApp::with_sample();
        assert!(app.dialogs.open.is_none());
        repaired(&mut app, "Parcels");
        let said: Vec<&str> = app.session.notices.iter().map(|n| n.text.as_str()).collect();
        assert!(said.contains(&"Repaired Parcels"), "{said:?}");
    }

    /// A raster's own entries repair it: the helper says it's for feature layers.
    #[test]
    fn a_raster_is_for_its_own_repair() {
        use rustgis_engine::data::raster::{ColorHint, Crs, GeoTransform, Raster};
        let mut app = RustGisApp::with_sample();
        let gt = GeoTransform { origin_x: 0.0, origin_y: 1.0, pixel_width: 1.0, pixel_height: -1.0 };
        let r = Raster::new("dem", 1, 1, vec![vec![0.0]], gt, Crs::LonLat, None, ColorHint::Gray, "Float32").unwrap();
        let id = app.session.map_mut().add_raster("dem", rustgis_engine::map::RasterLayer::from_path(r, "dem.tif"));
        let e = app.run("ui.dialog", json!({"name": NAME, "layer": id})).unwrap_err();
        assert_eq!(e, "Repair Source… is for feature layers");
    }

    /// Only a broken layer has something to repair.
    #[test]
    fn an_open_layer_has_nothing_to_repair() {
        let mut app = RustGisApp::with_sample();
        let id = app.session.map().layer_by_name("Cities").unwrap().id;
        assert!(app.run("ui.dialog", json!({"name": NAME, "layer": id})).is_err());
        assert!(app.dialogs.open.is_none());
    }

    /// The file found in another folder: the layer reads it, and the other broken layer from its
    /// old folder is fixed too when asked.
    #[test]
    fn browsing_for_the_file_offers_to_fix_the_rest_of_its_folder() {
        let dir = std::env::temp_dir().join(format!("rustgis-repairbroken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = RustGisApp::with_sample();
        let new = dir.join("countries.shp").display().to_string();
        app.run("share.exportLayer", json!({"layer": "Countries", "path": new})).unwrap();
        let old = dir.join("gone").join("countries.shp").display().to_string();
        let roads = broken_file(&mut app, "Roads", &old);
        let other = broken_file(&mut app, "Other", &old);
        let mut h = harness(app, roads);
        h.run_steps(3);
        assert!(h.query_by_label_contains("The file isn't at").is_some());
        crate::dialogs::PICKED.with(|p| *p.borrow_mut() = Some(new.clone()));
        h.get_by_label("Browse for the file…").click();
        h.run_steps(6);
        assert!(h.query_by_label_contains("1 other broken layer had data in the same folder").is_some());
        h.get_by_label("Yes").click();
        h.run_steps(6);
        let s = &h.state().session;
        for id in [roads, other] {
            assert!(!s.map().layer(id).unwrap().source.as_ref().unwrap().is_broken(), "layer {id} still broken");
        }
        assert!(h.state().dialogs.open.is_none());
        let said: Vec<&str> = s.notices.iter().map(|n| n.text.as_str()).collect();
        assert!(said.iter().any(|t| t.contains("Other")), "{said:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
