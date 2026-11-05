# Enterprise geodatabases (SDE) as a data source, and File Geodatabases in the browser

Design, 2026-11-05. Brainstormed with the user who asked for it; decisions marked **(user)** are
theirs, decisions marked **(call)** were made by the designer and can be overruled.

## Intent

The user's company keeps its production data in Esri enterprise geodatabases ("SDE") on SQL
Server, reached from ArcGIS Pro through several saved connections (database sign-in with a user
and password; one database per connection). Two things stop that data reaching RustGIS today:

1. A Pro map imported into RustGIS brings its SDE layers in **broken**: the import records where
   Pro read each one (server, database, table) but RustGIS has no way to open it, and the Repair
   Source… helper only repairs files and services.
2. Data Sources has SQL Server connections, but they show a geodatabase as raw tables: system
   tables, delta tables and versioned views mixed in with the data, no feature datasets, no
   aliases, and a versioned feature class read from its base table shows stale rows.

The user also wants File Geodatabases (`.gdb`) to open in the **web app**, where today they are
desktop-only because a `.gdb` is a folder and the web app takes file uploads.

**(user)** "Make it easy to fix those broken layers, and easy to add SDE files into RustGIS."
**(user)** SQL Server is the only platform in use here; other users may have other platforms
later, so the design leaves room for PostgreSQL (RustGIS already reads PostGIS) without building
it now.

This design is three parts. Parts 1 and 2 are one piece of work with two entry points; Part 3 is
independent and can ship first or last.

1. **Enterprise geodatabase in Data Sources** (desktop): connect with the same four things Pro's
   dialog asks for, or from a `.sde` file, and browse the geodatabase as Pro's Catalog shows it.
2. **Repairing imported SDE layers**: Repair Source… on one broken SDE layer connects once and
   fixes every broken layer on that geodatabase.
3. **File Geodatabases in the browser**: a zipped `.gdb`, or a picked `.gdb` folder, opens in the
   web app with the reader RustGIS already has.

## Goals

- From Data Sources, New Connection › Enterprise Geodatabase asks for server, database, user and
  password (Windows sign-in too), or takes a Pro `.sde` connection file, and saves a named
  connection the way SQL Server connections are saved today, password in the OS keychain.
- Browsing that connection shows **feature datasets, feature classes and tables by their
  geodatabase names**, with aliases, and nothing of the geodatabase's machinery: no `sde`/`dbo`
  system tables, delta tables, versioned views or archive tables.
- A feature class added as a layer shows the **current rows of the Default version**, whether it
  is unversioned, traditionally versioned or branch versioned, with the field aliases Pro shows.
- Repair Source… on an imported SDE layer explains what it is, offers to connect (an existing named
  connection to that server and database, or a new one), and after one connection repairs **every
  broken layer on the same server and database** that it can find, keeping each layer's style.
- The next Pro map imported from the same geodatabase opens its SDE layers **without asking**,
  when a named connection to that server and database exists and its password is remembered.
- In the web app, dropping or picking a zipped `.gdb` adds its feature classes as layers and its
  tables as standalone tables, exactly as the desktop does for the folder. Picking the `.gdb`
  folder itself works in browsers that offer a folder picker.
- Everything is reachable as commands, as the rest of RustGIS is.

## Non-goals

- **Editing** an enterprise geodatabase. Connections are read-only, as the SQL Server reader is
  today. Versioned editing, branch editing, archiving and attribute rules are later projects.
- Reading a **named version** other than Default. Pro's `VERSION=` in a connection string is
  recorded; a layer imported from another version is read at Default with a note saying so.
- **PostgreSQL and Oracle** geodatabases. The catalog layer is written so PostgreSQL's queries
  can be dropped in (same system tables, different schema and geometry calls); Oracle has no
  reader in RustGIS at all. Both are noted follow-ons, not built.
- Esri's older **SDEBINARY** storage on SQL Server (`F<id>`/`S<id>` tables). A class stored that
  way is listed and refuses to open with a notice naming the storage. Native `geometry` and
  `geography` columns, what every supported SQL Server geodatabase uses, are what is read.
