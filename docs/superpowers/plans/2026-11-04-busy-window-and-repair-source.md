# Busy Window, Repair Source Helper and One ArcGIS Online Sign-In Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tell the user when a long import/save/open is running, give broken layers a guided Repair Source… dialog, and let the ArcGIS Online plugin's sign-in open token-secured layers.

**Architecture:** Engine gets three small commands (`layer.diagnose`, `layer.repairFolder`, `layer.relinkService`) and `arcgis.adoptToken`; the egui front end gets a deferred-action busy modal (`busy.rs`) and a new dialog (`repairbroken.rs`); the plugin repo hands its token to the host after each command. New code goes in new files; shared files change in small marked places.

**Tech Stack:** Rust 2024 workspace (engine, map, tiles, ui-egui with egui + egui_kittest), plugin repo `rustgis-arcgis-online` (process plugin, JSON-lines protocol).

**Spec:** `docs/superpowers/specs/2026-11-04-busy-window-and-repair-source-design.md` (read its "Decisions settled after the builder check" section — they bind).

## Global Constraints

- AGENTS.md: no `unwrap`/`expect`/`panic!`/direct indexing outside tests; everything below `apps/` builds for wasm32 (gate native-only code with `#[cfg(not(target_arch = "wasm32"))]`).
- Privacy: no company, client, people or G: drive names in code, tests, fixtures, docs or commit messages. Test servers are `127.0.0.1` or `example.com`/`example.org`.
- Tokens never reach the project file, the session log, notices, or command answers.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never push.
- Tooling: `PATH="$HOME/.cargo/bin:$PATH" CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2`; never two cargo commands at once on one target dir.
- Every new command with an optional `layer` must be classified for `record_tests::every_optional_layer_is_classified` (ui-egui lib.rs `DEFAULTS_TO_ACTIVE_LAYER` ~4288). Simplest: make `layer` required.
- Plugin repo: clippy denies unwrap/expect/panic in non-test code; plugin.json and Cargo.toml versions must match (`scripts/package.py`).

## Review Focus

1. **Save prompt then continue.** "Save changes?" → Save, then the replaced command (open/new/quit) runs: it must run only after the save finished, and not at all if the save failed. (Task 5 test.)
2. **Adopted token vs built-in sign-in.** Adopting the plugin's token must never replace or out-rank a user's own built-in sign-in for the same portal, never cover hosts outside `scopes_for`, and an expired adopted token without a refresh must not be picked over a live session. (Task 4 tests.)
3. **Batch folder repair with misfits.** Siblings in the new folder with the same file name but a different geometry type or missing fields are skipped and listed; the rest are repaired; the batch is one undo step. (Task 2 test.)
4. **Diagnosis from a reloaded project.** A broken layer read back from a saved `.rgis` has only `reason` text and no fresh error: it still gets the right diagnosis. (Task 1 test.)
5. **Busy window input.** A click or key pressed while the UI thread was blocked must not land on the UI behind the modal after it closes. (Task 5 test.)

---

### Task 1: `layer.diagnose` — what's wrong with a broken layer

**Files:**
- Create: `crates/engine/src/cmd/broken.rs` (register in `crates/engine/src/cmd/mod.rs` next to `repair`)
- Modify: `crates/engine/src/cmd/retry.rs` (a failed retry records its error on the layer)
- Test: `crates/engine/src/tests.rs` (near the retry tests, ~25791; reuse `broken_countries`, `broken_service`)

