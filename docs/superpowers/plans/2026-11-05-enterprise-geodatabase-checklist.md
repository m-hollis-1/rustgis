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
      for; the connection saves. Note whether the file holds `KEY=VALUE;` text or is a binary
      property set (key strings each followed by a value): the property-set reading
      (`sdefile::parse`'s fallback) is confirmed here. The server includes the instance or port
      Pro's dialog shows (from `DB_CONNECTION_PROPERTIES`/`INSTANCE`), not just the machine name.
- [ ] A `.sde` file for a PostgreSQL geodatabase (if one exists) is refused with the platform
      named; a plain SQL Server connection (non-geodatabase database) browses by schema as before.

## 2. Browsing

- [ ] The tree shows Feature datasets (each with its classes), root classes, Tables, Other spatial
      tables; no `GDB_*`, `SDE_*`, `A<n>`/`D<n>`, `_evw` or `_H` entries; a class with an alias
      is shown by its alias as its name (one without, by its short name, as Pro's Catalog names
      it), and adding it gives a layer of that same name; versioned classes marked
      "· traditional" / "· branch" on hover.
- [ ] An account that can't read the catalog (if one exists) browses as plain SQL Server with the
      notice "the geodatabase catalog couldn't be read: …".

## 3. Reading

- [ ] Add an unversioned class: the layer is named by its alias; fields carry their aliases;
      feature count and a known feature match Pro.
- [ ] Add a traditionally versioned class: count and a **recently edited** feature match Pro's
      Default version (not the base table).
- [ ] Add a branch-versioned class (if any): the same; edit a feature twice and delete another in
      Pro (Default), then check that the edited one appears once, as last edited, and the deleted
      one not at all. This confirms the Default read (the latest row per ObjectID on
      `GDB_BRANCH_ID = 0` by `GDB_FROM_DATE`, `GDB_IS_DELETE = 0`) and the column names.
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
      imported layers on its own; Repair Source… → Connect and repair through it connects as you,
      with no password asked for.
- [ ] Connect and repair through a database-login connection with no saved password: the
      connection's form opens for the password, and Save continues the repair.
- [ ] `layer.retry {all: true}` on several broken SDE layers with the server unreachable tries
      the connection once.
- [ ] The web build (CI): `cargo check --target wasm32-unknown-unknown` for `rustgis-ui-egui`
      and `rustgis-web` passes (this machine lacks clang for zstd-sys).

## Findings

(record here)
