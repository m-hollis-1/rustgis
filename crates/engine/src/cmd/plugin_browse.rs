//! `plugin.browse`: lists the nodes of a plugin's browser (the Browser pane's tree).
//!
//! The plugin's browse command runs off the session, so a slow listing never holds the project:
//! [`job`] prepares it on the session's thread and [`BrowseJob::run`] runs it on any thread.
//! The command can only ask `host.info` (and log), so it cannot read or change the project,
//! which is why browsing is no undo step.

use rustgis_plugins::browser::{Listing, listing, request};
use rustgis_plugins::{Host, PluginCommand};
use serde_json::{Value, json};

use super::{plugins::info_answer, ps};
use crate::{CmdResult, Session};

/// A prepared run of a plugin's browse command.
pub struct BrowseJob {
    command: PluginCommand,
    params: Value,
    info: Value,
}

/// Prepares listing the children of `node` (the roots when `None`), narrowed by `query`, in the
/// browser of plugin `id`.
pub fn job(id: &str, node: Option<&str>, query: &str) -> Result<BrowseJob, String> {
    let plugin = rustgis_plugins::get(id).ok_or_else(|| format!("no plugin '{id}'"))?;
    let name = &plugin.manifest.name;
    if !plugin.enabled {
        return Err(format!("the plugin {name} is disabled"));
    }
    let decl = plugin.manifest.browser.as_ref().ok_or_else(|| format!("the plugin {name} has no browser section"))?;
    let command = rustgis_plugins::command(&rustgis_plugins::command_id(id, &decl.browse))
        .ok_or_else(|| format!("the plugin {name} has no command '{}'", decl.browse))?;
    Ok(BrowseJob { command, params: request(node, query), info: info_answer(&plugin.manifest) })
}

impl BrowseJob {
    /// Runs the browse command and reads its listing. Safe on any thread; a panic in the plugin
    /// run is an error, not a crash.
    pub fn run(&self) -> Result<Listing, String> {
        let mut host = BrowseHost { info: self.info.clone() };
        let answer =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rustgis_plugins::run_command(&self.command, &mut host, &self.params)))
                .unwrap_or_else(|_| Err(format!("{} failed unexpectedly", self.command.decl.label)))?;
        listing(&answer)
    }
}

/// The host a browse command runs against: `host.info` and nothing else.
pub(crate) struct BrowseHost {
    info: Value,
}

impl Host for BrowseHost {
    fn call(&mut self, op: &str, _: &Value) -> Result<Value, String> {
        match op {
            "host.info" => Ok(self.info.clone()),
            _ => Err(format!("browse commands may only call host.info, not '{op}'")),
        }
    }
}

/// `plugin.browse`: the nodes under `node` (the roots when absent) of plugin `id`'s browser.
pub(super) fn browse(_: &mut Session, p: &Value) -> CmdResult {
    let id = ps(p, "id").ok_or("id is required")?;
    let node = ps(p, "node");
    let query = ps(p, "query").unwrap_or("");
    let found = job(id, node, query)?.run()?;
    let nodes = serde_json::to_value(&found).map_err(|e| e.to_string())?;
    Ok(json!({"id": id, "node": node, "nodes": nodes["nodes"], "truncated": nodes["truncated"], "notes": nodes["notes"]}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_host_answers_info_only() {
        let mut h = BrowseHost { info: json!({"plugin": "x"}) };
        assert_eq!(h.call("host.info", &json!({})).unwrap(), json!({"plugin": "x"}));
        let e = h.call("host.layers", &json!({})).unwrap_err();
        assert!(e.contains("browse commands may only call host.info") && e.contains("host.layers"));
        assert!(h.call("layer.remove", &json!({"layer": 1})).is_err());
    }

    #[test]
    fn a_job_runs_on_another_thread() {
        fn send<T: Send>() {}
        send::<BrowseJob>();
    }
}
