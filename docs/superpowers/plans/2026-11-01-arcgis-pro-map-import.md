# ArcGIS Pro Map Import (.aprx, .mapx, .lyrx) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Opening an ArcGIS Pro project (`.aprx`), map file (`.mapx`) or layer file (`.lyrx`) brings its maps into RustGIS with every layer of Pro's Contents pane in the same group tree, with its symbology, labels, scale range and definition query, a layer whose data can't open coming in as a broken layer that keeps its style and can be repaired, and a picker of the project's maps and layouts.

**Architecture:** Part 2 built the CIM reader (`rustgis_map::cim`), a map reader good enough for a template's embedded shapefile maps, and `cmd/arcgis_pro.rs`, which adds those maps. Part 3 widens each: the `Document` learns a project's items, open views and zipped pictures; the map reader returns the layer *tree* with a typed source per layer, every renderer, stacked and drawn markers (a new `Marker::Path`), label placements and label expressions (a new label *template*: `{Field}` text); the engine gains broken layers (`SourceLayer.origin`, `layer.retry`), group creation, the source table, and the `project.importArcgis`/`project.inspectArcgis` commands; the UI gains the picker, a warning glyph with Repair/Retry, and the drop, Browser and backstage entries. The layout path from Part 2 is refactored to share the map code, so a layout ticked in the picker binds to the maps imported with it.

**Tech Stack:** Rust, `serde_json`, `rustgis_data::zip`, tiny-skia renderer, egui UI, `base64`, `image` (engine).

**Spec:** `docs/superpowers/specs/2026-10-30-arcgis-pro-import-design.md`, Part 1 and Part 3 (Part 2 was `docs/superpowers/plans/2026-10-30-arcgis-pro-layout-import.md`, merged into `hlr` as 64bbeb4). Facts learned from the user's real files override the spec where they differ (see "Facts from the test project" below).

## Global Constraints

