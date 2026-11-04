//! What's wrong with a broken layer: its stored reason sorted into a kind the Repair Source
//! dialog can offer fixes for, plus where it was read from. Nothing is probed: the diagnosis
//! comes from the layer's reason (or the error its last retry gave).

use crate::{CmdResult, CommandSpec as C, Session};
use serde::Serialize;
use serde_json::{Value, json};

pub fn commands() -> Vec<C> {
    vec![C::new("layer.diagnose", "Diagnose Broken Layer", "Layers", diagnose).view().params(
        r#"{"layer": name|id} → the layer's id as "layer", its "kind" (signIn, certificate, unreachable, serviceMissing, fileMissing or other) and "reason"; for a service layer its "server" (https://host); for a file layer its "path", its "folder" and the "siblings" (ids of the map's other broken file layers read from that folder)"#,
    )]
}

/// What stops a broken layer opening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// The service wants a sign-in, or refused the one it was given.
    SignIn,
    /// The connection isn't trusted (a certificate the computer doesn't know).
    Certificate,
    /// The server didn't answer.
    Unreachable,
    /// The server answered that the service isn't there.
    ServiceMissing,
    /// The file or folder isn't where the layer remembers it.
    FileMissing,
    Other,
}

const SIGN_IN: &[&str] =
    &["token required", "invalid token", "token was refused", "http status: 401", "http status: 403", "(499)", "(498)"];
const CERTIFICATE: &[&str] = &["certificate", "unknownissuer", "untrusted", "rustls"];
const UNREACHABLE: &[&str] = &[
    "host not found",
    "connection failed",
    "timeout",
    "connection refused",
    "unreachable",
    // How the system words a refused or unanswered connection (Windows', Linux').
    "actively refused",
    "timed out",
    "os error 10061",
    "os error 10060",
    "os error 111",
];

fn mentions(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| text.contains(p))
}

/// The kind of failure a reason describes, for a layer read from a service when `service`.
fn classify(reason: &str, service: bool) -> Kind {
    let text = reason.to_lowercase();
    if mentions(&text, SIGN_IN) {
        Kind::SignIn
    } else if mentions(&text, CERTIFICATE) {
        Kind::Certificate
    } else if mentions(&text, UNREACHABLE) {
        Kind::Unreachable
    } else if text.contains("http status: 404") || (service && text.contains("not found")) {
        Kind::ServiceMissing
    } else {
        Kind::Other
    }
}

/// The kind of failure a reason describes, as for a layer read from a web service.
pub(crate) fn diagnose_text(reason: &str) -> Kind {
    classify(reason, true)
}

/// A broken feature layer's file, if it names one on this computer.
fn file_of(src: &rustgis_map::source::SourceLayer) -> Option<&str> {
    let path = src.path.as_deref().filter(|_| src.service.is_none())?;
    rustgis_map::paths::is_local(path).then(|| rustgis_map::paths::local_path(path))
}

/// Whether the file is gone from this computer (the web app has no files to be gone).
fn is_missing(path: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        !std::path::Path::new(path).exists()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        false
    }
}

/// The folder a file is in, as the layer remembers it.
fn folder_of(path: &str) -> Option<&str> {
    let i = path.rfind(['/', '\\'])?;
    path.get(..i).filter(|f| !f.is_empty())
}

/// The scheme and host (with its port) of a web address, without any login in it.
fn server_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next().filter(|h| !h.is_empty())?;
    Some(format!("{scheme}://{host}"))
}

fn same_folder(a: &str, b: &str) -> bool {
    let norm = |f: &str| f.replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

fn diagnose(s: &mut Session, p: &Value) -> CmdResult {
    let id = s.layer_id(p)?;
    let l = s.map().layer(id).ok_or("the layer no longer exists")?;
    let src = l.source.as_ref().filter(|src| src.is_broken()).ok_or_else(|| format!("{} isn't broken", l.name))?;
    let reason = src.why_broken().unwrap_or_default();
    let mut kind = if src.service.is_some() { diagnose_text(reason) } else { classify(reason, false) };
    let mut out = json!({"layer": id, "reason": reason});
    if let Some(service) = &src.service {
        if let Some(server) = server_of(&service.url) {
            out["server"] = json!(server);
        }
    } else if let Some(path) = file_of(src) {
        if is_missing(path) {
            kind = Kind::FileMissing;
        }
        out["path"] = json!(path);
        if let Some(folder) = folder_of(path) {
            out["folder"] = json!(folder);
            let siblings: Vec<u32> = s
                .map()
                .layers
                .iter()
                .filter(|o| o.id != id)
                .filter(|o| {
                    o.source
                        .as_ref()
                        .is_some_and(|x| x.is_broken() && file_of(x).and_then(folder_of).is_some_and(|f| same_folder(f, folder)))
                })
                .map(|o| o.id)
                .collect();
            out["siblings"] = json!(siblings);
        }
    }
    out["kind"] = json!(kind);
    Ok(out)
}
