# ArcGIS Pro import: layouts (.pagx) and maps (.aprx)

Design, 2026-10-30. Brainstormed with Mike Hollis (Holland-LR); decisions marked **(Mike)** are
his, decisions marked **(call)** were made by the designer and can be overruled.

## Intent

Holland-LR produces its maps in ArcGIS Pro today and wants to produce them in RustGIS. Two things
stand in the way: the layout templates (six `.pagx` files, one per page size) exist only in Pro,
and the project maps (`.aprx`), with years of symbology and labelling, exist only in Pro. This
design adds two import tools:

1. **Layout import**: RustGIS's existing Import Layout tool reads `.pagx` as well as
   `.rgislayout`, and the result is as close to the Pro layout as possible, including dynamic text
   that stays linked to the right map frame. **(Mike)** "Any differences would be not good."
2. **Map import**: opening an `.aprx` shows a picker of its maps and layouts; each map ticked
   becomes a RustGIS map with every layer of Pro's Contents pane, its symbology and its labels.

Both read the same format, Esri's Cartographic Information Model (CIM): a `.pagx` is one JSON
document, an `.aprx` is a zip of such documents. A shared reader serves both tools.

**Order of delivery (Mike):** layouts first, then maps. The layout templates are ready to use
the day they import; a map import pays off only once its data sources open.

## Goals

- The six Holland-LR templates (`11x17 LANDSCAPE`, `11x17 PORTRAIT`, `8x11 LANDSCAPE`,
  `8x11 LANDSCAPE - MH FIX`, `8x11 PORTRAIT`, `ARCH D`) import and print as they do from Pro.
- Dynamic text keeps reading from the frame Pro linked it to (scale and spatial reference of
  `MAIN MAP`), the project name, the date and the service layer credits.
- Every layer in a Pro map's Contents pane appears in RustGIS's Layers pane, in the same group
  tree, with the same name, visibility, scale range, symbology, labels and definition query.
- A layer whose data RustGIS can't reach still appears, as a broken layer that keeps its style
  and can be repaired later. **(Mike)**
- What can't be carried over is said, layer by layer and element by element, as the QGIS project
  importer does with its notes (`rustgis_map::qgis::Report`). Nothing is dropped silently.

## Non-goals

- Writing `.aprx`, `.pagx` or `.stylx` files (export to Pro).
- Importing Pro's favourites and styles (`.stylx`). A follow-on project; see the end.
- Matching Maplex label placement pixel for pixel. The placement engines differ; the import maps
  each Pro placement choice to the nearest RustGIS one and stops there. Mike wants a new, simpler
  label engine for RustGIS eventually; that is a later project, not this one.
- Pro's 3D scenes, animations, charts, pop-ups, metadata, geoprocessing history, toolboxes,
  styles and folder connections in a project.
- Reading ArcMap-era items an `.aprx` still carries as XML (`new_data_frame.xml`,
  `new_ownership_map.xml`). They are noted and skipped.

## Anti-goals

- An import that quietly simplifies. A note saying "drawn differently" is fine; a layout that
  looks right but has lost the link between its scale text and its frame is a failure.
- Putting the importer in the ArcGIS Online plugin. The plugin can only issue commands; the
  symbology mapping needs the map crate's own types. **(call)** The importer lives in core
  RustGIS, in the `hlr` fork, written to the repo's conventions so it can be offered upstream.
- Crashing on a hostile or odd file. CIM documents are user data: bounded recursion, bounded
  lists, `get()` over indexing, sizes clamped, as `AGENTS.md` requires.

## Constraints

- Everything the repo's `AGENTS.md` says: never crash, no `unwrap`, builds for
  `wasm32-unknown-unknown` (the reader is pure Rust over `serde_json`; file and zip access go
  through the paths the QGIS importer already uses: `path` natively, `text`/`data` on the web),
  every tool is a command, interface text in `tl!`, `cargo xtask ci` green.
- Follows the QGIS importer's shape: `crates/map/src/qgis/` (reader, into map types, with a
  `Report`) and `crates/engine/src/cmd/qgis.rs` (commands). The ArcGIS Pro one is
  `crates/map/src/cim/` and `crates/engine/src/cmd/arcgis_pro.rs` (`cmd/arcgis.rs` already
  holds the ArcGIS sign-in commands and stays as it is).
- Existing `.rgislayout` and project files keep reading: every new field on a layout element or
  a layer has a serde default and is left out when unset.
