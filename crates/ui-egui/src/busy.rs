//! The working window: what the interface runs that can take minutes (importing an ArcGIS Pro
//! project, saving, opening a project file) blocks the UI thread, so the window can't draw while
//! it runs. Such a call is queued with what it does; the window saying so is painted first, and
//! the call runs on a later frame, while that window is on screen.
//!
//! Only the interface queues: `RustGisApp::run`, and the commands as scripts, plugins, agents
//! and tests run them, stay synchronous.

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::RustGisApp;
use crate::theme::semibold;

/// A queued call.
type Action = Box<dyn FnOnce(&mut RustGisApp)>;

/// What waits behind the working window: calls in the order queued, each with what the window
/// says while it runs.
pub(crate) struct Pending {
    steps: VecDeque<(String, Action)>,
    /// The first step's window was painted, visible: it runs on the next frame.
    painted: bool,
}

/// What the window says under what is happening.
const LINE: &str = "This can take several minutes for a large project. Windows may say RustGIS is not responding — don't close it.";

/// Queues `action` behind the working window saying `message`. It runs on a frame after the one
/// that painted the window (behind any queued before it).
pub(crate) fn queue(app: &mut RustGisApp, message: String, action: impl FnOnce(&mut RustGisApp) + 'static) {
    let step: (String, Action) = (message, Box::new(action));
    match &mut app.ui.busy {
        Some(pending) => pending.steps.push_back(step),
        None => app.ui.busy = Some(Pending { steps: VecDeque::from([step]), painted: false }),
    }
}

/// Each frame, before anything else: the first queued call runs once its window was painted.
/// On the frame after one ran, the pointer presses and keys that arrived while it blocked are
/// dropped, so nothing done meanwhile lands on what is under the window.
pub(crate) fn run_due(app: &mut RustGisApp, ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    // Not on a pass egui re-runs within the frame the call ran in: that frame's input was
    // pressed before it blocked.
    if app.ui.busy_ran.is_some_and(|ran| frame > ran) {
        app.ui.busy_ran = None;
        drop_input(ctx);
    }
    let Some(pending) = app.ui.busy.as_mut().filter(|p| p.painted) else { return };
    let Some((_, action)) = pending.steps.pop_front() else {
        app.ui.busy = None;
        return;
    };
    // The next step, if any, is painted before it runs.
    pending.painted = false;
    if pending.steps.is_empty() {
        app.ui.busy = None;
    }
    action(app);
    app.ui.busy_ran = Some(frame);
    ctx.request_repaint();
}

/// Forgets the pointer presses, keys and scrolling of this frame's input.
fn drop_input(ctx: &egui::Context) {
    ctx.input_mut(|i| {
        i.events.clear();
        i.raw.events.clear();
        i.keys_down.clear();
        i.pointer = Default::default();
        i.smooth_scroll_delta = egui::Vec2::ZERO;
    });
    ctx.stop_dragging();
}

/// The working window, while a call is queued. It has no close button: it closes when the call
/// has run.
pub(crate) fn show(app: &mut RustGisApp, ctx: &egui::Context) {
    let Some(pending) = app.ui.busy.as_mut() else { return };
    let Some((message, _)) = pending.steps.front() else { return };
    let id = egui::Id::unique("busy_window");
    let mut painted = false;
    // No fade: the frame painted is the one on screen while the call blocks.
    egui::Modal::new(id).area(egui::Modal::default_area(id).fade_in(false)).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new(message.as_str()).font(semibold(15.0)));
        });
        ui.add_space(6.0);
        ui.label(tl!(LINE));
        // A new window's first frame only measures it, unseen.
        painted = !ui.is_sizing_pass() && ui.is_visible();
    });
    pending.painted |= painted;
    ctx.request_repaint();
}

/// The file name at the end of `path` (either separator: a Windows path read anywhere).
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// "1 map", "3 maps": `one` for one, else `many` with `{n}` filled.
fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { one.to_string() } else { many.replace("{n}", &n.to_string()) }
}

/// What the window says while the maps and layouts picked import from `file`.
pub(crate) fn importing(maps: usize, layouts: usize, file: &str) -> String {
    let m = count(maps, tl_raw!("1 map"), tl_raw!("{n} maps"));
    let l = count(layouts, tl_raw!("1 layout"), tl_raw!("{n} layouts"));
    let what = match (maps, layouts) {
        (_, 0) => m,
        (0, _) => l,
        _ => tl_raw!("{maps} and {layouts}").replacen("{maps}", &m, 1).replacen("{layouts}", &l, 1),
    };
    crate::i18n::fmt("Importing {what} from {file}…", &[("what", &what), ("file", file_name(file))])
}

