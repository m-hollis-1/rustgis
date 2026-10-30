//! A plugin's section of the Browser pane: a lazily loaded tree, declared in `plugin.json`
//! (`browser`).
//!
//! The host asks the plugin's `browse` command for the children of one node at a time (none
//! for the root), with the node's id and the search text in the request (see [`request`]); the
//! plugin answers with the nodes (`{"nodes": […], "truncated"?, "notes"?}`). The answer is
//! hostile input like any other, so [`listing`] checks it: ids made unique, text kept to one
//! line, depth and count bounded. Nothing here knows about the user interface.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::panel::{MAX_STATE_LABEL, MAX_TREE_DEPTH, MAX_TREE_ID, MAX_TREE_NODES};

/// The longest section label, in characters.
pub const MAX_LABEL: usize = 32;
/// The most commands a node's menu may name.
pub const MAX_MENU: usize = 16;
/// The longest node detail, in characters.
pub const MAX_DETAIL: usize = 80;
/// The most notes a listing may carry.
pub const MAX_NOTES: usize = 16;
/// The longest node icon name, in characters.
const MAX_ICON: usize = 40;

/// The Browser section as `plugin.json` declares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDecl {
    /// The section's title.
    pub label: String,
    /// One of RustGIS's own icons for the section (a folder when unset or unknown).
    #[serde(default)]
    pub icon: String,
    /// The command that lists the children of a node (the root's when it gets no node).
    pub browse: String,
    /// The command run when a node is double-clicked (empty for none).
    #[serde(default)]
    pub open: String,
    /// Commands offered in a node's context menu.
    #[serde(default)]
    pub menu: Vec<String>,
    /// Whether the section has a search box.
    #[serde(default)]
    pub search: bool,
}

impl BrowserDecl {
    /// Checks the declaration; `command_exists` says whether the plugin has a command by that id.
    pub fn validate(&self, command_exists: &dyn Fn(&str) -> bool) -> Result<(), String> {
        let label = self.label.trim();
        if label.is_empty() || label.chars().count() > MAX_LABEL || label.contains('\n') {
            return Err(format!("plugin.json: browser.label must be 1 to {MAX_LABEL} characters"));
        }
        if !command_exists(&self.browse) {
            return Err(format!("plugin.json: browser.browse '{}' isn't one of the plugin's commands", clip(&self.browse, 80)));
        }
        if !self.open.is_empty() && !command_exists(&self.open) {
            return Err(format!("plugin.json: browser.open '{}' isn't one of the plugin's commands", clip(&self.open, 80)));
        }
        if self.menu.len() > MAX_MENU {
            return Err(format!("plugin.json: browser.menu has at most {MAX_MENU} entries"));
        }
        if let Some(c) = self.menu.iter().find(|c| !command_exists(c)) {
            return Err(format!("plugin.json: browser.menu '{}' isn't one of the plugin's commands", clip(c, 80)));
        }
        Ok(())
    }

    /// The commands a node offers: `open` first (when set), then `menu`, each once.
    pub fn menu_commands(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in std::iter::once(&self.open).chain(&self.menu) {
            if !c.is_empty() && !out.contains(c) {
                out.push(c.clone());
            }
        }
        out
    }
}

/// A node of the plugin's tree, as the plugin gave it and the host checked it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrowserNode {
    /// How the plugin names it: given back in the request for its children. Unique in a listing.
    pub id: String,
    pub label: String,
    /// A line of detail shown beside the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// One of RustGIS's own icons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A number shown beside it (how many datasets a theme holds, say).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// Whether asking the plugin for its children may find some.
    pub expandable: bool,
    pub children: Vec<BrowserNode>,
}

/// What the plugin answered for one node.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Listing {
    pub nodes: Vec<BrowserNode>,
    /// Whether the plugin says there are more than it listed.
    pub truncated: bool,
    /// Short remarks to show with the listing.
    pub notes: Vec<String>,
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `s` kept to one line of at most `n` characters, control characters as spaces.
fn line(s: &str, n: usize) -> String {
    s.chars().take(n).map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The checked listing in a plugin's answer to `browse` (the `ok` value): `{"nodes": [{"id",
/// "label"?, "detail"?, "icon"?, "count"?, "expandable"?, "children"?}], "truncated"?,
/// "notes"?}`. An error for an answer that isn't an object with a `nodes` array. A node
/// without an id, or with one taken already or too long, is left out with its children; at
/// most [`MAX_TREE_NODES`] are kept, [`MAX_TREE_DEPTH`] deep.
pub fn listing(answer: &Value) -> Result<Listing, String> {
    let obj = answer.as_object().ok_or("the plugin's browse answer isn't an object")?;
    let nodes = obj.get("nodes").and_then(Value::as_array).ok_or("the plugin's browse answer has no 'nodes' list")?;
    let mut left = MAX_TREE_NODES;
    let mut seen = HashSet::new();
    Ok(Listing {
        nodes: browser_nodes(nodes, 0, &mut left, &mut seen),
        truncated: obj.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        notes: obj
            .get("notes")
            .and_then(Value::as_array)
            .map(|n| n.iter().filter_map(Value::as_str).take(MAX_NOTES).map(|s| line(s, MAX_STATE_LABEL)).collect())
            .unwrap_or_default(),
    })
}

/// The nodes in `v`, at most `left` of them (counted down) and [`MAX_TREE_DEPTH`] deep below
/// `depth`. A label defaults to the id (when missing or blank).
fn browser_nodes(v: &[Value], depth: usize, left: &mut usize, seen: &mut HashSet<String>) -> Vec<BrowserNode> {
    let mut nodes = Vec::new();
    for n in v {
        if *left == 0 {
            break;
        }
        let Some(id) = n.get("id").and_then(Value::as_str).filter(|i| !i.is_empty() && i.chars().count() <= MAX_TREE_ID) else {
            continue;
        };
        if !seen.insert(id.to_string()) {
            continue;
        }
        *left -= 1;
        let text = |key: &str| n.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty());
        let children = match n.get("children").and_then(Value::as_array) {
            Some(c) if depth + 1 < MAX_TREE_DEPTH => browser_nodes(c, depth + 1, left, seen),
            _ => Vec::new(),
        };
        nodes.push(BrowserNode {
            id: id.to_string(),
            label: text("label").map_or_else(|| line(id, MAX_STATE_LABEL), |l| line(l, MAX_STATE_LABEL)),
            detail: text("detail").map(|d| line(d, MAX_DETAIL)),
            icon: text("icon").map(|i| line(i, MAX_ICON)),
            count: n.get("count").and_then(Value::as_u64),
            expandable: n.get("expandable").and_then(Value::as_bool).unwrap_or(false),
            children,
        });
    }
    nodes
}

