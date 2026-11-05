# File Geodatabases in the Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A zipped `.gdb` opens in the web app (and on the desktop), and the web app can pick a `.gdb` folder.

**Architecture:** `rustgis_data::filegdb` already reads a geodatabase from a name→bytes closure (`filegdb::read`). A new `filegdb::read_zip` serves that closure from a zip's entries; `io::read_zip_in` hands a zip holding `.gdbtable` entries to it, so every path that reads zips (`layer.addDataBytes`, drops, desktop `layer.addData` on a `.zip`) gets geodatabases for free. The web app's folder picker bundles a chosen folder's files into such a zip in memory.

**Tech Stack:** Rust, `rustgis_data::zip`, `web_sys` (`HtmlInputElement` with `webkitdirectory`), egui.

**Spec:** `docs/superpowers/specs/2026-11-05-enterprise-geodatabase-design.md`, Part 3.

## Global Constraints

- AGENTS.md: no `unwrap`/`expect`/`panic`/indexing outside tests; zips and geodatabases are hostile input (`filegdb::MAX_TABLE_BYTES`, `MAX_TABLES`, `MAX_UPLOAD_BYTES` 256 MiB on wasm / 512 MiB desktop, `io::MAX_BYTES`).
- `apps/rustgis-web` and `crates/ui-egui` build for `wasm32-unknown-unknown`; the folder picker is wasm-only code in `apps/rustgis-web`.
- Privacy rule: test fixtures are synthetic or the repo's existing sample geodatabase (find one under `tests/` or `crates/data/tests`; if none, build the fixture from a tiny `.gdb` the repo already uses in `filegdb::tests`).
- `cargo fmt`, `cargo clippy --workspace --all-targets`, and `cargo check -p rustgis-web --target wasm32-unknown-unknown` clean before each commit.

## Review Focus

1. A zip holding both a `.gdb` folder and Shapefiles: both open, within `MAX_ZIP_LAYERS` (Task 1).
2. A zip with two `.gdb` folders: the first (by name) is read and the other named in a notice, never silently merged (Task 1).
3. A `.gdb` zip whose tables are compressed: the table is named in a notice in the browser (no GDAL), and read through GDAL on the desktop when installed (Task 2).
4. A folder pick that contains no `.gdbtable` (the user picked the parent folder): a clear error naming what was expected (Task 3).
5. A `.gdb` whose `.gdbtable` is larger than the wasm cap: refused with the cap named, not a crash (Task 1).

---

### Task 1: `filegdb::read_zip` and zip detection

**Files:**
- Modify: `crates/data/src/filegdb.rs`, `crates/data/src/io.rs:1770-1800` (`read_zip_in`)
- Test: `crates/data/src/filegdb/tests.rs`, `crates/data/src/io.rs` tests

**Interfaces:**
- Produces:
  ```rust
  /// Whether a zip's entry names hold a File Geodatabase (an `a00000001.gdbtable`, at the root or under one `<name>.gdb/` folder).
  pub fn zip_has_geodatabase(names: &[String]) -> bool;
  /// The geodatabase folders in a zip, by their prefix ("" for the root, "Roads.gdb/"), in name order.
  pub fn zip_geodatabases(names: &[String]) -> Vec<String>;
  /// Reads the first geodatabase in the zip; `name` is the zip's file name (the dataset's source); others are listed in `Read.unread`-style notes by the caller.
  pub fn read_zip(name: &str, bytes: &[u8], layer: Option<&str>) -> Result<Read>;
  ```
