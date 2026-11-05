# Enterprise Geodatabase (SDE on SQL Server) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A SQL Server connection whose database holds an Esri enterprise geodatabase browses and reads it as Pro's Catalog does, connects from a `.sde` file, and repairs the SDE layers a Pro map import brings in broken.

**Architecture:** The geodatabase catalog (Esri's `GDB_ITEMS`, `SDE_table_registry`, `SDE_layers`, `SDE_spatial_references`) is read into a platform-independent `Catalog` in `rustgis_data::source::geodatabase`; the SQL Server queries live in a child module of the SQL Server reader so they share its private connection. The engine's browse, table listing, `layer.addDatabase` and a new `layer.repairGeodatabase` consult the catalog; the Pro map reader gains a `LayerSource::EnterpriseGeodatabase` variant whose structured origin the engine keeps on the broken layer, matches against saved connections, and repairs through the Repair Source… dialog.

**Tech Stack:** Rust workspace; `tiberius` (SQL Server), `roxmltree` through `rustgis_data::xml`, `serde_json` commands, `egui` dialogs. Desktop only (`cfg(not(target_arch = "wasm32"))`) except pure-text pieces.

**Spec:** `docs/superpowers/specs/2026-11-05-enterprise-geodatabase-design.md` (Parts 1 and 2; Part 3 has its own plan, `2026-11-05-file-geodatabase-in-browser.md`).

## Global Constraints

- AGENTS.md: no `unwrap`/`expect`/`panic`/direct indexing outside tests; `get()` over indexing; every server answer and file is hostile input (bounded lists, checked XML through `rustgis_data::xml::parse`, sizes clamped).
- Everything under `apps/` and `crates/ui-egui` builds for `wasm32`; the SQL Server pieces stay behind `#[cfg(not(target_arch = "wasm32"))]` like `sqlserver.rs`.
- Privacy rule: no company, server, database, user or layer names from the user's environment in code, tests, fixtures, docs or commit messages. Fixtures use made-up names (`gis-db`, `utility`, `Mains`, `sde`).
- Never a password, a `.sde` file's bytes or an unscrubbed connection string in a project, preference, label, log or error (`DataError::redact_secret`, `history::redacted_text`, `scrub_connection`).
- Reads keep the SQL Server reader's rules: `STATEMENT_TIMEOUT`, `PROBE_TIMEOUT`, `LIST_LIMIT`, bounded `TOP`, server-side sampling. Connections are read-only.
- Live tests are gated on `RUSTGIS_TEST_SDE` (an `mssql://` address of a SQL Server geodatabase) and `#[ignore]`d, as `RUSTGIS_TEST_MSSQL` tests are (`crates/engine/src/tests.rs:17478`).
- Windows sign-in never opens a layer without the user acting (no change to `integrated_ok` in `sqlserver.rs:324-336`).
- Run `cargo fmt` and `cargo clippy --workspace --all-targets` clean before each commit; `cargo check -p rustgis-ui-egui --target wasm32-unknown-unknown` for tasks that touch `crates/ui-egui` or `crates/map`.

## Review Focus

1. A geodatabase whose catalog is owned by `dbo` rather than `sde` (Task 3's `detect_owner` test covers both).
2. A versioned class whose `_evw` view is missing: refused with the administrator message, never read from the base table (Task 3).
3. A `.sde` file for another platform or with no server: refused with the platform or "enter the server" message, never a half-filled connection (Task 7).
4. A Pro table name with a database prefix (`GIS.sde.Mains`) and in another case than the catalog's: still matched (Task 10).
5. A saved connection on a differently written host (IP against name): offered as a near match, never used automatically at import or retry (Tasks 9, 12).

---

### Task 1: The `.sde` connection file reader (pure text)

**Files:**
- Create: `crates/data/src/source/sdefile.rs`
- Modify: `crates/data/src/source/mod.rs` (add `pub mod sdefile;`, no cfg)

**Interfaces:**
- Produces:
  ```rust
  pub struct SdeConnection {
      pub platform: String,      // "sqlserver" | "postgresql" | "oracle" | other, lower-case, from DBCLIENT else INSTANCE's "sde:<platform>:" prefix
      pub server: String,        // from SERVER, else DB_CONNECTION_PROPERTIES, else INSTANCE after its "sde:<platform>:" prefix
      pub database: String,      // DATABASE
      pub user: Option<String>,  // USER; None when os_auth
      pub os_auth: bool,         // AUTHENTICATION_MODE=OSA
      pub version: Option<String>, // VERSION
  }
  /// Largest .sde file read.
  pub const MAX_BYTES: usize = 1 << 20;
  pub fn parse(bytes: &[u8]) -> Result<SdeConnection, String>;
  /// The SQL Server address for a connection: mssql://<user>@<host>[:port][/<database>][?instance=<i>]; `host\inst` → ?instance, `host,port` → :port.
  pub fn mssql_address(c: &SdeConnection) -> Result<String, String>;
  ```
- `parse` extracts `KEY=VALUE` pairs found as ASCII and as UTF-16LE in the bytes (scan for each known key, take the value to the next `;`, NUL or end), using `connection_value`-style case-insensitive keys. It never returns `ENCRYPTED_PASSWORD*` or `PASSWORD` values.

- [ ] **Step 1: Write the failing tests** in `sdefile.rs` `#[cfg(test)] mod tests`:
  - `ascii_pairs_parse`: bytes `b"\x00\x01SERVER=gis-db;INSTANCE=sde:sqlserver:gis-db;DBCLIENT=sqlserver;DB_CONNECTION_PROPERTIES=gis-db;DATABASE=utility;USER=editor;ENCRYPTED_PASSWORD=00ff;VERSION=sde.DEFAULT;AUTHENTICATION_MODE=DBMS;\x00"` → `platform == "sqlserver"`, `server == "gis-db"`, `database == "utility"`, `user == Some("editor")`, `!os_auth`, `version == Some("sde.DEFAULT")`.
  - `utf16_pairs_parse`: the same string encoded UTF-16LE with junk bytes before and after → the same result.
  - `os_auth_has_no_user`: `AUTHENTICATION_MODE=OSA` and no `USER` → `os_auth`, `user == None`.
  - `instance_prefix_gives_platform_and_server`: only `INSTANCE=sde:postgresql:pg-host,5432;DATABASE=d` → `platform == "postgresql"`, `server == "pg-host,5432"`.
  - `no_server_is_refused`: `DATABASE=d;USER=u` → `Err` containing "enter the server".
  - `too_large_is_refused`: `MAX_BYTES + 1` bytes → `Err`.
  - `secrets_never_come_out`: the `ascii` input → `format!("{:?}", parsed)` contains neither `00ff` nor `PASSWORD`.
  - `addresses_compose`: `server "gis-db\\GIS"` → `mssql://editor@gis-db/utility?instance=GIS`; `"gis-db,1434"` → `mssql://editor@gis-db:1434/utility`; `os_auth` → `mssql://gis-db/utility`; a `platform` other than `sqlserver` → `Err` naming the platform ("a PostgreSQL geodatabase").
- [ ] **Step 2: Run** `cargo test -p rustgis-data sdefile` → FAIL (module missing).
- [ ] **Step 3: Implement** `sdefile.rs` with the signatures above. Decode UTF-16LE by taking even-indexed bytes where odd bytes are zero in runs (bounded by `MAX_BYTES`), then search both texts. The user is percent-encoded in the address (`DOMAIN\user` → `DOMAIN%5Cuser`, as `msconn` tests expect).
- [ ] **Step 4: Run** `cargo test -p rustgis-data sdefile` → PASS.
- [ ] **Step 5: Commit** `feat(data): read Pro .sde connection files (server, database, user; never the password)`.

### Task 2: The geodatabase catalog model and parser (platform independent)

**Files:**
- Create: `crates/data/src/source/geodatabase.rs`
- Modify: `crates/data/src/source/mod.rs` (add `pub mod geodatabase;`, no cfg)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "camelCase")]
  pub enum ItemKind { FeatureDataset, FeatureClass, Table }
  #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "camelCase")]
  pub enum Versioning { Unversioned, Traditional, Branch }  // serialized "unversioned" | "traditional" | "branch"
  #[derive(Clone, Debug, PartialEq)]
  pub struct Item {
      pub name: String,             // GDB_ITEMS.Name as given ("utility.sde.Mains" or "sde.Mains")
      pub schema: String,           // from PhysicalName "SDE.MAINS" → "SDE"
      pub table: String,            // "MAINS"
      pub kind: ItemKind,
      pub dataset: Option<String>,  // from Path "\Water\Mains" → "Water"; None at the root
      pub alias: Option<String>,    // Definition <AliasName>, when it differs from the short name
      pub shape_column: Option<String>,
      pub field_aliases: Vec<(String, String)>, // (field name, alias) from <GPFieldInfoExs><GPFieldInfoEx><Name>/<AliasName>
      pub versioning: Versioning,   // Traditional when <Versioned>true; Branch when <VersioningType> names branch (or "2"); else Unversioned
      pub archived: bool,           // <IsArchived>true
      pub rowid_column: Option<String>, // SDE_table_registry.rowid_column
      pub srid: Option<i64>,        // SDE_layers.srid
  }
  #[derive(Clone, Debug, Default)]
  pub struct Catalog { pub owner: String, pub release: Option<String>, pub items: Vec<Item>, pub srs: Vec<Srs> }
  #[derive(Clone, Debug, PartialEq)]
  pub struct Srs { pub srid: i64, pub auth_srid: Option<i64>, pub srtext: Option<String> }
  /// Raw rows, as a platform's queries return them (strings; parsing here, so tests need no server).
  pub struct ItemRow { pub name: String, pub physical: String, pub path: String, pub type_name: String, pub definition: Option<String> }
  pub struct RegistryRow { pub owner: String, pub table: String, pub rowid_column: Option<String> }
  pub struct LayerRow { pub owner: String, pub table: String, pub column: String, pub srid: Option<i64> }
  pub const MAX_ITEMS: usize = 10_000;
  pub const MAX_DEFINITION_BYTES: usize = 1 << 20;
  impl Catalog {
      pub fn from_rows(owner: &str, release: Option<String>, items: Vec<ItemRow>, registry: Vec<RegistryRow>, layers: Vec<LayerRow>, srs: Vec<Srs>) -> Catalog;
      /// The item a name picks: "Mains", "sde.Mains", "utility.sde.Mains", "SDE.MAINS" (case-insensitive; a 3-part name loses its first part).
      pub fn find(&self, name: &str) -> Option<&Item>;
      pub fn feature_datasets(&self) -> Vec<&Item>;
      pub fn children(&self, dataset: Option<&str>) -> Vec<&Item>; // classes and tables at the root (None) or in a dataset
      pub fn srs(&self, srid: i64) -> Option<&Srs>;
  }
  /// The geodatabase's own tables, never listed as data: GDB_*, SDE_*, A<n>, D<n>, i<n>, *_evw, *_vw, *_H (case-insensitive).
  pub fn is_machinery(table: &str) -> bool;
  /// How a registered class is read: given whether its shape column is a native spatial column (geometry/geography in INFORMATION_SCHEMA; false for SDEBINARY and other storage), the views that exist in its schema and the columns of its base table.
  pub enum ReadPlan { Base, VersionedView(String), Branch, Refused(String) }
  pub fn read_plan(item: &Item, native_spatial: bool, views: &[String], columns: &[String]) -> ReadPlan;
  /// Branch-versioned Default rows: used by every platform (the builder confirms the predicate against Esri's docs).
  pub const BRANCH_DEFAULT_FILTER: &str = "GDB_BRANCH_ID = 0 AND GDB_IS_DELETE = 0 AND GDB_TO_DATE IS NULL";
  ```
- `read_plan`: `Refused` when `!native_spatial` ("stored in a format RustGIS doesn't read (SDEBINARY or another non-native storage)"); `Branch` when `versioning == Branch` and the columns include `GDB_BRANCH_ID`, `GDB_IS_DELETE`, `GDB_TO_DATE` (case-insensitive), else `Refused` naming the missing columns; `VersionedView(name)` when `Traditional` and `views` holds `<table>_evw` or `<table>_vw` (case-insensitive, the `_evw` first); `Refused("<name> is versioned but its versioned view is missing; ask the geodatabase administrator to enable SQL access for it")` when `Traditional` without one; else `Base`.

- [ ] **Step 1: Write the failing tests** in `geodatabase.rs`:
  - `items_come_with_dataset_alias_and_fields`: rows for a Feature Dataset `\Water`, a Feature Class `utility.sde.Mains` physical `SDE.MAINS` path `\Water\Mains` with a Definition `<DEFeatureClassInfo><AliasName>Water Mains</AliasName><ShapeFieldName>SHAPE</ShapeFieldName><Versioned>true</Versioned><IsArchived>false</IsArchived><GPFieldInfoExs><GPFieldInfoEx><Name>DIAM</Name><AliasName>Diameter</AliasName></GPFieldInfoEx></GPFieldInfoExs></DEFeatureClassInfo>`, a Table `sde.Readings` at `\Readings`; registry `SDE`/`MAINS` rowid `OBJECTID`; layer `SDE`/`MAINS` column `SHAPE` srid 2 → the Mains item has `dataset == Some("Water")`, `alias == Some("Water Mains")`, `field_aliases == [("DIAM","Diameter")]`, `versioning == Traditional`, `rowid_column == Some("OBJECTID")`, `srid == Some(2)`, `shape_column == Some("SHAPE")`; `children(Some("Water"))` is `[Mains]`; `children(None)` is `[Readings]`; `feature_datasets()` is `[Water]`.
  - `names_match_in_every_form`: `find("mains")`, `find("sde.MAINS")`, `find("utility.sde.Mains")`, `find("SDE.MAINS")` all give Mains; `find("water.mains")` is `None`.
  - `a_bad_definition_leaves_the_item_listed`: Definition `"<not xml"` and one of `MAX_DEFINITION_BYTES + 1` bytes → the item is present with `alias == None`; `MAX_ITEMS + 1` item rows → `items.len() == MAX_ITEMS`.
  - `branch_versioning_is_read_from_the_definition`: `<VersioningType>esriVersioningTypeBranch</VersioningType>` (and the numeric `2`) → `Branch`; `<Versioned>false</Versioned>` → `Unversioned`.
  - `machinery_is_hidden`: `GDB_ITEMS`, `SDE_layers`, `A12`, `D12`, `i12`, `MAINS_evw`, `MAINS_vw`, `MAINS_H` → true; `Mains`, `A_Roads`, `Dams`, `History` → false.
  - `read_plans`: Traditional with views `["MAINS_EVW"]` → `VersionedView("MAINS_EVW")`; Traditional without → `Refused` containing "versioned view is missing"; Branch with the three columns → `Branch`; Branch without `GDB_TO_DATE` → `Refused` naming it; `native_spatial: false` → `Refused` containing "SDEBINARY"; plain → `Base`.
- [ ] **Step 2: Run** `cargo test -p rustgis-data geodatabase` → FAIL.
- [ ] **Step 3: Implement** `geodatabase.rs`. Parse Definition through `crate::xml::parse` with default options, every element optional. (Storage isn't read from `SDE_layers.eflags`: whether the shape column is a native spatial column is decided by the platform from `INFORMATION_SCHEMA`, Task 3.)
- [ ] **Step 4: Run** `cargo test -p rustgis-data geodatabase` → PASS.
- [ ] **Step 5: Commit** `feat(data): enterprise geodatabase catalog model, definition parsing and read plans`.

### Task 3: SQL Server catalog queries and geodatabase-aware table opening

**Files:**
- Create: `crates/data/src/source/sqlserver_gdb.rs` (child module: `#[path = "sqlserver_gdb.rs"] mod gdb;` inside `sqlserver.rs`, so it uses `Conn::query`, `at`, `text_of`, `int_of`, `ident`)
- Modify: `crates/data/src/source/sqlserver.rs` (`Conn` gains `geodatabase: OnceLock<Option<Arc<Catalog>>>`; `SqlServerSource` gains `from: String` and `extra_filter: Option<String>`; `open` consults the catalog; `tables`/`find_table` unchanged)
- Test: `crates/data/src/source/sqlserver.rs` live test (`sqlserver_live`, extended) and unit tests of the SQL text builders in `sqlserver_gdb.rs`

