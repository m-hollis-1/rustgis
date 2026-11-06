# Enterprise geodatabase (SDE): desktop verification checklist

Plan: `2026-11-05-enterprise-geodatabase.md`, Task 17. Run on the user's machine against a real
SQL Server enterprise geodatabase. Record what was seen under each item; anything off becomes a
follow-up task on the plan. No server names, database names, user names or layer names from the
user's environment go in this file.

## 1. Connecting

- [ ] Data Sources › New Connection ▾ › **Enterprise Geodatabase…**: fill Server, Sign in
      (Database user), User, Password, Database from the Pro connection dialog's values; Validate
      says "Enterprise geodatabase <release>, N tables with geometry"; Save with Remember ticked.
- [ ] The same from one of the user's `.sde` files (From .sde file…, and by dropping the file on
      the window): Server, Database, User and the name come from the file; the password is asked
      for; the connection saves.
- [ ] A `.sde` file for a PostgreSQL geodatabase (if one exists) is refused with the platform
      named; a plain SQL Server connection (non-geodatabase database) browses by schema as before.

## 2. Browsing

- [ ] The tree shows Feature datasets (each with its classes), root classes, Tables, Other spatial
      tables; no `GDB_*`, `SDE_*`, `A<n>`/`D<n>`, `_evw` or `_H` entries; classes named as Pro's
      Catalog names them, aliases in tooltips; versioned classes marked "· traditional" /
      "· branch".
- [ ] An account that can't read the catalog (if one exists) browses as plain SQL Server with the
      notice "the geodatabase catalog couldn't be read: …".

## 3. Reading

- [ ] Add an unversioned class: the layer is named by its alias; fields carry their aliases;
      feature count and a known feature match Pro.
- [ ] Add a traditionally versioned class: count and a **recently edited** feature match Pro's
      Default version (not the base table).
- [ ] Add a branch-versioned class (if any): the same; note whether deleted/historic rows appear
      (confirms `BRANCH_DEFAULT_FILTER`).
- [ ] Add a registered non-spatial table: a standalone table opens; a table over 100,000 rows
      shows the "first 100000 rows of N" notice.
- [ ] A class whose spatial column's SRID is 0 (if any): the coordinate system comes from the
      geodatabase's spatial reference (note on the layer).
- [ ] A versioned class whose `_evw` view is missing (if any): refused with the administrator
      message, never read from the base table.

## 4. Pro map import and repair

- [ ] With no connection saved, import the utility Pro map: SDE layers come in broken with the
      reason "in an enterprise geodatabase: connect to it with Repair Source…"; their origins
      name server, database and table, never a user or password.
- [ ] Repair Source… on one: "RustGIS needs a connection to it" → **Add connection…** → the
      form is prefilled with server and database → type the user and password, Save → the layer
      repairs, style kept → "N other broken layers are in the same geodatabase. Repair them too?"
      → Yes → all repaired; the notice lists any skipped with why.
- [ ] Re-import the map with that connection remembered: the SDE layers open on their own,
      placed in their groups, styles and labels intact; a layer from another Pro version carries
      the "read at the Default version" note.
- [ ] Save and reopen the project: the SDE layers reopen through the connection; a broken one
      (connection removed) keeps its origin and Repair Source… offers the right thing.
- [ ] Near match: save a second connection to the same database on the host written another
      way (IP against name); Repair Source… offers it as "Use <name>", never automatically.
- [ ] A wrong saved password: the first layer's open fails once with a redacted reason, the
      others say "not tried again after <name> failed", and Data Sources asks for the password.

## 5. Other checks

- [ ] Windows sign-in connection (if usable): connects from the tree this session; never opens
      imported layers on its own; Repair Source… still works when picked.
- [ ] `layer.retry {all: true}` on several broken SDE layers with the server unreachable tries
      the connection once.
- [ ] The web build (CI): `cargo check --target wasm32-unknown-unknown` for `rustgis-ui-egui`
      and `rustgis-web` passes (this machine lacks clang for zstd-sys).

## Findings

(record here)