- Decrypting the password inside a `.sde` file. Esri's scheme is proprietary; RustGIS reads the
  server, database, user and version from the file and asks for the password once.
- Relationship classes, topologies, networks, attachments, subtypes and domains as anything other
  than fields. Subtype and domain **codes are read as stored**; showing their descriptions is a
  noted follow-on (the catalog already has the data).
- Folder drag-and-drop in the web app. egui's drop event hands over files, not directory
  entries; a folder picker covers the need.

## Anti-goals

- A geodatabase connection that lists the same data twice (base table and versioned view, or a
  class under its dataset and again at the top level).
- A versioned class that silently shows the base table (stale and missing rows). If the versioned
  view or branch columns can't be read, the class is **refused with the reason**, not read wrong.
- Writing a password, a `.sde` file's contents, or a connection string with secrets into the
  project, the preferences, the log or an error message. Origins and labels carry server, database
  and table only, as the Pro import does today.
- Putting the company's server addresses, database names, user names or geodatabase contents into
  code, tests, fixtures, docs or commit messages (the repo's privacy rule). Tests use made-up names.
- Breaking the plain SQL Server path: a database that is not a geodatabase browses and reads
  exactly as it does today.
- A fork change that makes upstream merges harder than needed: new code in new files; shared
  files touched in small, well-marked places.

## Constraints

- AGENTS.md: no `unwrap`/`expect`/`panic`/direct indexing outside tests; every server answer is
  hostile input (bounded lists, checked XML, sizes clamped); crates under `apps/` build for
  `wasm32` (Part 3 must compile there; Parts 1 and 2 stay `cfg(not(wasm32))` like the SQL Server
  reader).
- The SQL Server reader's rules carry over: verified TLS by default, statement and probe
  timeouts, bounded reads, server-side sampling, `LIST_LIMIT` on listings.
- A live geodatabase can't be in CI. Engine tests that need one are gated on an environment
  variable (`RUSTGIS_TEST_SDE`, naming a connection to a SQL Server geodatabase), as
  `RUSTGIS_TEST_MSSQL` gates the SQL Server tests today. Everything that doesn't need a server
  (catalog parsing from recorded rows, `.sde` parsing, connection-string handling, the repair
  matching, Part 3 entirely) is tested without one.
- Desktop verification on the user's machine against a real geodatabase closes the work, as the
  Pro import projects were closed.

## Part 1: Enterprise geodatabase in Data Sources

### Connecting

- **(call)** No new connection kind. A geodatabase connection **is** a `sqlserver` connection
  (`mssql://user@host:port/database?…`), so everything that exists for SQL Server connections
  (keychain, `connection.*` commands, favourites, `layer.addDatabase`) works unchanged, and a
  PostgreSQL geodatabase later is a `postgres` connection the same way. What's new is how a
  connection whose database holds a geodatabase is **browsed and read**.
- The New Connection menu gains **Enterprise Geodatabase…**. It opens the existing connection
  dialog in a **form mode**: Platform (SQL Server now; the control exists so PostgreSQL can be
  added), Server (host, or `host\instance`, or `host,port`), Sign in (Database user / Windows),
  User, Password, Remember, Database, Name. The form composes the `mssql://` address; the plain
  address field stays available under "Address" for people who have one. **(call)** The same form
  is offered for the plain SQL Server kind, since it is the same address; the geodatabase entry
  just pre-selects it and names the connection "Geodatabase on <server>".
- **From a `.sde` file**: a button in the form, a drop of a `.sde` file on the window, double-click
  in the Browser, or `connection.new {kind: "sqlserver", sdeFile: <path>}`. RustGIS reads the
  file and fills Server, Database, User, sign-in type and Pro's `VERSION`, then asks for the
  password (unless Windows sign-in). The file's name becomes the connection name. The file is
  read once and not kept; the connection is saved as any other.
  - `.sde` files are Esri's own format and undocumented. From inspection they carry the same
    `KEY=VALUE;` connection properties Pro writes into a `.aprx` (`SERVER`, `INSTANCE`,
    `DBCLIENT`, `DB_CONNECTION_PROPERTIES`, `DATABASE`, `USER`, `AUTHENTICATION_MODE`,
    `VERSION`, `ENCRYPTED_PASSWORD…`) as text inside a binary container. **Left to the builder**
    to confirm against one of the user's files (which must not be committed): the reader looks for
    those keys as ASCII and as UTF-16LE, takes each value up to the next `;` or end of string,
    and reuses the Pro import's `connection_value`/`scrub_connection` for the lookup and the
    scrub. A file that yields no `SERVER`/`INSTANCE` and `DATABASE` is refused: "this `.sde` file
    isn't one RustGIS can read; enter the server and database instead", with the form open.
  - `INSTANCE=sde:sqlserver:<host>` and `DBCLIENT=sqlserver` name the platform; a file for
    another platform is refused with the platform named ("a PostgreSQL geodatabase; RustGIS reads
    SQL Server geodatabases").
  - `AUTHENTICATION_MODE=OSA` means Windows sign-in (no user, no password).
- **Windows sign-in keeps today's rule (call).** The SQL Server reader only sends the Windows
  identity to a server added in this session, and a saved connection has no passwordless login.
  That stays: a Windows sign-in connection connects from the form or the tree each session, and is
  never used to open a layer without the user (no auto-open at import or on project reopen; Repair
  Source… offers it, and connecting is the user's action). The user here signs in with a database
  user, so nothing is lost; relaxing the rule is a security decision for its own day.

### What a geodatabase is, to RustGIS

A SQL Server database holds a geodatabase when it has the Esri catalog tables, owned by `sde` or
by `dbo` (both layouts exist; the one found is used): `GDB_ITEMS`, `GDB_ITEMTYPES`,
`SDE_table_registry`, `SDE_layers`, `SDE_spatial_references`, `SDE_versions`. Detection is one
query (`OBJECT_ID` of `sde.GDB_ITEMS` then `dbo.GDB_ITEMS`), run when the connection is browsed
or a table opened, and cached on the connection for its life.

**(call)** The catalog reading lives in a new `crates/data/src/source/geodatabase.rs`, platform
independent in its types and parsing, with the SQL Server queries in one place
(`geodatabase/sqlserver.rs` or a `Platform` enum) so PostgreSQL's can sit beside them. The
SQL Server reader (`sqlserver.rs`) is touched only where it opens a table (below).

The catalog gives, for each item of `GDB_ITEMTYPES` name `Feature Dataset`, `Feature Class` or
`Table`:

| From | What |
|---|---|
| `GDB_ITEMS.Name` | the geodatabase name, `DB.owner.Table` or `owner.Table` as Pro writes it |
| `GDB_ITEMS.PhysicalName` | the database table (`OWNER.TABLE`), what is queried |
| `GDB_ITEMS.Path` | `\Dataset\Class` or `\Class`: the feature dataset it belongs to, if any |
| `GDB_ITEMS.Definition` (XML) | alias (`AliasName`), shape field, geometry type, field aliases (`GPFieldInfoExs`), `Versioned`, `IsArchived`, and for a branch-versioned class its `VersioningType`/branch marker |
| `SDE_table_registry` | `rowid_column` (the ObjectID column: ids come from it, not from a primary key SDE tables don't have) and `object_flags` |
| `SDE_layers` | the spatial column and SDE `srid`, and `eflags` (which encode the storage: native geometry/geography or SDEBINARY) |
| `SDE_spatial_references` | `auth_srid` and `srtext`, for when the column's own SRID is 0 or unknown |

The XML is user data: parsed with the crate's existing bounded XML reader, every element optional,
an unreadable Definition leaves the item listed with its physical name and no alias.

### Browsing

`connection.browse` on a database that holds a geodatabase returns, instead of schemas:

- **Feature datasets**, each listing its feature classes (name as Pro shows it: the part after
  the last `.`; the alias in the tooltip when it differs), sorted;
- **Feature classes** not in a dataset;
- **Tables** (registered non-spatial tables), which add as standalone tables. **(call)** RustGIS
  has no non-spatial database reader today (the SQL Server `Table` is a spatial column; a project
  table is a whole dataset with no source). A registered table is **loaded whole** into a
  standalone table through a small new branch of `layer.addDatabase`: the versioned view when it
  is versioned, else the base table, capped at `MAX_EDITABLE` rows with a notice past the cap,
  fields with their aliases, no source kept (it doesn't reopen or repair; add it again). Pro maps'
  SDE standalone tables stay as they are today, dropped with a note; opening them through a
  connection is a noted follow-on;
- **Other spatial tables**: spatial tables and views the geodatabase doesn't register (a DBA's
  own), as the plain SQL Server browse lists them today, **minus** the geodatabase's machinery:
  `GDB_*`, `SDE_*`, `A<n>`/`D<n>` delta tables, `*_evw`/`*_vw` views, `*_H` archive tables and
  `i<n>` tables. Empty, the node is left out.

Each class's `add` is `layer.addDatabase {database, layer: "<OWNER.TABLE>"}`, the physical name,
so the command surface doesn't change. Each node's `probe`/`answer` flow (passwords, activation)
is the existing one. A geodatabase whose catalog can't be read (permissions) browses as a plain
SQL Server database, with a notice saying the geodatabase catalog wasn't readable and why.

### Reading a feature class

`SqlServerSource::open` on a table that the catalog registers as a feature class does what it
does today, with these differences, each decided by the catalog, not by the user:

| Class | How it's read |
|---|---|
| Unversioned | the base table, as today |
| Traditionally versioned | the versioned view `<OWNER>.<TABLE>_evw` (or `_vw` from 10.1), which the server joins with the delta tables; read without setting a version, so it is the current state of Default. The view is found in `INFORMATION_SCHEMA.VIEWS`; a versioned class whose view is missing is refused: "<class> is versioned but its versioned view is missing; ask the geodatabase administrator to enable SQL access for it" |
| Branch versioned | the base table filtered to the Default branch's current rows (`GDB_BRANCH_ID = 0`, not deleted, open-ended `GDB_TO_DATE`). **Left to the builder** to confirm the exact predicate against Esri's documentation and a branch-versioned class |
| Archived (`IsArchived`) and not versioned | the base table; the `_H` archive table is hidden |
| SDEBINARY storage | refused with the storage named |

Also:

- Ids come from `rowid_column` (ObjectID). Keyless row numbering stays for unregistered tables.
- Fields get their aliases from the Definition; `GlobalID`/`GUID` columns (`uniqueidentifier`)
  are read as text; the shape's length/area columns are ordinary fields.
- The SRID: the column's `STSrid` as today, falling back to `SDE_spatial_references.auth_srid`,
  then its `srtext` (WKT, through `Crs::parse`), before the "no coordinate system" notice.
- The layer's name is the class alias when it has one, else the class name **(call)**; the
  `name` parameter overrides either.
- The streamed/editable split and limits are the SQL Server reader's. Nothing is editable.
- The source kept in the project is today's `SqlServerOrigin` (`database`, `table`, `family`,
  `crs`) with `table` the physical name. Reopening re-reads the catalog, so a class that is
  versioned later is still read right. **(call)** No geodatabase flag is stored.

### Commands

- `connection.new` takes `sdeFile`. `connection.test` reports, for a database that holds a
  geodatabase, `geodatabase: {owner: "sde"|"dbo", version: <release from GDB_RELEASE or
  SDE_version>}` so the dialog can say "Enterprise geodatabase (11.x), sign-in OK".
- `info.databaseTables` returns each table's `dataset`, `alias`, `kind` (`featureClass`,
  `table`, `other`) and `versioning` (`none`, `traditional`, `branch`) when the database holds a
  geodatabase, so the CLI and assistant see what the tree sees.
- Everything else is the existing command set.

## Part 2: Repairing imported SDE layers

### What the Pro import keeps

Today the import keeps only an origin sentence. It now also keeps a structured
`SourceLayer.geodatabase: Option<GeodatabaseOrigin { platform, server, database, table, version }>`
**(call)**, filled from the Pro connection string (`SERVER`/`DB_CONNECTION_PROPERTIES`/`INSTANCE`,
`DATABASE`, the `dataset`, `VERSION`; never `USER` or any password; `DBCLIENT`/`INSTANCE` give
the platform). It is saved with the project like `origin`, cleared when the layer opens, and
`.aprx`, `.mapx` and `.lyrx` all fill it. `layer.diagnose` returns a new kind, `geodatabase`,
for a broken layer that has one.

### Matching a layer to a connection (call)

A named connection **matches** a layer's `GeodatabaseOrigin` when its platform is SQL Server and
its address names the same database and the same server. "Same server" compares the host part
after normalising case, `host\instance` against `?instance=`, `host,port` against `:port`, and a
missing port against SQL Server's default; `sde:sqlserver:host` loses its prefix. The user in the
address doesn't matter: the data is the same database, and the password only ever goes to the
server the connection was saved for. A host written one way by Pro and another by the user (an
IP address against a name, a short name against a fully qualified one) is **not** a match:
RustGIS won't send a saved password on a guess. Instead the Repair dialog lists connections to a
database of the same name on a different host as **near matches** ("<name> is on <host>; use
it?") for the user to pick, and a pick is a user's action, not an automatic one. Several exact
matches (different users) use the existing choose-a-connection flow and the dialog names the one
picked.

### The Repair Source… dialog

| Diagnosis | Shown | Actions |
|---|---|---|
| `geodatabase`, a named connection to that server and database exists | "This layer is in an enterprise geodatabase on <server>, database <database> (table <table>). You have a connection to it: <name>." | **Connect and repair** (signs in through that connection, asking for the password if it isn't remembered), Remove |
| `geodatabase`, no exact match, a near match | "…on <server>, database <database>. <name> is a connection to a database called <database> on <host>." | **Use <name>**, **Add connection…**, Remove |
| `geodatabase`, no connection | "This layer is in an enterprise geodatabase on <server>, database <database>. RustGIS needs a connection to it." | **Add connection…** (the Part 1 form, prefilled with server and database; saving it continues the repair), Remove |
| `geodatabase`, another platform | "…a <platform> geodatabase, which RustGIS can't connect to yet." | Remove |

After connecting, the repair runs `layer.repairGeodatabase {layer, connection}` (new command):

1. Reads the catalog and finds the class whose geodatabase name or physical name matches the
   layer's `table`, case-insensitively, with or without the `DB.` prefix (Pro writes
   `GIS.sde.Mains`-style names; the catalog has both forms).
2. Opens it as Part 1 reads it, checks the geometry family and the fields the style, labels and
   definition query use (as `layer.repairSource` checks a file), and replaces the source with the
   layer's style kept.
3. Then, as the file repair offers: "N other broken layers are in the same geodatabase. Repair
   them too?" Yes repairs each the same way, and the dialog lists any it couldn't (class not found,
   fields don't fit, other version) with the reason, leaving those broken.
4. A layer whose `version` isn't Default (`sde.DEFAULT`, case-insensitive, or empty) is repaired
   at Default with a note on the layer.

The command works on its own too (`layer.repairGeodatabase {layer, connection, all: true}`) for
the CLI and plugins.

### Opening without asking

**(call)** When a Pro map is imported and a layer's `GeodatabaseOrigin` exactly matches a named
connection whose password is remembered (never a Windows sign-in one), the import opens the
layer through it straight away, exactly as it opens a File Geodatabase layer, so the second map
from the same geodatabase needs no repair at all. A connection that needs a password asked is not
used at import time (the import must not stop on a prompt); the layer comes in broken and Repair
Source… offers the connection. The same matching runs on `layer.retry`.

## Part 3: File Geodatabases in the browser

- **Zipped `.gdb`**: `read_zip` recognises an archive whose entries include `*.gdbtable` files
  (under a `<name>.gdb/` folder, or at the root) and reads the geodatabase through the existing
  in-memory entry point `filegdb::read(name, open, layer)` with `open` serving the zip's entries.
  Feature classes become layers, non-spatial tables standalone tables, as `add_geodatabase` does
  on the desktop, with the same notices for tables the native reader can't read (compressed
  storage, raster fields); in the browser there is no GDAL fallback, so those are named in a
  notice. A zip that also holds Shapefiles or other vector files keeps today's behaviour for
  them, within the archive's 32-layer cap.
- This works on the **desktop too** (a zipped `.gdb` dropped or picked opens), since the code is
  shared.
- **Folder picker**: the web app's Add Data gains **File Geodatabase folder…**, which uses the
  browser's directory input (`<input type="file" webkitdirectory>`, which Chrome, Edge, Firefox
  and Safari all support; `rfd` has no folder picker on wasm, so this is a small `web_sys` piece
  in `apps/rustgis-web`). The chosen files are bundled as one in-memory geodatabase and go
  through the same path as a zip. Only the `.gdbtable`, `.gdbtablx`, `.gdbindexes`, `.spx`,
  `.freelist`, `.horizon` and `.atx` members are taken; `.lock` and `timestamps` are ignored.
- Limits: the web upload cap (256 MiB for one upload) and `filegdb::MAX_TABLE_BYTES` (512 MiB on
  wasm) apply; a larger geodatabase is refused with the cap named, as uploads are today.
- `layer.addDataBytes` takes the zip as any other; `info.fileLayers` on the bytes lists its
  layers, so the `layer` parameter picks one.

## Documentation

- `docs/user-guide/data.md`: a new "Enterprise geodatabases (SDE)" section under SQL Server
  (connecting, `.sde` files, what the tree shows, versioning, what isn't read); Data Sources and
  Named connections mention the Enterprise Geodatabase entry.
- `docs/user-guide/maps.md` "Repair Source…": the geodatabase diagnosis and "repair them too".
- `docs/user-guide/workspace.md` Pro import table: an SDE layer opens through a matching named
  connection, else comes in broken with Repair Source….
- `docs/user-guide/data-formats.md`: File Geodatabase row gains the zip and the browser; the
  "desktop-only" line in the intro changes.
- Assistant skill `add-data` learns the geodatabase tree fields of `info.databaseTables`.

## Verification (desktop, on the user's machine)

1. New Connection › Enterprise Geodatabase: fill the form from the Pro dialog's values; Validate
   says geodatabase and release; save; the tree shows datasets, classes with aliases, tables, no
   system tables. Then the same from one of the user's `.sde` files.
2. Add an unversioned class, a traditionally versioned one and (if there is one) a branch
   versioned one; compare the feature count and a known recently edited feature with Pro.
3. Import the utility Pro map with no connection saved: SDE layers broken; Repair Source… on one
   → Add connection… → the layer and the other seven repaired, styles kept. Re-import the map:
   the SDE layers open on their own.
4. Web app: drop a zipped `.gdb`; pick a `.gdb` folder; layers and tables match the desktop.
5. A plain SQL Server database (no geodatabase) browses and reads as before.

## Decisions settled after the builder check (the designer's calls, not the user's)

1. Windows sign-in connections never open a layer without the user acting (Part 1, Connecting).
2. Registered non-spatial tables load whole as standalone tables with a row cap; Pro maps' SDE
   tables stay dropped with a note (Part 1, Browsing).
3. Repair and auto-open match a connection on server and database, not user; a differently
   written host is offered as a near match, never used automatically (Part 2, Matching).

## Left to the builder

- The `.sde` file's exact encoding (see Part 1), confirmed on a real file that stays out of the
  repo; the branch-versioned predicate; whether `eflags` or the Definition is the better test for
  SDEBINARY storage.
- Module layout for the catalog code and the `Platform` seam for PostgreSQL.
- How the connection dialog's form mode and the address field share state.
- The shape of the test catalog fixtures (recorded rows and Definition XML with made-up names).
- The dialog wording beyond what's given; the warning and notice texts.

## Follow-ons, not in this design

- PostgreSQL geodatabases (catalog queries, `ST_Geometry` through `sde.st_asbinary`, `_evw`
  views); Oracle needs a reader first.
- Named versions; editing; subtype and domain descriptions; relationship classes; attachments.
- Reading `.sde` files for connections in Pro `.aprx` files that reference one by path.