**Interfaces:**
- Produces (in `sqlserver.rs`, `pub`):
  ```rust
  /// The geodatabase the connection's database holds, read once per connection; None when it holds none or the catalog isn't readable (the reason is logged, not returned: the database then browses as plain SQL Server).
  pub fn geodatabase(conn: &Conn) -> Option<Arc<geodatabase::Catalog>>;
  /// Why the catalog couldn't be read, when `geodatabase` is None for a database that has GDB_ITEMS (permissions): for the browse notice.
  pub fn geodatabase_problem(conn: &Conn) -> Option<String>;
  pub struct Table { …existing…, pub alias: Option<String> /* new: the class alias */, pub dataset: Option<String> /* new */ }
  ```
- In `gdb`:
  ```rust
  pub(super) fn detect_owner(conn: &Conn) -> Result<Option<String>>; // "sde" then "dbo": SELECT CASE WHEN OBJECT_ID('sde.GDB_ITEMS') IS NOT NULL THEN 'sde' WHEN OBJECT_ID('dbo.GDB_ITEMS') IS NOT NULL THEN 'dbo' END
  pub(super) fn read_catalog(conn: &Conn, owner: &str) -> Result<Catalog>;
  // Queries (TOP (MAX_ITEMS)): items: SELECT i.Name, i.PhysicalName, i.Path, t.Name, CAST(i.Definition AS nvarchar(max)) FROM <o>.GDB_ITEMS i JOIN <o>.GDB_ITEMTYPES t ON t.UUID = i.Type WHERE t.Name IN ('Feature Dataset','Feature Class','Table')
  // registry: SELECT owner, table_name, rowid_column FROM <o>.SDE_table_registry
  // layers: SELECT owner, table_name, spatial_column, srid FROM <o>.SDE_layers
  // srs: SELECT srid, auth_srid, srtext FROM <o>.SDE_spatial_references
  // release: SELECT TOP (1) major, minor, bugfix FROM <o>.SDE_version (joined as "major.minor.bugfix"; absent → None)
  pub(super) struct Reading { pub from: String, pub filter: Option<String>, pub rowid: Option<String>, pub alias: Option<String>, pub field_aliases: Vec<(String, String)>, pub srid_fallback: Option<u32>, pub notes: Vec<String> }
  /// How to read `table` when the catalog registers it; Ok(None) for an unregistered table.
  pub(super) fn reading(conn: &Conn, table: &Table, columns: &[String]) -> Result<Option<Reading>>;
  ```