/// Runs command `id` the user asked for: opening a project file is queued behind the working
/// window (the map redraws once it has run); anything else runs now.
pub(crate) fn run(app: &mut RustGisApp, id: &str, params: Value) -> Result<Value, String> {
    let path = params.get("path").and_then(Value::as_str).filter(|_| id == "project.open").map(str::to_string);
    let Some(path) = path else { return app.run(id, params) };
    let message = crate::i18n::fmt("Opening {file}…", &[("file", file_name(&path))]);
    let id = id.to_string();
    queue(app, message, move |app| {
        let _ = app.run(&id, params);
        app.map.invalidate();
    });
    Ok(Value::Null)
}

/// Save (`as_new`: Save As) from the interface. A project never saved, or Save As, asks where
/// first; cancelling that saves nothing and runs nothing. The save runs behind the working
/// window, and `then` after it, only when it succeeded and left nothing unsaved. A failure is
/// said by the command, as ever.
pub(crate) fn save(app: &mut RustGisApp, as_new: bool, then: impl FnOnce(&mut RustGisApp) + 'static) {
    let (id, params, path) = match app.session.path.clone().filter(|_| !as_new) {
        Some(path) => ("project.save", Value::Null, path.display().to_string()),
        None => {
            let name = format!("{}.rgis", app.session.project.name);
            let Some(path) = crate::dialogs::pick_save(&name, &["rgis"], tl!("RustGIS project")) else { return };
            ("project.saveAs", json!({"path": path}), path)
        }
    };
    let message = crate::i18n::fmt("Saving {project}…", &[("project", file_name(&path))]);
    queue(app, message, move |app| {
        if app.run(id, params).is_ok() && !app.session.dirty() {
            then(app);
        }
    });
}

/// In a unit test without frames: runs every queued call now.
#[cfg(test)]
pub(crate) fn finish(app: &mut RustGisApp) {
    while let Some((_, action)) = app.ui.busy.as_mut().and_then(|p| p.steps.pop_front()) {
        action(app);
    }
    app.ui.busy = None;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use egui::vec2;
    use egui_kittest::kittest::Queryable;
    use std::cell::Cell;
    use std::rc::Rc;

    /// One frame of `ctx` running `f`.
    fn pass(ctx: &egui::Context, app: &mut RustGisApp, f: impl Fn(&mut RustGisApp, &egui::Context)) {
        ctx.run_pass(Default::default(), |ctx| f(app, ctx)).drop_without_applying_deltas();
    }

    #[test]
    fn an_action_runs_only_after_its_window_was_painted() {
        let mut app = RustGisApp::with_sample();
        let ctx = egui::Context::default();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        queue(&mut app, "Saving test.rgis…".into(), move |_| flag.set(true));
        pass(&ctx, &mut app, run_due);
        assert!(!ran.get(), "not painted yet");
        // The first frame that shows a new window only measures it (egui's sizing pass): it
        // isn't on screen yet.
        pass(&ctx, &mut app, show);
        pass(&ctx, &mut app, run_due);
        assert!(!ran.get(), "measured, not painted");
        pass(&ctx, &mut app, show);
        pass(&ctx, &mut app, run_due);
        assert!(ran.get() && app.ui.busy.is_none());
    }

    #[test]
    fn a_click_during_the_block_is_dropped() {
        let clicks = Rc::new(Cell::new(0));
        let counted = clicks.clone();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(600.0, 400.0)).build_ui_state(
            move |ui, app: &mut RustGisApp| {
                let ctx = ui.ctx().clone();
                if !fonts {
                    crate::theme::install_fonts(&ctx, &[], None);
                    fonts = true;
                    return;
                }
                run_due(app, &ctx);
                if ui.button("Probe").clicked() {
                    counted.set(counted.get() + 1);
                }
                show(app, &ctx);
            },
            RustGisApp::with_sample(),
        );
        h.run_steps(2);
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        queue(h.state_mut(), "Opening test.rgis…".into(), move |_| flag.set(true));
        // (A new window's first frame only measures it.)
        h.run_steps(2);
        assert!(!ran.get() && h.query_by_label_contains("Opening test.rgis").is_some(), "the window shows first");
        // It runs on a frame after the one that painted it.
        for _ in 0..3 {
            if !ran.get() {
                h.step();
            }
        }
        assert!(ran.get(), "the action ran");
        // Pressed while it blocked: the desktop shell hands what piled up meanwhile to the next
        // frame, all at once.
        let pos = h.get_by_label("Probe").rect().center();
        let modifiers = egui::Modifiers::default();
        let button = egui::PointerButton::Primary;
        let pressed = [
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button, pressed: true, modifiers },
            egui::Event::PointerButton { pos, button, pressed: false, modifiers },
            egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers },
        ];
        h.input_mut().events.extend(pressed.clone());
        h.run_steps(3);
        assert_eq!(clicks.get(), 0, "the click during the block didn't land");
        // Afterwards the same click lands.
        h.input_mut().events.extend(pressed);
        h.run_steps(2);
        assert_eq!(clicks.get(), 1);
    }
}