/// The request params for the `browse` command: the node whose children are wanted (`None` for
/// the root) and the search text (empty for none).
pub fn request(node: Option<&str>, query: &str) -> Value {
    json!({"browser": {"node": node, "query": query}})
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn validate_checks_names_and_limits() {
        let has = |c: &str| ["b", "q", "h"].contains(&c);
        let ok = BrowserDecl {
            label: "Tree".into(),
            icon: String::new(),
            browse: "b".into(),
            open: "q".into(),
            menu: vec!["h".into()],
            search: true,
        };
        assert!(ok.validate(&has).is_ok());
        assert_eq!(ok.menu_commands(), vec!["q".to_string(), "h".to_string()]);
        assert!(BrowserDecl { browse: "nope".into(), ..ok.clone() }.validate(&has).unwrap_err().contains("browse"));
        assert!(BrowserDecl { open: "nope".into(), ..ok.clone() }.validate(&has).unwrap_err().contains("open"));
        assert!(BrowserDecl { label: "x".repeat(33), ..ok.clone() }.validate(&has).unwrap_err().contains("label"));
        assert!(BrowserDecl { label: String::new(), ..ok.clone() }.validate(&has).is_err());
        assert!(
            BrowserDecl { menu: (0..17).map(|_| "h".to_string()).collect(), ..ok.clone() }.validate(&has).unwrap_err().contains("menu")
        );
        assert!(BrowserDecl { menu: vec!["q".into(), "q".into()], ..ok.clone() }.validate(&has).is_ok(), "duplicates collapse");
    }

    #[test]
    fn listing_sanitises_nodes() {
        let v = json!({"nodes": [
            {"id": "a", "label": "A\nline", "detail": "d".repeat(100), "icon": "folder", "count": 3, "expandable": true},
            {"id": "a", "label": "dup"}, 7, {"label": "no id"},
            {"id": "b", "children": [{"id": "c", "label": "C", "children": [{"id": "d"}]}]}
        ], "truncated": true, "notes": ["n1", 5, "n2"]});
        let l = listing(&v).unwrap();
        assert_eq!(l.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(l.nodes[0].label, "A line");
        assert_eq!(l.nodes[0].detail.as_deref().map(str::len), Some(MAX_DETAIL));
        assert_eq!(l.nodes[0].count, Some(3));
        assert!(l.nodes[0].expandable);
        assert_eq!(l.nodes[1].label, "b", "label defaults to the id");
        assert_eq!(l.nodes[1].children[0].children[0].id, "d");
        assert!(l.truncated);
        assert_eq!(l.notes, vec!["n1", "n2"]);
    }

    #[test]
    fn listing_cuts_depth_and_count() {
        let mut deep = json!({"id": "n9", "label": "9"});
        for i in (0..9).rev() {
            deep = json!({"id": format!("n{i}"), "children": [deep]});
        }
        let l = listing(&json!({"nodes": [deep]})).unwrap();
        let mut d = 0;
        let mut n = &l.nodes[0];
        while let Some(c) = n.children.first() {
            d += 1;
            n = c;
        }
        assert_eq!(d, crate::panel::MAX_TREE_DEPTH - 1);
        let many: Vec<Value> = (0..6000).map(|i| json!({"id": i.to_string()})).collect();
        assert_eq!(listing(&json!({"nodes": many})).unwrap().nodes.len(), crate::panel::MAX_TREE_NODES);
    }

    #[test]
    fn listing_of_a_non_object_is_an_error() {
        assert!(listing(&json!(7)).is_err());
        assert!(listing(&json!({"ok": true})).unwrap_err().contains("nodes"));
        assert!(listing(&json!({"nodes": "x"})).is_err());
        assert_eq!(listing(&json!({"nodes": []})).unwrap(), Listing::default());
    }

    #[test]
    fn request_shape() {
        assert_eq!(request(None, ""), json!({"browser": {"node": null, "query": ""}}));
        assert_eq!(request(Some("x"), "q"), json!({"browser": {"node": "x", "query": "q"}}));
    }
}
