# Site settings

An organisation can give RustGIS defaults without changing the program: put a `site.json` file
beside the RustGIS executable. Nothing is needed for a normal install; without the file RustGIS
behaves as it always does.

## Where the file is looked for

1. The file named by the `RUSTGIS_SITE_FILE` environment variable, when it is set.
2. Otherwise `site.json` in the folder of the executable.

A missing file is normal. A file that can't be read, is over 1 MiB, or isn't a JSON object is
skipped with a warning in the log, and RustGIS starts with its own defaults. A key of the wrong
type is ignored; the rest of the file still applies.

## The keys

```json
{
  "build": "acme-2026.1",
  "prefs": { "language": "de", "theme": "dark" },
  "plugins": {
    "install": ["plugins/survey.rgplugin", "C:/share/rustgis/reports.rgplugin"],
    "settings": { "survey": { "portal": "https://example.org/portal" } }
  }
}
```

| Key | Meaning | Limits |
|---|---|---|
| `build` | A label shown beside the version, for example `0.9.0 acme-2026.1`. | One line (control characters become spaces), at most 64 characters. |
| `prefs` | Defaults for preferences, by preference name. | String values only; others are ignored. |
| `plugins.install` | Plugin packages (`.rgplugin`) to offer to install at start-up. | At most 32. A relative path is read from the folder of `site.json`; an absolute path is used as it is. |
| `plugins.settings` | Settings handed to a plugin, by plugin id. | Any JSON value per plugin. |

## What plugins see

A plugin's `host.info` request answers with two more fields when a site file is in use:

- `build`: the `build` label, or `null`.
- `site`: that plugin's own entry from `plugins.settings`, or `null`. A plugin never sees the
  settings of another plugin.

## Preferences

A `prefs` entry is a default, not a rule. A preference the user has set themselves always wins;
the site's value applies only while the user hasn't chosen one, and choosing a value in
Preferences replaces it. The site file is never written by RustGIS.

## Installing plugins

On its first frame the desktop app offers the packages in `plugins.install` that aren't installed
yet or are installed in an older version, in one prompt (Install or Not now; Not now asks again at
the next start, and a process plugin is marked Not sandboxed). It installs only what the user
approves. The command line and Python read the same file (for `build` and the defaults) but never
install packages from it.