- `reading` lists the schema's views (`SELECT TABLE_NAME FROM INFORMATION_SCHEMA.VIEWS WHERE TABLE_SCHEMA = @P1`), calls `geodatabase::read_plan` with `native_spatial = table.kind is "geometry" or "geography" and table.column equals the item's shape_column (case-insensitive) or the item has none`, and maps: `Base` → `from = table.sql_from()`; `VersionedView(v)` → `from = [schema].[v]`; `Branch` → base table with `filter = Some(BRANCH_DEFAULT_FILTER)`; `Refused(why)` → `Err(DataError::Unsupported(why))`. `srid_fallback`: when the item's `srid` resolves through `Catalog::srs` to an `auth_srid > 0`, that; else `srtext` parsed by `Crs::parse(...).epsg()`.
- `SqlServerSource::open` changes: after reading the columns and before `projection(...)`, call `gdb::reading`; when `Some`: `key = reading.rowid` (overrides the primary-key lookup when the column exists), `from`/`extra_filter` set, `info.name = alias.unwrap_or(table.label())`, field aliases applied to `fields` by case-insensitive name, `notes` appended, and the SRID probe's `raw_srid == 0` case tries `srid_fallback` before the `crs` parameter. Every SQL that used `self.table.sql_from()` as the FROM target uses `self.from`; `filter()` appends ` AND (<extra_filter>)`. `count_rows`'s `sys.partitions` estimate and `has_index` keep using the base table (`table.sql_from()`), which is what has partitions and the spatial index.

- [ ] **Step 1: Write the failing unit tests** in `sqlserver_gdb.rs`:
  - `catalog_sql_names_the_owner`: the five query builders with owner `dbo` contain `dbo.GDB_ITEMS`, `dbo.SDE_table_registry`, `dbo.SDE_layers`, `dbo.SDE_spatial_references`, `dbo.SDE_version` and `TOP (10000)`; an owner `x]y` is refused by `ident`-style escaping (the owner is only ever `sde` or `dbo`: `detect_owner` returns nothing else, assert with a unit test on the mapping function).
  - `readings_map_plans_to_sql` (pure: a function `reading_of(plan: ReadPlan, table: &Table, item: &Item, srs: Option<&Srs>) -> Result<Reading>` tested without a server): `VersionedView("MAINS_evw")` → `from == "[SDE].[MAINS_evw]"`; `Branch` → `from == "[SDE].[MAINS]"`, `filter == Some(BRANCH_DEFAULT_FILTER)`; `Refused` → `Err(Unsupported)`; an `Srs { auth_srid: Some(2263) }` → `srid_fallback == Some(2263)`; `srtext` of a known WKT without `auth_srid` → the EPSG `Crs::parse` gives.
- [ ] **Step 2: Run** `cargo test -p rustgis-data sqlserver` → FAIL.
- [ ] **Step 3: Implement** `gdb` and the `open` changes. Cache: `Conn.geodatabase` is a `OnceLock<Option<Arc<Catalog>>>` filled on first `geodatabase(conn)`; `geodatabase_problem` is a second `OnceLock<Option<String>>` set when `GDB_ITEMS` exists but a query failed.
- [ ] **Step 4: Run** `cargo test -p rustgis-data sqlserver` → PASS; `RUSTGIS_TEST_SDE=<address> cargo test -p rustgis-data sqlserver_live -- --ignored` opens a registered class (extend `sqlserver_live` to assert `geodatabase(&conn).is_some()` when `RUSTGIS_TEST_SDE` is set and that `SqlServerSource::open` on `catalog.items` first feature class succeeds with `info.name` equal to its alias or name).
- [ ] **Step 5: Commit** `feat(data): SQL Server reads enterprise geodatabase classes through the catalog (versioned views, branch default, aliases)`.

### Task 4: Geodatabase listing for the engine (`sqlserver::geodatabase_tables`) and a non-spatial table reader

**Files:**
- Modify: `crates/data/src/source/sqlserver.rs`, `crates/data/src/source/sqlserver_gdb.rs`

**Interfaces:**
- Produces:
  ```rust
  /// What a geodatabase lists: registered items with the spatial Table (when the class has a geometry column) and the unregistered spatial tables that aren't machinery.
  pub struct GdbListing { pub datasets: Vec<String>, pub classes: Vec<(geodatabase::Item, Option<Table>)>, pub tables: Vec<geodatabase::Item>, pub other: Vec<Table> }
  pub fn geodatabase_tables(conn: &Conn) -> Result<Option<GdbListing>>; // None: not a geodatabase
  /// A registered non-spatial table loaded whole (through its versioned view when versioned), at most `limit` rows, fields with aliases; `Dataset.geometry_type == None`.
  pub fn read_table(conn: &Conn, item: &geodatabase::Item, limit: usize) -> Result<(Dataset, ReadReport)>;
  ```
- `geodatabase_tables` joins `tables(conn)` with the catalog: a class's `Table` is the spatial column named by `shape_column` (else the first spatial column of that schema.table); `other` is every `Table` not registered and with `!is_machinery(name)`.
- `read_table` selects `TOP (limit)` every column through `field_cast`, ordered by `rowid_column` when present; the report notes "the first N rows of M" when the count query (`PROBE_TIMEOUT`) says more.

- [ ] **Step 1: Write the failing test** `listing_groups_items_and_hides_machinery` as a pure test of a function `listing_of(catalog: &Catalog, spatial: Vec<Table>) -> GdbListing` (split from the connection): catalog from Task 2's fixture plus spatial tables `SDE.MAINS/SHAPE`, `SDE.MAINS_evw/SHAPE`, `SDE.A12/SHAPE`, `dbo.Parcels/geom` → `classes == [(Mains, Some(SDE.MAINS))]`, `tables == [Readings]`, `other == [dbo.Parcels]`, `datasets == ["Water"]`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** `listing_of`, `geodatabase_tables`, `read_table`.
- [ ] **Step 4: Run** `cargo test -p rustgis-data sqlserver` → PASS.
- [ ] **Step 5: Commit** `feat(data): geodatabase listing and whole-table reads for SQL Server`.

### Task 5: Engine listing and browse show the geodatabase tree

**Files:**
- Modify: `crates/engine/src/cmd/connection_use.rs:97-125` (`read_tables`: SQL Server branch), `crates/engine/src/cmd/connection_browse.rs:254-315`, `crates/engine/src/cmd/layer.rs:2049-2073` (`database_tables`), `crates/engine/src/cmd/connections.rs:808-857` (`connection.test` result), `crates/engine/src/cmd/connections.rs` command docs for `connection.browse` and `info.databaseTables`
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- `read_tables` (SQL Server): when `sqlserver::geodatabase_tables` gives `Some(listing)`, each class row is `{"label": <physical label as Table::label()>, "schema", "name", "column", "kind": <geometry kind>, "item": "featureClass", "dataset": <dataset or null>, "alias": <alias or null>, "versioning": "none"|"traditional"|"branch", "display": <alias else short name>}`; each registered table `{"label": "<schema>.<table>", "item": "table", "dataset", "alias", "display"}`; `other` rows as today plus `"item": "other"`. A plain database's rows are unchanged (no `item` key).
- `connection.browse` root for a database whose rows carry `item`: children `[{key: "datasets", name: "Feature datasets", what: "folder", children: [{key: "dataset:<b64>", node, name, what: "dataset"}…]}, …classes at the root as "table" children (name: display, detail: "feature class · <versioning>" when versioned)…, {key: "tables", name: "Tables", what: "folder", children: [registered tables, what: "table", detail: "table"]}, {key: "other", name: "Other spatial tables", what: "folder", children: […]}]`, empty folders left out; node `dataset:<b64 name>` lists that dataset's classes. Every class/table child keeps `add: {command: "layer.addDatabase", params: {database, layer: label}}`.
- `info.databaseTables` result gains `"items": [rows as above]` next to `layers` (which keeps listing labels, now of classes and other spatial tables; registered non-spatial tables are listed under a new `"tables": [labels]`).
- `connection.test` (database, SQL Server) result gains `"geodatabase": {"owner": "sde"|"dbo", "release": "11.3.0" | null}` when the listing is a geodatabase; `"geodatabaseProblem": "<why>"` when `sqlserver::geodatabase_problem` has one.

- [ ] **Step 1: Write the failing tests** in `tests.rs` (pure, on the browse tree builder, which Step 3 exposes as `pub(crate) fn geodatabase_tree(rows: &[Value], address: &str, node: Option<&str>) -> Found` in `connection_browse.rs`):
  - `a_geodatabase_browses_by_dataset_class_and_table`: rows (a class `Mains` in `Water`, a class `Valves` at the root, a table `Readings`, an other `dbo.Parcels`) → root children named `["Feature datasets", "Valves", "Tables", "Other spatial tables"]` in that order; the `Valves` child's `add.params.layer == "sde.Valves"`; node `dataset:<b64 "Water">` → `[Mains]` with `detail` containing "traditional".
  - `a_plain_database_still_browses_schemas`: rows without `item` → `schemas()` as today (assert the existing test for schemas still passes).
- [ ] **Step 2: Run** `cargo test -p rustgis-engine geodatabase_browses` → FAIL.
- [ ] **Step 3: Implement** the four modifications; update the command parameter docs (`connection.browse`: "A geodatabase's root lists feature datasets, feature classes, tables and other spatial tables"; `info.databaseTables` → `items`, `tables`).
- [ ] **Step 4: Run** `cargo test -p rustgis-engine connection` and `cargo test -p rustgis-engine database` → PASS.
- [ ] **Step 5: Commit** `feat(engine): browse an enterprise geodatabase by dataset, class and table`.

### Task 6: `layer.addDatabase` opens classes by alias and loads registered tables

**Files:**
- Modify: `crates/engine/src/cmd/layer.rs` (`add_live` ~2260+, `LiveDb::find`, `LiveDb::open`), command doc for `layer.addDatabase` (:48)
- Test: `crates/engine/src/tests.rs` (live, `RUSTGIS_TEST_SDE`)

**Interfaces:**
- `layer.addDatabase {database, layer}` on SQL Server: when `layer` names a registered non-spatial table (`sqlserver::geodatabase(conn)` → `catalog.find(layer)` with `kind == Table`), the result is a standalone table: `sqlserver::read_table(conn, item, sqlserver::MAX_EDITABLE)` → `s.project.add_table(...)` as `add_reads` does for a `GeometryType::None` dataset; result `{"tables": [id], "layers": [], "features": 0}`. A spatial class opens as today (its `info.name` is now the alias, so the layer is named by it unless `name` is given). Without `layer`, a geodatabase adds every class (the registered ones; `other` too), never the non-spatial tables (said in a notice: "N tables not added; add one by name").
- `find_table` on a 3-part Pro name: `LiveDb::find` for SQL Server first tries `catalog.find(name)` and, when found, looks the `Table` up by `schema.table`.

- [ ] **Step 1: Write the failing live test** `sde_live_classes_and_tables` (gated on `RUSTGIS_TEST_SDE`, `#[ignore]`): `info.databaseTables` has `items` with at least one `featureClass`; `layer.addDatabase {layer: <first class's label>}` opens a layer whose name is the class's `display`; `layer.addDatabase {layer: <first table's label>}` gives `tables.len() == 1` and `s.project.tables` grew by one; the project text holds no password (as `sqlserver_live_end_to_end` checks).
- [ ] **Step 2: Run** `RUSTGIS_TEST_SDE=… cargo test -p rustgis-engine sde_live -- --ignored` → FAIL (no `items`/tables).
- [ ] **Step 3: Implement** the `add_live`/`LiveDb` changes and the doc line.
- [ ] **Step 4: Run** the live test → PASS; `cargo test -p rustgis-engine` → PASS.
- [ ] **Step 5: Commit** `feat(engine): add geodatabase classes by alias and registered tables as standalone tables`.