- `read_zip` extracts with `crate::zip::read_matching(bytes, |n| n starts with the chosen prefix && extension in {gdbtable, gdbtablx, gdbindexes, spx, freelist, horizon, atx})`, then calls `read(stem, &|file| Ok(entries.get(file).cloned()), layer)` where the lookup is by the name after the prefix, case-insensitive. The whole entry set is bounded by `io::MAX_BYTES`; one entry by `MAX_TABLE_BYTES` (the existing `read` enforces it).
- `io::read_zip_in`: after listing entry names (`crate::zip::read_matching` with a name-only pass, or a cheap `zip::names(bytes)` the builder adds), when `zip_has_geodatabase(&names)`: `filegdb::read_zip` → its `layers` are pushed first into the result (each `Dataset.source = Some(file_name)`), its `unread` become `ReadReport` messages on the first dataset ("<table> not read: <why>"), a second `.gdb` in the zip adds a message "<prefix> not read: one geodatabase per zip"; Shapefiles and other files in the same zip follow as today, under `MAX_ZIP_LAYERS` in total.

- [ ] **Step 1: Write the failing tests**:
  - `filegdb/tests.rs` `a_zipped_geodatabase_reads_like_the_folder`: zip the fixture geodatabase's files under `Fixture.gdb/` with `crate::zip::write`, `read_zip("Fixture.gdb.zip", &zip, None)` → the same layer names and feature counts as `read_dir` on the folder; `layer: Some("<one table>")` → that one.
  - `zip_detection`: names `["Fixture.gdb/a00000001.gdbtable", "readme.txt"]` → `zip_has_geodatabase`, `zip_geodatabases == ["Fixture.gdb/"]`; `["a00000001.gdbtable"]` → `[""]`; `["A.gdb/a00000001.gdbtable", "B.gdb/a00000001.gdbtable"]` → `["A.gdb/", "B.gdb/"]`; `["x/a00000001.gdbtablx"]` → false.
  - `io.rs` `a_zip_with_a_geodatabase_and_a_shapefile_opens_both`: `read_zip` on such a zip → datasets of both; `a_second_geodatabase_is_noted`: two `.gdb` folders → one read and a report message containing "one geodatabase per zip".
  - `an_oversized_table_is_refused`: an entry named `Big.gdb/a00000002.gdbtable` of `MAX_TABLE_BYTES + 1` bytes (stored uncompressed) → `Err` naming the size (the existing `read` error text).
- [ ] **Step 2: Run** `cargo test -p rustgis-data filegdb` and `cargo test -p rustgis-data read_zip` → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** → PASS; `cargo test -p rustgis-data --target wasm32-unknown-unknown` is not run (no runner); `cargo check -p rustgis-data --target wasm32-unknown-unknown` → OK.
- [ ] **Step 5: Commit** `feat(data): read a zipped File Geodatabase (browser and desktop)`.

### Task 2: Engine: a zipped geodatabase adds its layers and tables; GDAL fallback on the desktop

**Files:**
- Modify: `crates/engine/src/cmd/layer.rs:1429+` (`add_layers`: a zip with a geodatabase goes through `read_zip` as today — the data crate did the work — but on the desktop, `unread` tables are tried through GDAL by extracting the zip to the cache folder, as `add_geodatabase` does for a folder; the builder reuses `add_gdal` with the extracted path), `crates/engine/src/cmd/layer.rs` `info.fileLayers` (:1669: for a `.zip` path with a geodatabase, list its tables with `filegdb::layers`-equivalent on the zip: add `filegdb::zip_layers(bytes) -> Result<Vec<String>>` in Task 1's file if `layers(&Path)` can't be reused)
- Test: `crates/engine/src/tests.rs`

- [ ] **Step 1: Write the failing tests**: `a_zipped_geodatabase_adds_layers_and_tables` (`layer.addDataBytes {fileName: "Fixture.gdb.zip", data}` → `layers` non-empty, a non-spatial fixture table in `tables` if the fixture has one; `{layer: "<table>"}` adds that one); `a_zipped_geodatabase_opens_by_path_on_the_desktop` (`layer.addData {path: <zip in temp>}` → the same; `cfg(not(wasm32))`).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `cargo test -p rustgis-engine geodatabase` → PASS.
- [ ] **Step 5: Commit** `feat(engine): zipped File Geodatabases open through Add Data and uploads`.

### Task 3: Web app folder picker for `.gdb` folders