- Test files: the six templates and the Ovintiv project are Mike's working files on `G:` and are
  not committed. Tests use small synthetic CIM fixtures that exercise each mapping; the real files
  are used for manual verification (below).

## Part 1: the CIM reader (shared)

A module `rustgis_map::arcgis` that turns CIM JSON into map and layout types.

**Documents and references.** A `.pagx` is a `CIMLayoutDocument` with `layoutDefinition`,
`mapDefinitions` (maps the layout's frames embed), `layerDefinitions` (their layers),
`binaryReferences` (metadata XML, pictures) and `elevationSurfaceLayerDefinitions`. A `.mapx` is a
`CIMMapDocument` of the same shape without the layout. An `.aprx` is a zip: `GISProject.json`
(`projectItems` of `itemType` `Map` and `Layout`, each with a `catalogPath` of the form
`CIMPATH=folder/file.json`), one JSON per map (`CIMMap` with a `layers` list of `CIMPATH=` URIs),
one per layer, one per layout. The reader resolves `CIMPATH=` URIs against the zip's entries
(case-insensitively: the Ovintiv project has both `MAIN_MAP1/` and `main_map/`) or against the
embedded definition lists of a `.pagx`/`.mapx`. An `.aprx` can hold a thousand entries and three
megabytes; the reader reads only the entries it is pointed at.

**Units.** Page units are a `uwkid` (109008 inches in all six templates; also millimetres,
centimetres, points). Layout coordinates are in page units from the page's bottom-left corner;
RustGIS's are millimetres from the top-left, so every y is flipped: `y_rg = page_h - (y + h)`.
CIM sizes, widths and font heights are typographic points (1/72 inch). RustGIS's stroke
widths, marker sizes, rectangle and line widths and the extent indicator's width are 1/96 inch
(`PT_PER_MM = 96 / 25.4` in `layout.rs`; the QGIS importer multiplies real points by 96/72 in
`in_points`), while a layout `Text` element's `size` and `letter_spacing` are true 1/72 points.
One helper in the reader applies the 96/72 factor to every width and marker size and leaves
text height and letter spacing alone; a 1 pt border in Pro must come out 1 pt wide on paper.

**Colours.** `CIMRGBColor` (values 0–255, alpha 0–100), `CIMCMYKColor`, `CIMHSVColor`,
`CIMHSLColor`, `CIMGrayColor`, `CIMLABColor` all become `Rgba`; colour spaces are ignored.

**Symbols.** A CIM symbol is a stack of symbol layers drawn bottom to top. The mapping to
RustGIS's `Symbol`, `LineStroke`, `FillLayer`, `Marker` and `LabelClass` is in Part 3; Part 2
uses the same code for layout graphics, borders and backgrounds.

**Report.** The same `Report` of `approximated` and `skipped` notes, keyed by layer or element
name, counted, bounded, returned by the commands and summarised in a notice.

## Part 2: layout import (first)

### Entry points

- `layout.import` accepts `.pagx` by extension, or by sniffing a JSON `"type": "CIMLayoutDocument"`
  when given `text`. The Import Layout button's file filter, the Browser pane's open action and
  dropping a file on the window all take `.pagx` beside `.rgislayout`.
- A `.pagx` carries no project, so the command takes an optional `maps` parameter mapping frame
  names to RustGIS map names (`{"MAIN MAP": "Ovintiv"}`). On the desktop, when a frame isn't
  bound (see Map frames below), a small dialog asks which map each unbound frame shows, defaulting
  to the active map. On the command line and in the assistant the default is used silently and a
  note says so.
- Layouts inside an `.aprx` come through the map import's picker (Part 3) and the same code.

### Page and guides

Page width and height in millimetres; guides as `Guides` (x and y lists, y flipped). Printer
preferences are ignored. Page units other than inches, millimetres, centimetres and points are
noted and taken as inches.

### Element geometry

Three things in CIM decide where an element lands, and the import resolves all three into the
plain box RustGIS stores:

1. **Frame or shape.** A frame element (`CIMMapFrame`, `CIMLegend`, scale bars, north arrows,
   `CIMGroupElement`, table and chart frames) has a `frame` polygon or envelope. A
   `CIMGraphicElement` has its `graphic`'s `shape`: a polygon for rectangles, a path for lines, an
   envelope for paragraph text and pictures, a point for point text.
2. **Rotation.** `rotation` (degrees) turns the shape about the element's `anchor` point
   (`TopLeftCorner`, `CenterPoint`, `RightMidPoint` and so on). The template's `LOGO RIBBON` is a
   rectangle stored below the page and rotated 180° about its top-left anchor, which puts it in
   the top-left corner behind the logo. The import applies the rotation and stores the resulting
   box; a rotation that isn't a multiple of 90° is applied to the box's corners and the bounding
   box kept, with a note, since RustGIS elements don't rotate.
3. **Point text.** A `CIMTextGraphic` has a point and an anchor, not a box. The import makes a box
   from the point, the anchor, the font size and the measured width of the text (one line per
   `\n`), so the text lands where Pro draws it. How to measure is left to the builder; RustGIS's
   renderer already measures text for labels.

Elements wholly or partly off the page (the template's `EXHIBIT A` and `SUBTITLE` sit above the
top edge as placeholders; `MAP PROJECT NAME` hangs below the bottom edge) keep their coordinates.
`Layout::clamp` already allows ±5000 mm.

Element `name`, `visible` and `locked` carry over.

### Element kinds

| CIM | RustGIS | Notes |
|---|---|---|
| `CIMMapFrame` | `MapFrame` | Below. |
| `CIMGraphicElement` with `CIMPolygonGraphic` | `Rectangle` (a four-corner ring) or `Ellipse`; other polygons: bounding `Rectangle` with a note | Fill and outline from the polygon symbol's solid fill and stroke; a rounded-corner effect becomes `radius`. |
| … with `CIMLineGraphic` | `Line` | Two-point paths; direction from the end points; arrowheads from marker layers at the ends. Longer paths: a bounding `Line` with a note. |
| … with `CIMTextGraphic` / `CIMParagraphTextGraphic` | `Text` | Below. |
| … with `CIMPictureGraphic` | `Picture` | Below. |
| `CIMGroupElement` | `Group` (new) | Below. |
| `CIMLegend` | `Legend` | Below. |
| `CIMDoubleFillScaleBar`, `CIMSingleFillScaleBar`, `CIMLineScaleBar`, `CIMSteppedLineScaleBar`, `CIMHollowScaleBar`, `CIMAlternatingScaleBar` | `ScaleBar` | Below. |
| `CIMMarkerNorthArrow` | `NorthArrow` | Below. |
| `CIMTableFrame` | `Table` | Layer and row count; the rest noted. |
| `CIMChartFrame` | skipped, noted | Pro charts don't map to RustGIS's bar chart. |
| Map grids in a frame (`CIMMapGrid` kinds) | the frame's `Grid` | Graticule and measured grids; the rest noted. |
| `CIMMapSeries` on the layout | `MapSeries` | When its index layer is in the frame's map; else noted. |
| Anything else | skipped, noted by type and name | |

### Map frames

- **Binding.** A frame with a `uRI` shows a map embedded in the `.pagx` (`mapDefinitions`): the
  template's `SMALL DETAIL MAP` shows `NEW DETAIL MAP`, two shapefile layers. The import adds that
  map to the project through Part 3's map reader and binds the frame to it. (Since layouts ship
  first, the map reader is built in Part 2 to the depth these embedded maps need, feature layers
  from files with simple and unique-value renderers, and completed in Part 3.) When the project
  already holds a map of that name whose layers have the same sources, the frame binds to it
  instead of adding another **(call)**: all six templates embed the same `NEW DETAIL MAP`, and
  importing them into one project must not pile up six copies. A frame without a
  `uRI` (`MAIN MAP`) is bound through the `maps` parameter or the dialog, default the active map.
  Inside an `.aprx`, a frame's `uRI` names one of the project's maps; ticking a layout in the
  picker ticks the maps its frames show.
