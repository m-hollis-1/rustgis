//! A plugin's section of the Browser pane (`browser` in `plugin.json`): a tree whose levels the
//! plugin's `browse` command lists, one level at a time as it is opened.
//!
//! A level is read on a background thread (`plugin.browse`'s `BrowseJob` runs off the frame),
//! one at a time; the frame shows a spinner for it and [`poll`] lands the answer. In the
//! browser (wasm) there are no threads, so the job runs inline. The answers are kept until the
//! section is refreshed, its search text changes or the loaded plugins do.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::collapsing_header::CollapsingState;
use egui::{Label, RichText, Sense, Stroke, Ui, pos2, vec2};
use rustgis_engine::cmd::plugin_browse;
use rustgis_engine::plugins::{self, BrowserDecl, BrowserNode, Listing, Plugin};
use serde_json::json;

use crate::theme::{Tokens, semibold};
use crate::{RustGisApp, Slot, i18n, icons};

/// (plugin id, node id; empty for the roots).
type Key = (String, String);

/// How far each level of the tree is indented, in points.
const INDENT: f32 = 14.0;
/// The size of a row's icon.
const ICON: f32 = 16.0;

#[derive(Default)]
pub struct State {
    /// What each level lists, or is waiting to.
    entries: HashMap<Key, Entry>,
    /// The nodes opened.
    open: HashSet<Key>,
    selected: Option<Key>,
    /// What each section's search box holds.
    query: HashMap<String, String>,
    /// The fetch under way (one at a time).
    running: Option<Running>,
    /// The loaded plugins ([`plugins::revision`]) the entries were read from.
    revision: u64,
    /// Goes up when entries are dropped, so a fetch started before that is not applied after.
    epoch: u64,
}

pub enum Entry {
    Loading,
    Ready(Arc<Listing>),
    Failed(String),
}

/// A level being read on its thread.
struct Running {
    key: Key,
    epoch: u64,
    slot: Slot<Result<Listing, String>>,
}