### Task 7: `connection.new` from a `.sde` file; `connection.test` reports the geodatabase

**Files:**
- Modify: `crates/engine/src/cmd/connections.rs` (`new`, command doc), `crates/engine/src/connections.rs` (no model change expected)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- `connection.new {kind: "sqlserver", sdeFile: <path>, name?, group?}` (desktop): reads at most `sdefile::MAX_BYTES` of the file, `sdefile::parse`, `sdefile::mssql_address`, then proceeds as `connection.new {kind, address, name: name or the file stem}`; the result gains `"needsPassword": !os_auth`, `"user": user`, `"version": version`, `"osAuth": os_auth`. Errors: a non-SQL Server platform → "<file>: a <platform> geodatabase; RustGIS reads SQL Server geodatabases"; no server → "<file>: this .sde file isn't one RustGIS can read; enter the server and database instead". The file's bytes are dropped after parsing; nothing of it is logged.
- Also: `connection.new {kind: "sqlserver", sdeData: <base64>}` is **not** offered (the web app has no database connections).

- [ ] **Step 1: Write the failing tests**:
  - `a_connection_comes_from_an_sde_file`: write Task 1's ASCII fixture to a temp `.sde`; `connection.new {kind: "sqlserver", sdeFile}` → `address == "mssql://editor@gis-db/utility"`, `name == <file stem>`, `needsPassword == true`, `version == "sde.DEFAULT"`; `connection.list` shows it with no password anywhere in `serde_json::to_string(&s.connections)`.
  - `an_sde_file_for_another_platform_is_refused`: the postgresql fixture → `Err` containing "PostgreSQL" and no connection added.