- **Camera.** `view.camera` gives x, y in the map's spatial reference and a scale; the import
  converts to a longitude/latitude centre and keeps the scale. `autoCamera` of type `Extent`
  fits the `extent` instead. An unbound frame (a template's `MAIN MAP`) carries the camera of
  whatever project it was last saved from and no spatial reference for it, so its saved camera
  is ignored: the frame shows what the bound map is showing at import **(Mike)**, centre and
  scale from the map's view. The saved camera is kept only as a note.
- **Linked frames (new).** `autoCamera.source = MapFrameLink` with `mapFrameLinkName` and
  `autoCameraType` of `Center`, `Extent` or `Scale`, plus `syncRotation`, means the frame follows
  another at draw time. **(Mike)** This must keep working after import, not just be resolved
  once. `MapFrame` gains `link: Option<FrameLink { frame: u32, mode: Center | Extent | Scale,
  rotation: bool }>`; the renderer works a linked frame's camera out from its source frame's
  before drawing; the layout view shows the link in the frame's properties and lets it be set
  and cleared; a frame can't link to itself or in a cycle (the second link in a cycle is dropped
  with a note).
- **Extent indicators.** Each `extentIndicators[]` entry names a `sourceMapFrame` and a symbol.
  RustGIS keeps one indicator per frame (`extent_of`, `extent_color`, `extent_width`): the first
  is taken, with its stroke colour and width; further ones are noted.
