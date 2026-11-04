# ArcGIS Pro Map Import: Verification Checklist

Task 15 of `2026-11-01-arcgis-pro-map-import.md`, run on 2026-11-03 against the user's real test project. The
project and everything in it stay private. This file describes them in general terms only.

**The test project:** an `.aprx` with 12 maps and 7 layouts. The main map (the one whose pane was open)
has 72 Contents entries: 58 layers and 14 groups, nested up to 3 deep. Its sources are FileGDB feature classes (some
inside feature datasets), shapefiles, 2 JPEG rasters, 8 SDE feature classes, 12 hosted feature service
layers (8 need sign-in), a vector tile layer, 2 tiled map services and an image service. It also has 2 standalone tables.

## What ran headless

The checks were run with `rustgis-cli`, the CLI build of the importer at this branch's head. The desktop window and ArcGIS Pro were not available.

- [x] **Inspect.** `project.inspectArcgis` lists all 12 maps and 7 layouts, as Pro's `projectItems` does. The open map
  is flagged `open`, and each layout lists the maps its frames show.
- [x] **Step 1: import the main map.** Run as `project.importArcgis` with `maps: [the main map]`. The notice reads
  "Imported 1 maps and 0 layouts: 58 layers, 20 broken; 36 things drawn differently, 85 left out".
  Notes by kind:
  - Broken layers, each noted with where Pro read it: 8 SDE layers ("data in SDE workspaces, which RustGIS can't
    open"), 8 feature service layers that need a token, 1 image service the network refused, 1 FileGDB table too
    large to read, and 2 JPEG rasters (see the decisions below).
  - Approximated: selection symbols (20), small caps shown as upper case (11), dashed polygon outlines drawn solid (2),
    a drawn marker shape shown as a circle, a joined field read by its own name, and an Arcade `Round(…)` label
    expression drawn as its field.
  - Left out: Maplex "can overrun feature" (46), feature weights (21), pop-ups (12), leader-line callouts (2), thinning
    duplicate labels (2), label offsets along a line, and a label field the layer lacks.
- [x] **Step 2: compare the layer tree with Pro's Contents (headless).** A script walked the map's raw CIM (names,
  group nesting, order, visibility, expanded, connection type) and compared it with the saved, imported project.
  After the fixes below, all 72 entries match in order, nesting depth, group or layer, visibility and expanded state.
  The tiled and vector tile layers come in as tile layers at the same place in the drawing order. The only remaining
  difference is names (see decision 1).
  - A second map (305 entries, 251 layers, 55 groups, with WMS, KML, annotation, graphics and map image layers)
    brought in all 251 layers.
- [x] **Step 4: import a layout with its maps.** Run as `project.importArcgis` with `layouts: [an open layout]`. It brings
  in the layout and its 2 maps (60 layers). Exported with `layout.export`: the main frame shows the subject section at
  its saved scale, and the overview frame, the logo picture, north arrow, scale bar, legend and title texts all come in.
- [x] **Step 5: headless check.** `rustgis-cli run --cmd 'project.importArcgis={"path": "<the .aprx>", "maps": ["<the main map>"]}' --cmd 'share.exportMap={"path": "out.png"}'`
  exports the map at Pro's saved centre and scale. The imagery basemap is at Pro's 50% transparency. The land-survey
  polygons draw with three-line labels and no garbage text.
- [x] **Settled from the real JSON:**
  - (a) A WMS layer is `CIMWMSLayer` whose `subLayers[]` holds a `CIMServiceCompositeSubLayer` (the service's root,
    with a name of its own). That composite holds `CIMWMSSubLayer`s whose `name` is the layer's title and whose
    `subLayerID` is the WMS layer name.
  - (b) The project has no `CIMProportionalRenderer`, so its key names are still unconfirmed.
  - (c) `canStackLabel` and `labelStackingProperties` are in the label class's `maplexLabelPlacementProperties`. `callout`
    is on the label's `CIMTextSymbol`. A shadow is `shadowColor` plus `shadowOffsetX`/`shadowOffsetY` on the text
    symbol. Pro writes an opaque black `shadowColor` on most text symbols, with no offset.
  - (d) Pro omits `expanded` when it is false. `true` is the only value written (a layer or group without it is
    collapsed). Pro writes `scaleSymbols` on every feature layer, `true` 556 times and `false` 8 times, so it does not
    omit it when true.

## Fixed (each with a regression test in `crates/map/src/cim/tests.rs`)

- [x] Layers, maps and layouts that came over from ArcMap keep a `.xml` path in Pro, but the entry holds CIM JSON. 19
  such entries were read as missing: 7 Contents entries of the main map (two whole groups) came in as "isn't defined in
  this document", and 1 map and 1 layout were greyed as ArcMap. `.xml` entries that parse as JSON are now read, and an
  item counts as ArcMap only when its `.xml` path holds no JSON.
- [x] Shadows: every label class with Pro's default black `shadowColor` was noted as "a text shadow". The offset keys
  were also misnamed (`shadowX` where Pro writes `shadowOffsetX`). Now only a shadow with an offset, in a colour that
  shows, is noted.
- [x] WMS: the reader took the composite root's name as the layer to draw. It now looks inside the root and takes the
  sublayer's `subLayerID`.
- [x] Stacked labels: a three-line Arcade expression with a three-line stacking limit was still given a wrap character,
  so every space broke it again ("H / A / 3"). A label whose expression already fills `maximumNumberOfLines` is now not
  stacked further.
- [x] Layout frames: a frame camera in a state plane was dropped ("unsupported coordinate system"), so the frame
  showed the map's own view at the wrong scale. Cameras and extents are now converted through RustGIS's projections (EPSG,
  then Esri codes), and geographic systems are read as degrees.
- [x] `CIMMapImageLayer` (a dynamic map service) was left out as "not drawn". It now comes in as its map service, the same
  way a tiled one does.

## For the user (needs the desktop app, Pro, or sign-in)

- [ ] **Step 1 in the app.** Drop the `.aprx` on the window. The picker should tick the main map and grey nothing: the 2 `.xml`
  items (a map and a layout) hold JSON and now import. Import, and check that the notice matches the one above.
- [ ] **Step 2 in the app.** Compare the Layers pane with Pro's Contents:
  - the warning glyph on the 20 broken layers;
  - each tooltip names where Pro read the layer: the SDE server and table (with no password), the feature service URL
    and layer, and the FileGDB path;
  - after Sign in to ArcGIS, Retry on the 8 token-secured feature service layers opens them.
- [ ] **Step 3: compare the drawing against Pro at three scales** (a township, a section, a parcel). Check colours, widths,
  dash patterns, markers (the drawn and stacked ones in particular), fills, and labels (text, font, size, halo,
  placement, stacking). Check that definition queries draw the same features.
  - **Rotation:** Pro rotates this map about 14°, and RustGIS draws it north up (decision 4).
  - **Stacked labels:** check the land-survey labels against Pro's line breaks.
- [ ] **Step 4 in the app.** Import the open layout with its maps from the picker, and compare the frames, the overview
  frame's extent and the pictures with Pro.

## Decisions for the controller

1. **Same-named layers are renamed.** Pro allows the same name in different groups (the main map has 4 groups, each
   holding a points and a polylines layer with the same 2 names, and 2 layers named alike elsewhere). The import renames
   them "<name> (2)" … "<name> (5)", numbering from the bottom up, and the layout's legend shows a renamed one. Should the import keep Pro's names (rename after adding), or keep RustGIS's unique
   names?
2. **Georeferenced JPEG rasters** (world file `.jgwx` plus `.aux.xml`) go down `layer.addData`'s photo path. They come in
   broken with "1 photo left out: no EXIF". RustGIS reads a JPEG raster only as an unplaced scan and reads no world
   file. The options are to read world files for JPEG and PNG rasters, or to break the layer with a clearer reason.
3. **Maplex `canOverrunFeature: true`** is on 86% of the project's label classes and produces the most frequent note
   (46 on the main map, 206 on the second). RustGIS already lets a line label run about a letter past the line's ends.
   Should `true` be treated as the default and left unnoted?
4. **Map and frame rotation.** Pro's saved camera has a `heading` of about 14° (the map's `defaultRotation` is its
   negative), and its layout frames have about the same `heading`. The import drops both silently. Should the import apply them as the camera
   bearing (the sign needs checking against a Pro screenshot), or note them?
5. **Saved project size.** Saving the imported main map writes a project of about 2.9 GB, because non-streamed file layers are
   saved inline. The largest are FileGDB layers of up to about 1.1 GB. This is RustGIS's general save behaviour, not
   the importer's, but a big Pro map makes it obvious.