/// The rows of `listing` as drawn: depth and node, in order. A node's children are inline only
/// when the node is not `expandable` (an expandable one asks the plugin for its own listing).
pub(crate) fn visible<'a>(listing: &'a Listing, open: &HashSet<String>) -> Vec<(usize, &'a BrowserNode)> {
    fn walk<'a>(nodes: &'a [BrowserNode], depth: usize, open: &HashSet<String>, out: &mut Vec<(usize, &'a BrowserNode)>) {
        for n in nodes {
            out.push((depth, n));
            if !n.expandable && open.contains(&n.id) {
                walk(&n.children, depth + 1, open, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&listing.nodes, 0, open, &mut out);
    out
}

/// The pure part of landing an answer for `key`: the entry stored under it is taken out and
/// handed back for the caller to replace, unless the plugin is gone (`still_there` false), which
/// leaves nothing behind.
pub(crate) fn poll_drops_result(state: &mut State, key: &Key, still_there: bool) -> Option<Entry> {
    let entry = state.entries.remove(key);
    if still_there { entry } else { None }
}

fn take<T>(slot: &Slot<T>) -> Option<T> {
    slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
}

/// Forgets what plugin `plugin`'s section has read, so it reads again.
pub fn refresh(app: &mut RustGisApp, plugin: &str) {
    let st = &mut app.ui.plugin_browser;
    st.entries.retain(|k, _| k.0 != plugin);
    st.open.retain(|k| k.0 != plugin);
    if st.selected.as_ref().is_some_and(|k| k.0 == plugin) {
        st.selected = None;
    }
    st.epoch += 1;
}

/// `name` when it is one of RustGIS's own icons.
fn known_icon(name: &str) -> Option<&str> {
    Some(name).filter(|n| icons::ALL.contains(n))
}

/// Whether plugin `id` is loaded, enabled and has a browser.
fn listed(id: &str) -> bool {
    plugins::get(id).is_some_and(|p| p.enabled && p.manifest.browser.is_some())
}

/// The sections of every enabled plugin with a browser, as part of the Browser pane.
pub fn show(app: &mut RustGisApp, ui: &mut Ui, t: &Tokens) {
    let lang = app.lang().code();
    let mut sections: Vec<(Arc<Plugin>, BrowserDecl)> =
        plugins::list().into_iter().filter(|p| p.enabled).filter_map(|p| p.manifest.browser.clone().map(|d| (p, d))).collect();
    sections.sort_by(|a, b| a.0.manifest.id.cmp(&b.0.manifest.id));
    for (p, decl) in sections {
        let id = p.manifest.id.as_str();
        let title = p.manifest.translate(lang, &decl.label);
        // The section's icon, then its title: a header of our own, as the connection tree's folders have.
        let cid = ui.make_persistent_id(("pluginbrowser", id));
        let state = CollapsingState::load_with_default_open(ui.ctx(), cid, false);
        let was_open = state.is_open();
        let mut clicked = false;
        let mut header = state.show_header(ui, |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
            icons::paint(ui.painter(), r, known_icon(&decl.icon).unwrap_or("open"), t.icon, t.accent);
            let title = Label::new(RichText::new(title).font(semibold(12.5))).sense(Sense::click());
            let label = ui.add(title);
            clicked = label.clicked();
            label
        });
        if clicked {
            header.set_open(!was_open);
        }
        let (_, label, _) = header.body(|ui| {
            if decl.search {
                let query = app.ui.plugin_browser.query.entry(id.to_string()).or_default();
                if crate::widgets::search_field(ui, query, tl!("Search")).changed() {
                    refresh(app, id);
                }
            }
            level(app, ui, t, &p, &decl, lang, "", 0);
        });
        let (mut reload, mut collapse) = (false, false);
        label.inner.context_menu(|ui| {
            if ui.button(tl!("Refresh")).clicked() {
                reload = true;
                ui.close();
            }
            if ui.button(tl!("Collapse All")).clicked() {
                collapse = true;
                ui.close();
            }
        });
        if reload {
            refresh(app, id);
        }
        if collapse {
            app.ui.plugin_browser.open.retain(|k| k.0 != id);
        }
    }
}

/// One level of a section: waiting, its error with a way to try again, or its rows. `base` is
/// how deep the level sits, for the indent.
#[allow(clippy::too_many_arguments)]
fn level(app: &mut RustGisApp, ui: &mut Ui, t: &Tokens, p: &Plugin, decl: &BrowserDecl, lang: &str, node: &str, base: usize) {
    let key = (p.manifest.id.clone(), node.to_string());
    let shown = match app.ui.plugin_browser.entries.get(&key) {
        None => None,
        Some(Entry::Loading) => Some(Err(None)),
        Some(Entry::Failed(e)) => Some(Err(Some(e.clone()))),
        Some(Entry::Ready(l)) => Some(Ok(l.clone())),
    };
    let indent = base as f32 * INDENT;
    match shown {
        None => {
            app.ui.plugin_browser.entries.insert(key.clone(), Entry::Loading);
            launch(app, ui.ctx(), &key);
            ui.ctx().request_repaint();
        }
        Some(Err(None)) => {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.spinner();
                ui.label(tl!("Loading…"));
            });
        }
        Some(Err(Some(e))) => {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                ui.vertical(|ui| {
                    ui.label(RichText::new(e).small().color(t.red));
                    if ui.small_button(tl!("Retry")).clicked() {
                        app.ui.plugin_browser.entries.remove(&key);
                    }
                });
            });
        }
        Some(Ok(l)) => {
            let open: HashSet<String> = app.ui.plugin_browser.open.iter().filter(|k| k.0 == key.0).map(|k| k.1.clone()).collect();
            for (depth, n) in visible(&l, &open) {
                row(app, ui, t, p, decl, lang, n, base + depth);
                if n.expandable && open.contains(&n.id) {
                    level(app, ui, t, p, decl, lang, &n.id, base + depth + 1);
                }
            }
            let weak = |ui: &mut Ui, text: String| {
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    ui.label(RichText::new(text).small().weak());
                });
            };
            if l.nodes.is_empty() {
                weak(ui, tl!("Nothing here").to_string());
            }
            if l.truncated {
                weak(ui, i18n::fmt("The first {n} are listed.", &[("n", &l.nodes.len().to_string())]));
            }
            for note in &l.notes {
                weak(ui, note.clone());
            }
        }
    }
}