**Interfaces:**
- Produces: command `layer.diagnose` `{"layer": name|id}` (required) → `{"layer": id, "kind": "signIn"|"certificate"|"unreachable"|"serviceMissing"|"fileMissing"|"other", "reason": text, "server"?: "https://host", "path"?: old path, "folder"?: old folder, "siblings"?: [ids of other broken file layers in that folder]}`. `.view()` (no undo step). Errors for a layer that isn't a broken feature layer: "<name> isn't broken".
- Produces: `pub(crate) fn diagnose_text(reason: &str) -> Kind` (pure, for tests and the dialog).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn diagnose_text_sorts_the_reasons_rustgis_gives() {
    use crate::cmd::broken::{diagnose_text, Kind};
    let cases = [
        ("https://x/FeatureServer: the service answered with an error: Token Required (the service is secured: …)", Kind::SignIn),
        ("https://x: the service answered with an error: Invalid Token (the ArcGIS token was refused, perhaps expired: sign in to the portal again)", Kind::SignIn),
        ("https://x: http status: 401", Kind::SignIn),
        ("https://x: http status: 403", Kind::SignIn),
        ("https://x: rustls: invalid peer certificate: UnknownIssuer", Kind::Certificate),
        ("https://x: http status: 404", Kind::ServiceMissing),
        ("https://x: host not found", Kind::Unreachable),
        ("https://x: connection failed", Kind::Unreachable),
        ("https://x: timeout: global", Kind::Unreachable),
        ("RustGIS can't open this kind of data", Kind::Other),
    ];
    for (text, want) in cases {
        assert_eq!(diagnose_text(text), want, "{text}");
    }
}

#[test]
fn diagnose_names_the_server_and_the_folders_other_broken_layers() {
    // Two broken file layers in one missing folder, one broken service layer.
    // layer.diagnose on the first file layer → kind fileMissing, path, folder, siblings == [second id].
    // layer.diagnose on the service layer (reason "…: Token Required …") → kind signIn, server "https://127.0.0.1:9".
    // layer.diagnose on a healthy layer → Err containing "isn't broken".
}

#[test]
fn a_broken_layer_read_back_from_a_project_is_still_diagnosed() {
    // Save a project with a broken service layer whose reason says Token Required, open it again,
    // layer.diagnose → kind signIn (no fresh error, reason only).
}

#[test]
fn a_failed_retry_records_its_error_for_the_next_diagnosis() {
    // broken_service (unreachable 127.0.0.1:9): layer.retry, then layer.diagnose → reason is the
    // retry's error (not the original "it needs a sign-in"), kind unreachable.
}
```

- [ ] **Step 2: Run them to see them fail** — `cargo test -p rustgis-engine --lib diagnose` → FAIL (no module `broken`).

- [ ] **Step 3: Implement** `enum Kind` (derive Debug, PartialEq, Clone, Copy; serialises to the camelCase names above), `diagnose_text`, and the command in `broken.rs`. Matching is case-insensitive substring on fixed phrases: sign-in = `token required`, `invalid token`, `token was refused`, `http status: 401`, `http status: 403`, `(499)`, `(498)`; certificate = `certificate`, `unknownissuer`, `untrusted`, `rustls`; service missing = `http status: 404`, `not found` when the layer has a service; unreachable = `host not found`, `connection failed`, `timeout`, `connection refused`, `unreachable`. A file layer (has `path`, no `service`) is `fileMissing` when its local path doesn't exist (native: `Path::exists`; wasm: never), whatever the text. `server` is the scheme+host of the service URL. In `retry.rs`, when a layer's retry fails, set that layer's `source.error` and `source.reason` to the error before returning it in `failed`.

- [ ] **Step 4: Run the tests** — same command → PASS; then `cargo test -p rustgis-engine --lib retry` → PASS.

- [ ] **Step 5: Commit** — `feat(engine): layer.diagnose says what's wrong with a broken layer`

### Task 2: `layer.repairFolder` — fix every broken file layer from one folder

**Files:**
- Modify: `crates/engine/src/cmd/repair.rs` (new command beside `layer.repairSource`)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- Consumes: `layer.repairSource` (run through `s.run_part`, so the batch is one undo step).
- Produces: `layer.repairFolder` `{"from": old folder, "to": new folder}` → `{"repaired": [names], "skipped": [{"layer": name, "why": text}]}`. For each broken file layer whose path's parent folder equals `from` (compare case-insensitively on Windows, normalised separators), repair to `to/<same file name>` when that exists; otherwise skip with "not in <to>". A `repairSource` error skips with its text. Mutating; desktop only (wasm: Err like `repairSource`'s). Err when nothing was repaired and nothing was skipped: "no broken layer had its data in <from>".

