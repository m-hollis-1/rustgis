//! Repair Source…: a broken feature layer explained (where its data was, why it doesn't open)
//! with the fix that fits what went wrong (`layer.diagnose`): signing in to ArcGIS Online,
//! another address for a service, the file found elsewhere (and the other broken layers from its
//! folder), trying again, or removing the layer. A fix that works closes the dialog with a notice;
//! one that fails keeps it open, diagnosed again.

use crate::RustGisApp;
use crate::theme::Tokens;
use rustgis_engine::Session;
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
    folder: Option<String>,
    /// How many other broken layers read their data from the same folder.
    siblings: usize,
}

/// The file found in another folder, when its old folder held other broken layers' data.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Siblings {
    from: String,
    to: String,
    count: usize,
}

/// What the dialog was asked to do this frame.
enum Action {
    Retry,
    SignIn,
    Relink(String),
    Browse,
    FixSiblings(bool),
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
    ask: Option<Siblings>,
    /// The last action's error, when the reason doesn't already say it.
    error: Option<String>,
    /// A sign-in started here is under way.
    signing_in: bool,
}

impl Form {
    /// Opens on broken feature layer `layer`; an error for one that isn't broken.
    pub fn new(app: &mut RustGisApp, layer: u32) -> Result<Self, String> {
        let s = &mut app.session;
        let l = s.map().layer(layer).ok_or_else(|| tl!("Layer not found").to_string())?;
        if l.is_raster() {
            return Err(crate::i18n::fmt("{name} isn't broken", &[("name", &l.name)]));
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
    })
}

/// The folder a file is in (either separator).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn folder_of(path: &str) -> Option<&str> {
    let i = path.rfind(['/', '\\'])?;
    path.get(..i).filter(|f| !f.is_empty())
}

/// What the diagnosis means, in words.
fn explain(d: &Diagnosis) -> String {
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
                ui.add(egui::Label::new(ask_text(ask.count)).wrap());
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
            ui.add(egui::Label::new(egui::RichText::new(explain(&f.diagnosis)).strong()).wrap());
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
                if f.diagnosis.path.is_some() {
                    let fill = (kind == Kind::FileMissing).then_some(t.accent);
                    if crate::widgets::dialog_button(ui, tl!("Browse for the file…"), fill, size).clicked() {
                        action = Some(Action::Browse);
                    }
                }
                if kind != Kind::FileMissing && crate::widgets::dialog_button(ui, tl!("Retry"), None, size).clicked() {
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
    app.dialogs.broken = Some(f);
    match action {
        None => {}
        // A service can take a while to answer, or not to: the working window says so meanwhile.
        Some(Action::Retry) => {
            let message = crate::i18n::fmt("Trying {name} again…", &[("name", &name)]);
            crate::busy::queue(app, message, move |app| retry(app, layer));
        }
        Some(Action::Relink(url)) => {
            let message = crate::i18n::fmt("Opening {name} from its new address…", &[("name", &name)]);
            crate::busy::queue(app, message, move |app| relink(app, layer, &url));
        }
        Some(Action::SignIn) => {
            let started = crate::arcgisui::sign_in_online(app);
            if let Some(f) = app.dialogs.broken.as_mut() {
                f.signing_in = started.is_ok();
                f.error = started.err();
            }
        }
        Some(Action::Browse) => browse(app, layer),
        Some(Action::FixSiblings(yes)) => fix_siblings(app, yes),
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

/// The layer opens again: the dialog closes with a notice.
fn repaired(app: &mut RustGisApp, layer: u32) {
    let name = form(app, layer).map(|f| f.name.clone()).unwrap_or_default();
    done(app, crate::i18n::fmt("Repaired {name}", &[("name", &name)]));
}

fn retry(app: &mut RustGisApp, layer: u32) {
    match app.run("layer.retry", json!({"layer": layer})) {
        Ok(r) => {
            let error = r.get("failed").and_then(|f| f.get(0)).and_then(|f| f.get("error")).and_then(Value::as_str);
            match error {
                Some(e) => failed(app, layer, e.to_string()),
                None => repaired(app, layer),
            }
        }
        Err(e) => failed(app, layer, e),
    }
}

fn relink(app: &mut RustGisApp, layer: u32, url: &str) {
    match app.run("layer.relinkService", json!({"layer": layer, "url": url})) {
        Ok(_) => repaired(app, layer),
        Err(e) => failed(app, layer, e),
    }
}

/// Browse for the file…: the layer reads the file picked; when its old folder held other broken
/// layers' data, the dialog asks whether to fix those too.
#[cfg(not(target_arch = "wasm32"))]
fn browse(app: &mut RustGisApp, layer: u32) {
    let Some(path) = crate::dialogs::pick_open(&[], tl!("Data")) else { return };
    match app.run("layer.repairSource", json!({"layer": layer, "path": path})) {
        Ok(_) => {
            let Some(f) = form(app, layer) else { return };
            match (f.diagnosis.folder.clone(), folder_of(&path)) {
                (Some(from), Some(to)) if f.diagnosis.siblings > 0 => {
                    f.error = None;
                    f.ask = Some(Siblings { from, to: to.to_string(), count: f.diagnosis.siblings });
                }
                _ => repaired(app, layer),
            }
        }
        Err(e) => failed(app, layer, e),
    }
}

/// The web app can't read files on this computer by their path.
#[cfg(target_arch = "wasm32")]
fn browse(app: &mut RustGisApp, layer: u32) {
    if let Some(f) = form(app, layer) {
        f.error = Some(tl!("The web app can't link to files on this computer: add the file again with Add Data.").to_string());
    }
}

/// The answer to "Fix them too?": Yes repairs the old folder's other broken layers from the new
/// one, and the notices list which were and which weren't.
fn fix_siblings(app: &mut RustGisApp, yes: bool) {
    let Some(f) = app.dialogs.broken.as_ref() else { return };
    let (name, layer) = (f.name.clone(), f.layer);
    let folders = f.ask.as_ref().map(|a| (a.from.clone(), a.to.clone()));
    let Some((from, to)) = folders.filter(|_| yes) else {
        repaired(app, layer);
        return;
    };
    match app.run("layer.repairFolder", json!({"from": from, "to": to})) {
        Ok(r) => {
            let mut all = vec![name];
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
        Err(e) => {
            done(app, crate::i18n::fmt("Repaired {name}", &[("name", &name)]));
            app.session.notify(redact_text(&e), true);
        }
    }
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
        h.run_steps(3);
        assert!(h.query_by_label_contains("1 other broken layer had data in the same folder").is_some());
        h.get_by_label("Yes").click();
        h.run_steps(3);
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