- [ ] **Step 2: Run** `cargo test -p rustgis-engine sde_file` → FAIL.
- [ ] **Step 3: Implement**; update the `connection.new` doc: `"sdeFile"?: "an ArcGIS Pro .sde connection file (desktop): its server, database and user make the address; the password is asked for"`.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(engine): new SQL Server connections from Pro .sde files`.

### Task 8: The Pro map reader keeps an SDE layer's structured origin

**Files:**
- Modify: `crates/map/src/source.rs` (new `GeodatabaseOrigin`, `SourceLayer.geodatabase`), `crates/map/src/cim/map.rs:110-130` (`LayerSource::EnterpriseGeodatabase`), `:637-660` (`workspace`), `:498-511` (SQL query stays `Unopenable`), `:701-709` (`sde_place` reused)
- Test: `crates/map/src/cim/tests.rs` (the existing `every_connection_kind_has_a_source_and_an_origin` and `origins_never_carry_passwords` tests, extended)

**Interfaces:**
- Produces (in `rustgis_map::source`):
  ```rust
  #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
  pub struct GeodatabaseOrigin {
      pub platform: String,            // "sqlserver" | "postgresql" | "oracle" | "" (unknown): from DBCLIENT, else INSTANCE "sde:<p>:"
      pub server: String,              // SERVER, else DB_CONNECTION_PROPERTIES, else INSTANCE without its prefix
      pub database: String,            // DATABASE
      pub table: String,               // the connection's dataset ("utility.sde.Mains")
      #[serde(default)] pub version: Option<String>, // VERSION
  }
  // SourceLayer: #[serde(default, skip_serializing_if = "Option::is_none")] pub geodatabase: Option<GeodatabaseOrigin>  (cleared by now_open like origin)
  impl GeodatabaseOrigin { pub fn is_default_version(&self) -> bool /* None, "", or ends with ".DEFAULT" ignoring case */ }
  ```
- `LayerSource::EnterpriseGeodatabase(GeodatabaseOrigin)` replaces `Unopenable` for `CIMStandardDataConnection`/`CIMFeatureDatasetDataConnection` with factory `SDE` (the `report.skipped(... "SDE workspaces, which RustGIS can't open")` note goes: the engine decides whether it opens). `origin` text stays `"{sde_place}, table {dataset}"`. The reader still never puts `USER` or passwords anywhere in the struct.

- [ ] **Step 1: Extend the failing tests** in `cim/tests.rs`: the SDE row of `every_connection_kind_has_a_source_and_an_origin` expects `LayerSource::EnterpriseGeodatabase(GeodatabaseOrigin { platform: "sqlserver", server: "gis-db", database: "utility", table: "<dataset>", version: Some("sde.DEFAULT") })` for the string `ENCRYPTED_PASSWORD=00ff;SERVER=gis-db;INSTANCE=sde:sqlserver:gis-db;DBCLIENT=sqlserver;DATABASE=utility;USER=editor;VERSION=sde.DEFAULT`; `origins_never_carry_passwords` also asserts `format!("{:?}", source)` has no `editor`, `00ff`, `PASSWORD`; a new `an_sde_connection_without_dbclient_takes_the_instance_platform` (`INSTANCE=sde:postgresql:pg;DATABASE=d` → `platform == "postgresql"`, `server == "pg"`).
- [ ] **Step 2: Run** `cargo test -p rustgis-map cim` → FAIL.
- [ ] **Step 3: Implement**. `SourceLayer::broken` keeps its signature; callers set `geodatabase` afterwards.
- [ ] **Step 4: Run** `cargo test -p rustgis-map` → PASS; `cargo check -p rustgis-ui-egui --target wasm32-unknown-unknown` → OK.
- [ ] **Step 5: Commit** `feat(map): Pro import keeps an SDE layer's server, database, table and version (never its login)`.