**Files:**
- Modify: `apps/rustgis-web/src/web.rs:996-1062` (`open_async`: purpose `"geodatabaseFolder"`), `crates/ui-egui/src/lib.rs:273` (doc of purposes), `crates/ui-egui/src/ribbon.rs:463` (the "Add Data options" menu gains **File Geodatabase folder…** in the wasm build only, calling the app's `open("geodatabaseFolder")`)
- Create: `apps/rustgis-web/src/folder.rs` (`pub async fn pick_geodatabase_folder() -> Option<Vec<web_sys::File>>`: creates a hidden `<input type="file" webkitdirectory multiple>`, clicks it, awaits `change`, returns the `FileList`; `pub fn bundle(files: Vec<(String, Vec<u8>)>) -> Result<(String, Vec<u8>), String>`: keeps entries whose `webkitRelativePath` extension is one of the geodatabase extensions from Plan task 1, places them under `<folder>.gdb/` by the first path segment, zips them with `rustgis_data::zip::write`, and names the zip `<folder>.gdb.zip`; `Err("the folder picked holds no File Geodatabase (no a00000001.gdbtable): pick the .gdb folder itself")` when no `a00000001.gdbtable` is among them)
- Test: `apps/rustgis-web` unit test of `bundle` (pure; `cfg(test)` compiles natively since it takes `(String, Vec<u8>)`)

- [ ] **Step 1: Write the failing tests** in `folder.rs`: `bundle_keeps_geodatabase_members_under_the_folder` (paths `Roads.gdb/a00000001.gdbtable`, `Roads.gdb/a00000001.gdbtablx`, `Roads.gdb/timestamps`, `Roads.gdb/_gdb.Roads.lock` → zip names `["Roads.gdb/a00000001.gdbtable", "Roads.gdb/a00000001.gdbtablx"]`, zip name `Roads.gdb.zip`); `a_folder_without_a_catalog_is_refused` → `Err` containing "pick the .gdb folder itself"; `the_bundle_respects_the_upload_cap` (total over `MAX_UPLOAD_BYTES` → `Err` naming the cap; reuse `rustgis_engine::cmd::raster::MAX_UPLOAD_BYTES` if `pub`, else make it so).
- [ ] **Step 2: Run** `cargo test -p rustgis-web folder` → FAIL.
- [ ] **Step 3: Implement**; the picked files are read with `crate::parts::read_file`, bundled, and pushed to the inbox as one `(name, bytes)` so `open_bytes` → `layer.addDataBytes` takes it as a zip.
- [ ] **Step 4: Run** the test → PASS; `cargo check -p rustgis-web --target wasm32-unknown-unknown` → OK; `cargo clippy -p rustgis-ui-egui --all-targets` → clean.
- [ ] **Step 5: Commit** `feat(web): pick a File Geodatabase folder in the browser`.

### Task 4: Documentation and browser verification

**Files:**
- Modify: `docs/user-guide/data-formats.md:9-10` (the intro: "folder-based File Geodatabases are desktop-only" → zipped ones open in the web app, and the web app's **File Geodatabase folder…** picks a folder), `:37` (the File Geodatabase row: `.gdb` folder (desktop), `.gdb.zip` / zipped folder (desktop and web); how to add; one geodatabase per zip; no GDAL fallback in the browser), `docs/user-guide/data.md` where File Geodatabases are described (find "File Geodatabase" headings; add the zip and folder picker), `docs/superpowers/plans/2026-11-05-enterprise-geodatabase-checklist.md` (or its own short checklist) for the browser check
- [ ] **Step 1: Write** the docs.
- [ ] **Step 2: Verify in the browser** (the user, or the builder with `trunk serve`/the repo's web run instructions): drop a zipped `.gdb`; pick a `.gdb` folder with File Geodatabase folder…; layers and tables match the desktop; a parent folder pick gives the clear error.
- [ ] **Step 3: Commit** `docs: File Geodatabases in the web app (zip and folder picker)`.
