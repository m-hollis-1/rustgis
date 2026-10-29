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
    init_from(locate());
}

/// Loads the site file at `path` (none: the defaults); a failure is logged and leaves the
/// defaults.
fn init_from(path: Option<PathBuf>) {
    let site = match path {
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

/// The site's build label, if it sets one (a clone of just the label, for per-frame callers).
pub fn build() -> Option<String> {
    SITE.read().unwrap_or_else(|poisoned| poisoned.into_inner()).build.clone()
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

/// A package the site file offers that isn't installed yet, or is installed in an older version.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteInstall {
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub version: String,
    /// A process plugin: runs a program on the computer, not sandboxed.
    pub process: bool,
}

/// The site file's packages (`current().install`) to offer: those not installed, or installed in
/// an older version. A plugin loaded from a development folder is never offered over (the
/// developer's copy wins). A package that can't be read or parsed is logged and skipped.
pub fn site_installs() -> Vec<SiteInstall> {
    let mut out = Vec::new();
    for path in current().install {
        // Size-checked before reading: a huge file isn't read into memory.
        let len = match std::fs::metadata(&path) {
            Ok(meta) => meta.len(),
            Err(error) => {
                log::warn!("site package {} not read: {error}", path.display());
                continue;
            }
        };
        if len > rustgis_plugins::package::MAX_PACKAGE as u64 {
            log::warn!("site package {} is larger than {} MB", path.display(), rustgis_plugins::package::MAX_PACKAGE / (1024 * 1024));
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("site package {} not read: {error}", path.display());
                continue;
            }
        };
        let manifest = match rustgis_plugins::Package::from_zip(&bytes) {
            Ok(package) => package.manifest,
            Err(error) => {
                log::warn!("site package {} is not a plugin package: {error}", path.display());
                continue;
            }
        };
        let pending = match rustgis_plugins::get(&manifest.id) {
            None => true,
            // A development folder (`plugin.install` of a folder) is the plugin's source.
            Some(installed) if Path::new(&installed.source).is_dir() => false,
            Some(installed) => {
                rustgis_plugins::manifest::version_cmp(&installed.manifest.version, &manifest.version) == std::cmp::Ordering::Less
            }
        };
        if pending {
            out.push(SiteInstall {
                path,
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                process: manifest.runtime == rustgis_plugins::manifest::Runtime::Process,
            });
        }
    }
    out
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
        let base = std::env::temp_dir().join("site");
        let absolute = std::env::temp_dir().join("b.rgplugin");
        let text = json!({
            "build": "hlr-1",
            "prefs": {"language": "de", "n": 7},
            "plugins": {"install": ["plugins/a.rgplugin", absolute.to_string_lossy()], "settings": {"p": {"portal": "https://x"}}},
        })
        .to_string();
        let s = parse(&text, &base).unwrap();
        assert_eq!(s.build.as_deref(), Some("hlr-1"));
        assert_eq!(s.prefs.get("language").map(String::as_str), Some("de"));
        assert!(!s.prefs.contains_key("n"), "non-string prefs are ignored");
        assert_eq!(s.install, vec![base.join("plugins/a.rgplugin"), absolute]);
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
        let error = read(&dir.join("big.json")).unwrap_err();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(error.contains("1 MiB"));
    }

    #[test]
    fn init_from_a_bad_or_missing_file_leaves_the_defaults() {
        let _g = test_lock();
        let dir = std::env::temp_dir().join(format!("site-init-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("big.json"), vec![b' '; MAX_BYTES + 1]).unwrap();
        std::fs::write(dir.join("good.json"), r#"{"build": "hlr-1"}"#).unwrap();
        let bad = [Some(dir.clone()), Some(dir.join("big.json")), Some(dir.join("missing.json")), None];
        for path in bad {
            set(Site { build: Some("stale".into()), ..Default::default() });
            init_from(path.clone());
            assert_eq!(current(), Site::default(), "{path:?}");
        }
        init_from(Some(dir.join("good.json")));
        let got = current();
        set(Site::default());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(got.build.as_deref(), Some("hlr-1"));
        assert_eq!(got.path, Some(dir.join("good.json")));
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
        assert_eq!(build().as_deref(), Some("hlr-1"));
        set(Site::default());
        assert_eq!(build(), None);
        assert_eq!(version_label(), rustgis_plugins::manifest::RUSTGIS_VERSION);
    }

    /// Loads the test plugin `id` (version 1.0.0) from `source`.
    fn load_plugin(id: &str, source: &str) {
        let package = rustgis_plugins::Package::from_zip(&rustgis_plugins::testing::package(id, &[])).unwrap();
        rustgis_plugins::load(package, source).unwrap();
    }

    /// Writes the test plugin `id` at `version` as `dir/name`.
    fn write_package(dir: &Path, name: &str, id: &str, version: &str) -> PathBuf {
        let mut manifest: Value = serde_json::from_str(&rustgis_plugins::testing::manifest(id, &[])).unwrap();
        manifest["version"] = json!(version);
        let bytes = rustgis_plugins::testing::zip_of(&[
            ("plugin.json", manifest.to_string().as_bytes()),
            ("plugin.wasm", &rustgis_plugins::testing::wasm()),
        ]);
        std::fs::write(dir.join(name), bytes).unwrap();
        dir.join(name)
    }

    #[test]
    fn info_answers_site_settings_as_given() {
        let _g = test_lock();
        set(Site {
            build: Some("hlr-1".into()),
            settings: [("eng-info".to_string(), json!({"portal": "https://x", "odd": [1]}))].into(),
            ..Default::default()
        });
        let mut s = crate::Session::new(crate::Project::default());
        load_plugin("eng-info", "test");
        // The test plugin's command `f` sends `op` to the host and answers with the reply.
        let info = json!({"op": "host.info", "params": {}});
        let got = s.run("plugin:eng-info/f", &info).unwrap();
        assert_eq!(got["build"], "hlr-1");
        assert_eq!(got["site"], json!({"portal": "https://x", "odd": [1]}));
        assert_eq!(got["plugin"], "eng-info");
        set(Site::default());
        let got = s.run("plugin:eng-info/f", &info).unwrap();
        rustgis_plugins::unload("eng-info");
        assert_eq!(got["build"], Value::Null);
        assert_eq!(got["site"], Value::Null);
    }

    #[test]
    fn site_installs_skips_installed_and_newer() {
        let _g = test_lock();
        let dir = std::env::temp_dir().join(format!("site-inst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let new = write_package(&dir, "new.rgplugin", "site-new", "1.0.0");
        let same = write_package(&dir, "same.rgplugin", "site-same", "1.0.0");
        let older = write_package(&dir, "older.rgplugin", "site-older", "0.9.0");
        let newer = write_package(&dir, "newer.rgplugin", "site-newer", "2.0.0");
        for id in ["site-same", "site-older", "site-newer"] {
            load_plugin(id, "test"); // version 1.0.0 of each
        }
        set(Site { install: vec![new, same, older, newer], ..Default::default() });
        let got: Vec<(String, String)> = site_installs().into_iter().map(|i| (i.id, i.version)).collect();
        set(Site::default());
        for id in ["site-same", "site-older", "site-newer"] {
            rustgis_plugins::unload(id);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, vec![("site-new".to_string(), "1.0.0".to_string()), ("site-newer".to_string(), "2.0.0".to_string())]);
    }

    #[test]
    fn site_installs_never_offers_over_a_development_folder() {
        let _g = test_lock();
        let dir = std::env::temp_dir().join(format!("site-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let newer = write_package(&dir, "dev.rgplugin", "site-dev", "2.0.0");
        set(Site { install: vec![newer], ..Default::default() });
        // Loaded from a folder, as `plugin.install` of a development folder does.
        load_plugin("site-dev", &dir.display().to_string());
        let from_folder = site_installs().len();
        rustgis_plugins::unload("site-dev");
        load_plugin("site-dev", &dir.join("site-dev.rgplugin").display().to_string());
        let from_package = site_installs().len();
        set(Site::default());
        rustgis_plugins::unload("site-dev");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(from_folder, 0, "a development folder wins");
        assert_eq!(from_package, 1, "an installed package is offered the newer version");
    }

    #[test]
    fn site_installs_skips_oversized_packages() {
        let _g = test_lock();
        let dir = std::env::temp_dir().join(format!("site-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.rgplugin");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(rustgis_plugins::package::MAX_PACKAGE as u64 + 1).unwrap();
        drop(file);
        set(Site { install: vec![big], ..Default::default() });
        let got = site_installs();
        set(Site::default());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got.is_empty());
    }

    #[test]
    fn site_installs_skips_missing_packages() {
        let _g = test_lock();
        set(Site { install: vec![PathBuf::from("C:/nope/../x.rgplugin"), PathBuf::from("/also/missing.rgplugin")], ..Default::default() });
        let got = site_installs();
        set(Site::default());
        assert!(got.is_empty());
    }
}
