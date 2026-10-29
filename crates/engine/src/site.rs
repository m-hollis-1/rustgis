//! Site settings: an optional `site.json` beside the executable (or the file `RUSTGIS_SITE_FILE`
//! names) with which an organisation supplies defaults: a build label, preference defaults,
//! plugin packages to offer and per-plugin settings. Read once at start-up; a missing file is
//! normal, and an unreadable or malformed one is logged and leaves the built-in defaults.
//! See `docs/site-settings.md`.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock};

/// The file's name, looked for beside the executable.
pub const FILE: &str = "site.json";
/// The environment variable naming the file instead.
pub const ENV: &str = "RUSTGIS_SITE_FILE";
/// A larger file is refused unread.
pub const MAX_BYTES: usize = 1 << 20;

const MAX_BUILD_CHARS: usize = 64;
const MAX_INSTALL: usize = 32;

/// What the site file says. The default (no file) changes nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Site {
    /// The file these values came from; `None` when none was looked for.
    pub path: Option<PathBuf>,
    /// A one-line build label shown beside the version.
    pub build: Option<String>,
    /// Defaults for preferences the user hasn't set: string values only.
    pub prefs: BTreeMap<String, String>,
    /// Plugin packages to offer to install, as absolute paths (relative ones are joined to the
    /// file's folder).
    pub install: Vec<PathBuf>,
    /// Per-plugin settings by plugin id, handed to the plugin as they are.
    pub settings: BTreeMap<String, Value>,
}

static SITE: RwLock<Site> =
    RwLock::new(Site { path: None, build: None, prefs: BTreeMap::new(), install: Vec::new(), settings: BTreeMap::new() });
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Reads site settings from `text`; relative package paths are joined to `base`. Only a text
/// that isn't a JSON object is an error: a key of the wrong type is ignored. The error never
/// includes the file's values.
pub fn parse(text: &str, base: &Path) -> Result<Site, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("invalid site.json at line {}, column {}", error.line(), error.column()))?;
    let object = value.as_object().ok_or("site.json must be a JSON object")?;
    let build = object
        .get("build")
        .and_then(Value::as_str)
        .map(|build| build.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_BUILD_CHARS).collect());
    let prefs = object
        .get("prefs")
        .and_then(Value::as_object)
        .map(|prefs| prefs.iter().filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let plugins = object.get("plugins").and_then(Value::as_object);
    let install = plugins
        .and_then(|plugins| plugins.get("install"))
        .and_then(Value::as_array)
        .map(|paths| paths.iter().filter_map(Value::as_str).take(MAX_INSTALL).map(|path| base.join(path)).collect())
        .unwrap_or_default();
    let settings = plugins
        .and_then(|plugins| plugins.get("settings"))
        .and_then(Value::as_object)
        .map(|settings| settings.iter().map(|(id, value)| (id.clone(), value.clone())).collect())
        .unwrap_or_default();
    let site = Site { path: None, build, prefs, install, settings };
    Ok(site)
}

