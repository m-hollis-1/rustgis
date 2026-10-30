# ArcGIS Pro Layout Import (.pagx) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Import Layout reads ArcGIS Pro `.pagx` files and reproduces them in RustGIS, with the additions to RustGIS's layout model the templates need (linked frames, frame-bound dynamic text, groups, borders, legend fitting, scale bar details, picture north arrows, paragraph text).

**Architecture:** A new `rustgis_map::cim` module reads Esri CIM JSON (a `.pagx`, `.mapx`, `.lyrx` or an `.aprx` zip) into map and layout types and reports what it couldn't carry over through the QGIS importer's `Report`. The layout model, renderer, commands and UI gain the fields the design requires first, so the reader has somewhere to put what it reads. The `layout.import` command detects a `.pagx` and routes it through `cmd/arcgis_pro.rs`, which binds frames to maps, adds the embedded detail map, reads picture files and returns notes.

**Tech Stack:** Rust, `serde_json` (already a dependency of every crate), `rustgis_data::zip`, tiny-skia renderer, egui UI, `image` crate (`gif` feature in the engine already).

**Spec:** `docs/superpowers/specs/2026-10-30-arcgis-pro-import-design.md` (Parts 1 and 2; Part 3, the map import, is a later plan).

## Global Constraints

- `AGENTS.md`: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` in non-test code; no `unsafe`; `get()` over indexing; clamp sizes and bound recursion and lists; `cargo xtask ci` (fmt, clippy `-D warnings`, tests, layers) green before every commit.
- Everything below `apps/` builds for `wasm32-unknown-unknown`: no `std::fs` in `crates/map`; file reading happens in `crates/engine` behind `#[cfg(not(target_arch = "wasm32"))]` as `cmd/qgis.rs::read_file` does.
- Every new field on `Element`, `ElementKind` variants, `Symbol`/`Marker`, `TokenValues` and `Layout` has `#[serde(default)]` and `skip_serializing_if` for its default, so layouts and projects saved before read unchanged and save the same bytes. Test this for each (a task's "round trip" test).
- CIM sizes are 1/72-inch points; RustGIS widths and marker sizes are 1/96-inch: multiply by `96.0 / 72.0`. Layout `Text.size` and `letter_spacing` are true points: no factor.
- Page coordinates: CIM is page units from the bottom-left; RustGIS is millimetres from the top-left: `y_mm = (page_h - (y + h)) * unit_mm`.
- Interface text through `tl!("…")` and `i18n::fmt`; command ids and parameters never translated.
- `cargo` is at `~/.cargo/bin`: run tool shells with `PATH="$HOME/.cargo/bin:$PATH"`.
- Test fixtures are small synthetic CIM JSON strings in the test modules, never Mike's files from `G:`.
- Commit after every task with a conventional message (`feat(layout): …`, `feat(cim): …`, `docs: …`) ending in `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. **A `.pagx` whose element is rotated by a non-right angle** (say 30°): the import must keep the element (bounding box, note), never drop it or produce a NaN box. Test in Task 15.
2. **Dynamic text naming a frame that isn't on the page** (`<dyn type="mapFrame" name="GONE" property="scale"/>`): expands to nothing with a note; the text element is still created. Test in Task 16.
3. **A linked frame cycle** (A links B, B links A) saved by hand or crafted: the renderer must terminate and `clamp` must drop the second link. Test in Task 9.
4. **A group whose child id is missing or is the group itself**: `clamp` drops the reference; `layout.ungroup` on a non-group errors; deleting a group deletes only existing children. Test in Task 10.
5. **A text element with `wrap` on in a box narrower than one word**: the renderer must not loop; the word overflows on one line. Test in Task 7.

---

### Task 1: The CIM document reader

**Files:**
- Create: `crates/map/src/cim/mod.rs`
- Create: `crates/map/src/cim/tests.rs`
- Modify: `crates/map/src/lib.rs:21` (add `pub mod cim;` after `pub mod qgis;`)

**Interfaces:**
- Consumes: `rustgis_data::zip::read_matching_within(bytes, want, budget) -> Result<Vec<(String, Vec<u8>)>, String>`; `crate::qgis::{Report, Note, NoteKind}`.
- Produces:
  ```rust
  pub use crate::qgis::{Note, NoteKind, Report};
  pub const MAX_JSON: usize = 64 << 20;
  pub enum DocKind { Layout, Map, Layer, Project, Other(String) }
  pub struct Document { /* private */ }
  impl Document {
      pub fn from_json(text: &str) -> Result<Document, String>;   // a .pagx, .mapx or .lyrx
      pub fn from_zip(bytes: &[u8]) -> Result<Document, String>;  // an .aprx: GISProject.json is the root
      pub fn kind(&self) -> DocKind;
      pub fn root(&self) -> &Value;
      pub fn resolve(&self, uri: &str) -> Option<&Value>;  // "CIMPATH=…" → the definition
      pub fn binary(&self, uri: &str) -> Option<&str>;     // a binaryReferences entry's `data`
  }
  pub(crate) fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str>;
  pub(crate) fn num(v: &Value, k: &str) -> Option<f64>;      // finite only
  pub(crate) fn flag(v: &Value, k: &str, default: bool) -> bool;
  pub(crate) fn typed<'a>(v: &'a Value, ty: &str) -> Option<&'a Value>;  // v when v["type"] == ty
  ```
  `resolve` looks up, case-insensitively, the `uRI` of entries in the root's `mapDefinitions`, `layerDefinitions` and `elevationSurfaceLayerDefinitions`, then (for a zip) a zip entry whose name matches the path after `CIMPATH=`. `kind` reads `root["type"]`: `CIMLayoutDocument`, `CIMMapDocument`, `CIMLayerDocument`, `CIMGISProject`.

- [ ] **Step 1: Write the failing tests** in `crates/map/src/cim/tests.rs` (declared `#[cfg(test)] mod tests;` in `cim/mod.rs`)

```rust
#[test]
fn a_layout_document_resolves_its_embedded_maps_layers_and_binaries() {
    let text = r#"{"type":"CIMLayoutDocument","version":"3.7.0",
      "binaryReferences":[{"type":"CIMBinaryReference","uRI":"CIMPATH=Pictures/a.png","data":"iVBORw0KGgo="}],
      "layerDefinitions":[{"type":"CIMFeatureLayer","name":"L","uRI":"CIMPATH=NEW_DETAIL_MAP/L.json"}],
      "mapDefinitions":[{"type":"CIMMap","name":"M","uRI":"CIMPATH=Map/Map2.json","layers":["CIMPATH=NEW_DETAIL_MAP/L.json"]}],
      "layoutDefinition":{"type":"CIMLayout","name":"P","elements":[]}}"#;
    let d = Document::from_json(text).unwrap();
    assert!(matches!(d.kind(), DocKind::Layout));
    assert_eq!(d.resolve("CIMPATH=map/map2.json").unwrap()["name"], "M");
    assert_eq!(d.resolve("CIMPATH=NEW_DETAIL_MAP/L.json").unwrap()["name"], "L");
    assert!(d.resolve("CIMPATH=nothing.json").is_none());
    assert_eq!(d.binary("CIMPATH=Pictures/a.png"), Some("iVBORw0KGgo="));
}

#[test]
fn a_project_zip_resolves_entries_by_path() {
    let project = r#"{"type":"CIMGISProject","projectItems":[{"type":"CIMProjectItem","itemType":"Map","name":"MAIN MAP","catalogPath":"CIMPATH=main_map/main_map.json"}]}"#;
    let map = r#"{"type":"CIMMap","name":"MAIN MAP","layers":[]}"#;
    let zip = rustgis_data::zip::write(&[("GISProject.json".into(), project.into()), ("main_map/main_map.json".into(), map.into())]).unwrap();
    let d = Document::from_zip(&zip).unwrap();
    assert!(matches!(d.kind(), DocKind::Project));
    assert_eq!(d.resolve("CIMPATH=MAIN_MAP/MAIN_MAP.json").unwrap()["name"], "MAIN MAP");
}

#[test]
fn hostile_documents_are_refused_not_crashed() {
    assert!(Document::from_json("not json").is_err());
    assert!(Document::from_json(r#"{"type":"CIMLayoutDocument"}"#).is_ok()); // no definitions is fine
    assert!(Document::from_json(&"[".repeat(100_000)).is_err());
    assert!(Document::from_zip(b"PK\x03\x04junk").is_err());
    let big = format!(r#"{{"type":"CIMLayoutDocument","pad":"{}"}}"#, "x".repeat(MAX_JSON + 1));
    assert!(Document::from_json(&big).is_err());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p rustgis-map cim:: 2>&1 | tail -20`
Expected: compile error, `cim` not found.