- **Border and background (new).** `graphicFrame.borderSymbol` (a line symbol), `backgroundSymbol`
  (a fill) and `shadowSymbol` are Pro's per-element frame styling. **(Mike)** Border and
  background must come across. Since legends, scale bars, pictures and groups have the same
  `graphicFrame`, this goes on `Element`, not on `MapFrame`: `frame_style: Option<FrameStyle {
  border: Option<(Rgba, f64)>, background: Option<Rgba>, gap: f64 }>`, drawn behind and around
  any element kind. Shadows are noted and dropped.

### Text

- Font: `fontFamilyName` → `font_name` (drawn when installed, else the fallback family);
  `fontStyleName` → `bold` / `italic` (Bold, Italic, Bold Italic; Regular and Light as plain).
  `height` → `size`; the text symbol's fill colour → `color`; `horizontalAlignment` → `align`;
  `letterSpacing` (percent) → `letter_spacing` (points, computed from the size).
- A `CIMParagraphTextGraphic` keeps its envelope. Pro wraps paragraph text at the envelope's
  width and aligns it vertically; RustGIS today breaks only at `
` and draws from the top. The
  templates depend on both (the disclaimer is a 330-character paragraph in a 4 × 0.5 in box;
  the spatial reference and project name sit at the bottom of theirs), so `Text` gains
  **(call, from Mike's bar)** `wrap: bool` (lines broken at the box's width when drawn, so text
  whose tokens vary in length, like `{credits}`, still fits), `valign: Top | Middle | Bottom`,
  and `line_spacing: f64` (Pro's `lineSpacing`, a multiple). Import sets `wrap` for paragraph
  text and leaves it off for point text.
- `textCase: Allcaps` becomes `case: Upper` on `Text`, applied when drawn so tokens are
  upper-cased too; `Lowercase` likewise. `smallCaps` is drawn as upper case at the same size and
  noted (RustGIS has no small caps).
- Halos, shadows and callouts on layout text are noted and dropped (none of the six templates
  use them).
- Pro's inline formatting tags (`<BOL>`, `<ITA>`, `<CLR red="…">`, `<FNT size="…">`, `<UND>`,
  `<SUP>`, `<SUB>`) are stripped, with a note naming the element, because a RustGIS text element
  has one style. `<dyn …/>` tags become tokens (next).

### Dynamic text

Pro writes `<dyn type="…" property="…" …/>` tags with `preStr`, `postStr`, `newLine` and
`emptyStr` attributes. RustGIS writes `{token}` and today reads the map tokens from the page's
first map frame. The template needs:

| Pro tag | RustGIS |
|---|---|
| `mapFrame … property="scale"` | `{scale}` |
| `mapFrame … property="sr" srProperty="name"` | `{crsName}` (new) |
| `… srProperty="projection"` | `{crsProjection}` (new) |
| `… srProperty="units"` | `{crsUnits}` (new) |
| `… srProperty="wkid"` | `{crsWkid}` (new) |
| `mapFrame … property="center"` / `extent` | `{center}`; extent new as `{extent}` |
| `project property="name"` | `{project}` (new): the project's name, or its file name without extension |
| `layout … property="serviceLayerCredits"` | `{credits}` (new): the attribution of the web layers the layout's frames draw, as RustGIS already draws on the map |
| `layout property="name"` | `{layout}` |
| `date format="short|long|…"` | `{date}`; the format is noted and ignored **(Mike, accepted)** |
| `time` | `{time}` |
| `page property="number"`, `pageName`, `pageCount` | `{page}`, `{pageName}`, `{pages}` |
| `user`, `computer`, `project path`, metadata properties | noted and dropped |

Three behaviours are new and **required (Mike)**:

1. **Text bound to a named frame.** `Text` gains `frame: Option<u32>`; tokens in a text element
   read from that frame's map and camera, and from the first frame only when unset (today's
   behaviour, so existing layouts don't change). The import binds each text element to the frame
   its tags name; a text whose tags name two different frames takes the first and is noted. The
   layout view shows and lets the user change the frame in the text's properties, as it does for
   a scale bar.
2. **Prefix and suffix dropped when the value is empty.** `preStr="Name: " emptyStr=""` means
   "print `Name: ` and the value, or nothing at all when the value is empty". The token syntax
   needs a way to say this; the exact syntax is left to the builder (one option:
   `{crsName|Name: |\n}` as token, prefix, suffix). `newLine="true"` is a line break after the
   value, inside the conditional part.
3. **The new tokens above**, with `crsName` the coordinate system's full name (`NAD 1983
   StatePlane Texas Central FIPS 4203 (US Feet)`), `crsProjection` its method, `crsUnits` its
   linear or angular unit, `crsWkid` its EPSG code when it has one. RustGIS labels a projection
   by its code today, so **(Mike)** it gains a table of EPSG codes to names (and to method and
   unit where the PROJ string doesn't say), shipped as a data file under `crates/map` with its
   row in `ATTRIBUTION.md` (the EPSG dataset's terms allow this with attribution), used by the
   tokens, the projection dialog and the status bar. Codes outside the table fall back to
   `EPSG:nnnn`.

A `dyn type="layout"` tag reads from the layout it sits in, whatever its `name` attribute says
**(call)**: the templates' credits tags name layouts that no longer exist (`8x11 LANDSCAPE`,
`NEW STYLE LETTER LANDSCAPE`, `8 1/2 x 11 Portrait`) because they were copied from one another,
and what Mike means by them is "this layout's credits". A `mapFrame` tag naming a frame that
isn't on the page expands to nothing, with a note.

### Pictures

`sourceURL` is a file path or a `data:` URI. A `data:` URI is decoded and embedded. A path is
read on the desktop (relative to the `.pagx`'s folder when relative) and embedded; GIF and BMP
are converted to PNG (the `image` crate is built with `png` and `jpeg` only; the builder
enables `gif` and `bmp`). A path that can't be read gives an empty `Picture` with the path kept in a
new `source: Option<String>` field, shown in the element's properties so it can be relinked, and
a note. On the web a path can't be read and is noted.

### Groups (new)

**(Mike)** Groups make moving things easier and are wanted. `ElementKind::Group { children:
Vec<u32> }` **(call)**, an element whose box is the union of its children's; children also get
`group: Option<u32>` so either direction is cheap. Selecting a group selects its children; moving,
resizing, deleting, duplicating, locking and hiding act on all; a Group and Ungroup command pair
and ribbon buttons; the Layout pane lists children under their group; groups nest (bounded
depth, as layer groups are). Rendering order is the element order, as today. Import keeps Pro's
groups and nesting.

### Legend

`CIMLegend` carries `title`/`showTitle`, `autoAdd`, `autoVisibility`, `autoReorder`, `columns`,
`fittingStrategy` and `items` (one `CIMLegendItem` per layer: `showLayerName`, `showHeading`,
label overrides, patch sizes, `keepTogether`). `Legend` gains **(Mike)**: `columns: usize` and
`fit: LegendFit { AdjustSize | AdjustColumns | AdjustColumnsAndSize | AdjustFrame | Manual }`
honoured by the renderer, plus `show_layer_names: bool`. Per-item patch sizes and label
overrides are noted. `autoAdd=false` with an explicit item list becomes `exclude` for the layers
not listed.

### Scale bar

**(Mike)** Subdivisions, label placement, the unit label and the number format must come across.
`ScaleBar` gains `subdivisions: usize`, `divisions_before_zero: usize`, `label_position: Above |
Below`, `unit_label: Option<String>` with `unit_label_position: AfterBar | AfterLabels | Above |
Below`, `decimals: usize`, `thousands_separator: bool`, and a label `size`/`font`. The CIM style
maps to `ScaleBarStyle`: double fill → alternating, single fill → line with fill, stepped line →
stepped, line → line, hollow → alternating with no fill; `units` from the `uwkid` (9002 feet,
9001 metres, 9036 kilometres, 9093 miles); `fittingStrategy` (`AdjustDivision`,
`AdjustDivisionValue`, `AdjustWidth`…) becomes a `fit` field; the renderer keeps the bar's
division value a round number under `AdjustDivision` and `AdjustDivisionValue`, as Pro does.
Mark heights and frequencies are noted.

### North arrow

**(Mike)** The arrow must look as it does in Pro. `CIMMarkerNorthArrow.pointSymbol` is a point
symbol (the template's is a picture marker under a character marker). `NorthArrow` gains
`symbol: Option<Symbol>` drawn at the element's size and turned to the frame's bearing plus
`calibrationAngle`; this reuses Part 3's point symbol mapping, including the new image and path
markers, so a picture, a glyph or a drawn arrow all work. Without a symbol the four RustGIS styles
remain. `northType` other than true north (magnetic, grid) is noted.

### Verification for Part 2

- Unit tests on synthetic fixtures for each row of the tables above, for rotation about each
  anchor, for point-anchored text at each anchor, for every `dyn` tag, for a `.pagx` with an
  embedded map, and for a hostile file (deep nesting, huge counts, bad numbers, missing fields);
  `hostile_params_never_panic` stays green.
- Manual: Mike exports each of the six templates from Pro as PDF at the same page size, imports
  the `.pagx` into a project holding a map named for `MAIN MAP`, exports a PDF from RustGIS, and
  compares the two side by side. Fonts Roboto, Roboto Condensed, Arial and Ebrima are installed on
  the machine. The acceptance bar: every element at the same place (within half a millimetre),
  same size, same text, same font family; the notes list contains only the differences this
  design accepts (date format, text formatting tags, shadows).

## Part 3: map import (second)

### Entry points

- `project.importArcgis` (Project › Import › Import ArcGIS Pro Project) with `path` (`.aprx`,
  `.mapx`, or `.lyrx` for one layer), `text`/`data` as the QGIS importer, `maps: [names]`,
  `layouts: [names]`, `dir`. Without `maps`/`layouts` on the desktop, the picker opens; on the
  command line, nothing is imported and the result lists what the file holds (so a script can
  choose).
- **The picker (Mike).** A dialog listing the project's maps and layouts with a tick box each,
  the map Pro had open last ticked (from `GISProject.json`'s `views`) and nothing else; an "All"
  and a "None" link; ticking a layout ticks the maps its frames show. Dropping an `.aprx` on the
  window opens the picker; the Browser pane opens it too.
- ArcMap-era XML items are listed greyed out with "saved by ArcMap; open it in Pro first".

### Maps

One RustGIS map per ticked map, named as in Pro (made unique as `map.new` does). Spatial
reference `wkid` → `map.projection` (Web Mercator, the EPSG code, or a note and Web Mercator);
`defaultExtent` or the saved camera → the view; `referenceScale` → the map's reference scale.
Basemap blank: Pro's basemaps are ordinary layers and come in as such. Group layers →
`LayerGroup` (nested, `visibility`, `expanded`, transparency, scale ranges), in Pro's order.

### Layer sources

| Pro connection | RustGIS |
|---|---|
| `FileGDB` (`CIMStandardDataConnection` / `CIMFeatureDatasetDataConnection`, `workspaceConnectionString` `DATABASE=…gdb`, `dataset`) | `layer.addData` on the `.gdb` with `layer` = dataset (desktop; broken on the web) |
| `Shapefile` | `layer.addData` on the `.shp` |
| `Raster` (`CIMStandardDataConnection` to a file) | `layer.addData` when RustGIS reads the format (GeoTIFF, ASCII grid; JPEG/PNG with a world file through GDAL when installed); else broken |
| `FeatureService` (`workspaceConnectionString` `URL=…`, `dataset` the layer id) | `service.add` type arcgis; broken when the service needs a sign-in (`arcgis.signIn`) that hasn't happened, with the URL recorded |
| `CIMAGSServiceConnection` tiled map / image service | `service.add` as RustGIS adds ArcGIS map and image services |
| `CIMVectorTileDataConnection` | `map.addTiles` vector with the style URL |
| `SDE` (enterprise geodatabase) | broken, the connection string (server, database, table) recorded |
| `Cad` (a DWG/DXF feature class) | GDAL when installed, else broken with the path recorded |
| `CIMRelQueryTableDataConnection` (a join) | the base table; the join noted |
| anything else | broken, noted with the connection type |

Relative paths resolve against `dir`, else the `.aprx`'s folder (`defaultFolder` in
`GISProject.json` is tried too).

### Broken layers (new)

**(Mike)** A layer whose source can't be opened is still added: same name, place in the tree,
visibility, scale range, symbology, labels and definition query, with a new `origin:
Option<String>` on `SourceLayer` holding a readable description of where Pro read it from, and no
open handle. It draws nothing. The Layers pane shows a warning glyph with the origin in its
tooltip; its context menu offers Repair, which is the existing relink (`relinked`) for a file, and
Retry for a service (reopen after `arcgis.signIn`). `connections` already reopens unopened sources
after a login; broken service layers join that path. Saving a project keeps broken layers broken,
not dropped.

### Renderers

| Pro | RustGIS |
|---|---|
| `CIMSimpleRenderer` | `Single` |
| `CIMUniqueValueRenderer`, one field | `Unique` with `classes` from `groups[].classes[]` (`values[].fieldValues[0]` → `label`), `other` from `defaultSymbol` when `useDefaultSymbol`, `other_label` from `defaultLabel`, headings from `groups[].heading` into `class_labels` |
| … several fields | `Unique` on the first field; the others noted. (Mike may want a combined virtual field later; not in this design.) |
| `CIMClassBreaksRenderer` (graduated) | `Graduated` with `breaks`, `method` from `classificationMethod`, `min` from `minimumBreak` |
| … `classBreakType = UnclassedColor` | `Graduated` over the colour ramp with many classes, noted |
| `CIMProportionalRenderer` | `Proportional` |
| `visualVariables` size by field on a simple renderer | `Proportional` |
| heat map, dot density, chart, dictionary, representation renderers | `Single` from the first symbol found, noted as skipped |

### Symbols

Polygon symbol layers, bottom to top: `CIMSolidFill` → the fill colour (the top one wins; the
others become `FillLayer`s under it); `CIMHatchFill` → `FillPattern::Hatch` (angle, separation,
the hatch line's width and colour); `CIMGradientFill` → `FillPattern::Gradient`;
`CIMPictureFill` → noted; a `CIMVectorMarker` fill with `CIMMarkerPlacementInsidePolygon` →
`FillPattern::Dots`; `CIMSolidStroke` → the outline (the top one as `stroke`/`stroke_width`,
all of them as `class_strokes`/`other_strokes` `LineStroke`s with cap, join, colour, width).

Line symbol layers: each `CIMSolidStroke` → a `LineStroke`; `effects`: `CIMGeometricEffectDashes`
(`dashTemplate`) → `Dash`, `CIMGeometricEffectOffset` → the stroke's offset,
`CIMGeometricEffectCut`, `Arrow`, `Buffer`, `Wave` and the rest → noted; `CIMGradientStroke` →
a solid stroke in its first colour, noted; a `CIMVectorMarker` layer with
`CIMMarkerPlacementAlongLine…` / `AtExtremities` / `AtRatioPositions` → `LineMarkers`
(`Placements`, interval, offsets, the marker from the point mapping).

Point symbol layers:

- `CIMVectorMarker` whose `markerGraphics` draw one of the shapes RustGIS has (a circle from
  curve rings, a square or diamond from a four-corner ring and the marker's rotation, a triangle,
  a star, a cross or line from a two-point path) → that `Marker` with `size`, fill and outline.
  In the Ovintiv project this covers about nine in ten.
- Any other vector marker → **`Marker::Path` (new, call)**: the marker's graphics as normalised
  paths with per-path fill and stroke, scaled to `size`. Drawn by the renderer on the CPU and
  GPU paths, used in legends and the symbol dialog ("Custom").
- `CIMPictureMarker` (`url` a `data:` PNG/GIF/JPEG) → **`Marker::Image` (new, Mike)**: PNG
  bytes (GIF/JPEG converted), `size`, drawn at the point, in legends, and in the symbol dialog
  (choose a file; the import fills it). Tinting (`tintColor`) noted.
- `CIMCharacterMarker` → `Marker::Char` with the font and character index.
- Marker `rotation`, `offsetX/Y`, `anchorPoint` → `MarkerRotation`/offset where RustGIS has them;
  `dominantSizeAxis3D`, `rotateClockwise`, halos on markers noted.

Several marker layers stack into one symbol; the renderer draws them bottom to top (RustGIS's
`Symbol` is one marker, so a stacked point symbol needs either a `Vec` of markers or a `Path`
marker that holds the stack; **left to the builder**, with the note that the north arrow and
the stylx follow-on both want the same thing).

### Labels

`labelVisibility` → `Labeling.enabled`; each `CIMLabelClass` → a `LabelClass` (`name`,
`visibility`, `whereClause` → `where`, `minimumScale`/`maximumScale`, the text symbol → `size`,
`color`, `bold`, `italic`, `font`, halo from `haloSize` and `haloSymbol`'s colour, `callout` of
`CIMBackgroundCallout` → `LabelBackground`; `CIMSimpleLineCallout` and composite callouts → a
plain label, noted).

**Label expressions (new, Mike).** `expression` with `expressionEngine` Arcade, VBScript, Python
or JScript. `LabelClass.field` becomes a template: literal text with `{Field}` placeholders and
`\n` line breaks, evaluated per feature (a template that is just `{Field}` is today's field
label, so existing projects read unchanged). The import translates: field references
(`$feature.X`, `$feature["X"]`, `[X]`, `!X!`), string concatenation (`+`, `&`), string literals,
and newlines (`"\n"`, `vbNewLine`, `TextFormatting.NewLine`, `"<br>"`). Anything else (a
function call, arithmetic, a conditional) → the first field referenced, with the expression in
the note. The label pane shows the template in a text box with a field picker.

**Placement.** Maplex (`maplexLabelPlacementProperties`) by `featureType`:

| Pro | RustGIS |
|---|---|
| Point `AroundPoint` | `Auto` |
| Point `CenteredOnPoint` | `Center` |
| Point `NorthOfPoint` / `South…` / `East…` / `West…` / `NorthEast…` … | `Above` / `Below` / `Right` / `Left` / `AboveRight` … |
| Line `OffsetStraightFromLine`, `CenteredStraightOnLine`, `CenteredHorizontalOnLine` | `Parallel` |
| Line `OffsetCurvedFromLine`, `CenteredCurvedOnLine` | `Curved` |
| Polygon `HorizontalInPolygon`, `BoundaryPlacement` | `Center` |
| Polygon `StraightInPolygon` | `Parallel` |
| Polygon `CurvedInPolygon` | `Curved` |
| Polygon `RepeatAlongBoundary` | `Curved` with `repeat` |

Standard placement (`standardLabelPlacementProperties`) maps the same way from its point and
line positions. Stacking separators → `wrap_char`; `canOverrunFeature`, priorities, zone
priorities, conflict weights, key numbering, offsets along line → noted once per layer.

### Other layer properties

`name`, `visibility`, `minScale`/`maxScale`, `transparency`, `blendingMode` (where RustGIS has
the mode) carry over. `featureTable.definitionExpression` → the definition query (SQL as it is;
RustGIS reads SQL where clauses). `displayFilters`, `timeFilter`, selection, `charts`, pop-ups,
`layerMasks`, `featureReduction`, `symbolLayerDrawing` → noted. Standalone tables → RustGIS
standalone tables when their source opens.

### Verification for Part 3

- Unit tests per table row on synthetic fixtures; a synthetic `.aprx` zip with two maps, a
  layout and an ArcMap XML item; hostile input tests.
- Manual: import the Ovintiv project's `Ovintiv` map on Mike's machine with `G:` mounted and
  ArcGIS Online signed in. Acceptance: the Layers pane lists every layer of Pro's Contents in the
  same tree; each file-backed layer draws with the same colours, widths, markers and labels at
  the same zoom (compared by screenshot against Pro); service layers draw or are broken with the
  right URL; SDE and CAD layers are broken with their origin; the notes name every difference.

## Left to the builder

- The token syntax for prefix/suffix-when-empty and frame binding, and whether `{credits}` reads
  all frames or the bound one.
- Measuring point-anchored text at import.
- Whether a stacked point symbol is a `Vec<Marker>` or a `Path` marker holding the stack.
- The picker dialog's look; the broken-layer warning glyph; where Group/Ungroup sit on the ribbon
  (follow the ribbon guidelines).
- The Arcade subset beyond concatenation.
- Fixture design.
- Upstreaming: open an issue on opengeos/RustGIS describing the feature once Part 2 lands.

## Follow-ons (not in this design)

1. **Favourites and styles (`.stylx`).** A SQLite database of CIM symbols. Reuses Part 3's symbol
   mapping and the new marker kinds; each row becomes an `.rgisstyle` item. Small once this
   design ships.
2. **A new label engine** for RustGIS, simpler than Maplex. Mike's wish; this design only maps
   placements to what RustGIS has.
3. **`.lyrx` layer files** beyond a single layer; multi-field unique values as a combined field.