/// Reads the site file at `path`. A missing file is normal and gives the defaults; an unreadable, oversized or malformed one is an error.
pub fn read(path: &Path) -> Result<Site, String> {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Site::default()),
        Err(error) => return Err(format!("could not read site.json: {error}")),
    };
    if size > MAX_BYTES as u64 {
        return Err("site.json is over 1 MiB".into());
    }
    let bytes = std::fs::read(path).map_err(|error| format!("could not read site.json: {error}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "site.json isn't UTF-8 text".to_string())?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut site = parse(&text, base)?;
    site.path = Some(path.to_path_buf());
    Ok(site)
}

/// Where the site file is: the file `RUSTGIS_SITE_FILE` names, else `site.json` beside the
/// executable.
pub fn locate() -> Option<PathBuf> {
    match std::env::var_os(ENV) {
        Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
        _ => Some(std::env::current_exe().ok()?.parent()?.join(FILE)),
    }
}

/// Loads the site file for this process. Call once at start-up; a failure is logged and leaves
/// the defaults.
pub fn init() {
    let site = match locate() {
        Some(path) => read(&path).unwrap_or_else(|error| {
            log::warn!("site settings: {error}");
            Site::default()
        }),
        None => Site::default(),
    };
    set(site);
}

/// Replaces the process-wide site settings.
pub fn set(site: Site) {
    *SITE.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = site;
}

/// A copy of the process-wide site settings.
pub fn current() -> Site {
    SITE.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// The site's default for preference `key`, if it sets one.
pub fn pref_default(key: &str) -> Option<String> {
    SITE.read().unwrap_or_else(|poisoned| poisoned.into_inner()).prefs.get(key).cloned()
}

/// The site's settings for plugin `id`; `Null` when it has none.
pub fn plugin_settings(id: &str) -> Value {
    SITE.read().unwrap_or_else(|poisoned| poisoned.into_inner()).settings.get(id).cloned().unwrap_or(Value::Null)
}

/// The version, followed by the site's build label when there is one.
pub fn version_label() -> String {
    let version = rustgis_plugins::manifest::RUSTGIS_VERSION;
    match SITE.read().unwrap_or_else(|poisoned| poisoned.into_inner()).build.as_deref() {
        Some(build) => format!("{version} {build}"),
        None => version.to_string(),
    }
}

/// Serialises tests that call [`set`], which changes a process-wide value. Public only so other
/// crates' tests can take it too.
#[doc(hidden)]
pub fn test_lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_reads_every_key() {
        let s = parse(
            r#"{"build":"hlr-1","prefs":{"language":"de","n":7},
            "plugins":{"install":["plugins/a.rgplugin","C:/abs/b.rgplugin"],"settings":{"p":{"portal":"https://x"}}}}"#,
            Path::new("C:/site"),
        )
        .unwrap();
        assert_eq!(s.build.as_deref(), Some("hlr-1"));
        assert_eq!(s.prefs.get("language").map(String::as_str), Some("de"));
        assert!(!s.prefs.contains_key("n"), "non-string prefs are ignored");
        assert_eq!(s.install, vec![PathBuf::from("C:/site/plugins/a.rgplugin"), PathBuf::from("C:/abs/b.rgplugin")]);
        assert_eq!(s.settings["p"]["portal"], "https://x");
    }

    #[test]
    fn parse_rejects_what_is_not_an_object_and_clips_build() {
        assert!(parse("[]", Path::new(".")).is_err());
        assert!(parse("{", Path::new(".")).is_err());
        let s = parse(&format!(r#"{{"build":"{}"}}"#, "x".repeat(100)), Path::new(".")).unwrap();
        assert_eq!(s.build.as_deref().map(str::len), Some(64));
        let s = parse(r#"{"build":"a\nb"}"#, Path::new(".")).unwrap();
        assert_eq!(s.build.as_deref(), Some("a b"), "one line");
        let many: Vec<String> = (0..40).map(|i| format!("p{i}.rgplugin")).collect();
        let s = parse(&json!({"plugins": {"install": many}}).to_string(), Path::new(".")).unwrap();
        assert_eq!(s.install.len(), 32);
    }

    #[test]
    fn read_missing_is_default_and_unreadable_is_error() {
        let dir = std::env::temp_dir().join(format!("site-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&dir.join("site.json")).unwrap(), Site::default());
        assert!(read(&dir).is_err(), "a folder is not a site file");
        std::fs::write(dir.join("big.json"), vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(read(&dir.join("big.json")).unwrap_err().contains("1 MiB"));
    }

    #[test]
    fn site_unreadable_is_default_with_error() {
        let _g = test_lock();
        // read() on a folder errs; init() must not panic and leaves defaults.
        let dir = std::env::temp_dir();
        assert!(read(&dir).is_err());
        set(Site::default());
        assert_eq!(current(), Site::default());
    }

    #[test]
    fn accessors_follow_the_set_site() {
        let _g = test_lock();
        set(Site {
            build: Some("hlr-1".into()),
            prefs: [("language".to_string(), "de".to_string())].into(),
            settings: [("p".to_string(), json!({"portal": "https://x"}))].into(),
            ..Default::default()
        });
        assert_eq!(pref_default("language").as_deref(), Some("de"));
        assert_eq!(pref_default("nope"), None);
        assert_eq!(plugin_settings("p")["portal"], "https://x");
        assert_eq!(plugin_settings("other"), Value::Null);
        assert_eq!(version_label(), format!("{} hlr-1", rustgis_plugins::manifest::RUSTGIS_VERSION));
        set(Site::default());
        assert_eq!(version_label(), rustgis_plugins::manifest::RUSTGIS_VERSION);
    }
}