- [ ] **Step 3: Implement `cim/mod.rs`** with the interface above. `from_json` refuses text over `MAX_JSON` before parsing. `from_zip` reads every `.json` entry under a `MAX_JSON` budget with `read_matching_within`, requires `GISProject.json` (any case) and stores the other entries' parsed JSON by lower-cased name. Index definitions in a `HashMap<String, Value>` keyed by lower-cased `uRI`. A module doc comment says what CIM is and that the input is hostile, as `qgis/mod.rs` does.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p rustgis-map cim:: 2>&1 | tail -20`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/map/src/cim crates/map/src/lib.rs
git commit -m "feat(cim): read ArcGIS Pro CIM documents and projects"
```

---

### Task 2: Colours, units and symbol references

**Files:**
- Create: `crates/map/src/cim/color.rs`
- Create: `crates/map/src/cim/units.rs`
- Modify: `crates/map/src/cim/mod.rs` (declare `mod color; mod units;`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  // color.rs
  pub(crate) fn color(v: &Value) -> Option<Rgba>;   // CIMRGBColor (values 0–255, alpha 0–100), CIMCMYKColor (0–100 each), CIMHSVColor, CIMHSLColor, CIMGrayColor, CIMLABColor
  pub(crate) fn symbol_of<'a>(v: &'a Value) -> Option<&'a Value>;  // v["symbol"] when v is a CIMSymbolReference, else v itself when it has symbolLayers
  // units.rs
  pub(crate) fn page_unit_mm(page: &Value) -> Option<f64>;  // from page["units"]["uwkid"]: 109008 inch = 25.4; the millimetre, centimetre and point codes from Esri's linear-unit table (confirm against esri's documentation; the templates only use 109008); None when unknown
  pub(crate) fn pt96(v: f64) -> f32;          // v * 96 / 72, non-finite → 0, clamped 0..=1000
  pub(crate) fn text_pt(v: f64) -> f64;       // clamped to crate::graphics::FONT_SIZE
  ```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn cim_colours_become_rgba() {
    let rgb = json!({"type":"CIMRGBColor","values":[255, 128, 0, 50]});
    assert_eq!(color::color(&rgb), Some(Rgba::new(255, 128, 0, 128)));
    let cmyk = json!({"type":"CIMCMYKColor","values":[0, 100, 100, 0, 100]});
    assert_eq!(color::color(&cmyk), Some(Rgba::new(255, 0, 0, 255)));
    let gray = json!({"type":"CIMGrayColor","values":[50, 100]});
    assert_eq!(color::color(&gray).map(|c| c.r), Some(128));
    assert_eq!(color::color(&json!({"type":"CIMRGBColor","values":[1]})), None);
    assert_eq!(color::color(&json!({"type":"CIMRGBColor","values":["a","b","c"]})), None);
}

