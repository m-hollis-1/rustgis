//! A picture's Format rows: the file its image was read from, and Choose File… to read one.

use crate::RustGisApp;
use egui::Ui;
use rustgis_engine::map::layout::{Element, ElementKind, picture_file_name};
use serde_json::Value;

/// A picture's rows in the grid: File, the name of the file it remembers (its whole path on
/// hover) or "No file"; and, on the desktop, Choose File…, which reads the file picked into the
/// picture through `update`. Nothing is sent until a file is picked.
#[cfg_attr(target_arch = "wasm32", expect(unused_variables, reason = "the browser has no file paths to read a picture from"))]
pub(super) fn picture_rows(app: &mut RustGisApp, ui: &mut Ui, e: &Element, update: &dyn Fn(&mut RustGisApp, Value)) {
    if !matches!(e.kind, ElementKind::Picture { .. }) {
        return;
    }
    let source = e.kind.picture_source();
    ui.label(tl!("File"));
    match source.and_then(|s| Some((s, picture_file_name(s)?))) {
        Some((path, name)) => ui.add(egui::Label::new(name).truncate()).on_hover_text(path),
        None => ui.label(tl!("No file")),
    };
    ui.end_row();
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.label("");
        let hint = tl!("Read the picture from a PNG or JPEG file, and remember the file");
        if ui.button(tl!("Choose File…")).on_hover_text(hint).clicked()
            && let Some(path) = crate::dialogs::pick_open(&["png", "jpg", "jpeg"], tl!("Pictures"))
        {
            update(app, serde_json::json!({"path": path}));
        }
        ui.end_row();
    }
}