- [ ] **Step 1: Write the failing test** `repair_folder_fixes_the_layers_that_fit_and_lists_the_rest`: three broken layers with data in `<tmp>/old`: A (countries shapefile, copy present in `<tmp>/new`), B (file absent from `new`), C (a file of the same name in `new` holding points while C draws polygons). Run `layer.repairFolder {"from": old, "to": new}` → `repaired == ["A"]`, `skipped` has B ("not in") and C (geometry text). One `edit.undo` returns A to broken.
- [ ] **Step 2: Run** `cargo test -p rustgis-engine --lib repair_folder` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(engine): layer.repairFolder repairs every broken layer from one folder`

### Task 3: `layer.relinkService` — point a broken service layer at another address

**Files:**
- Modify: `crates/engine/src/cmd/retry.rs` (new command reusing `retry_service`)
- Test: `crates/engine/src/tests.rs` (use the local `ogc_server` as `a_retried_service_that_answers_a_table_leaves_no_table_behind` does)

**Interfaces:**
- Produces: `layer.relinkService` `{"layer": name|id, "url": "a service address"}` → `{"layer": id, "opened": true}`. Builds `ServiceOrigin { url, kind: old.kind, layer: old.layer }` and calls `retry_service`. Errors from the service pass through unchanged (the layer stays broken, its origin's URL unchanged). Mutating (one undo step). Only for broken layers that have a `service`; else Err "<name> isn't a broken service layer".

- [ ] **Step 1: Write the failing test** `relinking_a_broken_service_layer_opens_it_from_the_new_address`: broken service layer on `127.0.0.1:9`; local server answering an ArcGIS feature layer; `layer.relinkService` with the server's URL → opened, layer keeps its id, name and renderer, has features. A second test: relinking to an unreachable address returns Err and the layer stays broken.
- [ ] **Step 2: Run** `cargo test -p rustgis-engine --lib relink` → FAIL.
- [ ] **Step 3: Implement.** Check the opened layer's source records the new URL (the report noted `l.source = new.source` replaces the origin; assert on it in the test).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(engine): layer.relinkService opens a broken service layer from another address`

### Task 4: `arcgis.adoptToken` — take an ArcGIS Online token from a plugin

**Files:**
- Modify: `crates/tiles/src/arcgis_auth.rs` (new `pub fn adopt`, and `covering` skips dead sessions)
- Modify: `crates/engine/src/cmd/arcgis.rs` (new command), `crates/engine/src/cmd/tiles.rs` (add `"arcgis.adoptToken"` to `NETWORK`)
- Test: unit tests in `arcgis_auth.rs`; engine test in `tests.rs`

**Interfaces:**
- Produces: `pub fn adopt(portal: &str, token: &str, expires_at: i64, org: &str, username: &str) -> Result<bool, String>` — `Ok(true)` when the token kept changed. Refuses: portal not on https or not ArcGIS Online (`portal_base` + `is_online`), invalid token (`valid_token`), org not 1–64 ASCII alphanumerics. Session: `client_id = ADOPTED` (a new private const `"plugin"`), scopes `scopes_for(portal, Some(org), &[])`, no refresh token. Replaces only an earlier adopted session for the same portal; when the user has their own (non-adopted, non-token) sign-in for that portal that is unexpired or renewable, it keeps theirs and returns `Ok(false)`.
- `covering` (arcgis_auth.rs ~484) ignores sessions with `expires_at <= now` and no `refresh_token`.
- Produces: command `arcgis.adoptToken` `{"portal", "token", "expires": unix seconds, "org", "username"?}` → `{"portal": base, "changed": bool, "broken": n}` where `n` counts broken layers in the current map. `.view()`. Clears the tile cache only when `changed`. Never echoes the token.