#[test]
fn page_units_and_point_sizes_convert() {
    assert_eq!(units::page_unit_mm(&json!({"units":{"uwkid":109008}})), Some(25.4));
    assert_eq!(units::page_unit_mm(&json!({"units":{"uwkid":1}})), None);
    assert!((units::pt96(1.0) - 96.0 / 72.0).abs() < 1e-6);
    assert_eq!(units::pt96(f64::NAN), 0.0);
    assert_eq!(units::text_pt(1000.0), 400.0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p rustgis-map cim:: 2>&1 | tail -20`
Expected: compile error, `color`/`units` not found.

- [ ] **Step 3: Implement** the two files. CMYK → RGB: `r = 255 (1 - c/100)(1 - k/100)` and likewise. HSV and HSL use the standard conversions (write them; `crate::color` may already hold one, reuse if so). LAB → sRGB through XYZ (D65). Alpha is the last value (0–100) in every kind.

- [ ] **Step 4: Run to verify pass**, then **Step 5: Commit** `feat(cim): colours and units`.

---

### Task 3: An image marker kind

**Files:**
- Modify: `crates/map/src/symbology.rs:12-46` (`Marker`, add `Image(ImageRef)`; `ImageRef` after `FontName`), `Marker::label`
- Modify: `crates/render/src/lib.rs:1449` `marker_path`, `:1496` `marker_reach`, `:1504` `draw_marker`, `:2570` `marker_outline`; `crates/render/src/mesh.rs:283,344`; `crates/render/src/layout.rs:1472` `swatch`
- Modify: `crates/ui-egui/src/panes.rs:485` `patch`
- Modify: `crates/engine/src/cmd/layer.rs:245` (`layer.marker` takes `"image"`: base64 PNG, or `"imagePath"` natively)
- Test: inline tests in `symbology.rs`, `render/src/lib.rs`, `engine/src/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  /// An interned base64 PNG, as FontName interns names: Copy, bounded.
  pub struct ImageRef(&'static str);
  impl ImageRef {
      pub const MAX_BYTES: usize = 512 << 10;   // of base64 text
      pub const MAX_IMAGES: usize = 512;
      pub fn new(b64: &str) -> Option<ImageRef>; // None when too large, not base64, or the table is full
      pub fn as_str(&self) -> &'static str;
  }
  pub enum Marker { …, Image(ImageRef) }       // serialises as {"Image": "<base64>"}
  // render
  pub(crate) fn marker_image(r: &ImageRef) -> Option<Arc<Pixmap>>;  // decoded, cached by pointer in a Mutex<HashMap>
  ```
  `draw_marker` blits the image centred at (x, y) scaled so its longer side is `s.size * ppp` pixels, with `opacity`. `marker_path` returns a square path for `Image` (hit testing and the GPU fallback); `mesh.rs:344` refuses `Image` as it refuses `Char` so the CPU draws it. The legend `swatch` and the Layers-pane `patch` draw the image (egui: an `egui::TextureHandle` cached by pointer in the app's existing texture cache, or the image's average colour as a circle if that cache doesn't exist yet: say which in the commit).

- [ ] **Step 1: Write the failing tests**

```rust
// symbology.rs tests
#[test]
fn image_markers_intern_and_round_trip() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let m = Marker::Image(ImageRef::new(png).unwrap());
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(json, format!(r#"{{"Image":"{png}"}}"#));
    assert_eq!(serde_json::from_str::<Marker>(&json).unwrap(), m);
    assert!(ImageRef::new("***").is_none());
    assert!(ImageRef::new(&"A".repeat(ImageRef::MAX_BYTES + 4)).is_none());
    assert_eq!(m.label(), "Image");
}
// render/src/lib.rs tests
#[test]
fn an_image_marker_draws_its_pixels() {
    // a 2×2 red PNG, drawn at size 20 on a white 40×40 pixmap: the centre pixel is red
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p rustgis-map image_markers` fails to compile.

- [ ] **Step 3: Implement** the variant, the interning table (a `Mutex<Vec<&'static str>>` plus a `HashMap` for lookup, like `FontName`), the render cache and the swatch/patch arms, and the `layer.marker` parameter. Decode with `crate::picture::decode(b64)` (render/src/picture.rs:53).

- [ ] **Step 4: Run** `cargo test -p rustgis-map -p rustgis-render -p rustgis-engine marker` and `cargo xtask ci`. Expected: green, including `hostile_params_never_panic` (add `{"image": "***"}` and `{"image": 7}` to its junk list).

- [ ] **Step 5: Commit** `feat(symbology): image markers`.

---

### Task 4: The CIM symbol reader

**Files:**
- Create: `crates/map/src/cim/symbol.rs`
- Modify: `crates/map/src/cim/mod.rs` (declare `pub(crate) mod symbol;`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Consumes: Task 2 `color`, `symbol_of`, `pt96`, `text_pt`; Task 3 `ImageRef`; `crate::symbology::{Symbol, Marker, Glyph, LineStroke, LineCap, LineJoin, Dash}`; `crate::fills::{FillLayer, FillPattern}`; `crate::layout::TextAlign`.
- Produces:
  ```rust
  pub(crate) struct Paint { pub fill: Option<Rgba>, pub stroke: Option<(Rgba, f32)>, pub strokes: Vec<LineStroke>, pub fills: Vec<FillLayer> }
  pub(crate) fn polygon(sym: &Value, what: &str, report: &mut Report) -> Paint;
  pub(crate) fn line(sym: &Value, what: &str, report: &mut Report) -> Paint;
  pub(crate) fn point(sym: &Value, what: &str, report: &mut Report) -> Vec<Symbol>;  // bottom to top, one per enabled marker layer
  // VAlign and TextCase are Task 7's `crate::layout::{VAlign, TextCase}` (CIM "Top" | "Center" | "Bottom" | "Baseline" → Top | Middle | Bottom | Bottom): Task 7 lands before this task.
  pub(crate) struct Text { pub font: Option<String>, pub bold: bool, pub italic: bool, pub size: f64, pub color: Rgba,
                           pub align: TextAlign, pub valign: VAlign, pub case: TextCase, pub letter_spacing: f64, pub line_spacing: f64 }
  pub(crate) fn text(sym: &Value, what: &str, report: &mut Report) -> Text;
  pub(crate) fn vector_shape(marker: &Value) -> Option<Marker>;
  ```
  Rules: a symbol layer with `enable: false` is skipped. A `CIMSolidStroke` or `CIMSolidFill` whose colour alpha is 0 counts as none. `polygon`: `fill` is the topmost solid fill, `stroke` the topmost solid stroke (`pt96(width)`), every stroke also a `LineStroke` (cap `Round|Butt|Square`, join `Round|Miter|Bevel`, dash from a `CIMGeometricEffectDashes.dashTemplate` through `Dash::new`, offset from `CIMGeometricEffectOffset.offset`), hatch and gradient fills as `FillPattern::Hatch`/`Gradient`, `CIMPictureFill` noted. `line`: same for a line symbol (no fills). `point`: `CIMVectorMarker` → `vector_shape` (one graphic: `curveRings` → `Circle`; a 5-point ring with axis-aligned sides → `Square`, turned 45° → `Diamond`; 4 points → `Triangle`; 11 points → `Star`; else `None`) with the graphic's fill and stroke; `None` → `Circle` with a note `"<what>: a drawn marker shape, shown as a circle"`; `CIMPictureMarker` with a `data:image/…;base64,` URL → `Marker::Image` (GIF and BMP are passed through as the bytes they are; the engine converts them in Task 21, so here `ImageRef::new` takes any base64); `CIMCharacterMarker` → `Marker::Char(Glyph::new(char::from_u32(characterIndex), fontFamilyName))`; `size` → `pt96`. `text`: `fontFamilyName` → `font` through `crate::symbology::font_name`; `fontStyleName` containing `Bold` → bold, `Italic` or `Oblique` → italic; `height` → `text_pt`; colour from the text symbol's polygon `symbol`'s solid fill; `horizontalAlignment` `Left|Center|Right|Justify` (Justify → Left, noted); `letterSpacing` (percent of the size) → points; `lineSpacing` (default 1.0); `textCase` `Allcaps|Lowercase|Normal`; `smallCaps: true` → `TextCase::Upper` and a note; `haloSymbol` present → note `"<what>: a text halo, left out"`.

- [ ] **Step 1: Write the failing tests** (one per function, with the CIM JSON of each case inline; the templates' shapes are the examples: a polygon with a 1 pt black `CIMSolidStroke` over a white `CIMSolidFill`; a transparent stroke; a text symbol `Roboto Condensed`/`Bold`/14/`Center`/`Allcaps`; a point symbol with a `CIMPictureMarker` data URL under a `CIMCharacterMarker` index 178 of `ESRI North`; a vector marker whose single graphic has `curveRings`).

```rust
#[test]
fn polygon_symbols_take_their_top_fill_and_stroke_in_96ths() { /* fill white, stroke black width 96/72 */ }
#[test]
fn transparent_strokes_count_as_no_stroke() { /* alpha 0 → stroke None */ }
#[test]
fn text_symbols_read_font_style_size_colour_alignment_and_case() { /* bold, 14.0, Center, Upper; font Some("Roboto Condensed") */ }
#[test]
fn point_symbols_stack_picture_and_character_markers_bottom_first() { /* len 2; [0] Marker::Image, [1] Marker::Char with ch '\u{b2}' */ }
#[test]
fn vector_markers_are_recognised_by_shape() { /* curveRings → Circle; square ring → Square; 7-point ring → None and a note from point() */ }
```

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement `symbol.rs`.** - [ ] **Step 4: Run to verify pass** (`cargo test -p rustgis-map cim::`). - [ ] **Step 5: Commit** `feat(cim): symbols`.

---

### Task 5: Dynamic text tokens with prefixes, suffixes and page values

**Files:**
- Modify: `crates/map/src/graphics.rs:34` (`TOKENS`), `:130-152` (`TokenValues`), `:155-190` (`expand`)
- Modify: `crates/render/src/layout.rs:384-418` (`expand` builds the full `TokenValues`; drop its own replaces), `crates/render/src/svg.rs:39-91` (uses the same `expand`)
- Modify: `crates/map/src/layout.rs:27` (`PAGE_TOKENS` stays for the UI menu)
- Test: inline tests in `graphics.rs` and `render/src/layout.rs:1827-1829`

**Interfaces:**
- Produces:
  ```rust
  pub struct TokenValues {
      pub map: String, pub scale: f64, pub crs: String, pub layers: usize, pub center: Coord, pub now: u64,
      // new, all #[serde-less], plain fields with Default:
      pub page: usize, pub pages: usize, pub page_name: String, pub layout: String, pub decimal_comma: bool,
      pub project: String, pub credits: String,
      pub crs_name: String, pub crs_projection: String, pub crs_units: String, pub crs_wkid: String,
      pub extent: Option<rustgis_geom::Extent>,
  }
  impl Default for TokenValues {}
  pub fn expand(text: &str, v: &TokenValues) -> String;
  pub const TOKENS: &[(&str, &str)];  // adds {project}, {credits}, {crsName}, {crsProjection}, {crsUnits}, {crsWkid}, {extent}
  ```
  Token syntax: `{name}` as today, plus `{name|prefix}` and `{name|prefix|suffix}`: prefix and suffix are literal text (with `\n` meaning a line break) printed only when the value is non-empty (`scale` 0, `layers` 0 and an empty string count as empty). A `|` inside the prefix or suffix isn't supported (the third `|` ends the token). Unknown names are left as they are, including their braces. `{extent}` prints `xmin, ymin, xmax, ymax` in degrees with 4 decimals. `{scale}` honours `decimal_comma` as render's did (`crate::graphics::scale_ratio` with the comma swap from `rustgis_render::layout::decimal`: move `decimal` into `graphics.rs` so both use it).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn tokens_take_prefix_and_suffix_dropped_when_empty() {
    let mut v = TokenValues::default();
    v.crs_name = "NAD 1983 StatePlane Texas Central FIPS 4203 (US Feet)".into();
    assert_eq!(expand("{crsName|Name: |\\n}{crsWkid|WKID: |\\n}", &v), "Name: NAD 1983 StatePlane Texas Central FIPS 4203 (US Feet)\n");
    assert_eq!(expand("{scale|1:}", &v), "");
    v.scale = 24_000.0;
    assert_eq!(expand("{scale|1:}", &v), "1:24,000");
    assert_eq!(expand("{nothing|x|y}", &v), "{nothing|x|y}");
}
#[test]
fn page_tokens_expand_here_now() {
    let v = TokenValues { page: 2, pages: 5, page_name: "North".into(), layout: "Atlas".into(), ..Default::default() };
    assert_eq!(expand("{page} of {pages}, {pageName} · {layout}", &v), "2 of 5, North · Atlas");
}
```
The existing render test `expand("{page} of {pages}, {pageName} · {layout}", …)` at `render/src/layout.rs:1829` and the decimal-comma test at `:2718` keep passing through the render `expand`.

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement**: a single left-to-right scan over the text; on `{`, read to the matching `}` (at most 200 characters, else emit literally), split on `|`, look the name up, apply the rule. Render's `expand(t, layout, info, frame)` now only fills `TokenValues` (page fields from `info` and `layout`, map fields from the frame, `project` and `credits` left empty until Tasks 6 and 21) and calls `graphics::expand`. `svg.rs` already calls render's `expand`; confirm. - [ ] **Step 4: Run** `cargo test -p rustgis-map graphics -p rustgis-render expand` → pass. - [ ] **Step 5: Commit** `feat(layout): token prefixes, suffixes and page values in one expander`.

---

### Task 6: EPSG names, and the coordinate system tokens

**Files:**
- Create: `crates/data/src/raster/epsg_names.py` (generator, beside `epsg_axes.py`) and `crates/data/src/raster/epsg_names.rs` (generated: `pub(crate) const NAMES: &[(u32, &str)]`, one row per EPSG code `crs_definitions` or `epsg_gap` defines, plus every ESRI code in `esri.rs`, names from PROJ's database via pyproj, sorted by code)
- Modify: `crates/data/src/raster/crs.rs` (add `pub fn epsg_name(code: u32) -> Option<&'static str>` and `pub fn esri_name(code: u32)`, binary search), `crates/data/src/raster/mod.rs` (declare the module)
- Modify: `crates/map/src/projection.rs:340` area (add `pub struct CrsInfo { pub name: String, pub method: String, pub units: String, pub wkid: String }` and `pub fn describe(code: &str) -> Option<CrsInfo>`), `crates/map/src/globe.rs:44` (`Projection::info(&self) -> CrsInfo`; `label()` unchanged)
- Modify: `crates/render/src/layout.rs` `expand` (fill `crs_name`, `crs_projection`, `crs_units`, `crs_wkid` from `map.projection.info()`)
- Modify: `ATTRIBUTION.md` (a row for the EPSG names: "EPSG Geodetic Parameter Dataset, © IOGP, used under its terms of use, via PROJ")
- Test: inline tests in `crs.rs`, `projection.rs`, `render/src/layout.rs`

**Interfaces:**
- Produces: `rustgis_data::raster::crs::epsg_name(2277) == Some("NAD83 / Texas Central (ftUS)")`; `rustgis_map::projection::describe("EPSG:2277") == Some(CrsInfo { name: "NAD83 / Texas Central (ftUS)", method: "Lambert Conformal Conic", units: "US survey foot", wkid: "2277" })`. `method` comes from the PROJ string's `+proj=` through a table of the methods RustGIS implements (`tmerc` Transverse Mercator, `lcc` Lambert Conformal Conic, `merc` Mercator, `utm` Transverse Mercator, `aea` Albers Equal Area, `laea` Lambert Azimuthal Equal Area, `stere` Stereographic, `omerc` Hotine Oblique Mercator, `cass` Cassini, `eqc` Equirectangular, `longlat` Geographic); `units` from `+units=` (`m` metre, `us-ft` US survey foot, `ft` foot, degrees for `longlat`). Web Mercator's info is `WGS 84 / Pseudo-Mercator`, `Mercator (Auxiliary Sphere)`, `metre`, `3857`. A preset's info is its preset label, its method, `metre`, empty wkid.

Note: Pro prints Esri's spelling (`NAD 1983 StatePlane Texas Central FIPS 4203 (US Feet)`); the EPSG dataset's is `NAD83 / Texas Central (ftUS)`. The ESRI names from `esri.rs`'s source comments are the Esri spelling and are included for ESRI codes; for EPSG codes the EPSG name is what we have. Say so in the docs (Task 23).

- [ ] **Step 1: Write the failing tests**

```rust
// crs.rs
#[test] fn epsg_names_are_found_by_code() { assert_eq!(epsg_name(2277), Some("NAD83 / Texas Central (ftUS)")); assert_eq!(epsg_name(4326), Some("WGS 84")); assert_eq!(epsg_name(1), None); }
// projection.rs
#[test] fn describe_gives_name_method_units_and_wkid() { let i = describe("EPSG:2277").unwrap(); assert_eq!(i.method, "Lambert Conformal Conic"); assert_eq!(i.units, "US survey foot"); assert_eq!(i.wkid, "2277"); }
// render/src/layout.rs
#[test] fn crs_tokens_read_the_frame_maps_projection() { /* a map in EPSG:2277; expand("{crsName|Name: }{crsWkid|\\nWKID: }") contains "Name: NAD83 / Texas Central (ftUS)\nWKID: 2277" */ }
```

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Write the generator** (pyproj: `CRS.from_epsg(code).name` for every code in `crs_definitions` 0.6's list plus `epsg_gap::DEFS`, and `CRS.from_authority("ESRI", code).name` for `esri::DEFS`), run it once (`python crates/data/src/raster/epsg_names.py > crates/data/src/raster/epsg_names.rs`; pyproj must be installed: `pip install pyproj`), then the lookups, `describe`, the render fill, the attribution row. - [ ] **Step 4: Run** `cargo test -p rustgis-data epsg_name -p rustgis-map describe -p rustgis-render crs_tokens`, then `cargo xtask ci` (the notices check passes: no new crate). - [ ] **Step 5: Commit** `feat(crs): coordinate system names, methods and units`.

---

### Task 7: Text elements: bound frame, wrapping, vertical alignment, line spacing, case

**Files:**
- Modify: `crates/map/src/layout.rs:930-954` (`ElementKind::Text` fields), `:1309` (`clamp`: `line_spacing` 0.5..=3, drop a `frame` that isn't a MapFrame), `:1062` (`frame()` stays as is: a bound text is not a surround; instead `remove()` at `:1230` clears `Text.frame` when its frame goes)
- Modify: `crates/render/src/text.rs:304` (`wrap` gains a `Face` parameter: `pub fn wrap(line: &str, size: f32, face: Face, spacing: f32, max_w: f32) -> Vec<String>`; update its callers), `crates/render/src/layout.rs:242-252, 1674-1713` (`text_block` takes the new fields; the frame for tokens is `Text.frame` when set, else `first_frame()`), `crates/render/src/svg.rs:39-91` (same rules in SVG: wrapped lines as `<tspan>`s, `dominant-baseline` for valign, `text-transform` for case)
- Modify: `crates/engine/src/cmd/layout.rs:88` (params doc), `:2096` (`add_element` text: `frame`, `wrap`, `valign`, `lineSpacing`, `case`), `:2412` (`text_style` returns them too), `:2876` (`update` Text arm)
- Modify: `crates/ui-egui/src/layoutview.rs:1741` (text form: a Frame combo listing map frames by name with "First frame", a Wrap checkbox, Vertical combo, Line spacing drag value, Case combo), `crates/ui-egui/src/textform.rs` if that's where the form's `Fields` live
- Test: `map/src/layout.rs` (round trip), `render/src/layout.rs`, `engine/src/tests.rs`, `ui-egui/src/layoutview.rs:2537` style

**Interfaces:**
- Produces:
  ```rust
  pub enum VAlign { #[default] Top, Middle, Bottom }   // codes "top" | "middle" | "bottom"; from_name
  pub enum TextCase { #[default] None, Upper, Lower }   // codes "none" | "upper" | "lower"
  // on ElementKind::Text, each #[serde(default, skip_serializing_if = …)]:
  frame: Option<u32>, wrap: bool, valign: VAlign, line_spacing: f64 /* default 1.0 */, case: TextCase
  ```
  `layout.update` and `layout.addElement` take `"frame": id | null`, `"wrap": bool`, `"valign": "top"|"middle"|"bottom"`, `"lineSpacing": 0.5..3`, `"case": "none"|"upper"|"lower"` for text. Rendering: `wrap` breaks each `\n` line at the box width with `text::wrap`; `valign` positions the block within the box (`Middle` centres the block's height, `Bottom` sits it on the bottom edge); line pitch is `px * 1.25 * line_spacing`; `case` transforms the expanded text (tokens included) with `to_uppercase`/`to_lowercase`.

- [ ] **Step 1: Write the failing tests**

```rust
// map/src/layout.rs
#[test] fn text_elements_saved_before_read_and_save_the_same() { let j = r#"{"Text":{"text":"a","size":10.0,"color":[0,0,0,255]}}"#; let k: ElementKind = serde_json::from_str(j).unwrap(); assert_eq!(serde_json::to_string(&k).unwrap(), j); }
// render/src/layout.rs
#[test] fn wrapped_text_breaks_at_the_box_and_bottom_aligned_text_sits_on_the_bottom_edge() { /* a 30 mm wide box with 20 words: more than one line has ink; with valign Bottom, the lowest ink row is within 2 px of the bottom; a one-word box 1 mm wide draws one line and returns */ }
#[test] fn text_tokens_read_from_the_bound_frame() { /* two frames at scales 1:10,000 and 1:50,000; a text bound to the second prints 1:50,000; unbound prints 1:10,000 */ }
// engine tests.rs
#[test] fn layout_text_takes_frame_wrap_valign_spacing_and_case() { /* layout.update with each; read back; a frame id that is a Rectangle errors "frame must be a map frame" */ }
```

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement** model, render (CPU and SVG), command, UI. - [ ] **Step 4: Run** the four crates' tests and `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(layout): text bound to a frame, wrapping, vertical alignment, line spacing and case`.

---

### Task 8: Element borders and backgrounds

**Files:**
- Modify: `crates/map/src/layout.rs:1077-1092` (`Element` gains `#[serde(default, skip_serializing_if = "Option::is_none")] pub frame_style: Option<FrameStyle>`), `clamp` (width 0..=50 pt, gap 0..=50 mm)
- Modify: `crates/render/src/layout.rs:135` (before the `match`, draw the background behind the element's box grown by `gap`; after it, the border), `crates/render/src/svg.rs` (same, as `<rect>`s around every element kind, vector or raster)
- Modify: `crates/engine/src/cmd/layout.rs:2641` (`update` takes `"border": colour | null`, `"borderWidth": pt`, `"background": colour | null`, `"gap": mm`, any element kind; `add_element` takes the same), `:91` params doc
- Modify: `crates/ui-egui/src/layoutview.rs:1428` (`properties`: a "Frame" section after the common grid: Border colour button with a None option, Width, Background colour with None, Gap)
- Test: `map/src/layout.rs`, `render/src/layout.rs`, `engine/src/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct FrameStyle { pub border: Option<Rgba>, pub border_width: f64 /* points, default 1.0 */, pub background: Option<Rgba>, pub gap: f64 /* mm, default 0 */ }
  ```

- [ ] **Step 1: Write the failing tests**: `elements_without_frame_style_save_as_before` (an element JSON without the key round-trips byte for byte); `a_background_and_border_draw_around_any_element` (a Text element with white background and a 2 pt red border on a grey page: the pixel just inside the box's top-left is white, the pixel on the edge is red); `layout_update_sets_and_clears_border_and_background` (engine).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass**, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(layout): borders and backgrounds on any element`.

---

### Task 9: Linked map frames

**Files:**
- Modify: `crates/map/src/layout.rs:843-867` (`MapFrame` gains `#[serde(default, skip_serializing_if = "Option::is_none")] link: Option<FrameLink>`), `clamp` (drop a link to a non-frame, to itself, or that closes a cycle: walk links up to 500 steps), `remove()` (clear links to the removed frame)
- Add to `layout.rs`: `pub fn resolved_cameras(&self, maps: &[Map]) -> Vec<(u32, Camera)>` (every frame's camera with links applied: unlinked frames first, then linked ones in dependency order, at most 500 passes)
- Modify: `crates/render/src/layout.rs:101-113` (`frames` built from `resolved_cameras`), `:1056` note
- Modify: `crates/engine/src/cmd/layout.rs` (`update` MapFrame arm takes `"link": {"frame": id, "mode": "center"|"extent"|"scale", "rotation": bool} | null`; `layout.frameZoom`/pan on a linked frame errors "the frame follows <name>: unlink it first")
- Modify: `crates/ui-egui/src/layoutview.rs:1558` (MapFrame properties: a "Follows" combo of other frames with "Nothing", Mode combo, Rotation checkbox; the frame's scale and centre fields are disabled while linked), `:1076` `pan_frame` and `:1091` `zoom_frame` refuse a linked frame
- Test: `map/src/layout.rs`, `render/src/layout.rs`, `engine/src/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum LinkMode { #[default] Center, Extent, Scale }
  pub struct FrameLink { pub frame: u32, pub mode: LinkMode, #[serde(default)] pub rotation: bool }
  ```
  `Center`: the linked frame takes the source's centre and keeps its own scale. `Extent`: it fits the source's extent (`fit_scale` at `layout.rs:1393`). `Scale`: it takes the source's scale and keeps its centre. `rotation`: it takes the source's bearing.

- [ ] **Step 1: Write the failing tests**: `a_linked_frame_follows_its_sources_centre_scale_or_extent` (three cases through `resolved_cameras`); `link_cycles_and_self_links_are_dropped_by_clamp` (A→B, B→A: after `clamp` exactly one link remains; A→A: none); `removing_a_frame_clears_links_to_it`; render: `a_linked_frame_renders_at_its_sources_centre` (two frames, source panned, the linked frame's pixels change); engine: `layout_update_links_and_unlinks_frames_and_refuses_cycles`.
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass**, `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(layout): frames that follow another frame`.

---

### Task 10: Element groups

**Files:**
- Modify: `crates/map/src/layout.rs:842` (`ElementKind::Group { children: Vec<u32> }`; `code()` "group", `label()` "Group"), `clamp` (drop missing children, the group itself, and children already in another group; nesting depth ≤ 8, deeper groups flattened; children limited to 500), `remove()` (removing a group removes its children; removing a child drops it from its group), `hit()` (a group has no box of its own for hit testing: hits go to children; the Format pane selects a group through the Layout pane), add `pub fn group_of(&self, id: u32) -> Option<u32>`, `pub fn group_box(&self, id: u32) -> Option<(f64, f64, f64, f64)>` (the union of the children's boxes, recursively)
- Modify: `crates/render/src/layout.rs:135` (`Group` arm draws nothing; its `frame_style` from Task 8 draws around `group_box`), `crates/render/src/svg.rs:152` (`picture_dpi` vector list: Group is vector/nothing)
- Modify: `crates/engine/src/cmd/layout.rs` (new commands `layout.group {"layout"?, "elements": [ids], "name"?}` → `{"element": id}` and `layout.ungroup {"layout"?, "element": id}`; `update` with `x`/`y` on a Group moves every child by the same delta; `w`/`h` on a group errors; `layout.removeElement` and `layout.duplicateElement` cascade; `layout.arrange` moves the group's children together), `:88` params docs
- Modify: `crates/ui-egui/src/layoutview.rs` (selecting a child whose group is selected moves the whole group: drag `Move` on a child applies to the group when the group is the selection; the Format pane for a Group shows Name, Visible, Locked, the frame style, and an Ungroup button), `crates/ui-egui/src/ribbon.rs:2885` (Layout tab, Arrange group: Group (enabled with 2+ selected) and Ungroup), the Layout pane (`panes.rs`: the element list nests children under their group, indented, collapsible)
- Multi-selection: `app.layout.selected` is one id today (`layoutview.rs:970`). Add `app.layout.selection: Vec<u32>` filled by Shift-click and the existing Lasso drag; `layout.group` takes it. Keep `selected` as the primary.
- Test: `map/src/layout.rs`, `engine/src/tests.rs`, `ui-egui/src/layoutview.rs`

- [ ] **Step 1: Write the failing tests**: `groups_round_trip_and_clamp_drops_bad_children` (JSON with a missing child id and a self-reference: after `clamp`, children are only the real ones); `removing_a_group_removes_its_children`; `group_box_is_the_union_of_its_children`; engine: `layout_group_and_ungroup_move_and_delete_together` (group two rectangles, `layout.update {x}` on the group moves both by the delta, ungroup leaves both, delete removes both; `layout.ungroup` on a rectangle errors "not a group"; a group of a group nests and a 9-deep nest is flattened by clamp); ui: `shift_click_builds_a_multi_selection`.
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass**, `cargo xtask ci` (add `{"elements": [1, "x", null]}` to the hostile junk list). - [ ] **Step 5: Commit** `feat(layout): element groups`.

---

### Task 11: Legend columns and fitting

**Files:**
- Modify: `crates/map/src/layout.rs:889-907` (`Legend` gains `columns: usize` default 1, `fit: LegendFit` default `Manual`, `show_layer_names: bool` default true; `clamp` columns 1..=6)
- Modify: `crates/render/src/layout.rs:1294` (`legend`: lays rows into `columns` columns of equal width; `AdjustSize` shrinks the font sizes together (down to 6 pt, as Pro's `minFontSize`) until the rows fit the box; `AdjustColumns` adds columns until they fit, up to 6; `AdjustColumnsAndSize` tries columns first then size; `AdjustFrame` draws at the natural size, overflowing the box; `Manual` is today's behaviour), `:1400` `legend_size_mm` honours `columns`; `show_layer_names: false` drops `LegendRow::Plain`/`Head` rows for layers (classes keep their heads)
- Modify: `crates/engine/src/cmd/layout.rs` (`add_element` legend and `update` Legend arm: `"columns"`, `"fit": "manual"|"adjustSize"|"adjustColumns"|"adjustColumnsAndSize"|"adjustFrame"`, `"showLayerNames"`)
- Modify: `crates/ui-egui/src/layoutview.rs:1467` (Legend properties: Columns, Fit combo, Layer names checkbox)
- Test: `render/src/layout.rs`, `engine/src/tests.rs`

- [ ] **Step 1: Write the failing tests**: `a_two_column_legend_puts_rows_side_by_side` (a map with 8 layers in a wide short box: ink in both halves); `adjust_size_shrinks_until_rows_fit` (a tall list in a short box: with `Manual` the last layer's name is absent from the drawn rows, with `AdjustSize` every layer is drawn; count via the rows the function reports: refactor `legend` to return the rows it drew, `Vec<(String, f32 size)>`, for tests); engine: `layout_update_sets_legend_columns_fit_and_layer_names`.
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(layout): legend columns and fitting`.

---

### Task 12: Scale bar details

**Files:**
- Modify: `crates/map/src/layout.rs:879-888` (`ScaleBar` gains `subdivisions: usize` 0, `divisions_before_zero: usize` 0, `label_position: BarLabels` {Above, Below} default Below, `unit_label: Option<String>` (None: the unit's abbreviation), `unit_label_position: UnitLabel` {AfterBar, AfterLabels, Above, Below} default AfterLabels, `decimals: usize` 0, `label_size: f64` 8.0, `fit: ScaleFit` {AdjustDivisionValue, AdjustDivision, AdjustWidth} default AdjustDivisionValue; `clamp`: subdivisions 0..=10, before-zero 0..=2, decimals 0..=3, label size 4..=24)
- Modify: `crates/render/src/layout.rs:971` (`scale_bar` takes the `ElementKind::ScaleBar` fields as one struct `BarOptions`; draws subdivisions in the first division (alternating fill, half height ticks for line styles), divisions before zero to the left of 0 with negative labels, labels above or below, the unit label where asked, numbers with `decimals` and a thousands separator; `AdjustDivision` keeps the division value round and changes the count to fill the width; `AdjustWidth` keeps the count and value and draws the bar at the width they need)
- Modify: `crates/engine/src/cmd/layout.rs:2574` (`restyle` ScaleBar arm: `subdivisions`, `divisionsBeforeZero`, `labelPosition`, `unitLabel`, `unitLabelPosition`, `decimals`, `labelSize`, `fit`)
- Modify: `crates/ui-egui/src/layoutview.rs:1625` (ScaleBar properties: the new fields)
- Test: `render/src/layout.rs:2644` style, `engine/src/tests.rs:3900` style

- [ ] **Step 1: Write the failing tests**: `scale_bars_draw_subdivisions_and_place_labels_above_or_below` (ink rows above vs below the bar); `unit_labels_sit_after_the_bar_when_asked` (the rightmost ink is the word "Feet" when `unit_label: Some("Feet")`, `AfterBar`: compare the ink extent against a bar without a unit label); engine: `layout_scale_bars_take_their_details`.
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(layout): scale bar subdivisions, labels and units`.

---

### Task 13: North arrows drawn from a symbol

**Files:**
- Modify: `crates/map/src/layout.rs:868-878` (`NorthArrow` gains `#[serde(default, skip_serializing_if = "Vec::is_empty")] symbol: Vec<Symbol>`; `clamp` truncates to 8)
- Modify: `crates/render/src/layout.rs:897` (`north_arrow`: when `symbol` isn't empty, draw each `Symbol` bottom to top centred in the box, rotated by the bearing, scaled so the largest marker's `size` fills the box's shorter side and the others keep their ratio, with `draw_marker` from Task 3; no "N" is added)
- Modify: `crates/engine/src/cmd/layout.rs:2574` (`restyle` NorthArrow: `"symbol": null` clears it, so the style draws again; no other way to set it by command: the import fills it)
- Modify: `crates/ui-egui/src/layoutview.rs:1602` (NorthArrow properties: when `symbol` is set, show "Custom symbol (from import)" and a Clear button instead of the style combo)
- Test: `render/src/layout.rs:2598` style, `engine/src/tests.rs`

- [ ] **Step 1: Write the failing tests**: `a_north_arrow_with_a_symbol_draws_the_symbol_not_the_style` (an `Image` marker of a 2×2 red PNG: the box centre is red; with an empty `symbol` it isn't); engine: `layout_update_clears_a_north_arrow_symbol`.
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(layout): north arrows drawn from a point symbol`.

---

### Task 14: Pictures remember their file

**Files:**
- Modify: `crates/map/src/layout.rs:955` (`Picture` gains `#[serde(default, skip_serializing_if = "Option::is_none")] source: Option<String>`; `clamp` truncates it to 1000 chars)
- Modify: `crates/render/src/layout.rs:253` (an empty `image` draws a grey outline with the file name, through `placeholder` at `:859`)
- Modify: `crates/engine/src/cmd/layout.rs` (`update` on a Picture with `"path"` re-reads the picture through `super::graphics::picture` and sets `source` to the path; `add_element` picture sets `source` from `"path"` when given)
- Modify: `crates/ui-egui/src/layoutview.rs` (Picture properties: the source path, a "Relink…" button opening `dialogs::pick_open(&["png","jpg","jpeg"], …)` then `layout.update {path}`)
- Test: `engine/src/tests.rs`

- [ ] **Step 1: Write the failing test**: `pictures_keep_their_path_and_relink` (add a picture by `path` to a PNG written in a temp dir; `source` is the path; `layout.update {path: other}` changes `image`; a path that doesn't exist errors and leaves the element as it was).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(layout): pictures keep their file for relinking`.

---

### Task 15: The CIM layout reader: page, geometry and plain graphics

**Files:**
- Create: `crates/map/src/cim/layout.rs`
- Modify: `crates/map/src/cim/mod.rs` (declare `pub mod layout;` and re-export `pub use layout::{read_layout, LayoutImport, FrameBinding, PictureSource};`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Consumes: Tasks 1, 2, 4; `crate::layout::{Layout, Element, ElementKind, Guides, TextAlign, FrameStyle, …}`.
- Produces:
  ```rust
  pub struct LayoutImport {
      pub layout: Layout,                     // elements in Pro's draw order (first drawn first), groups included
      pub frames: Vec<FrameBinding>,          // one per MapFrame element
      pub pictures: Vec<(u32, PictureSource)>, // Picture elements whose bytes aren't in the document
      pub embedded_maps: Vec<String>,         // map URIs the frames refer to, in order of first use
      pub text_frames: Vec<(u32, String)>,    // text elements and the frame name their tokens read from (Task 16 fills, Task 17 resolves)
  }
  pub struct FrameBinding { pub element: u32, pub name: String, pub map_uri: Option<String>, pub saved_camera: Option<(f64, f64, f64)> /* x, y, scale */ }
  pub enum PictureSource { Path(String) }
  pub fn read_layout(doc: &Document, report: &mut Report) -> Result<LayoutImport, String>;
  ```
  This task covers: the page (`CIMPage` width/height/units; guides, y flipped; unknown units → inches and a note), the element walk (`layoutDefinition.elements`, recursing into `CIMGroupElement.elements` up to depth 8, at most 500 elements, noting the rest), `name`/`visible`/`locked` → `Element`, geometry (`frame` polygon/envelope for frame elements; a graphic's `shape`: `rings` → box, `paths` → box, `xmin..` → box, a point `{x, y}` for point text; `rotation` about `anchor`: resolve the anchor point of the unrotated box (`TopLeftCorner`, `TopMidPoint`, `TopRightCorner`, `LeftMidPoint`, `CenterPoint`, `RightMidPoint`, `BottomLeftCorner`, `BottomMidPoint`, `BottomRightCorner`), rotate the four corners, take the bounding box, note when the angle isn't a multiple of 90), point text (box from the point, the anchor, `Text.size` and `rustgis_render` isn't available in `crates/map`: measure with `crate::graphics`'s width estimate `0.55 * size * chars` per line, the tallest line count × `size * 1.25` for height; the renderer draws the text at its real width inside that box, aligned by `align`, so a slightly wrong estimate only moves the box edge, not the ink), and the plain graphic kinds: `CIMPolygonGraphic` → `Rectangle` (4-corner ring) / `Ellipse` (`curveRings`) / `Rectangle` with a note; `CIMLineGraphic` → `Line` (2-point path, `dir` from the end points' slope: horizontal, vertical, rising, falling; heads from marker layers at the ends, noted otherwise); `CIMPictureGraphic` → `Picture` (`referenceURI` through `doc.binary`, a `data:` `sourceURL` decoded in place, a path → empty `image` + `PictureSource::Path` + `source`); text (Task 16); frames and surrounds (Tasks 17, 18). Unknown element types → skipped with a note naming the type. Element ids are assigned by `Layout::add` in walk order; groups are added after their children with the children's ids. The `graphicFrame` of any element → `FrameStyle` (border from the line symbol's top non-transparent stroke, background from the polygon's top non-transparent fill, `gap` from `borderGapX` in points → mm; a shadow symbol with layers → note).

- [ ] **Step 1: Write the failing tests** (each with a minimal `CIMLayoutDocument` JSON fixture helper `fn layout_doc(elements: &str) -> Document` that wraps elements in a 17×11 inch page with one guide)

```rust
#[test] fn the_page_and_guides_come_in_millimetres_from_the_top_left() { /* width 431.8, height 279.4; a Vertical guide at 16.5 in → x 419.1; a Horizontal one at 10.25 → y (11-10.25)*25.4 */ }
#[test] fn a_rectangle_rotated_180_about_its_top_left_lands_where_pro_draws_it() { /* LOGO RIBBON's numbers: ring [2.314,6.263]–[4.347,8.569], anchor TopLeftCorner, rotation 180 → x 0.281 in, y from the top 0.125 in, w 2.033, h 2.306 (±0.01 in) */ }
#[test] fn a_rectangle_rotated_30_degrees_keeps_its_bounding_box_and_notes_it() { /* finite box, a note containing "rotated" */ }
#[test] fn point_text_gets_a_box_from_its_anchor() { /* "TITLE" 14 pt at (2.858, 10.566) LeftMidPoint: x = 2.858 in, the box's vertical middle = 10.566 in from the bottom */ }
#[test] fn pictures_resolve_embedded_binaries_or_remember_their_path() { /* referenceURI → image set; a G:\ path → image empty, pictures has (id, Path), source set */ }
#[test] fn graphic_frames_become_borders_and_backgrounds() { /* border black 1 pt → FrameStyle{border: Some(black), border_width: 1.0*96/72…} — note: FrameStyle width is RustGIS points (1/96): 1 CIM pt → 1.333; background white; a transparent border → None */ }
#[test] fn unknown_elements_are_noted_and_skipped() { /* {"type":"CIMChartFrame","name":"C"} → no element, a skipped note naming CIMChartFrame and C */ }
#[test] fn too_many_or_too_deep_elements_are_bounded() { /* 600 rectangles → 500 elements and a note; groups nested 12 deep → read without panic */ }
```

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement `cim/layout.rs`** for this task's kinds (text, frames and surrounds produce a skipped note for now; Tasks 16–18 replace those arms). - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(cim): layout pages, geometry and graphics`.

---

### Task 16: The CIM layout reader: text and dynamic text

**Files:**
- Modify: `crates/map/src/cim/layout.rs` (the `CIMTextGraphic`/`CIMParagraphTextGraphic` arm), add `cim/dyntext.rs` (`pub(crate) fn translate(text: &str, report: &mut Report, what: &str) -> (String, Option<String> /* frame name the tokens read from */)`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Produces: a `Text` element with `size`, `color`, `bold`, `italic`, `align`, `letter_spacing`, `font_name` (and `font: FontFamily::Sans`), plus Task 7's `wrap` (true for paragraph text, false for point text), `valign` (`Center` → `Middle`), `line_spacing`, `case`; a paragraph's `margin` (points) shrinks the box on every side; `frame` is filled in Task 17 from the frame name `translate` returns (store it on `FrameBinding`-like `text_frames: Vec<(u32, String)>` in `LayoutImport`).
- `translate` rewrites each `<dyn …/>` tag (attributes in any order, single or double quotes) per the design's table: `mapFrame/scale` → `{scale|pre|suf}`, `mapFrame/sr` with `srProperty` `name|projection|units|wkid` → `{crsName|…}` etc., `mapFrame/center` → `{center|…}`, `mapFrame/extent` → `{extent|…}`, `project/name` → `{project|…}`, `layout/serviceLayerCredits` → `{credits|…}` (the `name` attribute ignored), `layout/name` → `{layout|…}`, `date` → `{date|…}` (a `format` attribute → note "date format <f>, drawn as RustGIS's"), `time` → `{time|…}`, `page/number` → `{page|…}`, `page/name` → `{pageName|…}`, `page/count` → `{pages|…}`; `preStr` → prefix, `postStr` → suffix, `newLine="true"` appends `\n` to the suffix, `emptyStr` is what the design's rule already gives (nothing) and any other `emptyStr` value is noted; any other `type`/`property` → removed with a note naming it. Pro's formatting tags `<BOL>`, `</BOL>`, `<ITA>`, `<UND>`, `<CLR …>`, `<FNT …>`, `<SUP>`, `<SUB>`, `<ACP>`, `<LCS>`, `<UCS>` and their closers are stripped with one note per element. `&amp;`, `&lt;`, `&gt;`, `&quot;` are unescaped. The frame name is the `name` attribute of the first `mapFrame` tag; a second, different name is noted.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn dyn_tags_become_tokens_with_prefix_suffix_and_newline() {
    let (t, frame) = dyntext::translate(r#"Spatial Reference<dyn type="mapFrame" name="MAIN MAP" property="sr" srProperty="name" preStr="Name: " newLine="true" emptyStr=""/><dyn type="mapFrame" name="MAIN MAP" property="scale" preStr="1:"/>"#, &mut Report::default(), "T");
    assert_eq!(t, "Spatial Reference{crsName|Name: |\\n}{scale|1:}");
    assert_eq!(frame.as_deref(), Some("MAIN MAP"));
}
#[test] fn layout_and_project_and_date_tags_translate() { /* "Map Name: <dyn type="project" property="name"/>" → "Map Name: {project}"; credits with a stale name → "{credits}"; date short → "{date}" and a note */ }
#[test] fn unknown_tags_and_formatting_are_stripped_with_notes() { /* "<BOL>Title</BOL> <dyn type="user" property="name"/>" → "Title " and two notes */ }
#[test] fn a_frame_name_that_isnt_on_the_page_is_noted_and_the_text_still_imports() { /* via read_layout: a text naming GONE; the element exists; `text_frames` holds (id, "GONE"); a note */ }
#[test] fn paragraph_text_wraps_and_point_text_doesnt() { /* DISCLAIMER-like paragraph: wrap true, valign Middle, case Upper, box shrunk by margin 5 pt; TITLE: wrap false */ }
```

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(cim): text and dynamic text`.

---

### Task 17: The CIM layout reader: map frames

**Files:**
- Modify: `crates/map/src/cim/layout.rs` (the `CIMMapFrame` arm; a second pass after the walk that turns frame names into ids for `Text.frame`, `FrameLink.frame`, `extent_of`, and the surrounds of Task 18)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Produces: a `MapFrame` element per `CIMMapFrame`: `map` left as the frame's name (the engine binds it in Task 21), `center`/`scale` from `view.camera` when `uRI` is set and the embedded map's `spatialReference.latestWkid` is 4326 or 3857 (degrees, or Web Mercator metres converted with `rustgis_geom`'s Web Mercator functions) else from `autoCamera.extent` (which carries its own `spatialReference`), else the map's `defaultExtent`; an unbound frame (no `uRI`) keeps `center` (0, 0) and `scale` 0 as "take the map's view", with `saved_camera` on its `FrameBinding` for the note; `autoCamera.source == "MapFrameLink"` → `link` with `mode` from `autoCameraType` (`Center`, `Extent`, `Scale`; others noted and taken as Center) and `rotation` from `syncRotation`; `extentIndicators[0]` → `extent_of` (by name → id in the second pass), `extent_color`/`extent_width` from its symbol's top stroke (`pt96`), further indicators noted; `graphicFrame` → `frame_style` (Task 15's helper); `shape` stays `Rectangle` (a frame with a non-rectangular `frame` ring → `FrameShape::Polygon` of its points relative to the box). `FrameBinding` pushed per frame; `embedded_maps` collects distinct `uRI`s.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn an_embedded_frames_camera_converts_from_degrees_or_web_mercator() { /* camera x -100.554, y 30.333, scale 20_000_000 on a 4326 map → center (-100.554, 30.333), scale 20e6; a 3857 map with x -9650117.76, y 5619364.40 → lon ≈ -86.69, lat ≈ 45.1 */ }
#[test] fn an_unbound_frame_takes_no_camera_and_records_the_saved_one() { /* MAIN MAP: scale 0.0, binding.map_uri None, saved_camera Some */ }
#[test] fn linked_frames_and_extent_indicators_resolve_by_name() { /* SMALL DETAIL MAP links MAIN MAP (Center, rotation true), extent_of = MAIN MAP's id, extent_width 2*96/72, colour black; a second indicator → note */ }
#[test] fn text_bound_to_a_frame_gets_its_id() { /* from Task 16's text_frames: Text.frame == Some(MAIN MAP id); GONE → None and the note */ }
```

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(cim): map frames, links and extent indicators`.

---

### Task 18: The CIM layout reader: legend, scale bar, north arrow, table, series

**Files:**
- Modify: `crates/map/src/cim/layout.rs` (arms for `CIMLegend`, the six scale bar types, `CIMMarkerNorthArrow`, `CIMTableFrame`, and `layoutDefinition.mapSeries`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Produces: `Legend { frame: by mapFrame name, title (empty when showTitle false), all_classes: false, hide_other: false, exclude: the layer names of excludedLayers URIs that resolve in the document (others dropped silently: they belong to other maps), columns, fit: fittingStrategy → LegendFit (AdjustSize | AdjustColumns | AdjustColumnsAndSize | AdjustFrame | Manual, unknown → Manual with a note), show_layer_names: defaultLegendItem.showLayerName }`; per-item `items[]` with their own settings → one note per legend "per-layer legend item settings, drawn with the defaults". `ScaleBar { frame, units: from units.uwkid (9002 Feet, 9003 Feet, 9001 Meters, 9036 Kilometers, 9093 Miles, else Auto + note), divisions, subdivisions, divisions_before_zero, label_position: labelPosition Above|Below, unit_label: unitLabel, unit_label_position: unitLabelPosition AfterBar|AfterLabels|Above|Below, decimals: numberFormat.roundingValue when roundingOption is esriRoundNumberOfDecimals, label_size: labelSymbol's text height, fit: fittingStrategy AdjustDivision|AdjustDivisionValue|AdjustWidth (AdjustFrame → AdjustWidth), style: CIMDoubleFillScaleBar → Alternating, CIMSingleFillScaleBar → Alternating, CIMLineScaleBar → Line, CIMSteppedLineScaleBar → Stepped, CIMHollowScaleBar → Alternating, CIMAlternatingScaleBar → Alternating }`; mark heights and frequencies → one note. `NorthArrow { frame, style: Classic, fill/outline: the character marker's fill or ink, symbol: cim::symbol::point(pointSymbol) }`; `northType` other than `TrueNorth` → note. `CIMTableFrame` → `Table { frame, layer: the referenced layer's name through the document, rows: 12 }` with a note "table settings". `mapSeries` with `indexLayerURI` resolving to a layer name → `MapSeries { frame, layer, name_field: nameField, margin: 0.1, page: 0 }`, else note.

- [ ] **Step 1: Write the failing tests**: `legends_read_their_title_columns_fit_and_layer_names` (the template's LEGEND: title "", columns 1, AdjustColumnsAndSize, show_layer_names true); `scale_bars_read_units_divisions_subdivisions_labels_and_format` (the template's: Feet, 4, 2, Below, Some("Feet"), AfterBar, decimals 0, AdjustDivision, Alternating); `north_arrows_carry_their_point_symbol` (symbol len 2, [0] Image, [1] Char); `tables_and_series_resolve_their_layers`.
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(cim): surrounds, tables and map series`.

---

### Task 19: The CIM layout reader: groups, draw order and hostile input

**Files:**
- Modify: `crates/map/src/cim/layout.rs` (`CIMGroupElement` → `ElementKind::Group { children }` after its children, with its `graphicFrame`; draw order: Pro lists elements top-most first in `elements`? Verify against the templates: the `MAIN MAP` frame is first and the title last, and Pro draws the first element at the bottom. RustGIS draws `elements` in order, first at the bottom (`draw_order` at `layout.rs:1203`): keep Pro's order as it is. Write the test from the template's order and confirm by eye in Task 24.)
- Test: `crates/map/src/cim/tests.rs`; add the layout reader to `crates/engine/src/tests.rs::hostile_params_never_panic` junk (Task 21 wires the command)

- [ ] **Step 1: Write the failing tests**: `groups_keep_their_children_and_nesting` (LOGO AND LINK with two children → a Group whose children are the two ids; a group in a group nests); `hostile_layouts_never_panic` (a fuzz-style loop over 200 mutations of a valid fixture: numbers replaced by NaN/huge/negative, strings by 10 000-char ones, arrays emptied, objects replaced by numbers; every call to `read_layout` returns without panicking; then `layout.clamp()` and `serde_json::to_string` succeed).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(cim): groups and hostile input`.

---

### Task 20: A minimal CIM map reader for embedded maps

**Files:**
- Create: `crates/map/src/cim/map.rs`
- Modify: `crates/map/src/cim/mod.rs` (declare `pub mod map;` and re-export `read_map`, `MapImport`, `LayerImport`)
- Test: `crates/map/src/cim/tests.rs`

**Interfaces:**
- Consumes: Task 4's `symbol::{polygon, line, point, text}`; `crate::qgis::Style` (reused as the style carrier).
- Produces:
  ```rust
  pub struct MapImport { pub name: String, pub wkid: Option<u32>, pub extent: Option<[f64; 4]> /* in degrees when wkid is 4326, else as given with the wkid */, pub layers: Vec<LayerImport> /* top first, as CIMMap.layers lists them */ }
  pub struct LayerImport { pub name: String, pub visible: bool, pub path: Option<String> /* workspaceConnectionString's DATABASE= joined with dataset, for Shapefile and FileGDB */, pub table: Option<String> /* FileGDB dataset */, pub origin: String /* "<workspaceFactory> <dataset> in <DATABASE>" */, pub style: crate::qgis::Style }
  pub fn read_map(doc: &Document, uri: &str, report: &mut Report) -> Result<MapImport, String>;
  ```
  This task reads only what the templates' embedded maps need: `CIMFeatureLayer` with `CIMStandardDataConnection` of `workspaceFactory` `Shapefile` or `FileGDB`; `visibility`, `transparency`, `minScale`/`maxScale`; `CIMSimpleRenderer` → `Renderer::Single` (polygon `Paint` → `Symbol { fill, stroke, stroke_width }`, `strokes` → `Style.line_strokes`, `fills` → `Style.fill_layers`); `CIMUniqueValueRenderer` on one field → `Renderer::Unique` (classes from `groups[].classes[].values[].fieldValues[0]` and `label`, `defaultSymbol` when `useDefaultSymbol`); `labelClasses` → `Labeling` with `enabled: labelVisibility`, one `LabelClass` per class with `field` from an expression that is only a field (`$feature.X`, `$feature["X"]`, `[X]`, `!X!`; anything else → the first field referenced and a note), `textSymbol` → size (`text_pt`), colour, bold, italic, font, halo (`haloSize` > 0 **and** `haloSymbol` present), `visibility`, `minimumScale`/`maximumScale`. Other layer types and connections → a `LayerImport` with `path: None` and a skipped note (Part 3 extends this; keep the match open with a `_` arm that notes the type). `CIMGroupLayer` → its children flattened with a note (Part 3 adds groups).

- [ ] **Step 1: Write the failing tests**: `an_embedded_shapefile_map_reads_its_layers_and_styles` (NEW DETAIL MAP's two layers: names, visible, path `G:\JERICHO_SHAPEFILES\OTLS-PLSS\LANDGRID TOBIN\TAA_US_States.shp`, `Style.renderer` Single with stroke (52,52,52) width 1.333 and transparent fill, `Style.transparency` 30, labels enabled with field `STATE_NAME`, size 10, font `Yu Gothic UI`); `unique_value_renderers_read_their_classes`; `unsupported_layers_are_noted`.
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run to verify pass.** - [ ] **Step 5: Commit** `feat(cim): feature layers from files for embedded maps`.

---

### Task 21: The `layout.import` command reads `.pagx`

**Files:**
- Create: `crates/engine/src/cmd/arcgis_pro.rs`
- Modify: `crates/engine/src/cmd/mod.rs:3-57` (declare `pub mod arcgis_pro;`), `:64` (`v.extend(arcgis_pro::commands())`)
- Modify: `crates/engine/src/cmd/layout.rs:133-152` (`layout.import`: when the text starts, after whitespace, with `{` and holds `"CIMLayoutDocument"` in its first 200 bytes, or `path` ends with `.pagx`, call `arcgis_pro::import_layout(s, &text, dir, p)`; params doc adds `"path"?: "*.rgislayout|*.pagx"`, `"maps"?: {frame name: map name}`, `"dir"?`)
- Modify: `crates/engine/src/tests.rs` (tests; junk entries `{"maps": 1}`, `{"maps": {"a": null}}`, `{"text": "{\"type\":\"CIMLayoutDocument\"}"}`)
- Test: `crates/engine/src/tests.rs`

**Interfaces:**
- Produces:
  ```rust
  pub fn commands() -> Vec<C>;   // "layout.inspect": {"path"?: "*.pagx", "text"?} → {"frames": [{"name", "map": embedded map name | null}], "maps": [embedded map names]}
  pub(crate) fn import_layout(s: &mut Session, text: &str, dir: Option<&str>, p: &Value) -> CmdResult;  // → {"layout": i, "name", "maps": [names added or reused], "notes": [...]}
  ```
  `import_layout`: `Document::from_json`, `cim::read_layout`; for each `embedded_maps` URI, `cim::read_map`, then **reuse** a project map of the same name whose layers' `source.path`s equal the import's paths in order, else add it: `map.new`, `map.projection` (`"mercator"` for 3857, `"equirectangular"` for 4326, else the EPSG code; failures noted), basemap `Blank`, `graticule = false`, layers bottom first through `layer.addData {path, name, layer?}` (a path that fails to open → a note and no layer, until Part 3's broken layers), `style.apply`, visibility; then restore the active map to what it was. Bind each `FrameBinding`: `map_uri` → the added/reused map's name; else `p["maps"][name]` when it names a project map; else the active map, with a note "<frame>: shows <map> (the template doesn't say which map)". An unbound frame with `scale == 0.0` takes the bound map's `camera` centre and scale (`Map::camera`, `to_lonlat`). Pictures: natively read each `PictureSource::Path` (relative to `dir`, else the `.pagx`'s folder) with `std::fs` under `graphics::MAX_PICTURE`, convert GIF/BMP to PNG through `image` (the engine has `gif`; add `bmp` to `crates/engine/Cargo.toml`'s `image` features), set `image`; failures noted and the element left empty with its `source`. North arrow `symbol` entries whose `Marker::Image` holds GIF bytes are converted to PNG the same way (decode, re-encode, re-intern). `l.clamp()`, `unique_name`, push, `active_layout`, a notice as the QGIS importer gives (`"Imported layout <name>: <n> things drawn differently, <m> left out"`), return. `layout.inspect` reads the document and returns the frame bindings without changing the project.

- [ ] **Step 1: Write the failing tests** in `engine/src/tests.rs` (fixture: the 11x17 template reduced by hand to one `CIMLayoutDocument` string with MAIN MAP, SMALL DETAIL MAP (linked, with the extent indicator, uRI to an embedded map with one shapefile layer pointing at the sample's own shapefile under a temp dir), TOP BACKGROUND, LOGO RIBBON, TITLE, SCALE TEXT, SPATIAL REF DATA, DATE AUTO, LEGEND, SCALE BAR, NORTH ARROW (a 2×2 PNG picture marker), HLR LOGO as a `referenceURI` picture, and the two groups)

```rust
#[test] fn a_pagx_imports_with_its_frames_bound_and_its_embedded_map_added() {
    let mut s = world();
    let r = s.run("layout.import", &json!({"text": PAGX, "maps": {"MAIN MAP": "World Atlas"}})).unwrap();
    let l = &s.project.layouts[r["layout"].as_u64().unwrap() as usize];
    assert_eq!((l.width, l.height), (431.8, 279.4));
    let main = l.elements.iter().find(|e| e.name == "MAIN MAP").unwrap();
    let detail = l.elements.iter().find(|e| e.name == "SMALL DETAIL MAP").unwrap();
    assert!(matches!(&main.kind, ElementKind::MapFrame { map, scale, .. } if map == "World Atlas" && *scale > 0.0));
    assert!(matches!(&detail.kind, ElementKind::MapFrame { map, link: Some(FrameLink { frame, .. }), extent_of: Some(of), .. } if map == "NEW DETAIL MAP" && *frame == main.id && *of == main.id));
    assert!(s.project.maps.iter().any(|m| m.name == "NEW DETAIL MAP"));
    let scale_text = l.elements.iter().find(|e| e.name == "SCALE TEXT").unwrap();
    assert!(matches!(&scale_text.kind, ElementKind::Text { text, frame: Some(f), .. } if text == "{scale|1:}" && *f == main.id));
    assert!(l.elements.iter().any(|e| matches!(&e.kind, ElementKind::Group { children } if children.len() == 2)));
    assert!(r["notes"].as_array().unwrap().iter().all(|n| n["kind"] != "skipped"));
}
#[test] fn importing_the_same_pagx_twice_reuses_the_embedded_map() { /* maps.len() grows by one, not two */ }
#[test] fn an_unbound_frame_takes_the_active_maps_view_and_says_so() { /* no "maps" param: MAIN MAP shows the active map at its camera; a note mentions "template" */ }
#[test] fn layout_inspect_lists_frames_without_importing() { /* frames: MAIN MAP map null, SMALL DETAIL MAP map "NEW DETAIL MAP"; layouts unchanged */ }
#[test] fn a_pagx_by_path_resolves_pictures_beside_it() { /* write the fixture and a PNG to a temp dir with sourceURL = that PNG's path; image non-empty, source set */ }
#[test] fn rgislayout_files_still_import() { /* the existing round-trip test keeps passing: layouts_compose_export_and_round_trip */ }
```

- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-engine pagx -p rustgis-engine layout_inspect -p rustgis-engine hostile_params` and `cargo xtask ci`. - [ ] **Step 5: Commit** `feat(layout): import ArcGIS Pro .pagx layouts`.

---

### Task 22: Desktop and web wiring

**Files:**
- Modify: `crates/ui-egui/src/lib.rs:1402` (`.pagx` bytes → `layout.import {text}`), `:2790-2799` (`ui.importLayout`: `pick_open(&["rgislayout", "pagx"], tl!("Layout file"))`; for a `.pagx`, run `layout.inspect` first and, when any frame has `map: null` and the project has more than one map, open the `importLayoutFrames` dialog; else run `layout.import {path}`), `:2840` area (the `open_async("layout")` web path accepts `.pagx`)
- Modify: `crates/ui-egui/src/dialogs.rs` (new dialog `importLayoutFrames`: one row per unbound frame with a combo of the project's maps defaulting to the active one, Import and Cancel; Import runs `layout.import {path, maps}`)
- Modify: `crates/ui-egui/src/panes.rs:2564,2593` (Browser pane lists and opens `.pagx`), `crates/ui-egui/src/ribbon.rs:3141` area (the layout Dynamic Text menu adds `{project}`, `{credits}`, `{crsName}`, `{crsProjection}`, `{crsUnits}`, `{crsWkid}`, `{extent}` with their descriptions from `graphics::TOKENS`)
- Modify: `crates/ui-egui/src/chrome.rs:767` (unchanged: `layout.import` → `ui.importLayout`)
- Test: `crates/ui-egui/src/lib.rs` inline (the `.pagx` extension routes to `layout.import`; the dialog opens when a frame is unbound and the project has two maps, and not when it has one), `crates/ui-egui/src/dialogs.rs`

- [ ] **Step 1: Write the failing tests** (`RustGisApp::with_sample()`, `app.open_bytes_with("t.pagx", PAGX.as_bytes(), None)` → a layout exists; `app.run("ui.importLayout", …)` can't open a file dialog in tests: test the branch function `fn import_layout_file(&mut self, path: &str)` directly with a `.pagx` written to a temp dir, asserting `app.dialogs.open == Some("importLayoutFrames")` with two maps and `None` with one).
- [ ] **Step 2: Run to verify failure.** - [ ] **Step 3: Implement.** - [ ] **Step 4: Run** `cargo test -p rustgis-ui-egui pagx`, `cargo xtask ci`, and `cargo build --target wasm32-unknown-unknown -p rustgis-web`. - [ ] **Step 5: Commit** `feat(ui): open ArcGIS Pro layouts`.

---

### Task 23: Documentation

**Files:**
- Modify: `docs/user-guide/layouts.md:252-253` (Import Layout takes `.pagx`: what carries over, the frame dialog, the notes; new sections for Groups, Follows (linked frames), text Frame/Wrap/Vertical/Case, element Border and Background, legend Columns and Fit, scale bar details, north arrow custom symbol, picture Relink; the Dynamic Text list gains the new tokens and the `{token|prefix|suffix}` form)
- Modify: `docs/user-guide/workspace.md:113` area (a `### ArcGIS Pro layouts` subsection after the QGIS one, pointing at layouts.md, and the accepted differences: date format, formatting tags, shadows, small caps, per-item legend settings, mark frequencies, EPSG spelling of coordinate system names)
- Modify: `docs/user-guide/cartography.md` (image markers under point symbols), `GEOLIBRE_PARITY.md:228` (ArcGIS Pro layouts: done; maps: pending)
- Modify: `crates/engine/src/cmd/layout.rs` and `arcgis_pro.rs` params docs (checked in Tasks 7–14, 21; re-read them here for consistency)

- [ ] **Step 1: Write the docs.** - [ ] **Step 2: Build them** (`pip install -r requirements_docs.txt` once; `zensical build` or the command `zensical.toml` names) and read the rendered pages. - [ ] **Step 3: Commit** `docs: ArcGIS Pro layout import and the layout additions`.

---

### Task 24: Verification with the six templates

**Files:**
- Create: `docs/superpowers/plans/2026-10-30-arcgis-pro-layout-import-checklist.md` (the results of this task, not committed until filled)
- Use: `G:\1- NEW LAYOUT FILES\*.pagx`, Mike's PDF exports of the same six from Pro (ask Mike for them: File › Share › Export Layout, PDF, 300 dpi), a project with a map named `MAIN MAP`-worthy (any map with a few layers and a web basemap, so `{credits}` has something to say)

- [ ] **Step 1: Import each template** with the desktop app (`PATH="$HOME/.cargo/bin:$PATH" cargo run --release -p rustgis`): Home › Project › Import Layout, pick the `.pagx`, bind MAIN MAP in the dialog. Note every notice.
- [ ] **Step 2: Export each** (Publish › Export Layout, PDF, 300 dpi) and put it beside Pro's PDF.
- [ ] **Step 3: Compare, element by element**, with the checklist: position and size within 0.5 mm (measure in a PDF viewer), text content identical after token expansion (scale, spatial reference block, date, project name, credits), font family and size, colours, the logo ribbon in the top-left corner behind the logo, the detail map following the main map (pan the main frame in RustGIS: the detail frame's indicator moves), the legend's columns, the scale bar's divisions and unit label, the north arrow picture, the groups moving as one.
- [ ] **Step 4: Fix** what differs, each fix with a regression test in the task that owns the code, and re-run the comparison.
- [ ] **Step 5: Headless screenshots** for the pull request: a `ui_shot` script (`cargo run --release -p rustgis-ui-egui --example ui_shot -- script.jsonl`) that imports one template and screenshots the layout view; keep the PNG out of the repo (assets live in opengeos/rustgis-assets).
- [ ] **Step 6: Commit** the filled checklist: `docs: ArcGIS Pro layout import verification`.

---

## Self-review notes

- Spec coverage: Part 1 (reader, colours, units, symbols, report) → Tasks 1, 2, 4; Part 2 entry points → 21, 22; page and guides → 15; geometry → 15; element kinds table → 15–19; map frames (binding, camera, links, indicators, borders) → 8, 9, 17, 21; text → 7, 16; dynamic text → 5, 6, 16; pictures → 14, 15, 21; groups → 10, 19; legend → 11, 18; scale bar → 12, 18; north arrow → 3, 13, 18; verification → 24; EPSG names → 6; the embedded-map reuse rule → 21; the unbound-frame "take the map's view" rule → 17, 21. Not planned, by design: Part 3.
- **Execution order:** 1, 2, 3, 5, 6, 7, 4, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24. Task 4 uses Task 7's `VAlign`/`TextCase`; Task 7 uses Task 5's `expand`. Everything else follows the numbering.
- Types: `FrameStyle` widths are RustGIS 1/96 points, matching every other width. `ImageRef` is `Copy` so `Marker` and `Symbol` stay `Copy`.
- Proportion: the plan is about twice the spec's length; the code it describes is many times longer.