- `AGENTS.md`: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` in non-test code; no `unsafe`; `get()` over indexing; clamp sizes and bound recursion and lists; `cargo xtask ci` (fmt, clippy `-D warnings`, tests, layers) green before every commit.
- Everything below `apps/` builds for `wasm32-unknown-unknown`: no `std::fs` in `crates/map`; file reading in `crates/engine` behind `#[cfg(not(target_arch = "wasm32"))]` as `cmd/qgis.rs::read_file` and `cmd/arcgis_pro.rs::read_file` do.
- Every new field on `SourceLayer`, `LabelClass`, `Marker`/`Symbol` and `LayerGroup` has `#[serde(default)]` and `skip_serializing_if` for its default: projects saved before read unchanged and save the same bytes (a round-trip test per task that adds one).
- CIM sizes are 1/72-inch points; RustGIS widths and marker sizes are 1/96-inch: `cim::units::pt96`. Text sizes are true points: `cim::units::text_pt`. Pro scales (`minScale`, `maxScale`, `referenceScale`, label ranges) are multiplied by `rustgis_map::qgis::QGIS_SCALE` (Pro works at 96 dpi as QGIS does).
- Interface text through `tl!("…")` and `i18n::fmt`; command ids and parameters never translated. Ribbon placement follows `docs/contributing.md#ribbon-guidelines`.
- `cargo` is at `~/.cargo/bin`: run tool shells with `PATH="$HOME/.cargo/bin:$PATH"`.
- **Privacy (user, 2026-11-01):** nothing in code, tests, fixtures, docs or commit messages names the user's company, its people, its clients or its drive paths. Fixtures are synthetic (`D:\data\roads.shp`, `https://example.org/arcgis/rest/services/Parcels/FeatureServer`). A note or an `origin` string built from a connection string must never carry a password: every `PASSWORD`, `ENCRYPTED_PASSWORD*`, `TOKEN` key is dropped before the string is kept (Task 10).
- Branch `feat/arcgis-pro-map-import` off `hlr` (head 64bbeb4). Commit after every task with a conventional message (`feat(cim): …`, `feat(map): …`, `feat(engine): …`, `feat(ui): …`, `docs: …`) ending in `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Tests: `cargo test -p rustgis-map cim`, `cargo test -p rustgis-engine arcgis`, `cargo test -p rustgis-ui-egui aprx`, plus `crates/engine/src/tests.rs::hostile_params_never_panic` gains junk for every new command and parameter.

## Facts from the test project (override the spec)

Read on 2026-11-01 from the user's real `.aprx` (1027 zip entries, 15 MB; 718 JSON, 244 XML, 61 `.dat`, 3 pictures under `Pictures/`):

- `GISProject.json` (`type` `CIMGISProject`): `projectItems[]` of `{itemType: "Map"|"Layout"|"GDB"|"Style"|"GPHistory"|…, name, catalogPath: "CIMPATH=folder/file.json"}`; an ArcMap-era item's `catalogPath` ends in `.xml` (the zip holds the XML; it isn't CIM). `views[]`: `CIMMapView {viewableObjectPath, camera {x, y, scale, heading}}` (the map pane(s) open when saved, first is the one shown), `CIMLayoutView {viewableObjectPath}`, `CIMMapTableView` (ignored). `defaultFolder` is `.`.
- `CIMMap`: `name`, `spatialReference {wkid, latestWkid}`, `defaultExtent {xmin, ymin, xmax, ymax}`, `layers: ["CIMPATH=…"]` (top first), `standaloneTables: ["CIMPATH=…"]`, `referenceScale` (absent when none), `mapType: "Map"`.
- Layer types seen: `CIMFeatureLayer`, `CIMGroupLayer {layers, visibility, expanded, sublayerVisibilityMode, transparency?, minScale?, maxScale?}`, `CIMRasterLayer {dataConnection, colorizer, transparency?}`, `CIMTiledServiceLayer {serviceConnection: CIMAGSServiceConnection {url …/MapServer, objectType}, transparency}`, `CIMImageServiceLayer`, `CIMVectorTileLayer {dataConnection: CIMVectorTileDataConnection {uRI: …/root.json}}`, `CIMAnnotationLayer`, `CIMWMSLayer`-style layers with `CIMWMSServiceConnection {serverConnection.url}` and `subLayers`, `CIMStandaloneTable {dataConnection}`.
- Connections: `CIMStandardDataConnection {workspaceFactory: "FileGDB"|"Shapefile"|"FeatureService"|"SDE"|"Raster"|"Cad"|"Excel", workspaceConnectionString: "DATABASE=…" | "URL=…/FeatureServer" | "ENCRYPTED_PASSWORD_UTF8=…;SERVER=…;DATABASE=…;USER=…", dataset}`; `CIMFeatureDatasetDataConnection {featureDataset, workspaceConnectionString, workspaceFactory: FileGDB|SDE, dataset}` (a feature class inside a feature dataset: the file is still `DATABASE=`, the table `dataset`); `CIMRelQueryTableDataConnection {sourceTable, destinationTable, primaryKey, foreignKey}`; `CIMSqlQueryDataConnection`; `CIMKMLDataConnection`. Raster paths are relative (`DATABASE=.\IMG`, `dataset: Foster.jpg`).
- Renderers: `CIMSimpleRenderer` 444, `CIMUniqueValueRenderer` 120 (one field 110, three fields 10; `useDefaultSymbol` on 38), `visualVariables: [CIMSizeVisualVariable {valueExpressionInfo: "$view.scale", sizeValues, dataValues}]` on 7 (size by scale: noted, not a field). No class breaks in this project; the spec's rows still apply.
- Labels: `CIMLabelClass {name, expression, expressionEngine: "Arcade"|"VBScript"|"Python", visibility, whereClause?, minimumScale?, maximumScale?, priority, textSymbol, maplexLabelPlacementProperties, standardLabelPlacementProperties?}`. 452 of 561 expressions are one field; the 109 others are concatenations (`$feature.A + "\n " + $feature.B`, `"BLK " & [BLOCK]`, `"SEC. "+$feature.SECT_NO`, `[rods]&" RD"`), a joined field name with a dot (`$feature['BM_Parcel.StreetName']`), and a few function calls (`Split(...)[0]`, `Concatenate(...)`, `IIf(...)`). Maplex `featureType` Line 605 / Polygon 289 / Point 77; `pointPlacementMethod` `AroundPoint` 965, `WestOfPoint`, `CenteredOnPoint`, `NorthEastOfPoint`, `RotationField`; `linePlacementMethod` `OffsetCurvedFromLine` 425, `OffsetStraightFromLine` 112, `OffsetHorizontalFromLine` 11, `CenteredHorizontalOnLine` 8, `CenteredCurvedOnLine`, `CenteredStraightOnLine`; `polygonPlacementMethod` `AlwaysHorizontal` 410, `CurvedInPolygon` 275, `HorizontalInPolygon` 238, `StraightInPolygon` 44, `RepeatAlongBoundary` 4; `labelStackingProperties.separators[] {separator, splitAfter, visible}`. Standard placement: `lineLabelPosition {above, inLine, parallel, below, online, horizontal…}`, `pointPlacementMethod`, `polygonPlacementMethod: "AlwaysHorizontal"|"AlwaysStraight"|"MixedStrategy"`. Callouts: `CIMBackgroundCallout` 28, `CIMSimpleLineCallout` 15, `CIMCompositeCallout` 1. `shadowColor` on 227 text symbols (noted).
- Symbols: multi-layer point symbols 8 of 195 (2, 3 and 5 layers); vector markers with one graphic 183, two graphics 19; `CIMCharacterMarker` fonts ESRI Arrowhead, ESRI North, ESRI AMFM Water, ESRI Default Marker; picture markers PNG 33, GIF 4 (one tinted); marker placements `CIMMarkerPlacementAtRatioPositions` 28, `AtExtremities` 20, `AtMeasuredUnits` 6; effects Dashes 76, Offset 36, AddControlPoints 18, Cut 14, EnclosingPolygon 1.
- Layer properties: `transparency` 0–80, `blendingMode` `Alpha` (normal) or `Multiply`, `minScale`/`maxScale`, `expanded`, `scaleSymbols`, `symbolLayerDrawing`, `definitionExpression` (`LyrOn=1 And LyrFrzn <>1`, `CLIENT = 'X'`, `OBJECTID <> 5`, `name IN (…)`; RustGIS's SQL reads these, keywords in any case).
- A `.lyrx` is `{type: "CIMLayerDocument", layers: ["CIMPATH=…"], layerDefinitions: [...]}`: `layers` lists the top-level entries (a group or a layer), `layerDefinitions` holds every layer, groups included. A `.mapx` is `{type: "CIMMapDocument", mapDefinition: {CIMMap}, layerDefinitions, binaryReferences?}`.
- A layout inside an `.aprx` names its maps by the project's own URIs (`CIMPATH=map/map.json`) and its pictures by `referenceURI: CIMPATH=Pictures/<hash>.png`, a zip entry.

## Review Focus

1. **A connection string with a password** (`ENCRYPTED_PASSWORD=…;SERVER=…;DATABASE=…;USER=sa`): the broken layer's `origin`, every note and the command's result must carry server, database and table and never the password or user. Test in Task 10 (`origins_never_carry_passwords`).
2. **A project whose item points at a missing entry** (`catalogPath` names a JSON the zip lacks, or an `.xml`): `project.inspectArcgis` lists it (ArcMap-era ones marked), `project.importArcgis` notes and skips it, nothing panics. Test in Task 12.
3. **A label template whose field the layer lacks** (`{OWNER} {GONE}`): the class labels with the fields it has, a note names the missing one; a template with no field of the layer's switches the class off. Test in Task 3 (`a_template_with_a_missing_field_still_labels`) and Task 11 (`check_fields`).
4. **A broken layer saved and reopened**: the project keeps it (name, place in its group, style, definition query, `origin`), it draws nothing, Repair relinks it and the style survives. Test in Task 4 (`a_broken_layer_round_trips_and_repairs`).
5. **A drawn marker with hostile geometry** (NaN or 1e300 coordinates, 10 000 points, 1 000 parts, zero-size box): `PathRef::new` clamps or refuses, the renderer draws a bounded shape, serde reads it back. Test in Task 2 (`hostile_paths_are_clamped_or_refused`).

---

### Task 1: The document knows a project's items, open views, zipped pictures and standalone definitions

**Files:**
- Modify: `crates/map/src/cim/mod.rs` (`Document::from_zip`, new methods), `crates/map/src/cim/layout.rs:320` (`read_layout` split), `crates/map/src/cim/map.rs:71` (`read_map` split)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Consumes: `rustgis_data::zip::read_matching_within`; `cim::symbol::base64_text`.
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)] pub enum ItemKind { Map, Layout }
  #[derive(Clone, Debug, PartialEq)] pub struct ProjectItem { pub name: String, pub kind: ItemKind, pub uri: String, /// `catalogPath` ends in `.xml`: saved by ArcMap, not CIM.
      pub arcmap: bool }
  impl Document {
      /// A project's maps and layouts (`projectItems` of `itemType` Map or Layout), in file order, at most MAX_ITEMS (1000).
      pub fn items(&self) -> Vec<ProjectItem>;
      /// The URIs of the maps whose panes were open when the project was saved (`views[]` of type `CIMMapView`, `viewableObjectPath`), in order; empty for a document that isn't a project.
      pub fn open_maps(&self) -> Vec<String>;
      /// The saved camera of the view on `map_uri`: `(x, y, scale)` in the map's spatial reference.
      pub fn view_camera(&self, map_uri: &str) -> Option<(f64, f64, f64)>;
      /// The URI of a `.mapx`'s or `.lyrx`'s own definition: `mapDefinition` (a CIMMapDocument) is indexed under the key "mapDefinition"; a CIMLayerDocument's `layers` list is returned by `layer_uris()`.
      pub fn map_definition(&self) -> Option<&Value>;
      pub fn layer_uris(&self) -> Vec<String>;   // a CIMLayerDocument's `layers`, else empty
  }
  pub fn read_layout_def(doc: &Document, def: &Value, report: &mut Report) -> Result<LayoutImport, String>;  // what `read_layout` does once it has the CIMLayout value
  pub fn read_map_def(doc: &Document, def: &Value, report: &mut Report) -> Result<MapImport, String>;        // what `read_map` does once it has the CIMMap value
  ```
  `from_zip` also keeps, as base64 text under their lower-cased path, zip entries ending in `.png`, `.jpg`, `.jpeg`, `.gif`, `.bmp` of at most `MAX_PICTURE_ENTRY` (16 << 20) bytes each and `MAX_PICTURE_TOTAL` (64 << 20) together, so `binary("CIMPATH=Pictures/x.png")` answers for them (the layout reader's `referenceURI` path then works unchanged). `read_layout(doc, report)` becomes `read_layout_def(doc, doc.root()["layoutDefinition"], report)`; `read_map(doc, uri, report)` resolves `uri` and calls `read_map_def`.

- [ ] **Step 1: Write the failing tests** in `cim/tests.rs`: `a_project_lists_its_maps_layouts_open_views_and_pictures` (a zip built with `rustgis_data::zip::write` holding `GISProject.json` with two Map items, one Layout item, one Map item whose `catalogPath` ends `.xml`, `views` with a `CIMMapView` on the second map with camera `{x: 1.0, y: 2.0, scale: 5000.0}` and a `CIMLayoutView`; a `Pictures/a.png` entry of 8 bytes starting `\x89PNG`; assert `items()` is 4 long with `arcmap` true on the XML one, `open_maps()` == `[second's uri]`, `view_camera(second) == Some((1.0, 2.0, 5000.0))`, `view_camera(first) == None`, `binary("CIMPATH=Pictures/A.PNG")` is the base64 of the 8 bytes), `a_mapx_and_a_lyrx_expose_their_definitions` (`map_definition()` is Some for a `CIMMapDocument`, `layer_uris()` lists a `CIMLayerDocument`'s two entries, both empty for a `.pagx`), `oversize_picture_entries_are_left_out` (a 17 MB `Pictures/big.png` → `binary` None, the document still reads).
- [ ] **Step 2: Run to verify failure.** `PATH="$HOME/.cargo/bin:$PATH" cargo test -p rustgis-map cim::tests::a_project_lists` → compile error (no `items`).
- [ ] **Step 3: Implement** in `cim/mod.rs`; split `read_layout`/`read_map` as the Interfaces say (no behaviour change: the existing tests stay green).
- [ ] **Step 4: Run** `cargo test -p rustgis-map cim` → PASS.
- [ ] **Step 5: Commit** `feat(cim): project items, open views and zipped pictures`.

---

### Task 2: A drawn marker kind: `Marker::Path`

**Files:**
- Modify: `crates/map/src/symbology.rs:13-80` (`Marker`, `MarkerWire`, `label()`), new types after `ImageRef` (`:216`)
- Modify: `crates/render/src/lib.rs:1449` (`marker_path`), `:1512` (`draw_marker`), `:2577` (`marker_outline`); `crates/render/src/mesh.rs:286,344`; `crates/render/src/layout.rs:2161` (`swatch`)
- Modify: `crates/ui-egui/src/panes.rs:520-545` (the Layers pane patch), `:4553-4565` (the Shape combo: a Path shows as "Custom", not pickable)
- Modify: `crates/engine/src/cmd/layer.rs:263` (`layer.marker`: `shape` stays the five names; a Path marker is kept when only `size` is given)
- Test: `crates/map/src/symbology.rs` (inline), `crates/render/src/lib.rs` (inline)

**Interfaces:**
- Produces:
  ```rust
  /// A marker drawn from paths: a shape RustGIS hasn't, or several marker layers stacked. Unit space: the marker's size is the
  /// square from (−0.5, −0.5) to (0.5, 0.5), y up (as CIM's); parts may reach past it to ±2.0.
  #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct PathShape { pub parts: Vec<PathPart> }
  #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct PathPart {
      pub points: Vec<[f32; 2]>,            // 2..=MAX_PATH_POINTS (256)
      #[serde(default = "yes", skip_serializing_if = "is_yes")] pub closed: bool,
      #[serde(default, skip_serializing_if = "Option::is_none")] pub fill: Option<Rgba>,
      #[serde(default, skip_serializing_if = "Option::is_none")] pub stroke: Option<Rgba>,
      #[serde(default, skip_serializing_if = "is_zero")] pub width: f32,   // stroke width as a fraction of the size, 0..=0.5
  }
  pub const MAX_PATH_PARTS: usize = 64; pub const MAX_PATH_POINTS: usize = 256; pub const MAX_PATHS: usize = 4096;
  /// Interned like ImageRef: equal shapes are one kept shape; Copy, Eq and Hash by address.
  #[derive(Clone, Copy, Debug)] pub struct PathRef(&'static PathShape);
  impl PathRef {
      /// Clamps every coordinate to ±2.0 (a non-finite one to 0), drops parts with fewer than 2 points, cuts parts and points to the limits, clamps widths; `None` when nothing is left or the table is full.
      pub fn new(shape: PathShape) -> Option<PathRef>;
      pub fn shape(&self) -> &'static PathShape;
  }
  // Marker gains `Path(PathRef)`; `MarkerWire` reads `{"Path": {"parts": [...]}}`; `Marker::label()` → "Custom"; `Marker::ALL` unchanged.
  ```
  Renderer: `draw_marker` branches on `Marker::Path(p)` to `draw_path_marker(pm, x, y, r, &p, opacity)`: each part scaled by `2 * r`, y flipped, filled with its `fill` (none: no fill) and stroked with `stroke` at `width * 2 * r` pixels (at least 0.5) when both are set; the halo strokes every part. The symbol's own `fill`/`stroke` are not used (a Path carries its colours, as an Image does). `marker_path` → the unit square (hit tests), `marker_outline` → the square, `marker_reach` → 0.6 or the farthest point, `mesh::marker_instances` shape 0 and `tessellate` returns `TessError::Refused` for a Path on points (drawn on the CPU, as Char and Image are). Legend `swatch` and the Layers pane patch draw the parts (egui: `Shape::Path` per part, `convex_polygon` is wrong for concave parts, so use `PathShape::closed_line`/`line` with the fill as `PathShape { fill, .. }`).

- [ ] **Step 1: Write the failing tests**: in symbology `path_markers_intern_clamp_and_round_trip` (two equal shapes → `==`; a shape with a NaN and a 1e30 coordinate → clamped to 0 and 2.0; 1000 parts → 64; `serde_json` of a `Symbol` with a Path marker reads back equal; `Marker::label()` == "Custom"), `hostile_paths_are_clamped_or_refused` (empty parts → None; a part of one point → dropped; 10 000 points → 256); in render `a_path_marker_draws_its_parts_in_their_colours` (a two-part shape: a red filled square and a blue stroked diagonal; render one point at 20 pt; the pixel at the centre is red, a pixel on the diagonal outside the square is blue, a pixel outside both is the background) and `path_markers_are_drawn_on_the_cpu` (`mesh::tessellate` on a point layer with a Path marker returns `Refused`).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map path_marker`, `cargo test -p rustgis-render path_marker`, `cargo test -p rustgis-ui-egui` (the gallery and symbol pane tests), `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(map): drawn markers (Marker::Path)`.

---

### Task 3: Label templates: `{Field}` text as a label class's source

**Files:**
- Modify: `crates/map/src/symbology.rs` (new items near `wrapped`, `:2050`; `LabelClass.field` doc)
- Modify: `crates/render/src/lib.rs:3563-3564` (`label_fields`), `:3639-3645` (the per-row text)
- Modify: `crates/engine/src/cmd/labels.rs:420` (`convert`), `crates/engine/src/cmd/layer.rs:362-372` (`labels.field` accepts a template), `crates/engine/src/cmd/qgis.rs::check_fields` (`:198-205`)
- Modify: `crates/ui-egui/src/labelpane.rs:431-470` (`field`: a Field/Expression switch; Expression shows a text box and an "Insert field" combo)
- Test: `crates/map/src/symbology.rs` (inline), `crates/render/src/lib.rs` (inline), `crates/engine/src/tests.rs`, `crates/ui-egui/src/labelpane.rs` (inline)

**Interfaces:**
- Produces:
  ```rust
  pub const MAX_TEMPLATE: usize = 1000;   // characters of LabelClass.field
  /// Whether `field` is a template: holds a `{name}` (a `{{` is a literal brace).
  pub fn is_template(field: &str) -> bool;
  /// The field names a template refers to, in order, each once; empty for a plain field.
  pub fn template_fields(field: &str) -> Vec<&str>;
  pub enum Piece { Text(String), Field(usize) }
  /// Where a class's text comes from, resolved against a dataset once per frame.
  pub enum LabelSource { Field(usize), Template(Vec<Piece>) }
  impl LabelSource {
      /// `None` for a plain field the dataset lacks, or a template none of whose fields it has. A template's missing field becomes empty text.
      pub fn resolve(field: &str, ds: &Dataset) -> Option<LabelSource>;
      /// The label's text for `row`: a null plain field is `None`; a template whose every field is null is `None`; a null field inside a template is empty; `{{`/`}}` are braces; a newline in the template is a line break.
      pub fn text(&self, ds: &Dataset, row: usize) -> Option<String>;
  }
  ```
  `labels.field`: `field` may be a template; it is accepted when `template_fields` is non-empty and every one is a field of the layer (else `Err("no field named '{name}'")`); longer than `MAX_TEMPLATE` is refused. `check_fields` turns a class off only when `LabelSource::resolve` is `None`, and notes each missing template field ("labels from the field X, which the layer hasn't; the rest of the expression is drawn"). The render loop keeps `label_fields: Vec<Option<LabelSource>>` and takes `source.text(ds, row)`. The label pane: a two-way toggle `Field | Expression` (`egui::SelectableLabel`s); Expression shows a `TextEdit::singleline` of the template (set on losing focus or Enter through `labels.field`) and a combo "Insert field…" that appends `{name}`; a class whose `field` is a template opens on Expression.

- [ ] **Step 1: Write the failing tests**: symbology `templates_expand_fields_literals_braces_and_newlines` (dataset with `NAME`, `TYPE`; `"{NAME}\n({TYPE})"` → `"Paris\n(city)"`; `"{{{NAME}}}"` → `"{Paris}"`; all-null row → None; `is_template("NAME")` false; `template_fields("{A} {B} {A}")` == `["A", "B"]`), `a_template_with_a_missing_field_still_labels` (`"{NAME} {GONE}"` → Some(Template) and text `"Paris "`; `"{GONE}"` → None); render `a_label_template_draws_both_fields` (a two-row point dataset with `name` and `pop`, template `"{name} {pop}"`, rendered at a scale where both labels place: the frame's placed labels (`render_frame`'s label records, as the existing label tests read them) carry the texts `"Paris 2000"` and `"Lyon 500"`); engine `labels_field_accepts_a_template_and_refuses_a_missing_field` and `convert_to_annotation_uses_the_template`; ui `the_label_pane_expression_box_sets_a_template` (kittest: switch to Expression, type `{name} {pop_max}`, press Enter, the class's field is the template).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map template`, `-p rustgis-render template`, `-p rustgis-engine template`, `-p rustgis-ui-egui template`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(map): label templates`.

---

### Task 4: Broken layers: a source that remembers where it came from, and Retry

**Files:**
- Modify: `crates/map/src/source.rs:36-111` (`SourceLayer`: `origin`, `service`; `broken()`), `crates/map/src/qgis/mod.rs:126` (`Style::apply_all`)
- Create: `crates/engine/src/cmd/retry.rs` (`layer.retry`)
- Modify: `crates/engine/src/cmd/mod.rs` (declare and register), `crates/engine/src/cmd/repair.rs:40-80` (a repair of a broken layer keeps renderer, labels, strokes, fills, definition query; `origin` is cleared once the source opens)
- Test: `crates/map/src/source.rs` (inline), `crates/engine/src/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  // source.rs
  #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
  pub struct ServiceOrigin { pub url: String, #[serde(default)] pub kind: String /* "arcgis" | "wms" */, #[serde(default)] pub layer: String /* service.add's `layer` */ }
  // SourceLayer gains:
  #[serde(default, skip_serializing_if = "Option::is_none")] pub origin: Option<String>,   // "ArcGIS Pro read it from <where>", for people; never a password
  #[serde(default, skip_serializing_if = "Option::is_none")] pub service: Option<ServiceOrigin>,
  impl SourceLayer {
      /// An unopened source: `path`/`table` when there is a file to open later (Repair), `service` when a service to retry, `error` why it is broken now.
      pub fn broken(origin: String, path: Option<String>, table: Option<String>, service: Option<ServiceOrigin>, error: String) -> SourceLayer;
      pub fn is_broken(&self) -> bool;   // !is_open() && !is_pending() && origin.is_some()
  }
  // qgis/mod.rs
  impl Style { /// `apply` without the geometry checks: for a layer whose geometry isn't known yet (a broken one).
      pub fn apply_all(&self, layer: &mut crate::Layer); }
  // retry.rs
  pub fn commands() -> Vec<C>;  // "layer.retry": {"layer"?: name|id | "all"?: true (every broken layer of the map)} → {"retried": n, "opened": m, "failed": [{"layer", "error"}]}
  ```
  `layer.retry`: a layer whose `source.service` is set runs `service.add {url, type: kind, layer}`; when the result names a layer, the new layer's `dataset` and `source` move into the broken layer (its id, name, group, order, `visible`, renderer, labels, strokes, fills, `transparency`, scale range, `blend`, `reference_scale` stay), the new layer is removed (`Map::layers.remove` as `cmd/layer.rs::remove` does), `origin` is cleared, and `definition_query` is parsed against the new fields (kept when it parses, else cleared with a notice). A layer whose `source.path` is set calls `src.reopen()` and refreshes `dataset` from `info()` as `Layer::sync_source` does. Errors are collected, not returned: one failure doesn't stop `all`.

- [ ] **Step 1: Write the failing tests**: source `a_broken_source_round_trips_its_origin_and_service` (serde of a `SourceLayer::broken(...)` keeps `origin` and `service`, an old JSON without them reads with both None, a source with neither serialises without the keys); engine `a_broken_layer_round_trips_and_repairs` (build a broken layer with `path` to a temp path that doesn't exist, in a group, with a Unique renderer, labels, `definition_query`; `project.saveAs` + `project.open` keeps all of it and `source.error`; copy the sample's shapefile to the path; `layer.repairSource` opens it and the renderer, labels and query are still there, `origin` is None), `layer_retry_reopens_a_file_and_reports_a_service_it_cannot_reach` (a broken file layer whose file appears → `opened: 1`; a broken service layer at `https://127.0.0.1:9/x/FeatureServer` → `failed` names it, the layer is still there and still broken), `a_broken_layer_draws_nothing_and_is_not_a_table` (render the map: the pixels equal an empty map's; `project.tables` unchanged).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map broken`, `cargo test -p rustgis-engine broken`, `cargo test -p rustgis-engine retry`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(engine): broken layers with an origin, and layer.retry`.

---

### Task 5: The Layers pane shows broken layers and offers Repair and Retry

**Files:**
- Modify: `crates/ui-egui/src/icons.rs` (new icon `warning`: an amber triangle with a dark `!`; add to `ALL` at `:1196`), `crates/ui-egui/src/panes.rs:1930-1975` (`layer_row`: glyph after the name when `l.source.as_ref().is_some_and(|s| s.error.is_some())` or a raster with no data; tooltip adds the `origin` line and the error), `:2229-2232` (`layer_menu`: "Retry" when `source.service.is_some() || (source.is_broken() && source.path.is_some())`, running `layer.retry {layer}`; "Repair Data Source…" stays as it is), `crates/ui-egui/src/arcgisui.rs:155-180` (when `arcgis_auth::progress()` turns from `Pending` to `Done`, once, run `layer.retry {"all": true}` and show a notice with `opened`)
- Modify: `crates/ui-egui/src/lib.rs` (the `ArcgisUi` state gains `was_pending: bool`)
- Test: `crates/ui-egui/src/panes.rs` (inline), `crates/ui-egui/src/icons.rs` (the gallery test covers `warning`)

The glyph is its own interactive rect (`ui.interact`, 16 × 16, after the name) with the accessibility label "Broken layer" and the hover text `i18n::fmt("Broken: {origin}\n{error}", …)`, so tests and screen readers find it.

- [ ] **Step 1: Write the failing tests**: `a_broken_layer_shows_a_warning_with_its_origin` (kittest on the Layers pane with a broken layer whose origin is "a shapefile at D:\data\roads.shp": `h.get_by_label("Broken layer")` exists and its hover text contains the origin; an open layer's row has no such label), `retry_appears_for_service_origins_only` (the context menu of a service-origin layer has "Retry", a file-origin broken layer has both "Retry" and "Repair Data Source…", an open layer has neither "Retry").
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-ui-egui broken`, `cargo test -p rustgis-ui-egui icons`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(ui): broken layers in the Layers pane`.

---

### Task 6: CIM symbols: drawn and stacked markers, markers along lines, dots in polygons

**Files:**
- Modify: `crates/map/src/cim/symbol.rs:255-390` (`point`, `vector_marker`, `picture_marker`; new `path_marker`, `stacked`, `line_marker_placement`, `inside_polygon`), `crates/map/src/cim/map.rs:233-262` (`look`: a point symbol of several layers becomes one Path; a line symbol's `CIMVectorMarker` layers become `LineMarkers`; a polygon's `CIMVectorMarker` with `CIMMarkerPlacementInsidePolygon` becomes `FillPattern::Dots`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Consumes: Task 2's `PathShape`, `PathPart`, `PathRef`; `symbology::{LineMarkers, LineMark, Placements}`; `fills::FillPattern::Dots`.
- Produces (all `pub(crate)` in `cim/symbol.rs`):
  ```rust
  /// A CIMVectorMarker's graphics as paths in unit space: each graphic's `rings`/`paths`/`curveRings` (a curve ring with 4 arcs → a 32-point circle; other curves → their control points, noted) scaled so the marker's `frame` (else the graphics' box) is the unit square, turned by `rotation` (anticlockwise, as Pro) and moved by `offsetX`/`offsetY` (points, as a fraction of `size`), with each graphic's polygon symbol's top solid fill and top stroke. `None` for a marker with no drawable graphic.
  fn path_marker(l: &Value, size_pt: f64, what: &str, report: &mut Report) -> Option<PathShape>;
  /// Several marker layers as one Path: each vector layer's paths scaled by its size over the largest size; a Char or Image layer in the stack → the stack is the first marker, noted.
  fn stacked(symbols: &[(Symbol, Option<PathShape>)], what: &str, report: &mut Report) -> Symbol;
  /// A line symbol's marker layer as markers along the line: `CIMMarkerPlacementAlongLineSameSize`/`AlongLineVariableSize` → `Placements::INTERVAL` with `placementTemplate[0]` as the interval; `AtExtremities` → `FIRST|LAST` (`extremityPlacement` Both/JustBegin/JustEnd); `AtRatioPositions` with `[0.5]` → `MIDDLE`, else `INTERVAL` noted; `AtMeasuredUnits` → `INTERVAL`; `OnVertices` → `VERTICES`; `angleToLine` → `rotate`; `offset` → the stroke's offset. The marker itself from `vector_shape`, else a Path, else a Char/Image.
  fn line_markers(l: &Value, what: &str, report: &mut Report) -> Option<LineMarkers>;
  /// A polygon symbol's marker layer with `CIMMarkerPlacementInsidePolygon` → Dots: `stepX`/`stepY` (points) as spacing, `offsetX`/`offsetY`, `gridAngle`/`shiftOddRows` noted; the marker as above.
  fn inside_polygon(l: &Value, what: &str, report: &mut Report) -> Option<FillLayer>;
  ```
  In `point`, the current "a drawn marker shape, shown as a circle" becomes a Path; `rotation` ≠ 0 on a vector marker whose shape is one RustGIS has gives a Path too (the shape turned), with no note; a picture or character marker's rotation stays noted. `PointSymbol` gains `paths: Vec<Option<PathShape>>` beside `symbols` (same length) so `look` can stack.

- [ ] **Step 1: Write the failing tests**: `a_vector_marker_rustgis_lacks_becomes_a_path` (a hexagon ring in a 10 × 10 frame → `Marker::Path` with one closed part of 7 points inside ±0.5, red fill, no stroke), `a_rotated_square_becomes_a_turned_path` (a square at `rotation: 30` → Path, 5 points, the first not axis-aligned), `stacked_marker_layers_become_one_path_sized_by_the_largest` (a 12 pt circle under a 6 pt square → one Path of two parts; `size` 16 (12 pt as 96ths); the square's points within ±0.25), `a_stack_with_a_picture_keeps_its_first_marker_and_says_so`, `markers_along_a_line_become_line_markers` (a `CIMVectorMarker` with `CIMMarkerPlacementAtExtremities {extremityPlacement: "JustEnd", angleToLine: true}` → `LineMarkers { at: LAST, rotate: true, .. }` on the stroke; `AtRatioPositions [0.5]` → `MIDDLE`; `AlongLineSameSize {placementTemplate: [20]}` → `INTERVAL`, interval 26.67), `dots_inside_polygons_become_a_dots_fill` (`stepX: 10, stepY: 10` → `FillPattern::Dots { spacing: [13.33, 13.33], .. }`), `hostile_marker_graphics_are_bounded` (a graphic of 5000 points, one of NaN coordinates, 200 graphics: no panic, Path within limits).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map cim`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(cim): drawn, stacked and placed markers`.

---

### Task 7: CIM renderers: class breaks, proportional, visual variables, the rest

**Files:**
- Modify: `crates/map/src/cim/map.rs:305-340` (`renderer`), new `class_breaks`, `proportional`, `size_variable`, `first_symbol`
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Consumes: `Renderer::{Graduated, Proportional, Single}`, `ClassMethod::from_name`, `Ramp::default()`.
- Produces (private to `cim/map.rs`): `CIMClassBreaksRenderer {field, breaks[] {upperBound, label, symbol}, minimumBreak, classificationMethod, classBreakType, defaultSymbol}` → `Renderer::Graduated { field, breaks: upperBounds ascending, symbols, ramp: default, method: ClassMethod::from_name(classificationMethod) or Manual, min: minimumBreak or the first break, classes: n, class_labels (from `label` when set), class_strokes, class_fills }`; `classBreakType == "UnclassedColor"` → the same with a note "an unclassed colour ramp, drawn as its classes"; `normalizationType != "Nothing"` noted; `CIMProportionalRenderer {field, minSymbol/maxSymbol or minDataValue/maxDataValue, symbol}` → `Renderer::Proportional { field, base, min_size, max_size (96ths), min, max }`; a `CIMSizeVisualVariable` whose `valueExpressionInfo.expression` is `$feature.X` (or `field`) on a `CIMSimpleRenderer` → `Proportional` with `minSize`/`maxSize` and `minValue`/`maxValue`; one on `$view.scale` or on another renderer → noted "symbol sizes that change with the scale (or by X), drawn at one size"; `CIMHeatMapRenderer`, `CIMDotDensityRenderer`, `CIMChartRenderer`, `CIMDictionaryRenderer`, `CIMRepresentationRenderer`, any other → `Single(first symbol found in the renderer, depth-first, else the default)` with "X renderers, drawn with its first symbol".

- [ ] **Step 1: Write the failing tests**: `class_breaks_read_as_graduated` (3 breaks with labels, `classificationMethod: "NaturalBreaks"`, `minimumBreak: 0` → Graduated with breaks `[10, 20, 30]`, `method: NaturalBreaks`, labels for 0 and 2), `unclassed_colour_is_graduated_and_noted`, `proportional_and_size_variables_read` (a `CIMProportionalRenderer` → Proportional; a `CIMSimpleRenderer` with a size variable on `$feature.POP` → Proportional with `min_size` 5.33, `max_size` 26.67; one on `$view.scale` → Single, noted), `other_renderers_draw_their_first_symbol` (a `CIMHeatMapRenderer` holding a symbol → `Single(that symbol)` and the note).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map cim`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(cim): class breaks, proportional and other renderers`.

---

### Task 8: Label expressions as templates

**Files:**
- Create: `crates/map/src/cim/expr.rs`
- Modify: `crates/map/src/cim/mod.rs` (declare `mod expr;`), `crates/map/src/cim/map.rs:448-540` (`label_class` uses `expr::template`; `expression_field`/`first_reference` move into `expr.rs`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  pub(crate) struct Translated { pub template: String /* a plain field name, or a template (Task 3) */, pub exact: bool, pub fields: Vec<String> }
  /// `expression` in `engine` ("Arcade", "VBScript", "Python", "JScript") as a label template. Exact when the whole expression is field references, string literals, concatenation (`+`, `&`) and newlines; else the first field referenced (`exact: false`), or `None` when none.
  pub(crate) fn template(expression: &str, engine: &str) -> Option<Translated>;
  ```
  Grammar (one tokenizer for all engines): field references `$feature.X`, `$feature["X"]`, `$feature['X']`, `[X]`, `!X!`, a bare name when the whole expression is one; string literals in `"…"` or `'…'` with `\n`, `\r`, `\"`, `\'`, `\\` escapes (Arcade/Python/JScript) and `""` doubling (VBScript); concatenation `+`, `&`; newline tokens `TextFormatting.NewLine`, `vbNewLine`, `vbCrLf`, `vbLf`, `chr(10)`, `chr(13)`, `"\n"`, `"\r\n"` → a newline; `<br>`, `<BR>` inside a literal → a newline; Pro formatting tags (`<BOL>`, `</BOL>`, `<ITA>`, `<UND>`, `<FNT …>`, `<CLR …>`) inside a literal are dropped and noted once per class ("text formatting tags, left out"); `\r` alone → a newline; whitespace between tokens ignored; a leading `return` (any case) and a trailing `;` ignored. A field name with a dot (`BM_Parcel.StreetName`, a join) keeps the part after the last dot (`fields` lists it so; the caller notes "field X of a join, read as Y"). `{`/`}` in literals are doubled. Anything else (a function call, arithmetic, a comparison, `IIf`, `Split`) → `exact: false` with the first field.

- [ ] **Step 1: Write the failing tests**: `arcade_concatenations_become_templates` (`$feature.A + "\n " + $feature.B` → `"{A}\n {B}"`, exact; `"SEC. "+$feature.SECT_NO` → `"SEC. {SECT_NO}"`; `$feature['T.Name']` → `"{Name}"` with `fields == ["Name"]`), `vbscript_and_python_concatenations_become_templates` (`"BLK " & [BLOCK]` → `"BLK {BLOCK}"`; `[rods]&" RD"` → `"{rods} RD"`; `"A: " + !A! + vbNewLine + !B!` → `"A: {A}\n{B}"`; Python `"%s" % !X!` → `"X"` inexact), `function_calls_fall_back_to_the_first_field` (`Split($feature.OWNER, ' ')[0]` → `"OWNER"`, `exact: false`; `Return Concatenate([$feature.a, $feature.b], ' ')` → `"a"` inexact; `1 + 2` → None), `formatting_tags_and_braces` (`"<BOL>" + $feature.X + "</BOL>"` → `"{X}"`; `"{x}"` literal → `"{{x}}"`), `hostile_expressions_are_bounded` (a 100 000-character expression, unterminated quotes, nested brackets → no panic, result or None within 20 ms).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map cim::tests::arcade`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(cim): label expressions as templates`.

---

### Task 9: CIM labels: placement, callouts, stacking, halos

**Files:**
- Modify: `crates/map/src/cim/map.rs:448-500` (`label_class`), new `placement`, `callout`, `stacking` in `crates/map/src/cim/labels.rs` (create; `label_class` moves there)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Consumes: `LabelPlacement`, `LabelBackground`, `LABEL_OFFSET`, `MAX_WRAP_CHAR`; `symbol::polygon`, `symbol::line`.
- Produces (`pub(crate)` in `cim/labels.rs`): `fn placement(class: &Value, what: &str, report: &mut Report) -> (LabelPlacement, Option<f32> /* repeat */, [f32; 2] /* offset */)`: Maplex (`maplexLabelPlacementProperties`, preferred when present) by `featureType`: Point `AroundPoint` → `Auto`, `CenteredOnPoint` → `Center`, `NorthOfPoint` → `Above`, `SouthOfPoint` → `Below`, `EastOfPoint` → `Right`, `WestOfPoint` → `Left`, `NorthEastOfPoint` → `AboveRight`, `NorthWestOfPoint` → `AboveLeft`, `SouthEastOfPoint` → `BelowRight`, `SouthWestOfPoint` → `BelowLeft`, `RotationField` → `Auto` noted; Line `OffsetStraightFromLine`, `CenteredStraightOnLine`, `CenteredHorizontalOnLine`, `OffsetHorizontalFromLine` → `Parallel`, `OffsetCurvedFromLine`, `CenteredCurvedOnLine` → `Curved`; Polygon `HorizontalInPolygon`, `AlwaysHorizontal`, `BoundaryPlacement` → `Center`, `StraightInPolygon` → `Parallel`, `CurvedInPolygon` → `Curved`, `RepeatAlongBoundary` → `Curved` with `repeat: Some(repetitionInterval as 96ths, clamped to LABEL_REPEAT)`; `primaryOffset` (points) → `offset[1]` for point labels (sign by the zone). Standard (`standardLabelPlacementProperties`): `pointPlacementMethod` as Maplex's, `lineLabelPosition {parallel, horizontal, perpendicular, above, below, online, inLine}` → `Parallel` (curved when `produceCurvedLabels`), `polygonPlacementMethod` `AlwaysHorizontal`/`MixedStrategy` → `Center`, `AlwaysStraight` → `Parallel`. Noted once per layer: `canOverrunFeature`, `labelPriority`, zone priorities, `featureWeight`, key numbering, `offsetAlongLineProperties`, `thinDuplicateLabels`, `removeDuplicates`. `fn callout(text: &Value, what, report) -> (Option<LabelBackground>, bool /* halo kept */)`: `CIMBackgroundCallout {backgroundSymbol (polygon: fill, outline), margin {left, right, top, bottom} (points), cornerRadius}` → `LabelBackground { fill, stroke, stroke_width, padding: [left.max(right), top.max(bottom)] in 96ths, radius }`; `CIMSimpleLineCallout` → "a leader line, left out" (plain label); `CIMCompositeCallout` → the same. `fn stacking(class, what, report) -> Option<String>`: the first `labelStackingProperties.separators[]` whose `visible` is false (a separator that is dropped) → `wrap_char`, when the class `canStackLabel`; one that stays visible is noted ("stacking at X keeping it, drawn without"). `shadowColor` on a text symbol → noted "a text shadow, left out". The halo comes from `haloSize` and `haloSymbol` as now.

- [ ] **Step 1: Write the failing tests**: `maplex_placements_map_by_feature_type` (a table-driven test over every row above), `standard_placements_map_too`, `background_callouts_become_label_backgrounds` (a white fill, grey outline 0.5 pt, margins 2/2/1/1, radius 1 → `padding [2.67, 1.33]`, `stroke_width 0.67`), `line_callouts_and_shadows_are_noted`, `a_hidden_stacking_separator_becomes_the_wrap_character` (`separators: [{separator: " ", splitAfter: true}, {separator: ",", visible: true}]` → `wrap_char: Some(" ")`).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map cim`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(cim): label placement, callouts and stacking`.

---

### Task 10: The CIM map reader: the layer tree, every source, every layer type, layer and map properties

**Files:**
- Modify: `crates/map/src/cim/map.rs` (the whole module: `MapImport`, `LayerImport`, `Reader`, `connection`; new `LayerSource`, `Entry`, `GroupImport`, `TableImport`)
- Modify: `crates/map/src/cim/mod.rs:30` (re-exports)
- Modify: `crates/engine/src/cmd/arcgis_pro.rs:297-344` (`add_map` compiles against the new shape: for this task only, it flattens the tree as before; Task 11 rewrites it)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct MapImport { pub name: String, pub wkid: Option<u32>, pub extent: Option<[f64; 4]>, pub reference_scale: Option<f64> /* × QGIS_SCALE */,
      pub entries: Vec<Entry> /* top first */, pub tables: Vec<TableImport> }
  pub enum Entry { Layer(LayerImport), Group(GroupImport) }
  pub struct GroupImport { pub name: String, pub visible: bool, pub expanded: bool, pub transparency: Option<f32>, pub min_scale: Option<f64>, pub max_scale: Option<f64>, pub children: Vec<Entry> }
  pub struct LayerImport { pub name: String, pub visible: bool, pub expanded: bool, pub source: LayerSource,
      /// Where Pro read it, for people and for `SourceLayer.origin`: "<factory> <dataset> in <DATABASE>", "ArcGIS feature service <url> layer <id>", "an SDE geodatabase on <SERVER>, database <DATABASE>, table <dataset>" — never a password or user.
      pub origin: String, pub style: Style, pub definition_query: Option<String>, pub scale_symbols: bool /* takes the map's reference scale */,
      pub kind: LayerKind /* Feature | Raster | Tiles | Table-like, for notes and the broken layer's icon */ }
  pub enum LayerSource {
      File { path: String, table: Option<String> },           // Shapefile, FileGDB (a feature dataset's table is still `dataset`), Raster, Cad, KML (`CIMKMLDataConnection.uRI`)
      FeatureService { url: String, layer: String },           // URL=… and `dataset`
      MapService { url: String },                              // CIMAGSServiceConnection …/MapServer (tiles) or …/ImageServer
      Wms { url: String, layer: String },                      // CIMWMSServiceConnection + the first visible subLayers[].name
      VectorTiles { url: String },                             // CIMVectorTileDataConnection.uRI
      Unopenable,                                              // SDE, Excel, SQL query, anything else: broken with `origin`
  }
  pub struct TableImport { pub name: String, pub source: LayerSource, pub origin: String }
  pub fn read_map(doc: &Document, uri: &str, report: &mut Report) -> Result<MapImport, String>;
  pub fn read_map_def(doc: &Document, def: &Value, report: &mut Report) -> Result<MapImport, String>;
  /// A `.lyrx`'s layers (its `layers` list; groups kept), as a map with no name, projection or extent.
  pub fn read_layers(doc: &Document, uris: &[String], report: &mut Report) -> MapImport;
  pub(crate) fn scrub_connection(s: &str) -> String;  // KEY=VALUE pairs without PASSWORD, ENCRYPTED_PASSWORD*, TOKEN, USER keys
  ```
  Rules: `CIMGroupLayer` → `Entry::Group` (nested to `MAX_GROUP_DEPTH`; `visibility` is the group's own, not folded into children any more); `CIMRelQueryTableDataConnection` → `sourceTable`'s source with "a join to <destination dataset>, drawn without it"; a relative `DATABASE` (`.\IMG`, `..\data`) stays relative (the engine resolves against `dir`); `CIMRasterLayer` → `File` (the engine decides whether the format opens); `CIMTiledServiceLayer`/`CIMImageServiceLayer` → `MapService`; `CIMVectorTileLayer` → `VectorTiles`; `CIMAnnotationLayer`, `CIMDimensionLayer`, `CIMElevationSurfaceLayer`, `CIMKnowledgeGraphLayer`, others → `Unopenable` with "X layers, which RustGIS doesn't draw"; `featureTable.definitionExpression` → `definition_query` (kept as SQL; the engine checks it); `blendingMode` `Multiply`/`Screen`/`Overlay`/`Darken`/`Lighten`/`Difference` → `style.blend`, `Alpha` → Normal, others noted; `displayFilters` (when `enableDisplayFilters`), `layerMasks`, `featureReduction`, `symbolLayerDrawing.useSymbolLayerDrawing`, `charts`, `popupInfo`, `timeFilter`, `selectionSymbol` → noted once each per layer; `referenceScale` → `MapImport.reference_scale`; a layer's `scaleSymbols` → `scale_symbols`; `standaloneTables` → `tables`.

- [ ] **Step 1: Write the failing tests** (rewrite the existing three map tests for the new shape, then add): `groups_nest_with_their_own_visibility_and_settings` (a hidden group holding a visible layer and a subgroup → `Entry::Group { visible: false, children: [Layer { visible: true }, Group …] }`), `every_connection_kind_has_a_source_and_an_origin` (a table-driven test over Shapefile, FileGDB, FeatureDataset FileGDB, Raster relative, Cad, KML, FeatureService, AGS MapServer, ImageServer, WMS, vector tiles, SDE, Excel, SqlQuery, RelQuery → the `LayerSource` and `origin` given in the Interfaces), `origins_never_carry_passwords` (the SDE string with `ENCRYPTED_PASSWORD_UTF8=…;ENCRYPTED_PASSWORD=…;SERVER=s;INSTANCE=…;DATABASE=d;USER=sa;VERSION=…` → origin contains `s`, `d`, the table, and neither `PASSWORD`, `USER`, `sa` nor the hex), `layer_properties_and_the_map_reference_scale_carry_over` (`blendingMode: Multiply`, `definitionExpression`, `expanded: false`, `scaleSymbols: false`, map `referenceScale: 24000` → `reference_scale == Some(24000.0 * QGIS_SCALE)`), `other_layer_types_and_tables_are_read` (annotation → Unopenable with its note; a standalone table on a FileGDB → `tables[0]`), `a_lyrx_reads_its_layers` (`read_layers` on a `CIMLayerDocument` whose `layers` is one group of two feature layers), `hostile_maps_never_panic` (the existing fuzz loop over `hostile_map_base`, extended with the new keys).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-map cim`, `cargo test -p rustgis-engine pagx` (the layout path still works, flattening), `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(cim): the layer tree, every data source and layer kind`.

---

### Task 11: The engine adds a Pro map: groups, sources, broken layers, tables, view

**Files:**
- Modify: `crates/engine/src/cmd/arcgis_pro.rs:297-344` (`add_map` → `add_map_entries`, `add_entry`, `add_source`, `broken_layer`, `settle`), `:85-100` (`embedded_maps`), `:205-235` (`bind_frames` unchanged), `crates/engine/src/cmd/qgis.rs:198` (`check_fields` uses `LabelSource::resolve`, Task 3), `:118` (`Geographic` becomes `pub(crate)`)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- Consumes: Task 10's `MapImport`/`Entry`/`LayerSource`; Task 4's `SourceLayer::broken`, `ServiceOrigin`, `Style::apply_all`; commands `map.new`, `map.projection`, `group.add {name, parent, layers}`, `group.set {group, visible, transparency}`, `group.expand`, `group.scaleRange`, `layer.addData {path, name, layer}`, `service.add {url, type, layer}` (an ArcGIS feature layer is `layer: "<id>"`, a MapServer's tiles `layer: "map"`, an ImageServer `layer` left out), `map.addTiles {url, kind: "style", name, overlay: true}`, `layer.referenceScale`, `layer.expand`.
- Produces:
  ```rust
  pub(crate) struct Added { pub name: String /* the project map's name */, pub layers: usize, pub broken: usize }
  /// Adds `m` as a new map (its projection, groups, layers top to bottom, tables, view) and returns its name; the new map is active afterwards.
  pub(crate) fn add_map(s: &mut Session, m: &MapImport, dir: Option<&str>, camera: Option<(f64, f64, f64)>, report: &mut Report) -> Result<Added, String>;
  /// Adds `m`'s entries to the active map (a `.lyrx`, or a `.mapx`'s layers into an existing map).
  pub(crate) fn add_entries(s: &mut Session, m: &MapImport, dir: Option<&str>, group: Option<u32>, report: &mut Report) -> Added;
  ```
  Rules. The new map draws its layers alone: `basemap = Blank`, `graticule = false` (as now). Projection: `wkid` 3857/102100 → `mercator`; a geographic code (`qgis::Geographic::of(&format!("EPSG:{w}"))`) → `equirectangular`; else `EPSG:{latestWkid}`, a failure noted and Web Mercator kept. Order: entries are added bottom first, so each lands on top; a group is `group.add` with its children's ids after they are added (children added first, bottom first), then `group.set`/`group.expand`/`group.scaleRange`. Sources: `File` → `layer.addData {path: resolve(path, dir), name, layer: table}`; on `Err` → a broken layer with `path`/`table` kept so Repair works, the error as `error`; a relative path with no `dir` is broken too ("a relative path, and no folder to find it from"). `FeatureService` → `service.add {url, type: "arcgis", layer}`; on `Err` → a broken layer with `service: ServiceOrigin { url, kind: "arcgis", layer }` and the error (a secured service says so). `MapService` → `service.add {url, type: "arcgis", layer: "map"}` (ImageServer: without `layer`); its result is a tile layer: `visible`, `transparency` and `blend` set on `s.map_mut().tiles[index]` as `qgis::web_settings` does; a tile layer can't join a group: noted "a web layer, listed outside its group X". `Wms` → `service.add {url, type: "wms", layer, name}`. `VectorTiles` → `map.addTiles {url, kind: "style", name, overlay: true}`. `Unopenable` → broken. A broken layer: `s.map_mut().add_streamed(name, SourceLayer::broken(origin, path, table, service, error))`, then `style.apply_all`, `l.visible`, `l.expanded`, `l.definition_query = query` (unchecked: there are no fields yet; `layer.retry`/Repair check it). An opened feature layer: `style.apply`, `visible`, `expanded`, `check_fields`, `definition_query` through `rustgis_data::query::parse` (a failure noted and dropped, as the QGIS importer does), `reference_scale` = the map's when `scale_symbols`. Tables: `File` → `layer.addData` (a `GeometryType::None` dataset becomes a project table); others noted. View: `camera` (from `Document::view_camera`) → the map's centre and scale through the map's projection (`Map::to_lonlat` on `(x, y)`; `scale` → `camera.res` as `layout.frameZoom` converts), else `defaultExtent` as `go_to_extent` does now (generalised from 4326/3857 to any `wkid` RustGIS projects through `to_lonlat`). Group visibility: Pro's `visibility` on a group is RustGIS's `LayerGroup.visible`; a child's own `visibility` is its `visible`. `reusable` (the layout path's reuse of an earlier import) compares the flattened file list as now.

- [ ] **Step 1: Write the failing tests** in `engine/src/tests.rs` (fixture: a `CIMMapDocument` string `MAPX` with a group "Boundaries" (hidden, expanded false) holding the sample shapefile's path twice under different names, a FeatureService layer at `https://127.0.0.1:9/arcgis/rest/services/P/FeatureServer` dataset `0`, an SDE layer, a vector tile layer, a raster layer on a missing `.tif`, a standalone table that doesn't exist, `referenceScale: 50000`, a definition query `pop_est > 1000` on one shapefile layer and `nope = 1` on the other): `a_pro_map_adds_its_groups_layers_and_broken_layers` (map named as the document; `groups[0].name == "Boundaries"`, `!visible`, `!expanded`, both shapefile layers its children; the service layer broken with `source.service.url` ending `/FeatureServer`, `origin` naming the URL; the SDE layer broken with `origin` naming server, database and table and no password; the raster broken; the vector tiles in `tiles` with `kind` style; `reference_scale == Some(50000.0 * QGIS_SCALE)` on both shapefile layers; the first query kept, the second noted and None; `project.tables` unchanged and the table noted), `a_pro_map_opens_at_its_saved_camera` (camera `(x, y, 100000)` in EPSG:3857 → the map centre within 1e-6 degrees of the converted point), `a_label_template_with_a_missing_field_is_noted_not_dropped` (the fixture's first shapefile layer labels with `$feature.name + " " + $feature.GONE`: the class stays visible with field `"{name} {GONE}"`, a note names `GONE`; a layer labelling with `$feature.GONE` alone has its class switched off with a note), `the_layout_path_now_keeps_groups` (the Part 2 `PAGX` fixture's embedded map: its group comes in as a `LayerGroup`, no "listed one by one" note).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-engine pro_map`, `cargo test -p rustgis-engine pagx`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(engine): add ArcGIS Pro maps with their groups, sources and broken layers`.

---

### Task 12: `project.importArcgis` and `project.inspectArcgis`

**Files:**
- Modify: `crates/engine/src/cmd/arcgis_pro.rs` (`commands()`, new `inspect_project`, `import_project`, `project_document`, `import_layout_in`; `import_layout` refactored to call `import_layout_in(s, &doc, def, bound, dir, p)`), `crates/engine/src/cmd/plugins.rs:275` area (the list of project-changing commands plugins may run, if `project.importQgis` is listed there add this one)
- Modify: `crates/engine/src/tests.rs` (tests; `hostile_params_never_panic` junk: `{"path": "/nonexistent/x.aprx"}`, `{"data": "UEsDBAo="}` (a zip header alone), `{"text": "{\"type\":\"CIMGISProject\"}"}`, `{"text": "{\"type\":\"CIMMapDocument\",\"mapDefinition\":7}"}`, `{"maps": 7}`, `{"maps": [null, 1e308, ""]}`, `{"layouts": {"a": 1}}`)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  // "project.inspectArcgis": {"path"?: "*.aprx|*.mapx|*.lyrx", "text"?: "a .mapx or .lyrx file's text", "data"?: "an .aprx, .mapx or .lyrx file, base64"}
  //   → {"kind": "project"|"map"|"layers", "maps": [{"name", "open": bool (its pane was open when saved; the first open one is the one shown), "arcmap": bool}], "layouts": [{"name", "maps": [names its frames show], "arcmap": bool}]}
  // "project.importArcgis": {"path"?, "text"?, "data"?, "maps"?: [names] (an .aprx: the maps to import; none given and none of layouts: nothing is imported and the result lists what the file holds, as inspect does), "layouts"?: [names] (an .aprx: layouts to import; a layout's maps are imported with it), "dir"?: "the folder its relative paths start from (default: the file's)"}
  //   → {"maps": [names added], "layouts": [names added], "layers": n, "broken": n, "notes": [...]} — and, when nothing was asked for, inspect's fields too
  ```
  Reading: `path` ends `.aprx` or the bytes start `PK` → `Document::from_zip` (natively read with a `MAX_PROJECT` of 512 MiB; the web passes `data`); else text → `Document::from_json`. Kind by `doc.kind()`: `Project` → items; `Map` → one map named as its `mapDefinition` (`maps` ignored); `Layer` → `add_entries` into the active map (a new map is not made); anything else → `Err("not an ArcGIS Pro project, map or layer file")`. For an `.aprx`: each name in `maps`/`layouts` matched case-insensitively against `items()` (unknown → `Err("no map named X")` before anything changes; an `arcmap` item → `Err("X was saved by ArcMap; open it in ArcGIS Pro first")`); the maps of every ticked layout (its frames' `uRI`s) are added to the set; maps are imported first (`add_map` with `doc.view_camera(uri)`), each giving `bound[uri_key] = name`; then each layout through `import_layout_in` with that `bound` (the frames bind to the maps just imported), pictures from `doc.binary`; the active map is the first imported map, the active layout the first imported layout. Notes are keyed by layer name as now; the notice reads "Imported {maps} maps and {layouts} layouts: {layers} layers, {broken} broken; {approximated} things drawn differently, {skipped} left out". Everything that can fail on the input (document, names) fails before the project changes.

- [ ] **Step 1: Write the failing tests** (fixture `APRX` built in the test with `rustgis_data::zip::write`: `GISProject.json` with items Map "Main", Map "Detail", Layout "Sheet", Map "Old" at an `.xml` path; `views` opening "Detail"; `main/main.json` a `CIMMap` with one shapefile layer (the sample's) and one broken SDE layer; `detail/detail.json` with one shapefile layer; `layout/sheet.json` a `CIMLayout` with a frame on `CIMPATH=main/main.json` and a picture `referenceURI: CIMPATH=Pictures/p.png`; `Pictures/p.png` a 2×2 PNG): `inspecting_a_project_lists_maps_layouts_and_the_open_map` (`kind: project`, `maps` 3 long, "Detail" `open: true`, "Old" `arcmap: true`, `layouts[0].maps == ["Main"]`), `importing_chosen_maps_and_a_layout_binds_the_frames` (`maps: ["Detail"], layouts: ["Sheet"]` → maps added `["Detail", "Main"]` (Main pulled in by the layout), the layout's frame shows "Main", the picture element has an image, `broken: 1`), `importing_nothing_lists_the_holdings` (no `maps`/`layouts` → `maps: []` added and the inspect fields; the project unchanged), `an_arcmap_item_is_refused_before_the_project_changes`, `a_mapx_imports_one_map_and_a_lyrx_adds_to_the_active_map`, `hostile_project_files_never_panic` (a zip with no `GISProject.json`, a `GISProject.json` of `[]`, an item whose `catalogPath` is 10 000 characters, a `views` of 10 000 entries, a layout entry whose `elements` is a number).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-engine arcgis`, `cargo test -p rustgis-engine hostile_params`, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(engine): import ArcGIS Pro projects, maps and layer files`.

---

### Task 13: Desktop and web wiring: the picker, drops, the Browser pane, the start page

**Files:**
- Modify: `crates/ui-egui/src/dialogs.rs:11-57` (`Dialogs` gains `arcgis_path: String`, `arcgis_data: Option<String>` (base64, the web), `arcgis_maps: Vec<(String, bool, bool /* arcmap */)>`, `arcgis_layouts: Vec<(String, Vec<String>, bool, bool)>`), `:74-194` (`open("importArcgisPro")`), `:278` (title "Import ArcGIS Pro Project"), `:522` (body `import_arcgis_pro`), new `import_arcgis_pro(app, ui) -> bool` and `pub(crate) fn finish_arcgis_import(app) -> Result<(), String>`
- Modify: `crates/ui-egui/src/lib.rs` (new `pub(crate) fn import_arcgis_file(&mut self, path: &str) -> Result<Value, String>`: runs `project.inspectArcgis {path}`; `kind: project` opens the picker; `map`/`layers` runs `project.importArcgis {path}` at once), `:1368-1489` (`open_bytes_with`: `.aprx`, `.mapx`, `.lyrx` → inspect with `data`/`text`, then the picker (with `arcgis_data`) or the import), `:2866-2899` (`ui.openItem`: `"arcgisProject" => ("project.importArcgis", &["aprx", "mapx", "lyrx"], "ArcGIS Pro project, map or layer file")`, the desktop branch calling `import_arcgis_file`, the web `open("arcgisProject")`), `:3883-3925` (drops: `.aprx`/`.mapx`/`.lyrx` → `import_arcgis_file`), `crates/ui-egui/src/panes.rs:2568-2624` (Browser pane lists and opens the three extensions through `import_arcgis_file`), `crates/ui-egui/src/backstage.rs:96-137` (card "ArcGIS Pro Project…", "An .aprx project's maps and layouts, or a .mapx or .lyrx", `ui.openItem {kind: "arcgisProject"}`), `crates/ui-egui/src/chrome.rs:764-770` (`"project.importArcgis" => OpenItem("arcgisProject")`, `"project.inspectArcgis"` hidden from search), `apps/rustgis-web/src/web.rs:931-995` (`open_async("arcgisProject")` → `["aprx", "mapx", "lyrx"]`)
- Test: `crates/ui-egui/src/lib.rs` (a `mod aprx_tests` beside `pagx_tests`), `crates/ui-egui/src/dialogs.rs`

**The picker (user):** a list of the maps, each a checkbox (an `arcmap` one disabled, with "saved by ArcMap; open it in Pro first" as its hover text and greyed), then the layouts the same way; "All" and "None" as link-styled buttons above; ticking a layout ticks the maps it shows (unticking a layout leaves them); the first `open` map ticked by default, nothing else; Import disabled until something is ticked; Import runs `project.importArcgis {path|data, maps, layouts}` through `finish_arcgis_import`, Cancel closes. Width 480.

- [ ] **Step 1: Write the failing tests**: `an_aprx_file_opens_the_picker_with_the_open_map_ticked` (write the `APRX` fixture bytes to a temp file; `app.import_arcgis_file(&path)` → `dialogs.open == Some("importArcgisPro")`, `arcgis_maps` has "Detail" ticked and "Old" marked arcmap; `finish_arcgis_import` adds the map), `ticking_a_layout_ticks_its_maps` (kittest: click "Sheet", then "Main" is ticked; click "Import"; the project has both maps and the layout), `a_dropped_aprx_opens_the_picker_and_a_dropped_lyrx_adds_layers` (the `Dropped` harness from `pagx_tests`), `aprx_bytes_open_the_picker_on_the_web_path` (`open_bytes_with("p.aprx", bytes, None)` → the picker with `arcgis_data` set; `finish_arcgis_import` imports from `data`), `the_browser_pane_lists_pro_files` (the extension list contains "aprx", "mapx", "lyrx").
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-ui-egui aprx`, `cargo xtask ci`, and `cargo build --target wasm32-unknown-unknown -p rustgis-web` where clang is available (note in the commit when it isn't; CI builds it). - [ ] **Step 5: Commit** `feat(ui): open ArcGIS Pro projects, maps and layer files`.

---

### Task 14: Documentation

**Files:**
- Modify: `docs/user-guide/workspace.md:193-230` (the "ArcGIS Pro layouts" subsection becomes "ArcGIS Pro projects, maps and layouts": the picker, what a map brings (the table of sources → what RustGIS does, including broken layers and Retry/Repair), the accepted differences (placement engines, label expressions beyond concatenation, size by scale, joins, display filters, picture fills, leader lines, shadows, tile layers outside groups), the command line example `rustgis-cli run --cmd 'project.importArcgis={"path": "…/Site.aprx", "maps": ["Main Map"]}'`)
- Modify: `docs/user-guide/cartography.md` (drawn markers under point symbols: "Custom", how they come in; label expressions: the Field/Expression switch, `{Field}` templates, `{{` for a brace), `docs/user-guide/data.md:1015` area (the formats table: `.aprx`, `.mapx`, `.lyrx` rows), `docs/user-guide/maps.md` (broken layers: the warning glyph, Repair Data Source…, Retry; what a project keeps), `GEOLIBRE_PARITY.md:228-230` (ArcGIS Pro maps: done; `.stylx` pending)
- Modify: `crates/engine/src/cmd/arcgis_pro.rs`, `retry.rs`, `layer.rs` (`labels.field`, `layer.marker`) params docs (re-read for consistency with the docs)

- [ ] **Step 1: Write the docs.** - [ ] **Step 2: Build them** (`zensical build`, or the command `zensical.toml` names) and read the rendered pages. - [ ] **Step 3: Commit** `docs: ArcGIS Pro map import, broken layers, drawn markers and label templates`.

---

### Task 15: Verification with the test project

**Files:**
- Create: `docs/superpowers/plans/2026-11-01-arcgis-pro-map-import-checklist.md` (the results, committed once filled; generic names only, per the privacy rule)
- Use: the user's test project `.aprx` on the user's drive (mounted), the ArcGIS portal signed in (`arcgis.signIn`), the user's screenshots of the same maps in Pro at the same scale

- [ ] **Step 1: Import the main map** with the desktop app (`PATH="$HOME/.cargo/bin:$PATH" cargo run --release -p rustgis`): drop the `.aprx` on the window, tick the main map in the picker, Import. Note the notice and every note (`project.importArcgis`'s `notes` through the CLI for a full list).
- [ ] **Step 2: Compare the Layers pane** with Pro's Contents: every layer and group present, same tree, same names, same ticks, same expansion; broken layers marked with the warning glyph, each tooltip naming where Pro read it; SDE and CAD layers broken with their origin; service layers drawn or broken with the right URL (sign in, Retry, they open).
- [ ] **Step 3: Compare the drawing** at three scales (a township, a section, a parcel) against Pro screenshots: colours, widths, dash patterns, markers (the drawn and stacked ones in particular), fills, labels (text, font, size, halo, placement class, stacking), definition queries (the same features drawn).
- [ ] **Step 4: Import a layout with its maps** from the picker; check its frames show the imported maps and its pictures come in.
- [ ] **Step 5: Headless check** for the pull request: `rustgis-cli run --cmd 'project.importArcgis={"path": "<the .aprx>", "maps": ["<the main map>"]}' --cmd 'share.exportMap={"path": "out.png"}'`.
- [ ] **Step 6: Fix** what differs, each fix with a regression test in the task that owns the code, and re-run the comparison.
- [ ] **Step 7: Commit** the filled checklist: `docs: ArcGIS Pro map import verification`.

---

## Self-review notes

- **Spec coverage.** Part 3 entry points (`project.importArcgis`, the picker, ArcMap items greyed, drop, Browser) → Tasks 12, 13; Maps (name, projection, extent/camera, reference scale, blank basemap, groups) → 10, 11; Layer sources table (FileGDB, Shapefile, Raster, FeatureService, AGS tiled/image, vector tiles, SDE, Cad, RelQuery, else) → 10, 11; relative paths against `dir` → 11; Broken layers (origin, warning glyph, Repair, Retry, reopen after sign-in, saved broken) → 4, 5, 11; Renderers table → 7 (Simple/Unique were Part 2); Symbols (fills, strokes, effects were Part 2; vector marker fill → Dots, line markers, Path marker, picture marker, stacks) → 6, 2; Labels (classes, halo, callout → background, expressions as templates, placement table, stacking → wrap_char, noted properties) → 3, 8, 9; Other layer properties (name, visibility, scales, transparency, blend, definition query, noted ones, standalone tables) → 10, 11; Verification → 15; `.lyrx`/`.mapx` → 1, 10, 12. "Left to the builder" decided here: a stacked point symbol is a `Path` marker holding the stack (Task 6, so the stylx follow-on and the north arrow get the same); the Arcade subset is concatenation, literals and newlines (Task 8); the picker's look (Task 13); the warning glyph (Task 5). Not planned, by design: `.stylx`, the label engine, multi-field unique values as a combined field (noted, drawn by the first field, as Part 2 does).
- **Execution order:** 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15. Tasks 2–5 are independent of each other and of 1; 6–9 depend on 2, 3; 10 depends on 6–9; 11 on 4, 10; 12 on 1, 11; 13 on 12.
- **Types:** `LayerSource`, `Entry`, `GroupImport`, `TableImport` (Task 10) are what Task 11 consumes; `SourceLayer::broken`, `ServiceOrigin`, `Style::apply_all` (Task 4) are what Tasks 5 and 11 consume; `LabelSource`/`is_template`/`template_fields` (Task 3) are what Tasks 8, 11 and the render consume; `PathShape`/`PathPart`/`PathRef` (Task 2) are what Task 6 consumes; `Document::{items, open_maps, view_camera, map_definition, layer_uris}`, `read_layout_def`, `read_map_def` (Task 1) are what Task 12 consumes.
- **Review Focus** tests are placed: 1 → Task 10, 2 → Task 12, 3 → Tasks 3 and 11, 4 → Task 4, 5 → Task 2.
- **Proportion:** the plan is about one and a half times the spec's Part 1 + Part 3 text; the code it describes is many times longer.