### Task 9: Matching a geodatabase origin to saved connections

**Files:**
- Create: `crates/engine/src/geodatabase.rs` (pure text; `pub mod geodatabase;` in `crates/engine/src/lib.rs`)
- Modify: `crates/engine/src/connections.rs` (two methods on `Connections`)
- Test: in `geodatabase.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct ServerKey { pub host: String /* lower-case */, pub port: Option<u16>, pub instance: Option<String> /* lower-case */ }
  /// Pro's server text: "host", "host\inst", "host,port", "host:port", "sde:sqlserver:host\inst"; a port absent and no instance → DEFAULT_PORT 1433.
  pub fn server_key(server: &str) -> ServerKey;
  /// A saved mssql:// address's server and lower-case database (None for other schemes).
  pub fn address_key(address: &str) -> Option<(ServerKey, String)>;
  pub enum Match { Exact, Near /* same database name, different host */, No }
  pub fn matches(origin: &rustgis_map::source::GeodatabaseOrigin, address: &str) -> Match;
  // Connections:
  pub fn matching_geodatabase(&self, g: &GeodatabaseOrigin) -> Vec<&Connection>; // kind sqlserver, Match::Exact, in list order
  pub fn near_geodatabase(&self, g: &GeodatabaseOrigin) -> Vec<&Connection>;     // Match::Near
  ```
- `matches` is `No` when `g.platform` isn't `sqlserver` or empty. The user part of the address is ignored.

- [ ] **Step 1: Write the failing tests** in `geodatabase.rs`:
  - `server_keys_normalise`: `"GIS-DB\\Inst"` → `{host: "gis-db", port: None, instance: Some("inst")}`; `"gis-db,1434"` → port 1434; `"gis-db"` → port Some(1433); `"sde:sqlserver:gis-db"` → host gis-db, port 1433.
  - `addresses_match_pro_servers`: origin `{server: "GIS-DB", database: "Utility"}` vs `mssql://sde@gis-db:1433/utility` → `Exact`; vs `mssql://other@gis-db/utility?instance=x` → `No` (instance differs); vs `mssql://sde@10.0.0.5/utility` → `Near`; vs `mssql://sde@gis-db/other` → `No`; `postgres:host=gis-db dbname=utility` → `No`; origin platform `postgresql` → `No` for every mssql address.
  - `connections_are_matched_in_order`: two exact and one near saved connection → `matching_geodatabase` has the two in list order, `near_geodatabase` the one.
