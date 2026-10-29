//! The site file's install prompt: the packages `site.json` lists that aren't installed (or are
//! older), offered once at start-up. See `docs/site-settings.md`.

use egui::vec2;
use serde_json::json;

use crate::RustGisApp;
use crate::pluginsui::{chip, installed};
use crate::theme::{Tokens, semibold};

/// Offers the packages the site file lists that aren't installed (or are older) as one prompt on
/// the first frame: Install runs them all, Not now asks again next start. The site file exists
/// on the desktop only.
pub fn site_prompt(app: &mut RustGisApp, ctx: &egui::Context, t: &Tokens) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let list = app.plugins.site_installs.get_or_insert_with(rustgis_engine::site::site_installs);
    if list.is_empty() {
        return;
    }
    let mut install = false;
    let mut dismiss = false;
    let screen = ctx.content_rect();
    let width = (screen.width() - 48.0).clamp(220.0, 460.0);
    let modal = egui::Modal::new(egui::Id::unique("site_install_prompt")).show(ctx, |ui| {
        ui.set_width(width);
        ui.label(egui::RichText::new(tl!("Plugins for this site")).font(semibold(15.0)));
        ui.add_space(6.0);
        for entry in list.iter() {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(format!("{} {}", entry.name, entry.version)).font(semibold(12.5)));
                if entry.process {
                    chip(ui, t, tl!("Not sandboxed"), t.red);
                }
            });
            if entry.process {
                ui.label(
                    egui::RichText::new(tl!(
                        "This plugin runs as a program on your computer, outside RustGIS's sandbox. It can do anything you can: read and change your files, use the network and run other programs. Install it only if you trust its author."
                    ))
                    .size(11.5)
                    .color(t.text_dim),
                );
            }
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if crate::widgets::primary_button(ui, tl!("Install"), true, vec2(80.0, 28.0)).clicked() {
                install = true;
            }
            if ui.button(tl!("Not now")).clicked() {
                dismiss = true;
            }
        });
    });
    if modal.should_close() {
        dismiss = true;
    }
    if install {
        let entries = std::mem::take(list);
        for entry in entries {
            let path = entry.path.display().to_string();
            match app.run("plugin.install", json!({"path": path})) {
                Ok(_) => match rustgis_engine::plugins::get(&entry.id) {
                    Some(plugin) => {
                        let text = installed(app, &plugin.manifest);
                        app.session.notify(text, false);
                    }
                    None => log::warn!("site package {} installed but not loaded", entry.id),
                },
                Err(error) => {
                    let text = crate::i18n::fmt("Could not install {name}: {error}", &[("name", &entry.name), ("error", &error)]);
                    app.session.notify(text, true);
                }
            }
        }
    } else if dismiss {
        list.clear();
    }
}