- [ ] **Step 1: Write the failing tests** (in `arcgis_auth.rs`, using `with_token`/`authorization` on example URLs):
  - `an_adopted_token_covers_the_organizations_hosted_services_only` — adopt for `https://www.arcgis.com`, org `abc123`; `authorization("https://services5.arcgis.com/abc123/arcgis/rest/services/S/FeatureServer/0")` is Some; for `.../otherorg/...` and `https://example.org/arcgis/rest/services/S/FeatureServer` is None.
  - `adopting_never_replaces_the_users_own_sign_in` — `sign_in(...)` a session with a refresh token for the portal, then adopt → `Ok(false)`, `list()` still shows the user's session and no adopted one.
  - `an_expired_adopted_token_is_not_sent` — adopt with `expires_at` in the past → `authorization` None.
  - `adopt_refuses_an_enterprise_portal_or_plain_http` — `https://gis.example.org/portal` and `http://www.arcgis.com` → Err.
  - Engine: `adopt_token_reports_broken_layers_and_keeps_no_token` — one broken service layer; `arcgis.adoptToken` answer has `"broken": 1`, and its JSON text doesn't contain the token; a plugin with only `network` permission may run it (assert `refused()` is None for it — follow the existing plugin permission tests in `plugins.rs`).
- [ ] **Step 2: Run** `cargo test -p rustgis-tiles arcgis_auth` and `cargo test -p rustgis-engine --lib adopt` → FAIL.
- [ ] **Step 3: Implement.** Tests touching the global session store must reset it (follow the existing `arcgis_auth` tests' pattern).
- [ ] **Step 4: Run** → PASS; also `cargo test -p rustgis-tiles` (the `covering` change).
- [ ] **Step 5: Commit** — `feat(arcgis): arcgis.adoptToken takes an ArcGIS Online token from a plugin`

### Task 5: Busy window for import, save and open

**Files:**
- Create: `crates/ui-egui/src/busy.rs` (declare in `lib.rs`)
- Modify: `crates/ui-egui/src/lib.rs` — `logic` (~3839, run a due action at its top), `ui` (~4140, paint next to `saveprompt::show`), `run_ui` `"ui.save"`/`"ui.saveAs"`/`"ui.open"` (~2822–2874), `import_arcgis` (~1409, the `.mapx`/`.lyrx` immediate import and the `.aprx` inspect); `crates/ui-egui/src/dialogs.rs` `finish_arcgis_import` (~679); `crates/ui-egui/src/saveprompt.rs` (~142–163)
- Test: `busy.rs` `mod tests` and `lib.rs` `mod aprx_tests`

**Interfaces:**
- Produces: `pub(crate) fn queue(app: &mut RustGisApp, message: String, action: impl FnOnce(&mut RustGisApp) + 'static)`; `pub(crate) fn run_due(app: &mut RustGisApp, ctx: &egui::Context)` (called first in `logic`); `pub(crate) fn show(app: &mut RustGisApp, ctx: &egui::Context)` (paints the modal, marks it painted, requests a repaint). State on `app.ui` as `busy: Option<busy::Pending>`. An action runs only in a `logic` after a frame in which `show` painted it. In the frame after it ran, pointer and key events are dropped (`ctx.input_mut(|i| i.events.clear())` plus pointer presses) so nothing queued during the block lands.
- Modal text: title from the message ("Importing 3 maps and 2 layouts from <file>…", "Saving <project>…", "Opening <file>…", "Reading <file>…"), then the fixed line "This can take several minutes for a large project. Windows may say RustGIS is not responding — don't close it." No close button.
- Save prompt: Save then continue becomes one queued action — save, and only if it succeeded, run the replaced command.

- [ ] **Step 1: Write the failing tests:**
  - `busy.rs`: `an_action_runs_only_after_its_window_was_painted` — queue an action that sets a flag; `run_due` before `show` → flag unset; `show` then `run_due` → set, and `app.ui.busy` is None.
  - `aprx_tests`: `import_shows_the_busy_window_before_it_imports` — drive the picker as the existing tests do, click Import; after one step the window text "Importing" is shown and the map count is unchanged; after more steps the maps are imported and the text is gone.
  - `saveprompt.rs`: `save_then_continue_waits_for_the_save` — a save that fails (unwritable path) does not run the replaced command; one that succeeds runs it after.
  - `busy.rs`: `a_click_during_the_block_is_dropped` — a pointer press pushed in the frame the action runs is not delivered the next frame.
- [ ] **Step 2: Run** `cargo test -p rustgis-ui-egui --lib busy` and `... aprx` and `... saveprompt` → FAIL.
- [ ] **Step 3: Implement.** Call sites route through `busy::queue` with their message; command results/errors are shown as they are today, from inside the action.
- [ ] **Step 4: Run** → PASS, and `cargo test -p rustgis-ui-egui --lib` whole.
- [ ] **Step 5: Commit** — `feat(ui): a working window while a project imports, saves or opens`

### Task 6: Repair Source… helper dialog

**Files:**
- Create: `crates/ui-egui/src/repairbroken.rs`
- Modify: `crates/ui-egui/src/panes.rs` `layer_menu` (~2287–2302: a broken feature layer gets **Repair Source…** instead of Retry / Repair Data Source…; healthy layers unchanged); `crates/ui-egui/src/dialogs.rs` (`Dialogs::open` match ~87, `show` early return like `repairSource` ~273); `crates/ui-egui/src/lib.rs` `run_ui` `"ui.dialog"` (~3479) for `"repairBroken"`; `crates/ui-egui/src/arcgisui.rs` (move "sign-in finished → `retry_broken_layers`" from `form` into a per-frame `pub(crate) fn poll(app)` called from `logic`, so it fires whichever dialog is open)
- Test: `repairbroken.rs` `mod tests`; existing `panes.rs` tests (`retry_appears_for_service_origins_only`, ~7717) updated

**Interfaces:**
- Consumes: `layer.diagnose` (Task 1), `layer.repairSource`, `layer.repairFolder` (Task 2), `layer.relinkService` (Task 3), `layer.retry`, `layer.remove`, `arcgis.signIn` (desktop) / `services.arcgis_sign_in` (web).
- Produces: dialog `"repairBroken"` opened by `ui.dialog {"name": "repairBroken", "layer": id}`. Shows layer name, where its data was, the reason, then by kind (spec table): signIn → "This layer is on <server>, which needs you to sign in." + **Sign in to ArcGIS Online** (starts the built-in sign-in for ArcGIS Online with RustGIS's client id; spinner while pending), Retry, Remove; certificate → the spec's certificate sentence + Retry, Remove; serviceMissing/unreachable → "<server> didn't answer" / "the service isn't there any more" + Retry, **Use another address…** (text field → `layer.relinkService`), Remove; fileMissing → "The file isn't at <path>." + **Browse for the file…** (native file picker; then `layer.repairSource`; then, if `siblings` is non-empty, "N other broken layers had data in the same folder. Fix them too?" Yes → `layer.repairFolder {from, to}` and a notice listing repaired and skipped), Remove; other → the reason + Retry, Remove. Success closes with a notice; failure re-runs `layer.diagnose` and shows the new reason.

- [ ] **Step 1: Write the failing tests** (kittest, `RustGisApp::with_sample()` harness as `aprx_tests` / `panes.rs` `add_broken` ~7665):
  - `a_broken_layers_menu_offers_repair_source` — right-click menu of a broken service layer has "Repair Source…" and not "Retry"; a healthy service layer still has "Retry".
  - `the_helper_explains_a_sign_in_layer` — dialog for a broken layer with a Token Required reason shows "needs you to sign in" and a "Sign in to ArcGIS Online" button.
  - `the_helper_explains_a_certificate_failure` — reason "rustls: invalid peer certificate: UnknownIssuer" → shows "isn't trusted on this computer"; no sign-in button.
  - `a_failed_retry_shows_the_new_reason` — Retry on an unreachable layer keeps the dialog open with the unreachable text.
  - `remove_takes_the_layer_out` — Remove → the layer is gone and the dialog closed.
  - `sign_in_finishing_retries_broken_layers_without_the_service_dialog` — the moved completion hook (adapt the existing `arcgisui.rs` tests ~234–310 to call `poll`).
- [ ] **Step 2: Run** `cargo test -p rustgis-ui-egui --lib repairbroken` and `... arcgisui` and `... panes` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS; `cargo test -p rustgis-ui-egui --lib` whole.
- [ ] **Step 5: Commit** — `feat(ui): Repair Source… explains a broken layer and offers the fix that fits`

### Task 7: The plugin hands its token to RustGIS (repo `C:\CODE\RUST GIS\rustgis-arcgis-online`)

**Files:**
- Modify: `src/creds.rs` (`TokenCache` gets `#[serde(default)] org: Option<String>`), `src/agol.rs` (`sign_in` keeps the org id from the `portals/self` body it already fetches, ~173–177, saved into the token cache; `generate_token` keeps an existing cached `org`), `src/commands/mod.rs` (`run` ~194: after a handler that used `real_app`, call `handoff`), `src/commands/sign_in.rs` (after sign-in: handoff, then `layer.retry {"all": true}` when `broken > 0`, and notify "Signed in to ArcGIS Online as <user>; <m> of <n> broken layers opened")
- Create: `src/commands/handoff.rs`
- Modify: `plugin.json`, `Cargo.toml` → version `0.4.0` (minRustgisVersion stays `0.4.0`)
- Test: `src/commands/handoff.rs` tests, `sign_in.rs` tests (queued replies shift — add the new host replies in order)

**Interfaces:**
- Consumes: host `arcgis.adoptToken` (Task 4) → `{"portal","changed","broken"}`; host `layer.retry`.
- Produces: `pub fn handoff(host: &mut Host, app: &App) -> Option<u64>` — when a fresh cached token with an `org` exists, runs `arcgis.adoptToken {portal, token, expires, org, username}` and returns `broken`; on any host error returns None silently (an older RustGIS doesn't know the command). Never called from `browse-nodes` (`light_app`). Sign-in, on None from handoff, notifies once: "This RustGIS can't use the plugin's sign-in for other layers yet; update RustGIS".

- [ ] **Step 1: Write the failing tests:** `handoff_sends_the_cached_token_to_rustgis` (queued `{"ok":{"portal":"…","changed":true,"broken":0}}`; captured output has `arcgis.adoptToken` with the org and expiry); `handoff_is_silent_on_an_older_rustgis` (queued `{"error":"unknown command"}` → None, no notify); `sign_in_retries_broken_layers_after_handing_over` (broken 2 → output has `layer.retry` with `"all":true`); `browse_nodes_never_hands_over` (existing light-app test still asserts no host output).
- [ ] **Step 2: Run** `cargo test --locked` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --locked`, `cargo clippy --locked --all-targets`, `cargo fmt --all --check`, `python scripts/package.py` → all pass.
- [ ] **Step 5: Commit** (in the plugin repo) — `Hand the sign-in to RustGIS so its broken layers open, version 0.4.0`

### Task 8: Docs and the whole gate

**Files:**
- Modify: `docs/user-guide/data.md` (Repair Source…, the folder fix, what each diagnosis means, including the certificate case in generic words), `docs/user-guide/workspace.md` (the working window), `docs/commands` or wherever command docs are generated (`layer.diagnose`, `layer.repairFolder`, `layer.relinkService`, `arcgis.adoptToken` — follow how existing commands are documented; regenerate if generated), plugin repo `README.md` (the handoff and the RustGIS version that carries it)
- Modify: `docs/superpowers/plans/2026-11-01-arcgis-pro-map-import-checklist.md` — the Retry item now reads: sign in with the plugin (0.4.0) or Repair Source… › Sign in; layers on a self-hosted server whose certificate isn't trusted stay broken with the certificate explanation.

- [ ] **Step 1:** Write the docs.
- [ ] **Step 2:** Run `cargo xtask ci` (≈45–70 min; run in the background) → green. Fix anything it finds.
- [ ] **Step 3: Commit** — `docs: Repair Source…, the working window and the plugin sign-in handoff`

## Follow-ons (not in this plan)

- RustGIS's HTTP client trusts its built-in certificate list (webpki-roots), not the Windows store: a server whose certificate comes from a company's own authority (trusted by Windows) would fail in RustGIS. Consider `rustls-platform-verifier`.
- Self-hosted ArcGIS Server that accepts ArcGIS Online logins: build once the server is reachable with a trusted certificate.
- Live progress and Cancel for import/save.