- [ ] **Step 2: Run** `cargo test -p rustgis-engine geodatabase::` → FAIL.
- [ ] **Step 3: Implement** (reuse `rustgis_data::source::msconn::split_password` and the `config`-style host/port split; don't call `sqlserver::config`, which is private and desktop-only).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(engine): match an imported SDE layer to saved SQL Server connections by server and database`.

### Task 10: The Pro import opens SDE layers through an exact, remembered connection, else keeps them broken with their origin

**Files:**
- Modify: `crates/engine/src/cmd/arcgis_pro.rs` (`Origin` gains `geodatabase: Option<GeodatabaseOrigin>`; `add_layer` handles `LayerSource::EnterpriseGeodatabase`; `broken_layer` sets `src.geodatabase`; `add_tables` notes an SDE table "a table in an enterprise geodatabase, not added; add it from Data Sources"), `crates/engine/src/cmd/connections.rs` (`credential` and `activate` become `pub(super)`)
- Create: `crates/engine/src/cmd/geodatabase_open.rs` (`#[path]` child of `cmd/mod.rs`, desktop)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- Produces (in `geodatabase_open.rs`):
  ```rust
  /// The one saved connection an origin may open through without asking: exactly one Match::Exact, `remembered`, not rejected, and not a Windows sign-in (no user in its address). None otherwise.
  pub(super) fn quiet_connection(s: &Session, g: &GeodatabaseOrigin) -> Option<Connection>;
  /// The `layer` name `layer.addDatabase` gets for a Pro dataset: a 3-part name loses its first part ("utility.sde.Mains" → "sde.Mains").
  pub(super) fn table_name(dataset: &str) -> String;
  /// Opens `layer.addDatabase {database: c.address, layer: table_name(g.table), name}` after activating the connection's credential (connections::credential + activate), as a new layer: the caller moves or keeps it.
  pub(super) fn add_through(s: &mut Session, c: &Connection, g: &GeodatabaseOrigin, name: &str) -> Result<Value, String>;
  ```
- `add_layer`: `EnterpriseGeodatabase(g)` → when `quiet_connection` is `Some(c)`: `(add_through(s, &c, g, name), Origin { geodatabase: Some(g.clone()), ..Default })`; a layer opened at a non-default `version` gets a note `report.approximated(name, "read at the Default version, not <version>")`. When `None`: `broken_layer` with `Origin { geodatabase: Some(g) }` and error `"in an enterprise geodatabase: connect to it with Repair Source…"`; `skipped_for_broken` as the Unopenable path does.
- `broken_layer` sets `src.geodatabase = from.geodatabase` before `add_streamed`.

- [ ] **Step 1: Write the failing tests** in `tests.rs`:
  - `an_sde_layer_comes_in_broken_with_its_geodatabase_origin`: the existing synthetic project fixture (`:26534`) → the SDE layer's `source.geodatabase == Some(GeodatabaseOrigin { server: "gis-db", database: "utility", table: "<fixture dataset>", platform: "sqlserver", .. })`, `reason` contains "Repair Source", `origin` contains "gis-db" and no "swordfish"/"editor"; saving and reopening the project keeps `geodatabase`.
  - `table_names_lose_the_database_prefix`: `table_name("utility.sde.Mains") == "sde.Mains"`, `table_name("sde.Mains") == "sde.Mains"`, `table_name("Mains") == "Mains"`.
  - `a_quiet_connection_needs_an_exact_remembered_sql_login`: connections `mssql://sde@gis-db/utility` remembered → `Some`; the same not remembered → `None`; `mssql://gis-db/utility` (Windows) remembered → `None`; two exact remembered → `None`.
- [ ] **Step 2: Run** `cargo test -p rustgis-engine sde_layer` → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `cargo test -p rustgis-engine` → PASS (the Pro import tests at `:26161-26587` updated for the new reason text).
- [ ] **Step 5: Commit** `feat(engine): Pro import opens SDE layers through a remembered connection, else keeps them repairable`.

### Task 11: `layer.diagnose` knows geodatabase layers; `layer.retry` tries the quiet connection

**Files:**
- Modify: `crates/engine/src/cmd/broken.rs` (`Kind::Geodatabase`, `diagnose`), `crates/engine/src/cmd/retry.rs` (`retry_layer`)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- `layer.diagnose` for a broken layer with `source.geodatabase`: `kind: "geodatabase"`, `server`, `database`, `table`, `platform`, `version`, `connections: [{id, name, exact: true|false}]` (exact matches first, then near), `siblings: [ids of the map's other broken layers whose geodatabase origin is Match::Exact against the same server+database]`. Doc line updated.
- `layer.retry` on such a layer: `quiet_connection` → `add_through` then move the new layer's source into the broken one as `retry_service` does (`Outcome::Opened`); else `Err("needs a connection to the geodatabase on <server>: use Repair Source…")`.

- [ ] **Step 1: Write the failing tests**: `diagnosing_an_sde_layer_names_its_connections` (fixture import with one exact and one near connection saved → `kind == "geodatabase"`, `connections` has both with `exact` true/false in that order, `siblings` holds the other SDE layer of the fixture when the fixture has two, else is empty); `retrying_an_sde_layer_without_a_connection_says_so` (`layer.retry` → `failed[0].error` contains "Repair Source").
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `cargo test -p rustgis-engine diagnos` and `retry` → PASS.
- [ ] **Step 5: Commit** `feat(engine): diagnose and retry imported SDE layers`.

### Task 12: `layer.repairGeodatabase`

**Files:**
- Create: `crates/engine/src/cmd/repair_geodatabase.rs` (registered in `cmd/mod.rs` next to `repair::commands()`)
- Modify: `crates/engine/src/cmd/repair.rs` (extract the fit check into `pub(super) fn fits(l: &rustgis_map::Layer, info: &rustgis_data::source::SourceInfo, what: &str) -> Result<(), String>` covering geometry type, required fields and the renderer expression, used by `repair_source` unchanged)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- `layer.repairGeodatabase {layer?: name|id, connection: id|name, all?: true}` → `{"repaired": [names], "skipped": [{"layer", "why"}]}`. Steps per layer (the one named, or with `all` every broken layer whose origin is `Match::Exact` against the connection's address):
  1. the layer must be broken with a `geodatabase` origin, else skipped "isn't an enterprise geodatabase layer";
  2. `add_through(s, &c, g, &l.name)` (which signs in through the connection; a login failure is the command's error, once, before any layer);
  3. `fits(l, info, "<connection name>")` on the new layer's source info; a misfit removes the added layer and skips with the reason;
  4. move the new layer's `source` and `dataset` into the broken layer (`replace_source`), remove the added layer, restore `camera`/`active_layer` as `retry_service` does, `check_query`;
  5. a non-default `version` → notice "<name>: read at the Default version, not <version>".
  An error for the named layer alone is the command's error; with `all`, none repaired is an error listing the reasons (no empty undo step), as `layer.repairFolder` does.

- [ ] **Step 1: Write the failing tests**: `repairing_needs_a_connection_and_a_geodatabase_layer` (a broken file layer → `Err` containing "isn't an enterprise geodatabase layer"; an unknown connection → `Err` "no connection"); `the_fit_check_is_shared` (unit test of `fits` with a layer whose renderer uses field `DIAM` against an info without it → `Err` naming `DIAM`); the live `sde_live_repair` (`RUSTGIS_TEST_SDE` plus `RUSTGIS_TEST_SDE_TABLE` naming a class): import a synthetic Pro map whose SDE layer names that server/database/table (built in the test from the env address with `geodatabase::address_key`), save a connection through `connection.new` and log in with `connection.login {id, password}` (session password, not remembered), then `layer.repairGeodatabase {layer, connection}` → the layer's source `is_open()`, `geodatabase == None`, its style kept (assert a renderer colour set before the repair).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `cargo test -p rustgis-engine repair` → PASS; the live test with the env vars → PASS.
- [ ] **Step 5: Commit** `feat(engine): layer.repairGeodatabase connects once and repairs every SDE layer of a geodatabase`.

### Task 13: Repair Source… dialog: the geodatabase diagnosis

**Files:**
- Modify: `crates/ui-egui/src/repairbroken.rs` (`Kind::Geodatabase`, `Diagnosis` gains `connections: Vec<(String, String, bool)>`, `siblings` reused, `Action::ConnectAndRepair(String)`, `Action::AddConnection`), `crates/ui-egui/src/connectionui.rs` (a `continue_repair: Option<u32>` on `State`: after a successful save + login, run `layer.repairGeodatabase {layer, connection}` and reopen the Repair dialog's notice flow)
- Test: `crates/ui-egui` builds for desktop and wasm; behaviour verified on the desktop (Task 16)

**Interfaces (dialog copy, from the spec):**
- Exact connection(s): "This layer is in an enterprise geodatabase on {server}, database {database} (table {table}). You have a connection to it: {name}." Buttons: **Connect and repair** (one exact: that; several: a combo of them, `picker_combo` with the exact ones), **Remove**.
- Near only: "… {name} is a connection to a database called {database} on {host}." Buttons: **Use {name}**, **Add connection…**, **Remove**.
- None: "This layer is in an enterprise geodatabase on {server}, database {database}. RustGIS needs a connection to it." Buttons: **Add connection…** (opens `ui.connection {kind: "sqlserver", form: true, server, database}`, Task 14, with `continue_repair = Some(layer)`), **Remove**.
- Another platform: "…a {platform} geodatabase, which RustGIS can't connect to yet." Button: **Remove**.
- After a repair with `siblings > 0`: "N other broken layers are in the same geodatabase. Repair them too?" Yes → `layer.repairGeodatabase {connection, all: true}` behind `busy::queue_short`, notices as `repair_folder` lists them.
- Password: `layer.repairGeodatabase` fails with a login error when the connection has no usable password; the dialog then shows the error and a **Log in…** button that runs `connectionui::log_in(app, &id)` with `continue_repair` set.

- [ ] **Step 1: Implement** the dialog changes (no unit test: egui code; the engine commands it calls are tested in Tasks 11–12).
- [ ] **Step 2: Run** `cargo clippy -p rustgis-ui-egui --all-targets` and `cargo check -p rustgis-ui-egui --target wasm32-unknown-unknown` → clean (the geodatabase branch is desktop-only behind `cfg`, with the wasm build showing the "Remove" action and a "Database connections are used by the desktop app" line).
- [ ] **Step 3: Commit** `feat(ui): Repair Source… connects to an enterprise geodatabase and repairs its layers`.

### Task 14: Connection dialog: Enterprise Geodatabase entry, form mode and the `.sde` button

**Files:**
- Modify: `crates/ui-egui/src/connectionui.rs` (`State` gains `form: bool, platform: String, server: String, login: enum-as-string "sql"|"windows", user: String, database: String, continue_repair: Option<u32>`; `definition()` composes the address from the form when `form`; `dialog()` draws the form; a **From .sde file…** button (desktop) runs `dialogs::pick_open(&["sde"], …)` then `connection.new {kind: "sqlserver", sdeFile}` and fills the form from the result; `summary()` shows "Enterprise geodatabase {release}" from `connection.test`'s `geodatabase`), `crates/ui-egui/src/sourcespane.rs:249-258` (New Connection menu gains **Enterprise Geodatabase…** after SQL Server: `ui.connection {kind: "sqlserver", form: true}`), `crates/ui-egui/src/lib.rs:2729` (`ui.connection` takes `form`, `server`, `database`), `crates/ui-egui/src/browser.rs` or wherever double-click on files is routed (a `.sde` file opens the connection dialog from it; find the `.pagx`/`.aprx` routing and add `sde`)
- Test: `crates/ui-egui/src/connectionui.rs` unit test of `definition()`

**Interfaces:**
- Form fields in order: Platform (combo: SQL Server; disabled with one entry), Server, Sign in (Database user / Windows), User, Password, Remember, Database, Name, Group; a link "Enter an address instead" toggles `form` off and keeps whatever the form composed. `definition()` with `form`: `sdefile::mssql_address`-style composition (reuse `rustgis_data::source::sdefile::mssql_address` by building an `SdeConnection` from the form).
- Default connection name when the form is used and `name` is blank: "Geodatabase on {server}".

- [ ] **Step 1: Write the failing test** `the_form_composes_the_address`: `State { form: true, server: "gis-db\\GIS", user: "sde", database: "utility", login: "sql", .. }` → `definition(&st)["address"] == "mssql://sde@gis-db/utility?instance=GIS"`, `name == "Geodatabase on gis-db\\GIS"`; `login: "windows"` → `mssql://gis-db/utility`.
- [ ] **Step 2: Run** `cargo test -p rustgis-ui-egui composes_the_address` → FAIL.
- [ ] **Step 3: Implement** the UI pieces.
- [ ] **Step 4: Run** the test → PASS; clippy and the wasm check → clean.
- [ ] **Step 5: Commit** `feat(ui): Enterprise Geodatabase connections by form or from a .sde file`.

### Task 15: Connection tree shows datasets and the assistant skill learns the listing

**Files:**
- Modify: `crates/ui-egui/src/connectiontree.rs:298-340` (`what` values `folder` and `dataset` render as expandable nodes with the "folder"/"feature dataset" label; `table` rows show `detail`), `crates/assistant/skills/add-data/SKILL.md` (a paragraph: `info.databaseTables` on a SQL Server geodatabase returns `items` with `item`, `dataset`, `alias`, `versioning`; add a class with `layer.addDatabase {database, layer}`; a registered table becomes a standalone table)
- [ ] **Step 1: Implement** both.
- [ ] **Step 2: Run** `cargo clippy -p rustgis-ui-egui --all-targets` → clean; `cargo test -p rustgis-assistant` → PASS (skills are checked by its tests if any).
- [ ] **Step 3: Commit** `feat(ui): geodatabase datasets in the connection tree; assistant add-data skill`.

### Task 16: Documentation

**Files:**
- Modify: `docs/user-guide/data.md` (new "### Enterprise geodatabases (SDE)" under "## SQL Server and Azure SQL" at :384: connecting by form, from a `.sde` file (password asked, never read from the file), what the tree shows, versioning (Default; versioned views; branch), registered tables load whole with the row cap, what isn't read (other versions, SDEBINARY, PostgreSQL/Oracle, editing); "### Named connections" at :588 mentions the Enterprise Geodatabase entry), `docs/user-guide/maps.md:372` ("### Repair Source…": the geodatabase diagnosis, Connect and repair, near matches, "Repair them too?"), `docs/user-guide/workspace.md:353` (the SDE row: opens through a remembered exact connection, else broken with Repair Source…), `docs/user-guide/data-formats.md:400` (one line pointing to the new section), `docs/comparison/index.md:73` (databases cell: "SQL Server enterprise geodatabases (read)")
- [ ] **Step 1: Write** the sections; no server names, no company names.
- [ ] **Step 2: Run** the docs build if the repo has one (`zensical.toml` → `zensical build` or as `CONTRIBUTING.md` says) → clean.
- [ ] **Step 3: Commit** `docs: enterprise geodatabases (SDE) as a data source and in Repair Source…`.

### Task 17: Desktop verification (the user's machine)

Not automatable here; the checklist the spec gives, run with the user and recorded in a short `docs/superpowers/plans/2026-11-05-enterprise-geodatabase-checklist.md` as the Pro import was:

- [ ] New Connection › Enterprise Geodatabase: form from the Pro dialog's values; Validate says geodatabase and release; save; the tree shows datasets, classes with aliases, tables, no system tables. Then the same from one of the user's `.sde` files (asked for the password).
- [ ] Add an unversioned class, a traditionally versioned one and (if present) a branch versioned one; counts and a recently edited feature match Pro.
- [ ] Import the utility Pro map with no connection saved: SDE layers broken; Repair Source… → Add connection… → the layer and the other seven repaired, styles kept. Re-import the map: the SDE layers open on their own.
- [ ] A plain SQL Server database browses and reads as before.
- [ ] Record what was seen; anything off becomes a follow-up task on this plan.

## Self-review notes

- Spec coverage: connecting (T1, T7, T14); catalog and browsing (T2–T5, T15); reading classes incl. versioning, aliases, SRID fallback, SDEBINARY refusal (T3); registered tables (T4, T6); `connection.test`/`info.databaseTables` (T5); Pro origin and auto-open (T8, T10); matching incl. near matches and Windows rule (T9, T10); diagnose/retry (T11); repair command and "repair them too" (T12, T13); dialog copy (T13); docs (T16); verification (T17). The PostgreSQL seam is the `Catalog`/`ReadPlan` split in T2 (platform-free) with SQL in T3's child module.
- Type consistency: `GeodatabaseOrigin` (T8) is what T9–T13 take; `Catalog::find` (T2) is what T6 and T10's `table_name` rely on; `quiet_connection`/`add_through` (T10) are what T11 and T12 call; `fits` (T12) is extracted from `repair.rs`.