/// One node: an arrow when it opens, its icon, label, detail and count. A click selects, a
/// double click opens a branch or runs the section's `open` command on a leaf, the menu runs
/// the section's commands.
#[allow(clippy::too_many_arguments)]
fn row(app: &mut RustGisApp, ui: &mut Ui, t: &Tokens, p: &Plugin, decl: &BrowserDecl, lang: &str, n: &BrowserNode, depth: usize) {
    let key = (p.manifest.id.clone(), n.id.clone());
    let branch = n.expandable || !n.children.is_empty();
    let is_open = app.ui.plugin_browser.open.contains(&key);
    let selected = app.ui.plugin_browser.selected.as_ref() == Some(&key);
    let (label, arrow) = ui
        .horizontal(|ui| {
            ui.add_space(depth as f32 * INDENT);
            let (r, arrow) = ui.allocate_exact_size(vec2(12.0, ICON), if branch { Sense::click() } else { Sense::hover() });
            if branch {
                let c = r.center();
                let pts = if is_open {
                    vec![pos2(c.x - 4.0, c.y - 2.0), pos2(c.x + 4.0, c.y - 2.0), pos2(c.x, c.y + 3.0)]
                } else {
                    vec![pos2(c.x - 2.0, c.y - 4.0), pos2(c.x + 3.0, c.y), pos2(c.x - 2.0, c.y + 4.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(pts, t.text_dim, Stroke::NONE));
            }
            let (r, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
            let named = n.icon.as_deref().and_then(known_icon);
            icons::paint(ui.painter(), r, named.unwrap_or(if branch { "open" } else { "table" }), t.icon, t.accent);
            let label = ui.selectable_label(selected, n.label.as_str());
            if let Some(d) = &n.detail {
                ui.add(Label::new(RichText::new(d).small().weak()).truncate());
            }
            if let Some(c) = n.count {
                ui.label(RichText::new(format!("[{c}]")).small().weak());
            }
            (label, arrow)
        })
        .inner;
    if label.clicked() || label.secondary_clicked() {
        app.ui.plugin_browser.selected = Some(key.clone());
    }
    let toggled = branch && (arrow.clicked() || label.double_clicked());
    if toggled && !app.ui.plugin_browser.open.remove(&key) {
        app.ui.plugin_browser.open.insert(key.clone());
    }
    if label.double_clicked() && !branch && !decl.open.is_empty() {
        run_command(app, &key.0, &decl.open, &n.id);
    }
    let mut chosen = None;
    label.context_menu(|ui| {
        for cmd in decl.menu_commands() {
            let name = p.manifest.commands.iter().find(|c| c.id == cmd).map_or(cmd.as_str(), |c| p.manifest.translate(lang, &c.label));
            if ui.button(name).clicked() {
                chosen = Some(cmd.clone());
                ui.close();
            }
        }
    });
    if let Some(cmd) = chosen {
        run_command(app, &key.0, &cmd, &n.id);
    }
}

/// Runs one of the plugin's commands on a node; a failure is a notice.
fn run_command(app: &mut RustGisApp, plugin: &str, command: &str, node: &str) {
    if let Err(e) = app.run(&plugins::command_id(plugin, command), json!({"browser": {"node": node}})) {
        app.session.notify(e, true);
    }
}

/// Starts reading the level `key` (whose entry is `Loading`) unless a read is under way: then
/// it waits, and [`poll`] starts it when the other lands. Where there are no threads (the
/// browser) the plugin runs here and then.
fn launch(app: &mut RustGisApp, ctx: &egui::Context, key: &Key) {
    let st = &mut app.ui.plugin_browser;
    if st.running.is_some() {
        return;
    }
    let query = st.query.get(&key.0).map(String::as_str).unwrap_or_default();
    let node = (!key.1.is_empty()).then_some(key.1.as_str());
    let job = match plugin_browse::job(&key.0, node, query) {
        Ok(job) => job,
        Err(e) => {
            st.entries.insert(key.clone(), Entry::Failed(e));
            return;
        }
    };
    let slot = Slot::default();
    let (out, ctx) = (slot.clone(), ctx.clone());
    let work = move || {
        let r = job.run();
        // Filled even when poisoned, or the section would wait for it forever.
        *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(r);
        ctx.request_repaint();
    };
    if cfg!(target_arch = "wasm32") {
        work();
    } else if let Err(e) = std::thread::Builder::new().name("plugin-browse".into()).spawn(work) {
        st.entries.insert(key.clone(), Entry::Failed(e.to_string()));
        return;
    }
    st.running = Some(Running { key: key.clone(), epoch: st.epoch, slot });
}

/// Lands the answer of the read of level `key`, started in `epoch`. The plugin gone
/// (`still_there` false) leaves no entry; an answer to a read begun before the entries were last
/// dropped leaves the level `Loading`, to be asked again; otherwise the entry takes the listing
/// or the error.
pub(crate) fn land(state: &mut State, key: &Key, epoch: u64, still_there: bool, result: Result<Listing, String>) {
    let stale = epoch != state.epoch;
    match poll_drops_result(state, key, still_there) {
        Some(Entry::Loading) if stale => {
            state.entries.insert(key.clone(), Entry::Loading);
        }
        Some(Entry::Loading) => {
            let entry = match result {
                Ok(l) => Entry::Ready(Arc::new(l)),
                Err(e) => Entry::Failed(e),
            };
            state.entries.insert(key.clone(), entry);
        }
        // Not waiting for it any more (dropped, or replaced since): the entry stays as it is.
        Some(other) => {
            state.entries.insert(key.clone(), other);
        }
        None => {}
    }
}

/// Lands the answer of the read under way, and starts the next level waiting.
pub fn poll(app: &mut RustGisApp, ctx: &egui::Context) {
    let st = &mut app.ui.plugin_browser;
    let now = plugins::revision();
    if st.revision != now {
        // Plugins loaded, removed or switched: what was read may be gone or different.
        st.entries.clear();
        st.open.clear();
        st.selected = None;
        st.running = None;
        st.epoch += 1;
        st.revision = now;
        st.query.retain(|id, _| listed(id));
    }
    if let Some(result) = st.running.as_ref().and_then(|r| take(&r.slot))
        && let Some(run) = st.running.take()
    {
        land(st, &run.key, run.epoch, listed(&run.key.0), result);
    }
    if st.running.is_none() {
        let next = st.entries.iter().filter(|(_, e)| matches!(e, Entry::Loading)).map(|(k, _)| k).min().cloned();
        if let Some(key) = next {
            launch(app, ctx, &key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, exp: bool, children: Vec<BrowserNode>) -> BrowserNode {
        BrowserNode { id: id.into(), label: id.into(), detail: None, icon: None, count: None, expandable: exp, children }
    }

    #[test]
    fn visible_follows_open_branches() {
        let l = Listing { nodes: vec![node("a", false, vec![node("a1", false, vec![])]), node("b", true, vec![])], ..Default::default() };
        let ids = |open: &[&str]| {
            visible(&l, &open.iter().map(|s| s.to_string()).collect()).into_iter().map(|(d, n)| format!("{d}:{}", n.id)).collect::<Vec<_>>()
        };
        assert_eq!(ids(&[]), ["0:a", "0:b"]);
        assert_eq!(ids(&["a"]), ["0:a", "1:a1", "0:b"]);
        assert_eq!(ids(&["b"]), ["0:a", "0:b"], "an expandable node's children come from its own listing, not inline");
    }

    #[test]
    fn poll_drops_results_of_gone_plugins() {
        let mut st = State::default();
        let key = ("p".to_string(), String::new());
        st.entries.insert(key.clone(), Entry::Loading);
        assert!(poll_drops_result(&mut st, &key, false).is_none());
        assert!(!st.entries.contains_key(&key), "a plugin that went away leaves no entry");
        st.entries.insert(key.clone(), Entry::Loading);
        assert!(matches!(poll_drops_result(&mut st, &key, true), Some(Entry::Loading)));
    }

    fn key() -> Key {
        ("p".to_string(), String::new())
    }

    #[test]
    fn land_fills_the_entry_with_the_listing_or_the_error() {
        let mut st = State::default();
        st.entries.insert(key(), Entry::Loading);
        let l = Listing { nodes: vec![node("a", false, vec![])], ..Default::default() };
        land(&mut st, &key(), 0, true, Ok(l));
        assert!(matches!(st.entries.get(&key()), Some(Entry::Ready(l)) if l.nodes.len() == 1));
        st.entries.insert(key(), Entry::Loading);
        land(&mut st, &key(), 0, true, Err("boom".into()));
        assert!(matches!(st.entries.get(&key()), Some(Entry::Failed(e)) if e == "boom"));
    }

    #[test]
    fn land_asks_again_when_the_answer_is_from_before_a_refresh() {
        let mut st = State::default();
        let started = st.epoch;
        st.epoch += 1;
        st.entries.insert(key(), Entry::Loading);
        land(&mut st, &key(), started, true, Ok(Listing::default()));
        assert!(matches!(st.entries.get(&key()), Some(Entry::Loading)), "the old answer is not applied");
    }

    #[test]
    fn land_leaves_nothing_for_a_gone_plugin() {
        let mut st = State::default();
        st.entries.insert(key(), Entry::Loading);
        land(&mut st, &key(), 0, false, Ok(Listing::default()));
        assert!(st.entries.is_empty());
    }
}
