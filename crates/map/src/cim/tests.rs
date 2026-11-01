//! Small CIM documents, written by hand in the shapes ArcGIS Pro writes them.

use serde_json::json;

use super::*;
use crate::Rgba;

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
    let zip =
        rustgis_data::zip::write(&[("GISProject.json".into(), project.into()), ("main_map/main_map.json".into(), map.into())]).unwrap();
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

#[test]
fn a_zip_needs_its_project_and_skips_entries_that_are_not_json() {
    let map = r#"{"type":"CIMMap","name":"M"}"#;
    let no_project = rustgis_data::zip::write(&[("m.json".into(), map.into())]).unwrap();
    assert!(Document::from_zip(&no_project).is_err());
    let odd = rustgis_data::zip::write(&[
        ("gisproject.JSON".into(), br#"{"type":"CIMGISProject"}"#.to_vec()),
        ("broken.json".into(), b"{".to_vec()),
        (r"Maps\M.json".into(), map.into()),
        ("thumb.png".into(), vec![0, 1, 2]),
    ])
    .unwrap();
    let d = Document::from_zip(&odd).unwrap();
    assert!(d.resolve("CIMPATH=broken.json").is_none());
    assert!(d.resolve("CIMPATH=thumb.png").is_none());
    assert_eq!(d.resolve("CIMPATH=maps/m.json").unwrap()["name"], "M");
    // The project's own file is its root, not an entry to resolve.
    assert!(d.resolve("CIMPATH=GISProject.json").is_none());
}

#[test]
fn kinds_follow_the_root_type() {
    let kind = |t: &str| Document::from_json(&format!(r#"{{"type":"{t}"}}"#)).unwrap().kind();
    assert!(matches!(kind("CIMMapDocument"), DocKind::Map));
    assert!(matches!(kind("CIMLayerDocument"), DocKind::Layer));
    assert!(matches!(kind("CIMGISProject"), DocKind::Project));
    assert!(matches!(kind("CIMSomethingNew"), DocKind::Other(t) if t == "CIMSomethingNew"));
    assert!(matches!(Document::from_json("{}").unwrap().kind(), DocKind::Other(t) if t.is_empty()));
    assert!(Document::from_json("[1]").is_err());
}

#[test]
fn the_readers_take_only_what_is_there() {
    let v: Value = serde_json::from_str(r#"{"type":"CIMPointSymbol","name":"n","size":4.5,"on":false,"w":"x"}"#).unwrap();
    assert_eq!(str_of(&v, "name"), Some("n"));
    assert_eq!(str_of(&v, "size"), None);
    assert_eq!(num(&v, "size"), Some(4.5));
    assert_eq!(num(&v, "name"), None);
    assert!(!flag(&v, "on", true));
    assert!(flag(&v, "missing", true));
    assert!(flag(&v, "w", true));
    assert!(typed(&v, "CIMPointSymbol").is_some());
    assert!(typed(&v, "CIMLineSymbol").is_none());
    assert!(typed(&Value::Null, "CIMPointSymbol").is_none());
}

#[test]
fn cim_colours_become_rgba() {
    let rgb = json!({"type":"CIMRGBColor","values":[255, 128, 0, 50]});
    assert_eq!(color::color(&rgb), Some(Rgba::new(255, 128, 0, 128)));
    let cmyk = json!({"type":"CIMCMYKColor","values":[0, 100, 100, 0, 100]});
    assert_eq!(color::color(&cmyk), Some(Rgba::new(255, 0, 0, 255)));
    let gray = json!({"type":"CIMGrayColor","values":[50, 100]});
    assert_eq!(color::color(&gray).map(|c| c.r()), Some(128));
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

#[test]
fn hue_and_lab_colours_and_symbol_references() {
    let c = |ty: &str, values: Value| color::color(&json!({"type": ty, "values": values}));
    assert_eq!(c("CIMHSVColor", json!([120, 100, 100])), Some(Rgba::new(0, 255, 0, 255)));
    assert_eq!(c("CIMHSVColor", json!([-120, 100, 50, 0])), Some(Rgba::new(0, 0, 128, 0)));
    assert_eq!(c("CIMHSLColor", json!([240, 100, 50])), Some(Rgba::new(0, 0, 255, 255)));
    assert_eq!(c("CIMHSLColor", json!([0, 0, 100])), Some(Rgba::new(255, 255, 255, 255)));
    assert_eq!(c("CIMLABColor", json!([100, 0, 0])), Some(Rgba::new(255, 255, 255, 255)));
    assert_eq!(c("CIMLABColor", json!([0, 0, 0])), Some(Rgba::new(0, 0, 0, 255)));
    // sRGB red in Lab (D65), to within a step.
    let red = c("CIMLABColor", json!([53.24, 80.09, 67.2])).map(|c| c.0);
    assert!(red.is_some_and(|[r, g, b, _]| r >= 254 && g <= 2 && b <= 2), "{red:?}");
    // Out of range values are clamped; unreadable ones, an unknown kind and a missing type are not colours.
    assert_eq!(c("CIMRGBColor", json!([300, -5, 0, 500])), Some(Rgba::new(255, 0, 0, 255)));
    assert_eq!(c("CIMRGBColor", json!([0, 0, 0])), Some(Rgba::new(0, 0, 0, 255)));
    assert_eq!(c("CIMCMYKColor", json!([0, 0, 0])), None);
    assert_eq!(c("CIMPatternColor", json!([0, 0, 0])), None);
    assert_eq!(color::color(&json!({"values": [0, 0, 0]})), None);
    assert_eq!(color::color(&json!({"type": "CIMRGBColor"})), None);

    let layers = json!({"type": "CIMPointSymbol", "symbolLayers": []});
    let reference = json!({"type": "CIMSymbolReference", "symbol": layers});
    assert_eq!(color::symbol_of(&reference), Some(&layers));
    assert_eq!(color::symbol_of(&layers), Some(&layers));
    assert_eq!(color::symbol_of(&json!({"type": "CIMSymbolReference"})), None);
    assert_eq!(color::symbol_of(&json!({"type": "CIMRGBColor"})), None);
}

#[test]
fn page_units_come_from_esris_linear_unit_table() {
    let mm = |wkid: i64| units::page_unit_mm(&json!({"units": {"uwkid": wkid}}));
    assert_eq!(mm(9001), Some(1000.0));
    assert_eq!(mm(109006), Some(10.0));
    assert_eq!(mm(109007), Some(1.0));
    assert_eq!(mm(109008), Some(25.4));
    assert!(mm(109016).is_some_and(|p| (p - 25.4 / 72.0).abs() < 1e-9));
    assert_eq!(mm(109001), None);
    assert_eq!(units::page_unit_mm(&json!({})), None);
    assert_eq!(units::page_unit_mm(&json!({"units": {"uwkid": "x"}})), None);
    assert_eq!(units::pt96(-3.0), 0.0);
    assert_eq!(units::pt96(1e9), 1000.0);
    assert_eq!(units::pt96(f64::INFINITY), 0.0);
    assert_eq!(units::text_pt(0.5), 2.0);
    assert_eq!(units::text_pt(f64::NAN), 2.0);
    assert_eq!(units::text_pt(12.0), 12.0);
}

// ---- symbols ----

fn rgb(r: u8, g: u8, b: u8) -> serde_json::Value {
    json!({"type": "CIMRGBColor", "values": [r, g, b, 100]})
}

fn solid_fill(c: serde_json::Value) -> serde_json::Value {
    json!({"type": "CIMSolidFill", "enable": true, "color": c})
}

fn solid_stroke(c: serde_json::Value, width: f64) -> serde_json::Value {
    json!({"type": "CIMSolidStroke", "enable": true, "capStyle": "Round", "joinStyle": "Round", "miterLimit": 10, "width": width, "color": c})
}

fn polygon_symbol(layers: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"type": "CIMPolygonSymbol", "symbolLayers": layers})
}

fn notes(report: &Report) -> Vec<String> {
    report.notes.iter().map(|n| format!("{}: {}", n.layer, n.text)).collect()
}

#[test]
fn polygon_symbols_take_their_top_fill_and_stroke_in_96ths() {
    // Bottom to top, as Pro writes them: a 1 pt black line under two fills.
    let sym = json!({"type": "CIMSymbolReference", "symbol": polygon_symbol(vec![
        solid_stroke(rgb(0, 0, 0), 1.0),
        solid_fill(rgb(255, 255, 255)),
        solid_fill(rgb(255, 0, 0)),
    ])});
    let mut report = Report::default();
    let p = symbol::polygon(&sym, "Frame", &mut report);
    assert_eq!(p.fill, Some(Rgba::rgb(255, 0, 0)));
    let (c, w) = p.stroke.unwrap();
    assert_eq!(c, Rgba::rgb(0, 0, 0));
    assert!((w - 96.0 / 72.0).abs() < 1e-5, "{w}");
    assert_eq!(p.strokes.len(), 1);
    assert_eq!(p.strokes[0].color, Some(Rgba::rgb(0, 0, 0)));
    // Two solid fills are two stacked fills; a lone one is the symbol's own fill.
    assert_eq!(p.fills.len(), 2);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    let one = symbol::polygon(&polygon_symbol(vec![solid_fill(rgb(255, 255, 255))]), "R", &mut report);
    assert_eq!((one.fill, one.stroke, one.fills.len()), (Some(Rgba::rgb(255, 255, 255)), None, 0));
}

#[test]
fn transparent_strokes_count_as_no_stroke() {
    let clear = json!({"type": "CIMRGBColor", "values": [0, 0, 0, 0]});
    let sym = polygon_symbol(vec![
        solid_fill(json!({"type": "CIMRGBColor", "values": [10, 20, 30, 0]})),
        solid_stroke(clear.clone(), 2.0),
        json!({"type": "CIMSolidStroke", "enable": false, "width": 4, "color": rgb(1, 2, 3)}),
    ]);
    let mut report = Report::default();
    let p = symbol::polygon(&sym, "R", &mut report);
    assert_eq!((p.fill, p.stroke), (None, None));
    assert!(p.strokes.is_empty() && p.fills.is_empty());
    let l = symbol::line(&json!({"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(clear, 1.0)]}), "L", &mut report);
    assert!(l.strokes.is_empty() && l.stroke.is_none());
}

#[test]
fn line_strokes_carry_caps_joins_dashes_and_offsets_bottom_first() {
    use crate::symbology::{LineCap, LineJoin};
    let mut casing = solid_stroke(rgb(0, 0, 0), 4.0);
    casing["capStyle"] = json!("Butt");
    casing["joinStyle"] = json!("Miter");
    let mut core = solid_stroke(rgb(255, 255, 0), 2.0);
    core["capStyle"] = json!("Square");
    core["joinStyle"] = json!("Bevel");
    core["effects"] = json!([
        {"type": "CIMGeometricEffectDashes", "dashTemplate": [4.0, 3.0], "lineDashEnding": "HalfPattern", "offsetAlongLine": 0},
        {"type": "CIMGeometricEffectOffset", "method": "Square", "offset": 3.0, "option": "Fast"},
    ]);
    let mut wavy = solid_stroke(rgb(9, 9, 9), 1.0);
    wavy["effects"] = json!([{"type": "CIMGeometricEffectWave", "amplitude": 1}]);
    let sym = json!({"type": "CIMLineSymbol", "symbolLayers": [casing, core, wavy]});
    let mut report = Report::default();
    let l = symbol::line(&sym, "Road", &mut report);
    assert_eq!(l.strokes.len(), 3);
    assert_eq!((l.strokes[0].cap, l.strokes[0].join), (LineCap::Butt, LineJoin::Miter));
    let top = &l.strokes[1];
    assert_eq!((top.cap, top.join), (LineCap::Square, LineJoin::Bevel));
    assert_eq!(top.dash.unwrap().steps(), &[4.0 * 96.0 / 72.0, 3.0 * 96.0 / 72.0]);
    assert!((top.offset - 4.0).abs() < 1e-5, "{}", top.offset);
    assert!((top.width.unwrap() - 2.0 * 96.0 / 72.0).abs() < 1e-5);
    assert_eq!(l.stroke, Some((Rgba::rgb(9, 9, 9), 96.0 / 72.0)));
    // The effect it can't draw is noted, and the stroke still comes through.
    assert_eq!(notes(&report), vec!["Road: line effects of type CIMGeometricEffectWave, left out".to_string()]);
    // A negative offset keeps its side.
    let left = json!({"type": "CIMLineSymbol", "symbolLayers": [{"type": "CIMSolidStroke", "width": 1, "color": rgb(0, 0, 0),
        "effects": [{"type": "CIMGeometricEffectOffset", "offset": -3.0}]}]});
    assert!((symbol::line(&left, "L", &mut report).strokes[0].offset + 4.0).abs() < 1e-5);
    // A dash pattern of more steps than a stroke holds is drawn solid, and noted.
    let long = json!({"type": "CIMLineSymbol", "symbolLayers": [{"type": "CIMSolidStroke", "width": 1, "color": rgb(0, 0, 0),
        "effects": [{"type": "CIMGeometricEffectDashes", "dashTemplate": [1, 1, 1, 1, 1, 1, 1, 1, 1, 1]}]}]});
    let mut r2 = Report::default();
    assert!(symbol::line(&long, "L", &mut r2).strokes[0].dash.is_none());
    assert_eq!(r2.notes.len(), 1);
}

#[test]
fn hatch_and_gradient_fills_become_fill_patterns() {
    use crate::fills::{FillPattern, GradientShape};
    let hatch = json!({"type": "CIMHatchFill", "enable": true, "rotation": 45.0, "separation": 6.0,
        "lineSymbol": {"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(rgb(0, 0, 255), 0.75)]}});
    let ramp = json!({"type": "CIMMultipartColorRamp", "colorRamps": [
        {"type": "CIMLinearContinuousColorRamp", "fromColor": rgb(255, 0, 0), "toColor": rgb(0, 255, 0)},
        {"type": "CIMLinearContinuousColorRamp", "fromColor": rgb(0, 255, 0), "toColor": rgb(0, 0, 255)}]});
    let gradient = json!({"type": "CIMGradientFill", "enable": true, "gradientType": "Linear", "angle": 90.0, "colorRamp": ramp});
    let picture = json!({"type": "CIMPictureFill", "enable": true, "url": "data:image/png;base64,AAAA"});
    let mut report = Report::default();
    let p = symbol::polygon(&polygon_symbol(vec![hatch, gradient, picture]), "Area", &mut report);
    assert_eq!(p.fills.len(), 2);
    assert_eq!(p.fills[0].color, Some(Rgba::rgb(0, 0, 255)));
    match p.fills[0].pattern {
        FillPattern::Hatch { angle, spacing, width, .. } => {
            assert_eq!(angle, 45.0);
            assert!((spacing - 8.0).abs() < 1e-5 && (width - 1.0).abs() < 1e-5, "{spacing} {width}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(p.fills[1].color, Some(Rgba::rgb(255, 0, 0)));
    match p.fills[1].pattern {
        FillPattern::Gradient { shape, color2, stops, angle, .. } => {
            assert_eq!((shape, color2, angle), (GradientShape::Linear, Rgba::rgb(0, 0, 255), 90.0));
            assert_eq!(stops.as_slice().len(), 1);
            assert_eq!((stops.as_slice()[0].at, stops.as_slice()[0].color), (0.5, Rgba::rgb(0, 255, 0)));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(notes(&report), vec!["Area: picture fills, left out".to_string()]);
}

#[test]
fn text_symbols_read_font_style_size_colour_alignment_and_case() {
    use crate::layout::{TextAlign, TextCase, VAlign};
    let sym = json!({"type": "CIMTextSymbol", "fontFamilyName": "Roboto Condensed", "fontStyleName": "Bold",
        "height": 14.0, "horizontalAlignment": "Center", "verticalAlignment": "Center", "textCase": "Allcaps",
        "letterSpacing": 10, "lineSpacing": 1.2, "haloSize": 1,
        "symbol": polygon_symbol(vec![solid_fill(rgb(10, 20, 30))])});
    let mut report = Report::default();
    let t = symbol::text(&sym, "Title", &mut report);
    assert_eq!(t.font.as_deref(), Some("Roboto Condensed"));
    assert!(t.bold && !t.italic);
    assert_eq!(t.size, 14.0);
    assert_eq!(t.color, Rgba::rgb(10, 20, 30));
    assert_eq!((t.align, t.valign, t.case), (TextAlign::Center, VAlign::Middle, TextCase::Upper));
    assert!((t.letter_spacing - 1.4).abs() < 1e-9 && (t.line_spacing - 1.2).abs() < 1e-9);
    assert!(report.notes.is_empty(), "no haloSymbol, no halo: {:?}", notes(&report));
    // A symbol reference holds it too; italics, lower case, a baseline, a halo, justification and small caps.
    let other = json!({"type": "CIMSymbolReference", "symbol": {"type": "CIMTextSymbol", "fontFamilyName": "Tahoma",
        "fontStyleName": "Bold Italic", "height": 9, "horizontalAlignment": "Justify", "verticalAlignment": "Baseline",
        "textCase": "Lowercase", "haloSize": 2,
        "haloSymbol": polygon_symbol(vec![solid_fill(rgb(255, 255, 255))]),
        "symbol": polygon_symbol(vec![solid_fill(rgb(0, 0, 0))])}});
    let t = symbol::text(&other, "Note", &mut report);
    assert!(t.bold && t.italic);
    assert_eq!((t.align, t.valign, t.case), (TextAlign::Left, VAlign::Bottom, TextCase::Lower));
    let n = notes(&report);
    assert!(n.iter().any(|s| s == "Note: a text halo, left out"), "{n:?}");
    assert!(n.iter().any(|s| s.starts_with("Note: justified text")), "{n:?}");
    // Small caps with Normal case is upper case, noted.
    let small = json!({"type": "CIMTextSymbol", "fontStyleName": "Regular", "height": 8, "textCase": "Normal", "smallCaps": true});
    let t = symbol::text(&small, "S", &mut report);
    assert_eq!((t.case, t.font, t.bold, t.color), (TextCase::Upper, None, false, Rgba::rgb(0, 0, 0)));
    assert!(notes(&report).iter().any(|s| s.starts_with("S: small caps")));
}

#[test]
fn point_symbols_stack_picture_and_character_markers_bottom_first() {
    use crate::symbology::Marker;
    // The smallest valid PNG is enough to be kept as a picture.
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let sym = json!({"type": "CIMPointSymbol", "symbolLayers": [
        {"type": "CIMPictureMarker", "enable": true, "size": 36.0, "url": png, "anchorPoint": {"x": 0, "y": 0}},
        {"type": "CIMCharacterMarker", "enable": true, "size": 24.0, "characterIndex": 178, "fontFamilyName": "ESRI North",
         "fontStyleName": "Regular", "symbol": polygon_symbol(vec![solid_fill(rgb(0, 0, 0))])},
        {"type": "CIMPictureMarker", "enable": false, "size": 5.0, "url": png},
    ]});
    let mut report = Report::default();
    let p = symbol::point(&sym, "North arrow", &mut report);
    assert_eq!(p.symbols.len(), 2);
    assert!(p.pending_pictures.is_empty());
    assert!(matches!(p.symbols[0].marker, Marker::Image(_)));
    assert!((p.symbols[0].size - 48.0).abs() < 1e-5);
    match p.symbols[1].marker {
        Marker::Char(g) => assert_eq!((g.ch, g.font.as_str()), ('\u{b2}', "ESRI North")),
        other => panic!("{other:?}"),
    }
    assert_eq!(p.symbols[1].fill, Rgba::rgb(0, 0, 0));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn gif_and_bmp_pictures_are_handed_back_to_be_converted() {
    use crate::symbology::Marker;
    let gif = "data:image/gif;base64,R0lGODlhAQABAAAAACw=";
    let bmp = "data:image/bmp;base64,Qk0=";
    let sym = json!({"type": "CIMPointSymbol", "symbolLayers": [
        {"type": "CIMPictureMarker", "size": 36.0, "url": gif},
        {"type": "CIMPictureMarker", "size": 10.0, "url": bmp},
        {"type": "CIMPictureMarker", "size": 10.0, "url": "https://example.com/x.png"},
        {"type": "CIMPictureMarker", "size": 10.0, "url": "data:image/png;base64,not a png"},
    ]});
    let mut report = Report::default();
    let p = symbol::point(&sym, "Arrow", &mut report);
    // The link is left out; the unreadable PNG is a circle.
    assert_eq!(p.symbols.len(), 3);
    assert_eq!(p.pending_pictures, vec![(0, gif.to_string()), (1, bmp.to_string())]);
    assert!(p.symbols.iter().all(|s| s.marker == Marker::Circle));
    assert_eq!(report.notes.len(), 2, "{:?}", notes(&report));
}

#[test]
fn vector_markers_are_recognised_by_shape() {
    use crate::symbology::Marker;
    let ring = |pts: &[[f64; 2]]| json!({"rings": [pts]});
    let graphic = |geometry: serde_json::Value| {
        json!({"type": "CIMVectorMarker", "enable": true, "size": 6.0, "frame": {"xmin": -5, "ymin": -5, "xmax": 5, "ymax": 5},
            "markerGraphics": [{"type": "CIMMarkerGraphic", "geometry": geometry,
                "symbol": polygon_symbol(vec![solid_stroke(rgb(0, 0, 0), 0.5), solid_fill(rgb(200, 0, 0))])}]})
    };
    let circle = graphic(json!({"curveRings": [[[5.0, 0.0], {"a": [[5.0, 0.0], [0.0, 0.0], 0, 1]}]]}));
    let square = graphic(ring(&[[-5.0, -5.0], [-5.0, 5.0], [5.0, 5.0], [5.0, -5.0], [-5.0, -5.0]]));
    let diamond = graphic(ring(&[[0.0, -5.0], [-5.0, 0.0], [0.0, 5.0], [5.0, 0.0], [0.0, -5.0]]));
    let triangle = graphic(ring(&[[-5.0, -5.0], [0.0, 5.0], [5.0, -5.0], [-5.0, -5.0]]));
    let star_pts: Vec<[f64; 2]> = (0..10)
        .map(|i| {
            let (r, a) = (if i % 2 == 0 { 5.0 } else { 2.0 }, std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0);
            [r * a.cos(), r * a.sin()]
        })
        .chain([[0.0, 5.0]])
        .collect();
    let star = graphic(ring(&star_pts));
    let seven = graphic(ring(&[[0.0, 0.0], [1.0, 0.0], [2.0, 1.0], [2.0, 2.0], [1.0, 3.0], [0.0, 2.0], [0.0, 0.0]]));
    let tilted = graphic(ring(&[[-5.0, 0.0], [0.0, 5.0], [5.0, 1.0], [0.0, -5.0], [-5.0, 0.0]]));
    assert_eq!(symbol::vector_shape(&circle), Some(Marker::Circle));
    assert_eq!(symbol::vector_shape(&square), Some(Marker::Square));
    assert_eq!(symbol::vector_shape(&diamond), Some(Marker::Diamond));
    assert_eq!(symbol::vector_shape(&triangle), Some(Marker::Triangle));
    assert_eq!(symbol::vector_shape(&star), Some(Marker::Star));
    assert_eq!(symbol::vector_shape(&seven), None);
    assert_eq!(symbol::vector_shape(&tilted), None);
    assert_eq!(symbol::vector_shape(&json!({"type": "CIMVectorMarker"})), None);

    // point() takes the graphic's fill and stroke, and the size in 96ths; a shape it can't tell is a noted circle.
    let mut report = Report::default();
    let p = symbol::point(&json!({"type": "CIMPointSymbol", "symbolLayers": [square, seven]}), "Pin", &mut report);
    assert_eq!(p.symbols.len(), 2);
    let s = p.symbols[0];
    assert_eq!((s.marker, s.fill, s.stroke), (Marker::Square, Rgba::rgb(200, 0, 0), Rgba::rgb(0, 0, 0)));
    assert!((s.size - 8.0).abs() < 1e-5 && (s.stroke_width - 0.5 * 96.0 / 72.0).abs() < 1e-5);
    assert_eq!(p.symbols[1].marker, Marker::Circle);
    assert_eq!(notes(&report), vec!["Pin: a drawn marker shape, shown as a circle".to_string()]);
}

#[test]
fn symbol_layers_are_bounded_and_hostile_symbols_give_nothing() {
    let many: Vec<_> = (0..200).map(|i| solid_stroke(rgb(i as u8, 0, 0), 1.0)).collect();
    let mut report = Report::default();
    let l = symbol::line(&json!({"type": "CIMLineSymbol", "symbolLayers": many}), "L", &mut report);
    assert_eq!(l.strokes.len(), crate::symbology::MAX_STROKES);
    assert!(!report.notes.is_empty());
    for bad in [json!(null), json!({}), json!({"symbolLayers": 3}), json!({"symbolLayers": [1, "x", null, {"type": 5}]})] {
        let mut r = Report::default();
        let p = symbol::polygon(&bad, "x", &mut r);
        assert!(p.fill.is_none() && p.stroke.is_none() && p.strokes.is_empty());
        assert!(symbol::point(&bad, "x", &mut r).symbols.is_empty());
        let _ = symbol::text(&bad, "x", &mut r);
    }
}

// ---- layouts ----

/// A 17 × 11 inch layout page with a vertical guide at 16.5 in and a horizontal one at 10.25 in,
/// holding `elements` (JSON objects, comma separated) and one embedded picture.
fn layout_doc(elements: &str) -> Document {
    Document::from_json(&format!(
        r#"{{"type":"CIMLayoutDocument","version":"3.7.0",
          "binaryReferences":[{{"type":"CIMBinaryReference","uRI":"CIMPATH=Pictures/logo.png","data":"iVBORw0KGgo="}}],
          "layoutDefinition":{{"type":"CIMLayout","name":"11x17 LANDSCAPE",
            "page":{{"type":"CIMPage","width":17,"height":11,"units":{{"uwkid":109008}},
              "guides":[{{"type":"CIMGuide","position":16.5,"orientation":"Vertical"}},
                        {{"type":"CIMGuide","position":10.25,"orientation":"Horizontal"}}]}},
            "elements":[{elements}]}}}}"#
    ))
    .unwrap()
}

/// A closed four-corner ring from (x0, y0) to (x1, y1), as Pro writes a rectangle.
fn corner_ring(x0: f64, y0: f64, x1: f64, y1: f64) -> serde_json::Value {
    json!({"rings": [[[x0, y0], [x0, y1], [x1, y1], [x1, y0], [x0, y0]]]})
}

/// A rectangle graphic element: a 1 pt black outline under a white fill.
fn rectangle_element(name: &str, polygon: serde_json::Value) -> serde_json::Value {
    json!({"type": "CIMGraphicElement", "name": name, "anchor": "BottomLeftCorner", "visible": true,
        "graphic": {"type": "CIMPolygonGraphic", "polygon": polygon,
            "symbol": {"type": "CIMSymbolReference", "symbol": polygon_symbol(vec![solid_stroke(rgb(0, 0, 0), 1.0), solid_fill(rgb(255, 255, 255))])}}})
}

fn read(elements: &[serde_json::Value]) -> (LayoutImport, Report) {
    let text: Vec<String> = elements.iter().map(|e| e.to_string()).collect();
    let mut report = Report::default();
    let imported = read_layout(&layout_doc(&text.join(",")), &mut report).unwrap();
    (imported, report)
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn the_page_and_guides_come_in_millimetres_from_the_top_left() {
    let (imp, report) = read(&[]);
    let l = &imp.layout;
    assert_eq!(l.name, "11x17 LANDSCAPE");
    assert!(near(l.width, 431.8, 1e-9) && near(l.height, 279.4, 1e-9), "{} × {}", l.width, l.height);
    assert_eq!(l.guides.x.len(), 1);
    assert!(near(l.guides.x[0], 419.1, 1e-9), "{:?}", l.guides.x);
    assert_eq!(l.guides.y.len(), 1);
    assert!(near(l.guides.y[0], (11.0 - 10.25) * 25.4, 1e-9), "{:?}", l.guides.y);
    assert!(l.elements.is_empty() && imp.frames.is_empty() && imp.pictures.is_empty());
    assert!(imp.embedded_maps.is_empty() && imp.text_frames.is_empty());
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    // A unit it doesn't know is read as inches, and said.
    let mut r = Report::default();
    let odd = r#"{"type":"CIMLayoutDocument","layoutDefinition":{"type":"CIMLayout","name":"P",
        "page":{"type":"CIMPage","width":10,"height":5,"units":{"uwkid":4242}},"elements":[]}}"#;
    let imp = read_layout(&Document::from_json(odd).unwrap(), &mut r).unwrap();
    assert!(near(imp.layout.width, 254.0, 1e-9) && near(imp.layout.height, 127.0, 1e-9));
    assert!(notes(&r).iter().any(|n| n.contains("inches")), "{:?}", notes(&r));
    // Not a layout at all: an error, not a panic.
    assert!(read_layout(&Document::from_json(r#"{"type":"CIMMapDocument"}"#).unwrap(), &mut r).is_err());
}

#[test]
fn a_rectangle_rotated_180_about_its_top_left_lands_where_pro_draws_it() {
    // The template's LOGO RIBBON: stored below where it shows, turned about its top-left corner.
    let mut ribbon = rectangle_element("LOGO RIBBON", corner_ring(2.314, 6.263, 4.347, 8.569));
    ribbon["anchor"] = json!("TopLeftCorner");
    ribbon["rotation"] = json!(180);
    ribbon["locked"] = json!(true);
    let (imp, report) = read(&[ribbon]);
    let e = &imp.layout.elements[0];
    let inch = |mm: f64| mm / 25.4;
    assert!(near(inch(e.x), 0.281, 0.01) && near(inch(e.y), 0.125, 0.01), "{} {}", inch(e.x), inch(e.y));
    assert!(near(inch(e.w), 2.033, 0.01) && near(inch(e.h), 2.306, 0.01), "{} {}", inch(e.w), inch(e.h));
    assert_eq!(e.name, "LOGO RIBBON");
    assert!(e.locked && e.visible);
    match e.kind {
        crate::layout::ElementKind::Rectangle { fill, outline, width, radius } => {
            assert_eq!((fill, outline, radius), (Rgba::rgb(255, 255, 255), Rgba::rgb(0, 0, 0), 0.0));
            assert!(near(width, 96.0 / 72.0, 1e-5), "{width}");
        }
        ref other => panic!("{other:?}"),
    }
    // A half turn leaves it upright: nothing to say.
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn a_rectangle_rotated_30_degrees_keeps_its_bounding_box_and_notes_it() {
    let mut r = rectangle_element("Tilted", corner_ring(1.0, 1.0, 3.0, 2.0));
    r["anchor"] = json!("CenterPoint");
    r["rotation"] = json!(30);
    let (imp, report) = read(&[r]);
    let e = &imp.layout.elements[0];
    assert!([e.x, e.y, e.w, e.h].iter().all(|v| v.is_finite()));
    // A 2 × 1 in box turned 30° about its centre (2, 1.5) covers 2cos30 + sin30 by 2sin30 + cos30.
    let (c, s) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let (w, h) = (2.0 * c + s, 2.0 * s + c);
    assert!(near(e.w, w * 25.4, 1e-6) && near(e.h, h * 25.4, 1e-6), "{} {}", e.w, e.h);
    assert!(near(e.x, (2.0 - w / 2.0) * 25.4, 1e-6) && near(e.y, (11.0 - 1.5 - h / 2.0) * 25.4, 1e-6), "{} {}", e.x, e.y);
    assert!(notes(&report).iter().any(|n| n.starts_with("Tilted: ") && n.contains("rotated")), "{:?}", notes(&report));
}

#[test]
fn rotation_turns_anticlockwise_about_each_anchor() {
    let b = layout::PageBox { xmin: 0.0, ymin: 0.0, xmax: 2.0, ymax: 1.0 };
    let anchors = [
        ("BottomLeftCorner", (0.0, 0.0)),
        ("BottomMidPoint", (1.0, 0.0)),
        ("BottomRightCorner", (2.0, 0.0)),
        ("LeftMidPoint", (0.0, 0.5)),
        ("CenterPoint", (1.0, 0.5)),
        ("RightMidPoint", (2.0, 0.5)),
        ("TopLeftCorner", (0.0, 1.0)),
        ("TopMidPoint", (1.0, 1.0)),
        ("TopRightCorner", (2.0, 1.0)),
    ];
    for (name, p) in anchors {
        assert_eq!(layout::anchor_point(b, name), p, "{name}");
    }
    // A quarter turn about the bottom-left corner stands the box up to the left of it.
    let r = layout::rotated(b, (0.0, 0.0), 90.0);
    assert!(near(r.xmin, -1.0, 1e-12) && near(r.xmax, 0.0, 1e-12) && near(r.ymin, 0.0, 1e-12) && near(r.ymax, 2.0, 1e-12), "{r:?}");
    assert!(layout::is_right_angle(-270.0) && layout::is_right_angle(360.0) && !layout::is_right_angle(45.0));
    // Millimetres from the top-left of an 11 in page.
    assert_eq!(layout::to_mm(b, 11.0, 25.4), (0.0, 10.0 * 25.4, 2.0 * 25.4, 25.4));
}

#[test]
fn a_quarter_turn_about_a_corner_goes_anticlockwise() {
    // A 2 × 1 in box turned 90° anticlockwise about its bottom-left (1, 1) stands up to its left:
    // x 0..1, y 1..3 (clockwise it would hang below: x 1..2, y -1..1).
    let mut r = rectangle_element("Quarter", corner_ring(1.0, 1.0, 3.0, 2.0));
    r["rotation"] = json!(90);
    let (imp, report) = read(&[r]);
    let e = &imp.layout.elements[0];
    assert!(near(e.x, 0.0, 1e-9) && near(e.w, 25.4, 1e-9), "{e:?}");
    assert!(near(e.y, 8.0 * 25.4, 1e-9) && near(e.h, 2.0 * 25.4, 1e-9), "{e:?}");
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn rounded_corners_become_a_radius() {
    let (w, h) = (20.0 / 25.4, 10.0 / 25.4);
    let mut stroke = solid_stroke(rgb(0, 0, 0), 1.0);
    stroke["effects"] = json!([{"type": "CIMGeometricEffectRounding", "rounding": 100}]);
    let mut effect = rectangle_element("Effect", corner_ring(1.0, 1.0, 1.0 + w, 1.0 + h));
    effect["graphic"]["symbol"]["symbol"]["symbolLayers"] = json!([stroke, solid_fill(rgb(255, 255, 255))]);
    let mut attr = rectangle_element("Attr", corner_ring(1.0, 1.0, 1.0 + w, 1.0 + h));
    attr["graphic"]["attributes"] = json!({"type": "PropertySet", "propertySetItems": ["Rounding", 50]});
    let mut square = rectangle_element("Square", corner_ring(1.0, 1.0, 2.0, 2.0));
    square["graphic"]["attributes"] = json!({"type": "PropertySet", "propertySetItems": ["Rounding", 0]});
    let (imp, report) = read(&[effect, attr, square]);
    let radius = |i: usize| match imp.layout.elements[i].kind {
        crate::layout::ElementKind::Rectangle { radius, .. } => radius,
        ref other => panic!("{other:?}"),
    };
    assert!(near(radius(0), 5.0, 1e-9) && near(radius(1), 2.5, 1e-9) && radius(2) == 0.0, "{} {}", radius(0), radius(1));
    let n = notes(&report);
    assert!(n.iter().any(|s| s == "Effect: rounded corners, approximated"), "{n:?}");
    assert!(!n.iter().any(|s| s.contains("line effects") || s.starts_with("Square: ")), "{n:?}");
    assert_eq!(layout::corner_radius(100.0, 400.0, 300.0), 50.0);
    assert_eq!(layout::corner_radius(f64::NAN, 20.0, 10.0), 0.0);
}

#[test]
fn point_text_gets_a_box_from_its_anchor() {
    // TITLE, 14 pt, at (2.858, 10.566) in, by its left middle.
    let b = layout::text_box((2.858, 10.566), "LeftMidPoint", 14.0, &["TITLE"], 25.4);
    assert!(near(b.xmin, 2.858, 1e-9), "{b:?}");
    assert!(near((b.ymin + b.ymax) / 2.0, 10.566, 1e-9), "{b:?}");
    // 0.55 of the size per character across, 1.25 of it per line down, in points.
    assert!(near(b.xmax - b.xmin, 0.55 * 14.0 * 5.0 / 72.0, 1e-9), "{b:?}");
    assert!(near(b.ymax - b.ymin, 14.0 * 1.25 / 72.0, 1e-9), "{b:?}");
    // Two lines are twice as tall, as wide as the longer; the anchor can be any corner or middle.
    let b = layout::text_box((10.0, 5.0), "TopRightCorner", 10.0, &["AB", "ABCD"], 25.4);
    assert!(near(b.xmax, 10.0, 1e-9) && near(b.ymax, 5.0, 1e-9), "{b:?}");
    assert!(near(b.xmax - b.xmin, 0.55 * 10.0 * 4.0 / 72.0, 1e-9) && near(b.ymax - b.ymin, 2.0 * 12.5 / 72.0, 1e-9));
    let b = layout::text_box((1.0, 1.0), "CenterPoint", 10.0, &["AB"], 1.0);
    assert!(near((b.xmin + b.xmax) / 2.0, 1.0, 1e-9) && near((b.ymin + b.ymax) / 2.0, 1.0, 1e-9));
    assert!(near(b.xmax - b.xmin, 11.0 * 25.4 / 72.0, 1e-9), "millimetre pages: {b:?}");
}

#[test]
fn pictures_resolve_embedded_binaries_or_remember_their_path() {
    let picture = |name: &str, graphic: serde_json::Value| {
        let mut g = graphic;
        g["type"] = json!("CIMPictureGraphic");
        g["box"] = json!({"xmin": 0.76, "ymin": 9.0, "xmax": 1.83, "ymax": 9.41});
        json!({"type": "CIMGraphicElement", "name": name, "anchor": "CenterPoint", "graphic": g})
    };
    let (imp, report) = read(&[
        picture("HLR LOGO", json!({"referenceURI": "CIMPATH=Pictures/logo.png", "sourceURL": r"G:\LOGOS\Holland LR Logo.png"})),
        picture("FILE", json!({"sourceURL": r"G:\LOGOS\Other.png"})),
        picture("INLINE", json!({"sourceURL": "data:image/png;base64,AAAA"})),
    ]);
    // A data: URL that isn't base64 is neither a picture nor a file.
    let (bad, bad_report) = read(&[picture("BAD", json!({"sourceURL": "DATA:image/png,abc"}))]);
    assert!(matches!(&bad.layout.elements[0].kind, Picture { image, source } if image.is_empty() && source.is_none()));
    assert!(bad.pictures.is_empty());
    assert!(notes(&bad_report).iter().any(|n| n.starts_with("BAD: ") && n.contains("data:")), "{:?}", notes(&bad_report));
    use crate::layout::ElementKind::Picture;
    let els = &imp.layout.elements;
    assert_eq!(els.len(), 3);
    assert!(
        matches!(&els[0].kind, Picture { image, source } if image == "iVBORw0KGgo=" && source.as_deref() == Some(r"G:\LOGOS\Holland LR Logo.png"))
    );
    assert!(matches!(&els[1].kind, Picture { image, source } if image.is_empty() && source.as_deref() == Some(r"G:\LOGOS\Other.png")));
    assert!(matches!(&els[2].kind, Picture { image, source } if image == "AAAA" && source.is_none()));
    assert_eq!(imp.pictures, vec![(els[1].id, PictureSource::Path(r"G:\LOGOS\Other.png".into()))]);
    let e = &els[0];
    assert!(near(e.x, 0.76 * 25.4, 1e-9) && near(e.y, (11.0 - 9.41) * 25.4, 1e-9) && near(e.w, 1.07 * 25.4, 1e-9));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn graphic_frames_become_borders_and_backgrounds() {
    let line_sym = |c: serde_json::Value| json!({"type": "CIMSymbolReference", "symbol": {"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(c, 1.0)]}});
    let mut framed = rectangle_element("Framed", corner_ring(1.0, 1.0, 2.0, 2.0));
    framed["graphicFrame"] = json!({"type": "CIMGraphicFrame", "borderSymbol": line_sym(rgb(0, 0, 0)),
        "backgroundSymbol": {"type": "CIMSymbolReference", "symbol": polygon_symbol(vec![solid_fill(rgb(255, 255, 255))])},
        "borderGapX": 5, "borderGapY": 5,
        "shadowSymbol": {"type": "CIMSymbolReference", "symbol": polygon_symbol(vec![solid_fill(rgb(128, 128, 128))])}});
    let mut clear = rectangle_element("Clear", corner_ring(1.0, 1.0, 2.0, 2.0));
    clear["graphicFrame"] = json!({"type": "CIMGraphicFrame", "borderSymbol": line_sym(json!({"type": "CIMRGBColor", "values": [0, 0, 0, 0]})),
        "backgroundSymbol": {"type": "CIMSymbolReference", "symbol": {"type": "CIMPolygonSymbol", "angleAlignment": "Map"}},
        "shadowSymbol": {"type": "CIMSymbolReference", "symbol": {"type": "CIMPolygonSymbol"}}});
    let (imp, report) = read(&[framed, clear]);
    let fs = imp.layout.elements[0].frame_style.clone().unwrap();
    assert_eq!((fs.border, fs.background), (Some(Rgba::rgb(0, 0, 0)), Some(Rgba::rgb(255, 255, 255))));
    assert!(near(fs.border_width, 96.0 / 72.0, 1e-5), "{}", fs.border_width);
    assert!(near(fs.gap, 5.0 * 25.4 / 72.0, 1e-9), "{}", fs.gap);
    assert_eq!(imp.layout.elements[1].frame_style, None);
    // A picture keeps its frame in its graphic, and a polygon may give its geometry as `shape`.
    let pic = json!({"type": "CIMGraphicElement", "name": "Pic", "graphic": {"type": "CIMPictureGraphic", "sourceURL": "a.png",
        "box": {"xmin": 1, "ymin": 1, "xmax": 2, "ymax": 2}, "frame": {"type": "CIMGraphicFrame", "borderSymbol": line_sym(rgb(255, 0, 0))}}});
    let mut shaped = rectangle_element("Shaped", json!(null));
    shaped["graphic"]["shape"] = corner_ring(1.0, 1.0, 2.0, 2.0);
    let (imp2, r2) = read(&[pic, shaped]);
    assert_eq!(imp2.layout.elements[0].frame_style.as_ref().and_then(|f| f.border), Some(Rgba::rgb(255, 0, 0)));
    assert!(near(imp2.layout.elements[1].w, 25.4, 1e-9), "{:?}", imp2.layout.elements[1]);
    assert!(r2.notes.is_empty(), "{:?}", notes(&r2));
    let n = notes(&report);
    assert!(n.iter().any(|s| s.starts_with("Framed: ") && s.contains("shadow")), "{n:?}");
    assert!(!n.iter().any(|s| s.starts_with("Clear: ")), "{n:?}");
}

#[test]
fn unknown_elements_are_noted_and_skipped() {
    let (imp, report) = read(&[json!({"type": "CIMChartFrame", "name": "C"}), json!({"name": "T"}), json!(7)]);
    assert!(imp.layout.elements.is_empty());
    let n = notes(&report);
    assert!(n.iter().any(|s| s == "T: an element with no type, left out"), "{n:?}");
    assert!(n.iter().any(|s| s == "Element: an element with no type, left out"), "{n:?}");
    let n = report.notes.iter().find(|n| n.layer == "C").unwrap();
    assert_eq!(n.kind, NoteKind::Skipped);
    assert!(n.text.contains("CIMChartFrame"), "{}", n.text);
}

#[test]
fn too_many_or_too_deep_elements_are_bounded() {
    let many: Vec<_> = (0..600).map(|i| rectangle_element(&format!("R{i}"), corner_ring(1.0, 1.0, 2.0, 2.0))).collect();
    let (imp, report) = read(&many);
    assert_eq!(imp.layout.elements.len(), crate::layout::MAX_ELEMENTS);
    assert_eq!(imp.layout.elements.last().map(|e| e.name.as_str()), Some("R499"));
    assert!(notes(&report).iter().any(|n| n.contains("500")), "{:?}", notes(&report));

    // Twelve groups, one inside the other, round one rectangle: the inner four fold into the eighth.
    let mut nested = rectangle_element("Inner", corner_ring(1.0, 1.0, 2.0, 2.0));
    for level in (1..=12).rev() {
        nested = json!({"type": "CIMGroupElement", "name": format!("G{level}"), "elements": [nested]});
    }
    let (imp, report) = read(&[nested]);
    let l = &imp.layout;
    let names: Vec<&str> = l.elements.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Inner", "G8", "G7", "G6", "G5", "G4", "G3", "G2", "G1"]);
    let id = |n: &str| l.elements.iter().find(|e| e.name == n).map(|e| e.id).unwrap();
    assert_eq!(l.element(id("G8")).unwrap().children(), [id("Inner")]);
    assert_eq!(l.element(id("G1")).unwrap().children(), [id("G2")]);
    assert_eq!(l.group_depth(id("G1")), crate::layout::MAX_GROUP_DEPTH);
    // The groups' boxes are their children's.
    let g = l.element(id("G1")).unwrap();
    assert!(near(g.x, 25.4, 1e-9) && near(g.w, 25.4, 1e-9) && near(g.y, 9.0 * 25.4, 1e-9), "{g:?}");
    assert!(notes(&report).iter().any(|n| n.contains("flattened")), "{:?}", notes(&report));

    // Forty groups deep (about as deep as the JSON parser goes) reads without a panic, and what
    // is past the walk's own limit is left out and said.
    let mut deep = rectangle_element("Deep", corner_ring(1.0, 1.0, 2.0, 2.0));
    for _ in 0..40 {
        deep = json!({"type": "CIMGroupElement", "name": "G", "elements": [deep]});
    }
    let mut r = Report::default();
    let imp = read_layout(&layout_doc(&deep.to_string()), &mut r).unwrap();
    assert!(imp.layout.elements.is_empty());
    assert!(notes(&r).iter().any(|n| n.contains("nested past 32")), "{:?}", notes(&r));
}

#[test]
fn lines_take_their_direction_and_heads_and_curves_are_ellipses() {
    let arrow = json!({"type": "CIMVectorMarker", "enable": true, "size": 6,
        "markerPlacement": {"type": "CIMMarkerPlacementAtExtremities", "extremityPlacement": "JustEnd"}});
    let line = |name: &str, path: serde_json::Value, layers: Vec<serde_json::Value>| {
        json!({"type": "CIMGraphicElement", "name": name, "graphic": {"type": "CIMLineGraphic", "line": {"paths": [path]},
            "symbol": {"type": "CIMSymbolReference", "symbol": {"type": "CIMLineSymbol", "symbolLayers": layers}}}})
    };
    use crate::layout::{ElementKind, LineDir, LineHead};
    let (imp, report) = read(&[
        // Right to left with a head at its end: the head is at the left, at the start of a horizontal line.
        line("Level", json!([[3.0, 5.0], [1.0, 5.0]]), vec![solid_stroke(rgb(255, 0, 0), 2.0), arrow.clone()]),
        // Up and to the right on the page.
        line("Up", json!([[1.0, 1.0], [2.0, 3.0]]), vec![solid_stroke(rgb(0, 0, 0), 1.0)]),
        json!({"type": "CIMGraphicElement", "name": "Oval", "graphic": {"type": "CIMPolygonGraphic",
            "polygon": {"curveRings": [[[3.0, 2.0], {"a": [[3.0, 2.0], [2.0, 2.0], 0, 1]}]]},
            "symbol": {"type": "CIMSymbolReference", "symbol": polygon_symbol(vec![solid_fill(rgb(0, 0, 255))])}}}),
        rectangle_element("Blob", json!({"rings": [[[0.0, 0.0], [1.0, 2.0], [2.0, 0.0], [1.0, -1.0], [0.0, 0.0]]]})),
        // Three straight sides and one arced: curved, but not an ellipse.
        rectangle_element(
            "Half",
            json!({"curveRings": [[[0.0, 0.0], [2.0, 0.0], {"a": [[2.0, 2.0], [2.0, 1.0], 0, 0]}, [0.0, 2.0], [0.0, 0.0]]]}),
        ),
        line("Down", json!([[1.0, 1.0], [1.0, 3.0]]), vec![solid_stroke(rgb(0, 0, 0), 1.0)]),
        line("Fall", json!([[1.0, 3.0], [2.0, 1.0]]), vec![solid_stroke(rgb(0, 0, 0), 1.0)]),
        line(
            "Begin",
            json!([[1.0, 1.0], [2.0, 1.0]]),
            vec![
                solid_stroke(rgb(0, 0, 0), 1.0),
                json!({"type": "CIMVectorMarker", "markerPlacement": {"type": "CIMMarkerPlacementOnLine", "relativeTo": "LineBeginning"}}),
                json!({"type": "CIMVectorMarker", "markerPlacement": {"type": "CIMMarkerPlacementOnLine", "relativeTo": "LineMiddle"}}),
            ],
        ),
    ]);
    let els = &imp.layout.elements;
    match els[0].kind {
        ElementKind::Line { color, width, dir, head } => {
            assert_eq!((color, dir, head), (Rgba::rgb(255, 0, 0), LineDir::Horizontal, LineHead::Start));
            assert!(near(width, 2.0 * 96.0 / 72.0, 1e-5));
        }
        ref other => panic!("{other:?}"),
    }
    assert!(near(els[0].x, 25.4, 1e-6) && near(els[0].w, 2.0 * 25.4, 1e-6), "{:?}", els[0]);
    assert!(near(els[0].y + els[0].h / 2.0, 6.0 * 25.4, 1e-6), "{:?}", els[0]);
    assert!(matches!(els[1].kind, ElementKind::Line { dir: LineDir::Rising, head: LineHead::None, .. }), "{:?}", els[1].kind);
    assert!(near(els[1].x, 25.4, 1e-6) && near(els[1].y, 8.0 * 25.4, 1e-6) && near(els[1].h, 2.0 * 25.4, 1e-6), "{:?}", els[1]);
    assert!(matches!(els[2].kind, ElementKind::Ellipse { fill, .. } if fill == Rgba::rgb(0, 0, 255)), "{:?}", els[2].kind);
    assert!(near(els[2].x, 25.4, 1e-6) && near(els[2].w, 2.0 * 25.4, 1e-6) && near(els[2].h, 2.0 * 25.4, 1e-6), "{:?}", els[2]);
    assert!(matches!(els[3].kind, ElementKind::Rectangle { .. }));
    assert!(near(els[3].h, 3.0 * 25.4, 1e-6));
    assert!(matches!(els[4].kind, ElementKind::Rectangle { .. }), "{:?}", els[4].kind);
    assert!(matches!(els[5].kind, ElementKind::Line { dir: LineDir::Vertical, .. }), "{:?}", els[5].kind);
    assert!(matches!(els[6].kind, ElementKind::Line { dir: LineDir::Falling, .. }), "{:?}", els[6].kind);
    assert!(matches!(els[7].kind, ElementKind::Line { dir: LineDir::Horizontal, head: LineHead::Start, .. }), "{:?}", els[7].kind);
    let n = notes(&report);
    assert!(n.iter().any(|s| s.starts_with("Blob: ")), "{n:?}");
    assert!(n.iter().any(|s| s == "Half: a polygon other than a rectangle, drawn as the rectangle round it"), "{n:?}");
    assert!(n.iter().any(|s| s == "Begin: markers along a line, left out"), "{n:?}");
    assert!(!n.iter().any(|s| s.starts_with("Level: ") || s.starts_with("Up: ") || s.starts_with("Oval: ")), "{n:?}");
}

// ---- text ----

use crate::layout::ElementKind;

/// A text graphic element (`kind` is `CIMTextGraphic` or `CIMParagraphTextGraphic`) with `text` at `shape`.
fn text_element(kind: &str, name: &str, shape: serde_json::Value, text: &str, symbol: serde_json::Value) -> serde_json::Value {
    json!({"type": "CIMGraphicElement", "name": name, "anchor": "TopLeftCorner", "visible": true,
        "graphic": {"type": kind, "shape": shape, "text": text, "margin": 5,
            "symbol": {"type": "CIMSymbolReference", "symbol": symbol}}})
}

fn text_symbol(extra: serde_json::Value) -> serde_json::Value {
    let mut s = json!({"type": "CIMTextSymbol", "fontFamilyName": "Arial", "fontStyleName": "Bold", "height": 24,
        "symbol": polygon_symbol(vec![solid_fill(rgb(10, 20, 30))])});
    if let (Some(o), Some(e)) = (s.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    s
}

#[test]
fn dyn_tags_become_tokens_with_prefix_suffix_and_newline() {
    let (t, frame) = dyntext::translate(
        r#"Spatial Reference<dyn type="mapFrame" name="MAIN MAP" property="sr" srProperty="name" preStr="Name: " newLine="true" emptyStr=""/><dyn type="mapFrame" name="MAIN MAP" property="scale" preStr="1:"/>"#,
        &mut Report::default(),
        "T",
    );
    // newLine starts a line before the text: a heading, then "Name: …" under it.
    assert_eq!(t, "Spatial Reference{crsName|\\nName: }{scale}");
    assert_eq!(frame.as_deref(), Some("MAIN MAP"));
}

#[test]
fn layout_and_project_and_date_tags_translate() {
    let mut report = Report::default();
    let (t, frame) = dyntext::translate(r#"Map Name: <dyn type="project" property="name"/>"#, &mut report, "T");
    assert_eq!((t.as_str(), frame), ("Map Name: {project}", None));
    let (t, _) = dyntext::translate(r#"<dyn type="layout" name="8x11 LANDSCAPE" property="serviceLayerCredits"/>"#, &mut report, "T");
    assert_eq!(t, "{credits}");
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    let (t, _) = dyntext::translate(r#"<dyn type="date" format="short"/>"#, &mut report, "T");
    assert_eq!(t, "{date}");
    assert_eq!(notes(&report), ["T: date format short, drawn as RustGIS's"]);
}

#[test]
fn unknown_tags_and_formatting_are_stripped_with_notes() {
    let mut report = Report::default();
    let (t, _) = dyntext::translate(r#"<BOL>Title</BOL> <dyn type="user" property="name"/>"#, &mut report, "T");
    assert_eq!(t, "Title ");
    assert_eq!(notes(&report), ["T: formatting tags, left out", "T: dynamic text user/name, left out"]);
}

#[test]
fn a_frame_name_that_isnt_on_the_page_is_kept_and_the_text_still_imports() {
    let text = r#"<dyn type="mapFrame" name="GONE" property="scale" preStr="Scale 1:"/>"#;
    let (imp, _) = read(&[text_element("CIMTextGraphic", "Scale", json!({"x": 1.0, "y": 5.0}), text, text_symbol(json!({})))]);
    assert_eq!(imp.layout.elements.len(), 1);
    let el = &imp.layout.elements[0];
    assert_eq!(imp.text_frames, vec![(el.id, "GONE".to_string())]);
    assert!(matches!(&el.kind, ElementKind::Text { text, frame: None, .. } if text == "{scale|Scale }"), "{:?}", el.kind);
}

#[test]
fn paragraph_text_wraps_and_point_text_doesnt() {
    use crate::layout::{FontFamily, TextAlign, TextCase, VAlign};
    let disclaimer = text_element(
        "CIMParagraphTextGraphic",
        "DISCLAIMER",
        json!({"xmin": 1.0, "ymin": 1.0, "xmax": 5.0, "ymax": 3.0}),
        "Not for <ITA>construction</ITA> &amp; so on.",
        text_symbol(
            json!({"fontStyleName": "Regular", "height": 8, "verticalAlignment": "Center", "textCase": "Allcaps", "horizontalAlignment": "Center"}),
        ),
    );
    let title = text_element("CIMTextGraphic", "TITLE", json!({"x": 1.0, "y": 10.0}), "Title", text_symbol(json!({})));
    let (imp, report) = read(&[disclaimer, title]);
    let els = &imp.layout.elements;
    let m = 5.0 * 25.4 / 72.0;
    match &els[0].kind {
        ElementKind::Text { text, size, wrap, valign, case, align, bold, font, font_name, color, .. } => {
            assert_eq!(text, "Not for construction & so on.");
            assert_eq!(
                (*size, *wrap, *valign, *case, *align, *bold),
                (8.0, true, VAlign::Middle, TextCase::Upper, TextAlign::Center, false)
            );
            assert_eq!((*font, font_name.as_deref(), *color), (FontFamily::Sans, Some("Arial"), Rgba::rgb(10, 20, 30)));
        }
        other => panic!("{other:?}"),
    }
    let e = &els[0];
    assert!(near(e.x, 25.4 + m, 1e-9) && near(e.w, 4.0 * 25.4 - 2.0 * m, 1e-9), "{e:?}");
    assert!(near(e.y, 8.0 * 25.4 + m, 1e-9) && near(e.h, 2.0 * 25.4 - 2.0 * m, 1e-9), "{e:?}");
    match &els[1].kind {
        ElementKind::Text { text, size, wrap, bold, valign, .. } => {
            assert_eq!((text.as_str(), *size, *wrap, *bold, *valign), ("Title", 24.0, false, true, VAlign::Top));
        }
        other => panic!("{other:?}"),
    }
    // Point text: the box is the text's own, its top left at the point.
    let t = &els[1];
    assert!(near(t.x, 25.4, 1e-9) && near(t.y, 25.4, 1e-9), "{t:?}");
    assert!(near(t.w, 0.55 * 24.0 * 5.0 / 72.0 * 25.4, 1e-9) && near(t.h, 24.0 * 1.25 / 72.0 * 25.4, 1e-9), "{t:?}");
    assert_eq!(imp.text_frames, vec![]);
    let n = notes(&report);
    assert!(n.iter().all(|s| !s.starts_with("TITLE: ")) && n.iter().any(|s| s == "DISCLAIMER: formatting tags, left out"), "{n:?}");
}

#[test]
fn a_margin_never_shrinks_a_paragraph_below_a_millimetre_and_text_without_a_position_is_skipped() {
    let tiny = text_element(
        "CIMParagraphTextGraphic",
        "Tiny",
        json!({"xmin": 1.0, "ymin": 1.0, "xmax": 1.1, "ymax": 1.1}),
        "x",
        text_symbol(json!({})),
    );
    let nowhere = text_element("CIMTextGraphic", "Nowhere", json!({}), "x", text_symbol(json!({})));
    let (imp, report) = read(&[tiny, nowhere]);
    assert_eq!(imp.layout.elements.len(), 1);
    let e = &imp.layout.elements[0];
    assert!(near(e.w, 1.0, 1e-9) && near(e.h, 1.0, 1e-9), "{e:?}");
    assert!(near(e.x, 25.4 + 0.77, 0.01), "{e:?}");
    assert!(notes(&report).iter().any(|s| s == "Nowhere: text with no position, left out"), "{:?}", notes(&report));
}

use crate::layout::{FrameLink, FrameShape, LinkMode};

// ---- map frames ----

/// An embedded map definition with `uri`, in spatial reference `wkid`.
fn embedded_map(uri: &str, wkid: u32, extra: serde_json::Value) -> serde_json::Value {
    let mut m = json!({"type": "CIMMap", "name": uri, "uRI": uri, "spatialReference": {"wkid": wkid, "latestWkid": wkid}});
    if let (Some(o), Some(e)) = (m.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    m
}

/// A map frame element over the rectangle (x0, y0)-(x1, y1) inches, with `extra` keys set on it.
fn map_frame_element(name: &str, (x0, y0, x1, y1): (f64, f64, f64, f64), extra: serde_json::Value) -> serde_json::Value {
    let mut f = json!({"type": "CIMMapFrame", "name": name, "anchor": "BottomLeftCorner", "visible": true,
        "frame": corner_ring(x0, y0, x1, y1)});
    if let (Some(o), Some(e)) = (f.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    f
}

fn camera(x: f64, y: f64, scale: f64) -> serde_json::Value {
    json!({"view": {"camera": {"type": "CIMViewCamera", "x": x, "y": y, "scale": scale, "heading": 0}}})
}

/// Reads `elements` from a layout that embeds `maps`.
fn read_with_maps(maps: &[serde_json::Value], elements: &[serde_json::Value]) -> (LayoutImport, Report) {
    let doc = json!({"type": "CIMLayoutDocument", "version": "3.7.0", "mapDefinitions": maps,
        "layoutDefinition": {"type": "CIMLayout", "name": "P",
            "page": {"type": "CIMPage", "width": 17, "height": 11, "units": {"uwkid": 109008}, "guides": []},
            "elements": elements}});
    let mut report = Report::default();
    let imported = read_layout(&Document::from_json(&doc.to_string()).unwrap(), &mut report).unwrap();
    (imported, report)
}

/// The id and kind of the element called `name`.
fn frame_of(imp: &LayoutImport, name: &str) -> (u32, ElementKind) {
    let el = imp.layout.elements.iter().find(|e| e.name == name).unwrap();
    (el.id, el.kind.clone())
}

#[test]
fn an_embedded_frames_camera_converts_from_degrees_or_web_mercator() {
    let maps = [embedded_map("CIMPATH=map/deg.json", 4326, json!({})), embedded_map("CIMPATH=map/web.json", 3857, json!({}))];
    let mut a = map_frame_element("DEG", (1.0, 1.0, 9.0, 8.0), camera(-100.554, 30.333, 20_000_000.0));
    a["uRI"] = json!("CIMPATH=map/deg.json");
    let mut b = map_frame_element("WEB", (9.0, 1.0, 16.0, 8.0), camera(-9650117.76, 5619364.40, 25_000.0));
    b["uRI"] = json!("CIMPATH=map/web.json");
    let (imp, report) = read_with_maps(&maps, &[a, b]);
    let (_, ElementKind::MapFrame { center, scale, shape, map, .. }) = frame_of(&imp, "DEG") else { panic!() };
    assert!(near(center.x, -100.554, 1e-12) && near(center.y, 30.333, 1e-12) && scale == 20_000_000.0, "{center:?} {scale}");
    assert_eq!((shape, map.as_str()), (FrameShape::Rectangle, "DEG"));
    let (_, ElementKind::MapFrame { center, scale, .. }) = frame_of(&imp, "WEB") else { panic!() };
    // Web Mercator metres to degrees (the brief's "about 45.1" latitude is really 44.986).
    assert!(near(center.x, -86.688_482_77, 1e-6) && near(center.y, 44.986_296_4, 1e-6) && scale == 25_000.0, "{center:?}");
    assert_eq!(imp.frames.len(), 2);
    assert_eq!(imp.frames[0].map_uri.as_deref(), Some("CIMPATH=map/deg.json"));
    assert_eq!(imp.frames[1].saved_camera, Some((-9650117.76, 5619364.40, 25_000.0)));
    assert_eq!(imp.embedded_maps, ["CIMPATH=map/deg.json", "CIMPATH=map/web.json"]);
    // The box: 8 x 7 inches from the left, 3 inches down from the top.
    let el = imp.layout.elements.first().unwrap();
    assert!(near(el.x, 25.4, 1e-9) && near(el.y, 3.0 * 25.4, 1e-9) && near(el.w, 8.0 * 25.4, 1e-9) && near(el.h, 7.0 * 25.4, 1e-9));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn a_camera_in_another_system_falls_back_to_the_auto_extent_then_the_default_extent() {
    let ext = |x0: f64, y0: f64, x1: f64, y1: f64, wkid: u32| json!({"type": "CIMEnvelope", "xmin": x0, "ymin": y0, "xmax": x1, "ymax": y1, "spatialReference": {"wkid": wkid}});
    let maps = [
        embedded_map("CIMPATH=map/sp.json", 2272, json!({"defaultExtent": ext(-80.0, 40.0, -78.0, 42.0, 4326)})),
        embedded_map("CIMPATH=map/none.json", 2272, json!({})),
    ];
    let mut auto = map_frame_element("AUTO", (1.0, 1.0, 5.0, 5.0), camera(2_000_000.0, 300_000.0, 12_000.0));
    auto["uRI"] = json!("CIMPATH=map/sp.json");
    auto["autoCamera"] = json!({"type": "CIMAutoCamera", "source": "None", "extent": ext(-10.0, 50.0, -8.0, 52.0, 4326)});
    let mut dflt = map_frame_element("DEFAULT", (6.0, 1.0, 9.0, 5.0), json!({}));
    dflt["uRI"] = json!("CIMPATH=map/sp.json");
    let mut lost = map_frame_element("LOST", (10.0, 1.0, 14.0, 5.0), camera(2_000_000.0, 300_000.0, 12_000.0));
    lost["uRI"] = json!("CIMPATH=map/none.json");
    let (imp, report) = read_with_maps(&maps, &[auto, dflt, lost]);
    // The extent's centre, the camera's scale (its x and y are in the map's own system).
    let (_, ElementKind::MapFrame { center, scale, .. }) = frame_of(&imp, "AUTO") else { panic!() };
    assert!(near(center.x, -9.0, 1e-9) && near(center.y, 51.0, 1e-9) && scale == 12_000.0, "{center:?} {scale}");
    assert_eq!(imp.frames[0].saved_camera, Some((2_000_000.0, 300_000.0, 12_000.0)));
    // The map's default extent; no camera scale, so none.
    let (_, ElementKind::MapFrame { center, scale, .. }) = frame_of(&imp, "DEFAULT") else { panic!() };
    assert!(near(center.x, -79.0, 1e-9) && near(center.y, 41.0, 1e-9) && scale == 0.0, "{center:?} {scale}");
    assert_eq!(imp.frames[1].saved_camera, None);
    // Nothing usable: the map's view, and the system said.
    let (_, ElementKind::MapFrame { center, scale, .. }) = frame_of(&imp, "LOST") else { panic!() };
    assert!(center.x == 0.0 && center.y == 0.0 && scale == 0.0);
    assert_eq!(notes(&report), ["LOST: frame camera in an unsupported coordinate system (2272); the map's view is used"]);
}

#[test]
fn absurd_camera_values_count_as_absent() {
    let maps = [embedded_map("CIMPATH=map/deg.json", 4326, json!({}))];
    let mk = |name: &str, x: f64, y: f64, s: f64| {
        let mut f = map_frame_element(name, (1.0, 1.0, 5.0, 5.0), camera(x, y, s));
        f["uRI"] = json!("CIMPATH=map/deg.json");
        f
    };
    let els = [mk("LAT", 10.0, 91.0, 1000.0), mk("LON", 181.0, 10.0, 1000.0), mk("SCALE0", 10.0, 10.0, 0.0), mk("HUGE", 10.0, 10.0, 2e9)];
    let (imp, _) = read_with_maps(&maps, &els);
    for name in ["LAT", "LON", "SCALE0", "HUGE"] {
        let (_, ElementKind::MapFrame { center, scale, .. }) = frame_of(&imp, name) else { panic!() };
        assert!(center.x == 0.0 && center.y == 0.0 && scale == 0.0, "{name}: {center:?} {scale}");
    }
}

#[test]
fn an_unbound_frame_takes_no_camera_and_records_the_saved_one() {
    let main = map_frame_element("MAIN MAP", (1.0, 1.0, 9.0, 8.0), camera(-12_000_000.0, 4_000_000.0, 24_000.0));
    let mut gone = map_frame_element("GONE MAP", (10.0, 1.0, 16.0, 8.0), camera(1.0, 2.0, 3.0));
    gone["uRI"] = json!("CIMPATH=map/missing.json");
    let (imp, report) = read_with_maps(&[], &[main, gone]);
    let (_, ElementKind::MapFrame { center, scale, map, .. }) = frame_of(&imp, "MAIN MAP") else { panic!() };
    // Not clamped up to the smallest scale: zero means "take the map's view".
    assert!(center.x == 0.0 && center.y == 0.0 && scale == 0.0, "{center:?} {scale}");
    assert_eq!(map, "MAIN MAP");
    let binding = &imp.frames[0];
    assert_eq!((binding.name.as_str(), binding.map_uri.as_deref()), ("MAIN MAP", None));
    assert_eq!(binding.saved_camera, Some((-12_000_000.0, 4_000_000.0, 24_000.0)));
    assert_eq!(binding.element, imp.layout.elements.first().unwrap().id);
    // A uRI that doesn't resolve is kept as given, and said.
    assert_eq!(imp.frames[1].map_uri.as_deref(), Some("CIMPATH=map/missing.json"));
    assert_eq!(imp.embedded_maps, ["CIMPATH=map/missing.json"]);
    let (_, ElementKind::MapFrame { scale, .. }) = frame_of(&imp, "GONE MAP") else { panic!() };
    assert_eq!(scale, 0.0);
    assert_eq!(notes(&report), ["GONE MAP: the map CIMPATH=map/missing.json isn't in the document; the map's view is used"]);
}

#[test]
fn linked_frames_and_extent_indicators_resolve_by_name() {
    let line = |w: f64| json!({"type": "CIMSymbolReference", "symbol": {"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(rgb(0, 0, 0), w)]}});
    let main = map_frame_element("MAIN MAP", (1.0, 1.0, 9.0, 8.0), json!({}));
    // Listed before the frame it follows and outlines, so the names resolve in a second pass.
    let mut small = map_frame_element("SMALL DETAIL MAP", (10.0, 1.0, 16.0, 4.0), camera(0.0, 0.0, 5000.0));
    small["autoCamera"] = json!({"type": "CIMAutoCamera", "source": "MapFrameLink", "mapFrameLinkName": "MAIN MAP", "autoCameraType": "Center", "syncRotation": true});
    small["extentIndicators"] = json!([
        {"type": "CIMExtentIndicator", "sourceMapFrame": "MAIN MAP", "symbol": line(2.0)},
        {"type": "CIMExtentIndicator", "sourceMapFrame": "MAIN MAP", "symbol": line(3.0)},
    ]);
    small["view"]["camera"]["heading"] = json!(30.0);
    small["uRI"] = json!("CIMPATH=map/detail.json");
    let (imp, report) = read_with_maps(&[embedded_map("CIMPATH=map/detail.json", 4326, json!({}))], &[small, main]);
    let main_id = frame_of(&imp, "MAIN MAP").0;
    let (small_id, ElementKind::MapFrame { link, extent_of, extent_color, extent_width, bearing, pitch, .. }) =
        frame_of(&imp, "SMALL DETAIL MAP")
    else {
        panic!()
    };
    assert_eq!(link, Some(FrameLink { frame: main_id, mode: LinkMode::Center, rotation: true }));
    assert_eq!(extent_of, Some(main_id));
    assert_ne!(small_id, main_id);
    assert_eq!(extent_color, Rgba::rgb(0, 0, 0));
    assert!(near(extent_width, 2.0 * 96.0 / 72.0, 1e-6), "{extent_width}");
    assert!(bearing == 30.0 && pitch == 0.0);
    assert_eq!(notes(&report), ["SMALL DETAIL MAP: 1 more extent indicators, left out"]);
}

#[test]
fn link_modes_and_links_that_cannot_stand() {
    let linking = |name: &str, target: &str, ty: &str| {
        let mut f = map_frame_element(name, (1.0, 1.0, 5.0, 5.0), json!({}));
        f["autoCamera"] = json!({"source": "MapFrameLink", "mapFrame": target, "autoCameraType": ty, "syncRotation": false});
        f
    };
    let els = [
        map_frame_element("A", (1.0, 1.0, 5.0, 5.0), json!({})),
        linking("EXTENT", "A", "Extent"),
        linking("SCALE", "A", "Scale"),
        linking("ODD", "A", "Bearing"),
        linking("SELF", "SELF", "Center"),
        linking("MISSING", "NOPE", "Center"),
    ];
    let (imp, report) = read_with_maps(&[], &els);
    let a = frame_of(&imp, "A").0;
    let link = |n: &str| match frame_of(&imp, n).1 {
        ElementKind::MapFrame { link, .. } => link,
        _ => panic!(),
    };
    assert_eq!(link("EXTENT"), Some(FrameLink { frame: a, mode: LinkMode::Extent, rotation: false }));
    assert_eq!(link("SCALE"), Some(FrameLink { frame: a, mode: LinkMode::Scale, rotation: false }));
    assert_eq!(link("ODD"), Some(FrameLink { frame: a, mode: LinkMode::Center, rotation: false }));
    assert_eq!((link("SELF"), link("MISSING")), (None, None));
    let n = notes(&report);
    for want in [
        "ODD: link type Bearing, taken as centre",
        "SELF: follows itself; the link is left out",
        "MISSING: follows the frame NOPE, which isn't on the page; the link is left out",
    ] {
        assert!(n.iter().any(|s| s == want), "{want}: {n:?}");
    }
    assert_eq!(n.len(), 3, "{n:?}");
}

#[test]
fn an_indicator_of_a_frame_not_on_the_page_is_dropped_and_duplicate_names_take_the_first() {
    let ind = |target: &str| json!([{"type": "CIMExtentIndicator", "sourceMapFrame": target}]);
    let mut gone = map_frame_element("X", (1.0, 1.0, 5.0, 5.0), json!({}));
    gone["extentIndicators"] = ind("NOPE");
    let mut me = map_frame_element("Y", (1.0, 1.0, 5.0, 5.0), json!({}));
    me["extentIndicators"] = ind("Y");
    let mut dup = map_frame_element("Y", (1.0, 1.0, 5.0, 5.0), json!({}));
    dup["extentIndicators"] = ind("Y");
    let mut z = map_frame_element("Z", (1.0, 1.0, 5.0, 5.0), json!({}));
    z["extentIndicators"] = ind("Y");
    let (imp, report) = read_with_maps(&[], &[gone, me, dup, z]);
    let ids: Vec<u32> = imp.layout.elements.iter().map(|e| e.id).collect();
    let of = |i: usize| match &imp.layout.elements[i].kind {
        ElementKind::MapFrame { extent_of, extent_color, extent_width, .. } => (*extent_of, *extent_color, *extent_width),
        _ => panic!(),
    };
    // No symbol: the defaults stay.
    assert_eq!(of(0), (None, crate::layout::INDICATOR_COLOR, crate::layout::INDICATOR_WIDTH));
    assert_eq!(of(1).0, None);
    assert_eq!(of(2).0, Some(ids[1])); // the second Y outlines the first
    assert_eq!(of(3).0, Some(ids[1]));
    let n = notes(&report);
    for want in [
        "X: outlines the frame NOPE, which isn't on the page; the indicator is left out",
        "Y: outlines its own extent; the indicator is left out",
        "Y: another map frame has this name; the first is the one links and text find",
    ] {
        assert!(n.iter().any(|s| s == want), "{want}: {n:?}");
    }
}

#[test]
fn text_bound_to_a_frame_gets_its_id() {
    let tag = |frame: &str| format!(r#"<dyn type="mapFrame" name="{frame}" property="scale" preStr="1:"/>"#);
    let main = map_frame_element("MAIN MAP", (1.0, 1.0, 9.0, 8.0), json!({}));
    let ok = text_element("CIMTextGraphic", "OK", json!({"x": 1.0, "y": 9.0}), &tag("MAIN MAP"), text_symbol(json!({})));
    let lost = text_element("CIMTextGraphic", "LOST", json!({"x": 1.0, "y": 10.0}), &tag("GONE"), text_symbol(json!({})));
    let (imp, report) = read_with_maps(&[], &[main, ok, lost]);
    let main_id = frame_of(&imp, "MAIN MAP").0;
    let frame_of_text = |n: &str| match frame_of(&imp, n).1 {
        ElementKind::Text { frame, .. } => frame,
        _ => panic!(),
    };
    assert_eq!(frame_of_text("OK"), Some(main_id));
    assert_eq!(frame_of_text("LOST"), None);
    // The names are kept for the caller.
    let names: Vec<&str> = imp.text_frames.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["MAIN MAP", "GONE"]);
    assert_eq!(notes(&report), ["LOST: reads frame GONE, which isn't on the page; the first frame is used"]);
}

#[test]
fn a_frame_shape_other_than_a_rectangle_comes_across() {
    // A triangle in a 4 x 2 inch box at (1, 1): points relative to the box, y from the top.
    let tri = json!({"rings": [[[1.0, 1.0], [3.0, 3.0], [5.0, 1.0], [1.0, 1.0]]]});
    let mut a = map_frame_element("TRI", (1.0, 1.0, 5.0, 3.0), json!({}));
    a["frame"] = tri;
    let circle = json!({"curveRings": [[[5.0, 2.0], {"a": [[5.0, 2.0], [3.0, 2.0], 0, 1]}]]});
    let mut b = map_frame_element("OVAL", (1.0, 1.0, 5.0, 3.0), json!({}));
    b["frame"] = circle;
    let mut c = map_frame_element("EMPTY", (1.0, 1.0, 5.0, 3.0), json!({}));
    c["frame"] = json!({});
    let (imp, report) = read_with_maps(&[], &[a, b, c]);
    let (_, ElementKind::MapFrame { shape, .. }) = frame_of(&imp, "TRI") else { panic!() };
    assert_eq!(shape, FrameShape::Polygon(vec![(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)]));
    let (_, ElementKind::MapFrame { shape, .. }) = frame_of(&imp, "OVAL") else { panic!() };
    assert_eq!(shape, FrameShape::Ellipse);
    // A frame with no shape can't be placed.
    assert_eq!(imp.layout.elements.len(), 2);
    assert_eq!(notes(&report), ["EMPTY: a map frame with no shape, left out"]);
}

#[test]
fn a_frames_border_comes_across_and_embedded_maps_are_distinct_in_order() {
    let maps = [embedded_map("CIMPATH=map/a.json", 4326, json!({})), embedded_map("CIMPATH=map/b.json", 4326, json!({}))];
    let mk = |name: &str, uri: &str| {
        let mut f = map_frame_element(name, (1.0, 1.0, 5.0, 5.0), json!({}));
        f["uRI"] = json!(uri);
        f
    };
    let mut first = mk("F1", "CIMPATH=map/b.json");
    first["graphicFrame"] = json!({"type": "CIMGraphicFrame", "borderSymbol": {"type": "CIMSymbolReference",
        "symbol": {"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(rgb(0, 0, 0), 2.0)]}}});
    let (imp, _) = read_with_maps(&maps, &[first, mk("F2", "CIMPATH=MAP/A.json"), mk("F3", "CIMPATH=MAP/B.JSON")]);
    assert_eq!(imp.embedded_maps, ["CIMPATH=map/b.json", "CIMPATH=MAP/A.json"]);
    let border = imp.layout.elements.first().and_then(|e| e.frame_style.as_ref()).and_then(|f| f.border);
    assert_eq!(border, Some(Rgba::rgb(0, 0, 0)));
}

// ---- surrounds, tables and map series ----

fn layer_def(name: &str, uri: &str) -> serde_json::Value {
    json!({"type": "CIMFeatureLayer", "name": name, "uRI": uri})
}

/// A surround element of type `ty` reading frame `frame`, over a box at the right of the page,
/// with `extra` keys set on it.
fn surround_element(ty: &str, name: &str, frame: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut e = json!({"type": ty, "name": name, "anchor": "BottomLeftCorner", "visible": true, "mapFrame": frame,
        "frame": corner_ring(10.0, 1.0, 16.0, 5.0)});
    if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
        o.extend(x.clone());
    }
    e
}

/// Reads `elements` from a layout holding `layers` as layer definitions and, unless null, `series`
/// as its map series.
fn read_surrounds(layers: &[serde_json::Value], series: serde_json::Value, elements: &[serde_json::Value]) -> (LayoutImport, Report) {
    let mut def = json!({"type": "CIMLayout", "name": "P",
        "page": {"type": "CIMPage", "width": 17, "height": 11, "units": {"uwkid": 109008}, "guides": []},
        "elements": elements});
    if !series.is_null() {
        def["mapSeries"] = series;
    }
    let doc = json!({"type": "CIMLayoutDocument", "version": "3.7.0", "layerDefinitions": layers, "layoutDefinition": def});
    let mut report = Report::default();
    let imported = read_layout(&Document::from_json(&doc.to_string()).unwrap(), &mut report).unwrap();
    (imported, report)
}

fn main_frame() -> serde_json::Value {
    map_frame_element("MAIN MAP", (1.0, 1.0, 9.0, 8.0), json!({}))
}

#[test]
fn legends_read_their_title_columns_fit_and_layer_names() {
    use crate::layout::LegendFit;
    let layers = [layer_def("Roads", "CIMPATH=map/roads.json"), layer_def("Mask", "CIMPATH=map/mask.json")];
    // The template's legend: no title, one column, the fit that adds columns and shrinks text.
    let template = surround_element(
        "CIMLegend",
        "LEGEND",
        "MAIN MAP",
        json!({"title": "Legend", "showTitle": false, "columns": 1, "fittingStrategy": "AdjustColumnsAndSize",
            "defaultLegendItem": {"type": "CIMLegendItem", "showLayerName": true, "showHeading": true},
            "excludedLayers": ["CIMPATH=map/mask.json", "CIMPATH=map/elsewhere.json", "CIMPATH=MAP/Mask.json"]}),
    );
    // Titled, too many columns, a font-size fit, layer names off, and items of their own.
    let item = |uri: &str, heading: bool| json!({"type": "CIMLegendItem", "layerURI": uri, "showLayerName": false, "showHeading": heading});
    let titled = surround_element(
        "CIMLegend",
        "KEY",
        "MAIN MAP",
        json!({"title": "Key", "showTitle": true, "columns": 9, "fittingStrategy": "AdjustFontSize",
            "defaultLegendItem": {"type": "CIMLegendItem", "showLayerName": false, "showHeading": true},
            "items": [item("CIMPATH=map/roads.json", true), item("CIMPATH=map/mask.json", false), item("CIMPATH=map/mask.json", false)]}),
    );
    let odd = surround_element("CIMLegend", "ODD", "MAIN MAP", json!({"columns": "many", "fittingStrategy": "Spiral"}));
    let manual = surround_element("CIMLegend", "MANUAL", "MAIN MAP", json!({"fittingStrategy": "ManualColumns", "columns": 0}));
    let (imp, report) = read_surrounds(&layers, json!(null), &[template, titled, odd, manual, main_frame()]);
    let main = frame_of(&imp, "MAIN MAP").0;
    let legend = |title: &str, exclude: &[&str], columns: usize, fit: LegendFit, show_layer_names: bool| ElementKind::Legend {
        frame: main,
        title: title.to_string(),
        all_classes: false,
        hide_other: false,
        exclude: exclude.iter().map(|s| s.to_string()).collect(),
        columns,
        fit,
        show_layer_names,
    };
    assert_eq!(frame_of(&imp, "LEGEND").1, legend("", &["Mask"], 1, LegendFit::AdjustColumnsAndSize, true));
    assert_eq!(frame_of(&imp, "KEY").1, legend("Key", &[], 6, LegendFit::AdjustSize, false));
    assert_eq!(frame_of(&imp, "ODD").1, legend("Legend", &[], 1, LegendFit::Manual, true));
    assert_eq!(frame_of(&imp, "MANUAL").1, legend("Legend", &[], 1, LegendFit::Manual, true));
    assert_eq!(
        notes(&report),
        ["KEY: per-layer legend item settings, drawn with the defaults", "ODD: fitting strategy Spiral, drawn as manual"]
    );
    // Its box is the frame's, in millimetres from the top-left of an 11 inch page.
    let el = imp.layout.elements.iter().find(|e| e.name == "LEGEND").unwrap();
    assert!(near(el.x, 254.0, 1e-6) && near(el.y, 6.0 * 25.4, 1e-6) && near(el.w, 152.4, 1e-6) && near(el.h, 101.6, 1e-6));
}

#[test]
fn scale_bars_read_units_divisions_subdivisions_labels_and_format() {
    use crate::layout::{BarLabels, ScaleBarStyle, ScaleFit, ScaleUnits, UnitLabel};
    let label = json!({"type": "CIMSymbolReference", "symbol": {"type": "CIMTextSymbol", "height": 9.0, "fontStyleName": "Regular"}});
    // The template's bar: feet, four divisions, the first cut in two, labels below.
    let template = surround_element(
        "CIMDoubleFillScaleBar",
        "SCALE BAR",
        "MAIN MAP",
        json!({"units": {"uwkid": 9002}, "divisions": 4, "subdivisions": 2, "divisionsBeforeZero": 0, "labelPosition": "Bottom",
            "unitLabel": "Feet", "unitLabelPosition": "AfterBar",
            "numberFormat": {"type": "CIMNumericFormat", "roundingOption": "esriRoundNumberOfDecimals", "roundingValue": 0},
            "labelSymbol": label, "fittingStrategy": "AdjustDivision"}),
    );
    let line = surround_element(
        "CIMLineScaleBar",
        "LINE",
        "MAIN MAP",
        json!({"units": {"uwkid": 9093}, "divisions": 99, "subdivisions": -3, "divisionsBeforeZero": 7, "labelPosition": "Above",
            "unitLabelPosition": "Sideways", "unitLabel": "",
            "numberFormat": {"roundingOption": "esriRoundNumberOfDecimals", "roundingValue": 9},
            "labelSymbol": {"type": "CIMSymbolReference", "symbol": {"type": "CIMTextSymbol", "height": 100.0}},
            "fittingStrategy": "AdjustWidth", "division": 2.5, "markFrequency": "DivisionsAndSubdivisions", "markHeight": 5.0}),
    );
    let stepped = surround_element(
        "CIMSteppedLineScaleBar",
        "STEPPED",
        "MAIN MAP",
        json!({"units": {"uwkid": 9001}, "fittingStrategy": "AdjustFrame"}),
    );
    let hollow = surround_element(
        "CIMHollowScaleBar",
        "HOLLOW",
        "MAIN MAP",
        json!({"units": {"uwkid": 9036}, "fittingStrategy": "AdjustDivisionValue"}),
    );
    let odd =
        surround_element("CIMAlternatingScaleBar", "ODD", "MAIN MAP", json!({"units": {"uwkid": 4326}, "fittingStrategy": "Sideways"}));
    let (imp, report) = read_surrounds(&[], json!(null), &[template, line, stepped, hollow, odd, main_frame()]);
    let main = frame_of(&imp, "MAIN MAP").0;
    let ElementKind::ScaleBar {
        frame,
        units,
        divisions,
        style,
        subdivisions,
        divisions_before_zero,
        label_position,
        unit_label,
        unit_label_position,
        decimals,
        label_size,
        fit,
    } = frame_of(&imp, "SCALE BAR").1
    else {
        panic!("not a scale bar")
    };
    assert_eq!(frame, main);
    assert_eq!((units, divisions, subdivisions, divisions_before_zero), (ScaleUnits::Feet, 4, 2, 0));
    assert_eq!((label_position, unit_label.as_deref(), unit_label_position), (BarLabels::Below, Some("Feet"), UnitLabel::AfterBar));
    assert_eq!((decimals, fit, style, label_size), (0, ScaleFit::AdjustDivision, ScaleBarStyle::Alternating, 9.0));
    // Everything out of range is clamped; what RustGIS has no word for takes the default.
    let ElementKind::ScaleBar {
        units,
        divisions,
        style,
        subdivisions,
        divisions_before_zero,
        label_position,
        unit_label,
        unit_label_position,
        decimals,
        label_size,
        fit,
        ..
    } = frame_of(&imp, "LINE").1
    else {
        panic!("not a scale bar")
    };
    assert_eq!((units, divisions, subdivisions, divisions_before_zero), (ScaleUnits::Miles, 10, 0, 2));
    assert_eq!((label_position, unit_label.as_deref(), unit_label_position), (BarLabels::Above, Some(""), UnitLabel::AfterLabels));
    assert_eq!((decimals, fit, style, label_size), (3, ScaleFit::AdjustWidth, ScaleBarStyle::Line, 24.0));
    let get = |n: &str| match frame_of(&imp, n).1 {
        ElementKind::ScaleBar { units, style, fit, label_size, unit_label, decimals, .. } => {
            (units, style, fit, label_size, unit_label, decimals)
        }
        _ => panic!("not a scale bar"),
    };
    assert_eq!(get("STEPPED"), (ScaleUnits::Meters, ScaleBarStyle::Stepped, ScaleFit::AdjustWidth, 8.0, None, 0));
    assert_eq!(get("HOLLOW"), (ScaleUnits::Kilometers, ScaleBarStyle::Alternating, ScaleFit::AdjustDivisionValue, 8.0, None, 0));
    assert_eq!(get("ODD"), (ScaleUnits::Auto, ScaleBarStyle::Alternating, ScaleFit::AdjustDivisionValue, 8.0, None, 0));
    assert_eq!(
        notes(&report),
        [
            "LINE: unit label position Sideways, drawn after the labels",
            "LINE: division value 2.5, drawn as RustGIS fits it",
            "LINE: mark heights and frequency, drawn as RustGIS's",
            "HOLLOW: a hollow bar, drawn as an alternating bar",
            "ODD: units 4326, drawn in automatic units",
            "ODD: fitting strategy Sideways, drawn as adjust division value",
        ]
    );
}

#[test]
fn scale_bars_use_pros_words_for_sides_and_the_division_value() {
    use crate::layout::{BarLabels, UnitLabel};
    let bar = |name: &str, extra: serde_json::Value| surround_element("CIMLineScaleBar", name, "MAIN MAP", extra);
    let els = [
        // Pro writes Top and Bottom for the sides of the bar.
        bar("TOP", json!({"labelPosition": "Top", "unitLabelPosition": "Bottom", "fittingStrategy": "AdjustDivision", "division": 500})),
        bar("BELOW", json!({"labelPosition": " below ", "unitLabelPosition": "Top", "fittingStrategy": "AdjustWidth", "divisionValue": 7})),
        bar(
            "AFTER",
            json!({"labelPosition": "Above", "unitLabelPosition": "AfterLabels", "fittingStrategy": "AdjustDivisionValue", "division": 9}),
        ),
        bar("ODD", json!({"labelPosition": "Middle", "unitLabelPosition": "Left"})),
        main_frame(),
    ];
    let (imp, report) = read_surrounds(&[], json!(null), &els);
    let get = |n: &str| match frame_of(&imp, n).1 {
        ElementKind::ScaleBar { label_position, unit_label_position, .. } => (label_position, unit_label_position),
        _ => panic!("not a scale bar"),
    };
    assert_eq!(get("TOP"), (BarLabels::Above, UnitLabel::Below));
    assert_eq!(get("BELOW"), (BarLabels::Below, UnitLabel::Above));
    assert_eq!(get("AFTER"), (BarLabels::Above, UnitLabel::AfterLabels));
    assert_eq!(get("ODD"), (BarLabels::Above, UnitLabel::AfterLabels));
    assert_eq!(
        notes(&report),
        [
            "TOP: division value 500, drawn as RustGIS fits it",
            "BELOW: division value 7, drawn as RustGIS fits it",
            "ODD: label position Middle, drawn above the bar",
            "ODD: unit label position Left, drawn after the labels",
        ]
    );
}

#[test]
fn north_arrows_carry_their_point_symbol() {
    use crate::symbology::Marker;
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let gif = "data:image/gif;base64,R0lGODlhAQABAAAAACw=";
    let picture = |url: &str| json!({"type": "CIMPictureMarker", "enable": true, "size": 36.0, "url": url});
    let character = |c: u8| {
        json!({"type": "CIMCharacterMarker", "enable": true, "size": 24.0, "characterIndex": 178, "fontFamilyName": "ESRI North",
            "fontStyleName": "Regular", "symbol": polygon_symbol(vec![solid_fill(rgb(c, 0, 0))])})
    };
    let reference = |layers: Vec<serde_json::Value>| json!({"type": "CIMSymbolReference", "symbol": {"type": "CIMPointSymbol", "symbolLayers": layers}});
    let template = surround_element(
        "CIMMarkerNorthArrow",
        "NORTH ARROW",
        "MAIN MAP",
        json!({"pointSymbol": reference(vec![picture(png), character(10)]), "northType": "TrueNorth"}),
    );
    let mixed = surround_element(
        "CIMMarkerNorthArrow",
        "GIF ARROW",
        "MAIN MAP",
        json!({"pointSymbol": reference(vec![character(20), picture(gif)]), "northType": "MagneticNorth", "calibrationAngle": 12.5}),
    );
    let many = surround_element(
        "CIMMarkerNorthArrow",
        "MANY",
        "MAIN MAP",
        json!({"pointSymbol": reference((0..10).map(|_| character(30)).collect())}),
    );
    let bare = surround_element("CIMMarkerNorthArrow", "BARE", "MAIN MAP", json!({}));
    let (imp, report) = read_surrounds(&[], json!(null), &[template, mixed, many, bare, main_frame()]);
    let main = frame_of(&imp, "MAIN MAP").0;
    let ElementKind::NorthArrow { frame, style, fill, outline, symbol } = frame_of(&imp, "NORTH ARROW").1 else {
        panic!("not a north arrow")
    };
    assert_eq!((frame, style), (main, crate::layout::ArrowStyle::Classic));
    assert_eq!(symbol.len(), 2);
    assert!(matches!(symbol[0].marker, Marker::Image(_)));
    assert!(matches!(symbol[1].marker, Marker::Char(_)));
    // The top character marker's fill is the arrow's colour.
    assert_eq!((fill, outline), (Rgba::rgb(10, 0, 0), Rgba::rgb(10, 0, 0)));
    // The GIF is a circle until the engine converts it; the arrow's colour is the character's.
    let (gif_id, ElementKind::NorthArrow { symbol, fill, .. }) = frame_of(&imp, "GIF ARROW") else { panic!("not a north arrow") };
    assert_eq!((symbol.len(), fill), (2, Rgba::rgb(20, 0, 0)));
    assert_eq!(imp.pending_pictures, vec![(gif_id, 1, gif.to_string())]);
    // More than eight marker layers keep the first eight; no symbol is the style's own arrow.
    let ElementKind::NorthArrow { symbol, .. } = frame_of(&imp, "MANY").1 else { panic!("not a north arrow") };
    assert_eq!(symbol.len(), 8);
    let ElementKind::NorthArrow { symbol, fill, outline, .. } = frame_of(&imp, "BARE").1 else { panic!("not a north arrow") };
    assert_eq!((symbol.len(), fill, outline), (0, Rgba::rgb(0, 0, 0), Rgba::rgb(0, 0, 0)));
    assert_eq!(
        notes(&report),
        [
            "GIF ARROW: Magnetic north, drawn as true north",
            "GIF ARROW: a calibration angle of 12.5 degrees, left out",
            "MANY: 10 marker layers, the first 8 kept",
        ]
    );
}

#[test]
fn tables_and_series_resolve_their_layers() {
    use crate::layout::MapSeries;
    let layers = [layer_def("Parcels", "CIMPATH=map/parcels.json"), layer_def("Index", "CIMPATH=map/index.json")];
    let table = surround_element("CIMTableFrame", "TABLE", "MAIN MAP", json!({"tableFrameURI": "CIMPATH=map/parcels.json"}));
    let by_layer = surround_element("CIMTableFrame", "BY LAYER", "MAIN MAP", json!({"layerURI": "CIMPATH=map/index.json"}));
    let lost = surround_element("CIMTableFrame", "LOST", "MAIN MAP", json!({"tableFrameURI": "CIMPATH=map/gone.json"}));
    let series = json!({"type": "CIMSpatialMapSeries", "indexLayerURI": "CIMPATH=map/index.json", "mapFrameName": "MAIN MAP",
        "nameField": "SHEET", "sortField": "SHEET", "extentOptions": "BestFit"});
    let (imp, report) = read_surrounds(&layers, series, &[table, by_layer, lost, main_frame()]);
    let main = frame_of(&imp, "MAIN MAP").0;
    assert_eq!(frame_of(&imp, "TABLE").1, ElementKind::Table { frame: main, layer: "Parcels".into(), rows: 12 });
    assert_eq!(frame_of(&imp, "BY LAYER").1, ElementKind::Table { frame: main, layer: "Index".into(), rows: 12 });
    assert!(imp.layout.elements.iter().all(|e| e.name != "LOST"));
    assert_eq!(imp.layout.series, Some(MapSeries { frame: main, layer: "Index".into(), name_field: "SHEET".into(), margin: 0.1, page: 0 }));
    assert_eq!(
        notes(&report),
        [
            "TABLE: table settings, drawn as RustGIS's",
            "BY LAYER: table settings, drawn as RustGIS's",
            "LOST: its layer isn't in the document, left out",
            "P: map series options, drawn as RustGIS's",
        ]
    );
    // A series whose layer or frame can't be found is left out, and says so.
    for (series, want) in [
        (
            json!({"indexLayerURI": "CIMPATH=map/gone.json", "mapFrameName": "MAIN MAP"}),
            "P: the map series' index layer isn't in the document, left out",
        ),
        (
            json!({"indexLayerURI": "CIMPATH=map/index.json", "mapFrameName": "NOPE"}),
            "P: the map series reads frame NOPE, which isn't on the page; left out",
        ),
    ] {
        let (imp, report) = read_surrounds(&layers, series, &[main_frame()]);
        assert_eq!(imp.layout.series, None);
        assert_eq!(notes(&report), [want]);
    }
}

#[test]
fn surrounds_of_a_frame_not_on_the_page_bind_to_the_first_frame_or_go() {
    let named = |n: &str| surround_element("CIMMarkerNorthArrow", n, "NOPE", json!({}));
    // Listed before the frames: the second pass finds the lowest id among them.
    let els = [
        named("ARROW"),
        surround_element("CIMLegend", "LEGEND", "SECOND", json!({})),
        main_frame(),
        map_frame_element("SECOND", (10.0, 6.0, 16.0, 9.0), json!({})),
    ];
    let (imp, report) = read_surrounds(&[], json!(null), &els);
    let main = frame_of(&imp, "MAIN MAP").0;
    let second = frame_of(&imp, "SECOND").0;
    assert_eq!(frame_of(&imp, "ARROW").1.frame(), Some(main));
    assert_eq!(frame_of(&imp, "LEGEND").1.frame(), Some(second));
    assert_eq!(notes(&report), ["ARROW: reads frame NOPE, which isn't on the page; MAIN MAP is used"]);
    // No map frame at all: the surrounds are removed, each with a note.
    let (imp, report) = read_surrounds(&[], json!(null), &[named("ARROW"), surround_element("CIMLegend", "LEGEND", "", json!({}))]);
    assert!(imp.layout.elements.is_empty());
    assert_eq!(
        notes(&report),
        [
            "ARROW: reads frame NOPE, and the page has no map frame; left out",
            "LEGEND: names no map frame, and the page has no map frame; left out",
        ]
    );
}

#[test]
fn hostile_surrounds_never_crash() {
    let junk = json!({"units": "feet", "divisions": "four", "subdivisions": null, "labelSymbol": 5, "numberFormat": [], "fittingStrategy": 3,
        "columns": 1e300, "defaultLegendItem": 7, "items": {"a": 1}, "excludedLayers": [1, null, "CIMPATH=x"], "pointSymbol": "x",
        "tableFrameURI": 4, "mapFrame": 9});
    let kinds = ["CIMLegend", "CIMDoubleFillScaleBar", "CIMLineScaleBar", "CIMMarkerNorthArrow", "CIMTableFrame"];
    let mut els: Vec<serde_json::Value> = kinds.iter().map(|k| surround_element(k, k, "MAIN MAP", junk.clone())).collect();
    // No shape: left out.
    els.push(json!({"type": "CIMLegend", "name": "SHAPELESS", "mapFrame": "MAIN MAP"}));
    // An object for the excluded layers, 10 000 items, a text for the number format.
    let items: Vec<serde_json::Value> = (0..10_000).map(|i| json!({"type": "CIMLegendItem", "showHeading": i % 2 == 0})).collect();
    els.push(surround_element("CIMLegend", "BIG", "MAIN MAP", json!({"excludedLayers": {"a": 1}, "items": items, "columns": 99})));
    els.push(surround_element("CIMAlternatingScaleBar", "TEXT FORMAT", "MAIN MAP", json!({"numberFormat": "x", "labelPosition": 4})));
    els.push(main_frame());
    let (imp, report) = read_surrounds(&[], json!({"indexLayerURI": 3, "mapFrameName": []}), &els);
    let main = frame_of(&imp, "MAIN MAP").0;
    // The junk legend's columns clamp; the junk scale bar keeps the defaults (and its style).
    assert!(matches!(frame_of(&imp, "CIMLegend").1, ElementKind::Legend { columns: 6, .. }));
    assert!(matches!(frame_of(&imp, "BIG").1, ElementKind::Legend { columns: 6, ref exclude, .. } if exclude.is_empty()));
    let mut line = ElementKind::scale_bar(main);
    if let ElementKind::ScaleBar { style, .. } = &mut line {
        *style = crate::layout::ScaleBarStyle::Line;
    }
    assert_eq!(frame_of(&imp, "CIMLineScaleBar").1, line);
    assert_eq!(frame_of(&imp, "TEXT FORMAT").1, ElementKind::scale_bar(main));
    assert!(imp.layout.elements.iter().any(|e| e.name == "CIMLegend"));
    assert!(imp.layout.elements.iter().all(|e| e.name != "SHAPELESS" && e.name != "CIMTableFrame"));
    assert!(notes(&report).iter().any(|n| n == "SHAPELESS: no shape, left out"), "{:?}", notes(&report));
}

// ---- groups, draw order and hostile layouts ----

/// A group element of `children`, with `extra` keys set on it.
fn group_element(name: &str, children: Vec<serde_json::Value>, extra: serde_json::Value) -> serde_json::Value {
    let mut g = json!({"type": "CIMGroupElement", "name": name, "visible": true, "elements": children});
    if let (Some(o), Some(e)) = (g.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    g
}

fn picture_element(name: &str) -> serde_json::Value {
    json!({"type": "CIMGraphicElement", "name": name, "anchor": "CenterPoint",
        "graphic": {"type": "CIMPictureGraphic", "referenceURI": "CIMPATH=Pictures/logo.png", "sourceURL": r"G:\LOGOS\Logo.png",
            "box": {"xmin": 0.76, "ymin": 9.0, "xmax": 1.83, "ymax": 9.41}}})
}

fn id_of(imp: &LayoutImport, name: &str) -> u32 {
    imp.layout.elements.iter().find(|e| e.name == name).map(|e| e.id).unwrap()
}

#[test]
fn groups_keep_their_children_and_nesting() {
    let link = text_element("CIMTextGraphic", "LINK", json!({"x": 2.0, "y": 9.5}), "www.example.com", text_symbol(json!({})));
    let (imp, report) = read(&[group_element("LOGO AND LINK", vec![picture_element("LOGO"), link], json!({}))]);
    let l = &imp.layout;
    let (logo, link, group) = (id_of(&imp, "LOGO"), id_of(&imp, "LINK"), id_of(&imp, "LOGO AND LINK"));
    assert_eq!(l.element(group).unwrap().children(), [logo, link]);
    assert!(group > logo && group > link, "the group is added after its children");
    assert_eq!(l.group_of(logo), Some(group));
    assert_eq!(l.group_of(link), Some(group));
    assert_eq!(l.group_of(group), None);
    assert_eq!(l.elements.last().map(|e| e.id), Some(group));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    // A group in a group: the outer one holds the inner one, and the inner one's children are two deep.
    let inner = group_element(
        "INNER",
        vec![rectangle_element("A", corner_ring(1.0, 1.0, 2.0, 2.0)), rectangle_element("B", corner_ring(3.0, 3.0, 4.0, 4.0))],
        json!({}),
    );
    let outer = group_element("OUTER", vec![inner, rectangle_element("C", corner_ring(5.0, 5.0, 6.0, 6.0))], json!({}));
    let (imp, _) = read(&[outer]);
    let l = &imp.layout;
    let (a, b, c) = (id_of(&imp, "A"), id_of(&imp, "B"), id_of(&imp, "C"));
    let (inner, outer) = (id_of(&imp, "INNER"), id_of(&imp, "OUTER"));
    assert_eq!(l.element(inner).unwrap().children(), [a, b]);
    assert_eq!(l.element(outer).unwrap().children(), [inner, c]);
    assert!(inner > a && inner > b && outer > inner && outer > c);
    assert_eq!(l.group_of(inner), Some(outer));
    assert_eq!(l.groups_of(a), vec![inner, outer]);
    assert_eq!(l.groups_of(c), vec![outer]);
    assert_eq!(l.groups_of(outer), Vec::<u32>::new());
    assert_eq!(l.group_depth(outer), 2);

    // A group's own graphic frame becomes its border and background.
    let line_sym = |c: serde_json::Value| json!({"type": "CIMSymbolReference", "symbol": {"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(c, 1.0)]}});
    let frame = json!({"type": "CIMGraphicFrame", "borderSymbol": line_sym(rgb(0, 0, 0)),
        "backgroundSymbol": {"type": "CIMSymbolReference", "symbol": polygon_symbol(vec![solid_fill(rgb(255, 255, 255))])}});
    let framed = group_element("FRAMED", vec![rectangle_element("D", corner_ring(1.0, 1.0, 2.0, 2.0))], json!({"graphicFrame": frame}));
    let (imp, _) = read(&[framed]);
    let g = imp.layout.element(id_of(&imp, "FRAMED")).unwrap();
    let fs = g.frame_style.clone().unwrap();
    assert_eq!((fs.border, fs.background), (Some(Rgba::rgb(0, 0, 0)), Some(Rgba::rgb(255, 255, 255))));
    assert_eq!(imp.layout.element(id_of(&imp, "D")).unwrap().frame_style, None);
}

#[test]
fn elements_keep_pros_draw_order() {
    // Pro lists the bottom element first: the map frame under the rectangle, the group and the text.
    let text = text_element("CIMTextGraphic", "TITLE", json!({"x": 2.0, "y": 10.0}), "Title", text_symbol(json!({})));
    let (imp, _) = read_with_maps(
        &[embedded_map("CIMPATH=map/m.json", 4326, json!({}))],
        &[
            map_frame_element("MAIN MAP", (1.0, 1.0, 9.0, 8.0), json!({"mapURI": "CIMPATH=map/m.json"})),
            rectangle_element("BOX", corner_ring(1.0, 1.0, 2.0, 2.0)),
            group_element("G", vec![rectangle_element("IN1", corner_ring(3.0, 3.0, 4.0, 4.0)), picture_element("IN2")], json!({})),
            text,
        ],
    );
    let l = &imp.layout;
    let names: Vec<&str> = l.elements.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["MAIN MAP", "BOX", "IN1", "IN2", "G", "TITLE"]);
    let n = names.len();
    for (i, name) in names.iter().enumerate() {
        assert_eq!(l.draw_order(id_of(&imp, name)), Some((i, n)), "{name}");
    }
    // The first listed is drawn first (underneath), and each group comes after its children.
    let group = l.element(id_of(&imp, "G")).unwrap();
    for child in group.children() {
        assert!(l.draw_order(*child) < l.draw_order(group.id));
    }
    assert!(matches!(l.elements.first().map(|e| &e.kind), Some(ElementKind::MapFrame { .. })));
}

/// Every path to a node of `v` (the root's is empty).
fn json_paths(v: &serde_json::Value, at: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    out.push(at.clone());
    match v {
        serde_json::Value::Object(m) => {
            for (k, c) in m {
                at.push(k.clone());
                json_paths(c, at, out);
                at.pop();
            }
        }
        serde_json::Value::Array(a) => {
            for (i, c) in a.iter().enumerate() {
                at.push(i.to_string());
                json_paths(c, at, out);
                at.pop();
            }
        }
        _ => {}
    }
}

fn node_at<'a>(v: &'a mut serde_json::Value, path: &[String]) -> Option<&'a mut serde_json::Value> {
    let Some((first, rest)) = path.split_first() else { return Some(v) };
    let next = match v {
        serde_json::Value::Object(m) => m.get_mut(first)?,
        serde_json::Value::Array(a) => a.get_mut(first.parse::<usize>().ok()?)?,
        _ => return None,
    };
    node_at(next, rest)
}

/// What a node is replaced with, one at a time: junk suited to its kind, and junk of another.
fn replacements(v: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut out = vec![json!(null), json!("x")];
    match v {
        serde_json::Value::Number(_) => {
            out.extend([json!(1e308), json!(-1e308), json!(0), json!(-1), json!(u64::MAX), json!(i64::MIN), json!(1e-308)])
        }
        serde_json::Value::String(_) => out.extend([json!("a".repeat(10_000)), json!("")]),
        serde_json::Value::Array(_) => out.extend([json!([]), json!(7), json!([[], {}, 1e308])]),
        serde_json::Value::Object(_) => out.extend([json!(7), json!({})]),
        serde_json::Value::Bool(_) => out.extend([json!(true), json!(false)]),
        serde_json::Value::Null => out.extend([json!(1e308), json!({}), json!([])]),
    }
    out
}

/// A layout document with a map frame, a group of two, a text with a dynamic tag, a picture, a
/// legend, a scale bar and a north arrow.
fn hostile_base() -> serde_json::Value {
    let text = text_element(
        "CIMTextGraphic",
        "SCALE TEXT",
        json!({"x": 2.0, "y": 10.0}),
        r#"Scale <dyn type="mapFrame" name="MAIN MAP" property="scale" preStr="1:"/>"#,
        text_symbol(json!({})),
    );
    let character = json!({"type": "CIMCharacterMarker", "enable": true, "size": 24.0, "characterIndex": 178, "fontFamilyName": "ESRI North",
        "fontStyleName": "Regular", "symbol": polygon_symbol(vec![solid_fill(rgb(10, 0, 0))])});
    let elements = vec![
        map_frame_element(
            "MAIN MAP",
            (1.0, 1.0, 9.0, 8.0),
            json!({"mapURI": "CIMPATH=map/m.json", "graphicFrame": {"type": "CIMGraphicFrame", "borderGapX": 3}}),
        ),
        group_element(
            "LOGO AND LINK",
            vec![picture_element("LOGO"), rectangle_element("RULE", corner_ring(1.0, 9.0, 3.0, 9.1))],
            json!({}),
        ),
        text,
        picture_element("PICTURE"),
        surround_element(
            "CIMLegend",
            "LEGEND",
            "MAIN MAP",
            json!({"title": "Legend", "columns": 2, "fittingStrategy": "AdjustColumnsAndSize", "excludedLayers": ["CIMPATH=map/roads.json"],
                "defaultLegendItem": {"type": "CIMLegendItem", "showLayerName": true, "showHeading": true},
                "items": [{"type": "CIMLegendItem", "layerURI": "CIMPATH=map/roads.json", "showHeading": false}]}),
        ),
        surround_element(
            "CIMDoubleFillScaleBar",
            "SCALE BAR",
            "MAIN MAP",
            json!({"units": {"uwkid": 9035}, "divisions": 2, "subdivisions": 1, "labelPosition": "AboveBar", "numberFormat": {"type": "CIMNumericFormat", "roundingValue": 0}}),
        ),
        surround_element(
            "CIMMarkerNorthArrow",
            "NORTH ARROW",
            "MAIN MAP",
            json!({"northType": "TrueNorth", "calibrationAngle": 0,
                "pointSymbol": {"type": "CIMSymbolReference", "symbol": {"type": "CIMPointSymbol", "symbolLayers": [character]}}}),
        ),
    ];
    json!({"type": "CIMLayoutDocument", "version": "3.7.0",
        "binaryReferences": [{"type": "CIMBinaryReference", "uRI": "CIMPATH=Pictures/logo.png", "data": "iVBORw0KGgo="}],
        "mapDefinitions": [embedded_map("CIMPATH=map/m.json", 3857, camera(-9_000_000.0, 4_000_000.0, 24_000.0))],
        "layerDefinitions": [layer_def("Roads", "CIMPATH=map/roads.json")],
        "layoutDefinition": {"type": "CIMLayout", "name": "P",
            "page": {"type": "CIMPage", "width": 17, "height": 11, "units": {"uwkid": 109008}, "guides": [{"type": "CIMGuide", "position": 16.5, "orientation": "Vertical"}]},
            "elements": elements}})
}

/// Reads `doc`; when that works, the layout must clamp and serialise (which refuses NaN and infinities).
fn read_and_check(doc: &serde_json::Value) {
    let Ok(doc) = Document::from_json(&doc.to_string()) else { return };
    let mut report = Report::default();
    if let Ok(mut imp) = read_layout(&doc, &mut report) {
        imp.layout.clamp();
        let text = serde_json::to_string(&imp.layout);
        assert!(text.is_ok(), "{:?}", text.err());
        let back = serde_json::from_str::<crate::layout::Layout>(&text.unwrap_or_default());
        assert!(back.is_ok(), "{:?}", back.err());
    }
}

#[test]
fn hostile_layouts_never_panic() {
    let base = hostile_base();
    // The fixture itself reads whole.
    let mut report = Report::default();
    let imp = read_layout(&Document::from_json(&base.to_string()).unwrap(), &mut report).unwrap();
    let has = |f: fn(&ElementKind) -> bool| imp.layout.elements.iter().any(|e| f(&e.kind));
    assert!(has(|k| matches!(k, ElementKind::MapFrame { .. })));
    assert!(has(|k| matches!(k, ElementKind::Group { .. })));
    assert!(has(|k| matches!(k, ElementKind::Text { .. })));
    assert!(has(|k| matches!(k, ElementKind::Picture { .. })));
    assert!(has(|k| matches!(k, ElementKind::Legend { .. })));
    assert!(has(|k| matches!(k, ElementKind::ScaleBar { .. })));
    assert!(has(|k| matches!(k, ElementKind::NorthArrow { .. })));
    read_and_check(&base);

    // Every node of the fixture, replaced one at a time by each junk value suited to it.
    let mut paths = Vec::new();
    json_paths(&base, &mut Vec::new(), &mut paths);
    let mut runs = 0;
    for path in &paths {
        let mut probe = base.clone();
        let Some(here) = node_at(&mut probe, path) else { continue };
        for junk in replacements(here) {
            let mut doc = base.clone();
            if let Some(node) = node_at(&mut doc, path) {
                *node = junk;
            }
            read_and_check(&doc);
            runs += 1;
        }
    }
    assert!(runs >= 200, "{runs} mutations");

    // Then pairs of mutations picked by a fixed-seed generator.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |n: usize| {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) as usize) % n.max(1)
    };
    for _ in 0..300 {
        let mut doc = base.clone();
        for _ in 0..2 {
            let Some(path) = paths.get(next(paths.len())) else { continue };
            let Some(here) = node_at(&mut doc, path) else { continue };
            let options = replacements(here);
            if let Some(junk) = options.get(next(options.len())) {
                *here = junk.clone();
            }
        }
        read_and_check(&doc);
    }
}

// ---- embedded maps ----

const LANDGRID: &str = r"G:\JERICHO_SHAPEFILES\OTLS-PLSS\LANDGRID TOBIN";

fn standard_connection(factory: &str, connection: &str, dataset: &str) -> serde_json::Value {
    json!({"type": "CIMStandardDataConnection", "workspaceConnectionString": connection, "workspaceFactory": factory,
        "dataset": dataset, "datasetType": "esriDTFeatureClass"})
}

fn feature_layer(name: &str, connection: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut l = json!({"type": "CIMFeatureLayer", "name": name, "uRI": format!("CIMPATH=new_detail_map/{}.json", name.to_lowercase()),
        "visibility": true, "featureTable": {"type": "CIMFeatureTable", "displayField": "NAME", "dataConnection": connection}});
    if let (Some(o), Some(e)) = (l.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    l
}

fn simple_renderer(symbol: serde_json::Value) -> serde_json::Value {
    json!({"type": "CIMSimpleRenderer", "symbol": {"type": "CIMSymbolReference", "symbol": symbol}})
}

fn symbol_ref(symbol: serde_json::Value) -> serde_json::Value {
    json!({"type": "CIMSymbolReference", "symbol": symbol})
}

/// A document embedding `layers` and a map "NEW DETAIL MAP" that lists the ones named in `listed`.
fn map_document(layers: &[serde_json::Value], listed: &[&str]) -> Document {
    let doc = json!({"type": "CIMLayoutDocument", "version": "3.7.0", "layerDefinitions": layers,
        "mapDefinitions": [{"type": "CIMMap", "name": "NEW DETAIL MAP", "uRI": "CIMPATH=map/new_detail_map.json", "layers": listed,
            "spatialReference": {"wkid": 4326, "latestWkid": 4326},
            "defaultExtent": {"xmin": -125.0, "ymin": 24.0, "xmax": -66.0, "ymax": 50.0, "spatialReference": {"wkid": 4326}}}]});
    Document::from_json(&doc.to_string()).unwrap()
}

fn read_new_detail_map(doc: &Document) -> (MapImport, Report) {
    let mut report = Report::default();
    let map = read_map(doc, "CIMPATH=map/new_detail_map.json", &mut report).unwrap();
    (map, report)
}

#[test]
fn an_embedded_shapefile_map_reads_its_layers_and_styles() {
    use crate::symbology::Renderer;
    let states_symbol = polygon_symbol(vec![
        solid_stroke(rgb(52, 52, 52), 1.0),
        json!({"type": "CIMSolidFill", "enable": true, "color": {"type": "CIMRGBColor", "values": [130, 130, 130, 0]}}),
    ]);
    let halo = symbol_ref(polygon_symbol(vec![solid_fill(rgb(255, 255, 255))]));
    let label_symbol = symbol_ref(text_symbol(json!({"fontFamilyName": "Yu Gothic UI", "fontStyleName": "Regular", "height": 10,
        "haloSize": 1, "haloSymbol": halo})));
    let states = feature_layer(
        "TAA_US_States",
        standard_connection("Shapefile", &format!("DATABASE={LANDGRID}"), "TAA_US_States.shp"),
        json!({"transparency": 30, "renderer": simple_renderer(states_symbol), "labelVisibility": true,
            "labelClasses": [{"type": "CIMLabelClass", "expression": "$feature.STATE_NAME", "expressionEngine": "Arcade",
                "visibility": true, "textSymbol": label_symbol, "name": "Class 1"}]}),
    );
    let counties = feature_layer(
        "Counties",
        standard_connection("Shapefile", &format!("DATABASE={LANDGRID}"), "Counties.shp"),
        json!({"visibility": false, "minScale": 5000000.0, "renderer": simple_renderer(polygon_symbol(vec![solid_fill(rgb(200, 100, 0))]))}),
    );
    let doc = map_document(&[states, counties], &["CIMPATH=new_detail_map/taa_us_states.json", "CIMPATH=new_detail_map/counties.json"]);
    let (map, report) = read_new_detail_map(&doc);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    assert_eq!(map.name, "NEW DETAIL MAP");
    assert_eq!(map.wkid, Some(4326));
    assert_eq!(map.extent, Some([-125.0, 24.0, -66.0, 50.0]));
    let names: Vec<&str> = map.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["TAA_US_States", "Counties"]);
    assert_eq!(map.layers.iter().map(|l| l.visible).collect::<Vec<_>>(), [true, false]);

    let s = &map.layers[0];
    assert_eq!(s.path.as_deref(), Some(r"G:\JERICHO_SHAPEFILES\OTLS-PLSS\LANDGRID TOBIN\TAA_US_States.shp"));
    assert_eq!(s.table, None);
    assert_eq!(s.origin, r"Shapefile TAA_US_States.shp in G:\JERICHO_SHAPEFILES\OTLS-PLSS\LANDGRID TOBIN");
    let Some(Renderer::Single(sym)) = &s.style.renderer else { panic!("{:?}", s.style.renderer) };
    assert_eq!(sym.stroke, Rgba::rgb(52, 52, 52));
    assert!((sym.stroke_width - 1.333).abs() < 1e-3, "{}", sym.stroke_width);
    assert_eq!(sym.fill.a(), 0);
    assert_eq!(s.style.transparency, Some(30.0));
    assert_eq!(s.style.fill_layers, Some(Vec::new()));
    let labels = s.style.labels.as_ref().unwrap();
    assert!(labels.enabled);
    assert!(labels.classes.is_empty());
    let c = &labels.default;
    assert_eq!((c.field.as_str(), c.size, c.font.as_deref()), ("STATE_NAME", 10.0, Some("Yu Gothic UI")));
    assert_eq!((c.color, c.visible, c.bold, c.italic), (Rgba::rgb(10, 20, 30), true, false, false));
    assert!(c.halo);
    assert_eq!(c.halo_color, Some(Rgba::rgb(255, 255, 255)));
    assert!((c.halo_width.unwrap() - 1.333).abs() < 1e-3);
    // Only what the file sets is set.
    assert_eq!((s.style.min_scale, s.style.max_scale), (None, None));

    let c = &map.layers[1];
    assert_eq!(c.path.as_deref(), Some(r"G:\JERICHO_SHAPEFILES\OTLS-PLSS\LANDGRID TOBIN\Counties.shp"));
    assert_eq!(c.style.transparency, None);
    // Pro's scales are at 96 dpi, as QGIS's are.
    let min = c.style.min_scale.flatten().unwrap();
    assert!((min - 5_000_000.0 * crate::qgis::QGIS_SCALE).abs() < 1e-6, "{min}");
    assert_eq!(c.style.max_scale, Some(None));
    assert!(c.style.labels.is_none());
    let Some(Renderer::Single(sym)) = &c.style.renderer else { panic!() };
    assert_eq!((sym.fill, sym.stroke_width), (Rgba::rgb(200, 100, 0), 0.0));
}

#[test]
fn file_paths_follow_the_connections_folder_and_factory() {
    let layer = |name: &str, factory: &str, connection: &str, dataset: &str| {
        feature_layer(name, standard_connection(factory, connection, dataset), json!({}))
    };
    let doc = map_document(
        &[
            layer("A", "Shapefile", "DATABASE=/data/shp;OTHER=1", "a.shp"),
            layer("B", "FileGDB", "database=C:\\data\\parcels.gdb", "Parcels"),
            layer("C", "Shapefile", "DATABASE=D:\\folder\\", "c"),
            layer("D", "SDE", "SERVER=x;DATABASE=y", "d"),
            layer("E", "Shapefile", "OTHER=1", "e.shp"),
        ],
        &[
            "CIMPATH=new_detail_map/a.json",
            "CIMPATH=new_detail_map/b.json",
            "CIMPATH=new_detail_map/c.json",
            "CIMPATH=new_detail_map/d.json",
            "CIMPATH=new_detail_map/e.json",
        ],
    );
    let (map, report) = read_new_detail_map(&doc);
    let got: Vec<(Option<&str>, Option<&str>)> = map.layers.iter().map(|l| (l.path.as_deref(), l.table.as_deref())).collect();
    assert_eq!(
        got,
        [
            (Some("/data/shp/a.shp"), None),
            (Some(r"C:\data\parcels.gdb"), Some("Parcels")),
            (Some(r"D:\folder\c.shp"), None),
            (None, None),
            (None, None),
        ]
    );
    assert_eq!(map.layers[1].origin, r"FileGDB Parcels in C:\data\parcels.gdb");
    let n = notes(&report);
    assert!(n.iter().any(|t| t.starts_with("D: ") && t.contains("SDE")), "{n:?}");
    assert!(n.iter().any(|t| t.starts_with("E: ")), "{n:?}");
}

fn unique_class(label: &str, values: &[&str], symbol: serde_json::Value) -> serde_json::Value {
    let values: Vec<_> = values.iter().map(|v| json!({"type": "CIMUniqueValue", "fieldValues": [v]})).collect();
    json!({"type": "CIMUniqueValueClass", "label": label, "patch": "Default", "visible": true, "symbol": symbol_ref(symbol), "values": values})
}

fn unique_renderer(fields: &[&str], classes: Vec<serde_json::Value>, default: Option<serde_json::Value>) -> serde_json::Value {
    let mut r = json!({"type": "CIMUniqueValueRenderer", "fields": fields, "useDefaultSymbol": default.is_some(),
        "groups": [{"type": "CIMUniqueValueGroup", "heading": fields.first(), "classes": classes}]});
    if let (Some(o), Some(d)) = (r.as_object_mut(), default) {
        o.insert("defaultSymbol".into(), symbol_ref(d));
    }
    r
}

#[test]
fn unique_value_renderers_read_their_classes() {
    use crate::symbology::Renderer;
    let red = polygon_symbol(vec![solid_stroke(rgb(0, 0, 0), 0.5), solid_fill(rgb(255, 0, 0))]);
    let blue = polygon_symbol(vec![solid_fill(rgb(0, 0, 255))]);
    let grey = polygon_symbol(vec![solid_fill(rgb(128, 128, 128))]);
    let mut hidden = unique_class("Hidden", &["H"], blue.clone());
    hidden["visible"] = json!(false);
    let mut zones = feature_layer(
        "Zones",
        standard_connection("FileGDB", r"DATABASE=C:\data\zones.gdb", "Zones"),
        json!({"renderer": unique_renderer(
            &["ZONE"],
            vec![
                unique_class("Residential", &["R"], red),
                unique_class("Business", &["B1", "B2"], blue.clone()),
                unique_class("C", &["C"], grey.clone()),
                hidden,
            ],
            Some(grey.clone()),
        )}),
    );
    zones["renderer"]["defaultLabel"] = json!(" Everything else ");
    // No default symbol: the rest is not drawn. Two fields: the first.
    let mut two = feature_layer(
        "Two",
        standard_connection("FileGDB", r"DATABASE=C:\data\zones.gdb", "Two"),
        json!({"renderer": unique_renderer(&["A", "B"], vec![unique_class("x", &["1"], blue)], None)}),
    );
    two["renderer"]["defaultLabel"] = json!("<all other values>");
    // A line's classes with stacked strokes keep them.
    let road = |w: f64, c: serde_json::Value| solid_stroke(c, w);
    let roads = feature_layer(
        "Roads",
        standard_connection("FileGDB", r"DATABASE=C:\data\zones.gdb", "Roads"),
        json!({"renderer": unique_renderer(
            &["KIND"],
            vec![
                unique_class("Highway", &["H"], json!({"type": "CIMLineSymbol", "symbolLayers": [road(6.0, rgb(0, 0, 0)), road(4.0, rgb(255, 200, 0))]})),
                unique_class("Street", &["S"], json!({"type": "CIMLineSymbol", "symbolLayers": [road(1.0, rgb(90, 90, 90))]})),
            ],
            None,
        )}),
    );
    let uris = ["CIMPATH=new_detail_map/zones.json", "CIMPATH=new_detail_map/two.json", "CIMPATH=new_detail_map/roads.json"];
    let (map, report) = read_new_detail_map(&map_document(&[zones, two, roads], &uris));

    let Some(Renderer::Unique { field, classes, other, other_label, class_labels, class_strokes, class_fills, .. }) =
        &map.layers[0].style.renderer
    else {
        panic!("{:?}", map.layers[0].style.renderer)
    };
    assert_eq!(field, "ZONE");
    let values: Vec<&str> = classes.iter().map(|(v, _)| v.as_str()).collect();
    assert_eq!(values, ["R", "B1", "B2", "C", "H"]);
    assert_eq!(classes[0].1.fill, Rgba::rgb(255, 0, 0));
    assert_eq!(classes[0].1.stroke, Rgba::rgb(0, 0, 0));
    assert_eq!(classes[1].1.fill, Rgba::rgb(0, 0, 255));
    assert_eq!(classes[4].1.fill.a(), 0, "a class switched off draws nothing");
    assert_eq!(other.fill, Rgba::rgb(128, 128, 128));
    assert_eq!(other_label.as_deref(), Some("Everything else"));
    // A label only where it isn't the value.
    assert_eq!(class_labels.get("R").map(String::as_str), Some("Residential"));
    assert_eq!(class_labels.get("B2").map(String::as_str), Some("Business"));
    assert!(!class_labels.contains_key("C"));
    assert!(class_strokes.is_empty() && class_fills.is_empty());

    let Some(Renderer::Unique { field, other, other_label, .. }) = &map.layers[1].style.renderer else { panic!() };
    assert_eq!(field, "A");
    assert_eq!(*other_label, None, "the usual label is not kept");
    assert_eq!(other.fill.a(), 0);
    assert_eq!(other.stroke.a(), 0);
    assert!(notes(&report).iter().any(|n| n.starts_with("Two: ") && n.contains("2 fields")), "{:?}", notes(&report));

    let r = &map.layers[2].style;
    let Some(Renderer::Unique { class_strokes, .. }) = &r.renderer else { panic!() };
    assert_eq!(class_strokes.get("H").map(Vec::len), Some(2));
    assert!(!class_strokes.contains_key("S"), "a plain stroke is the symbol's own");
}

#[test]
fn label_classes_read_their_expressions_text_and_ranges() {
    let label = |expression: serde_json::Value, extra: serde_json::Value| {
        let mut c = json!({"type": "CIMLabelClass", "visibility": true, "textSymbol": symbol_ref(text_symbol(json!({"height": 12})))});
        if let (Some(o), Some(e)) = (c.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        if let Some(o) = c.as_object_mut() {
            o.insert("expression".into(), expression);
        }
        c
    };
    let classes = vec![
        label(json!("$feature[\"NAME\"]"), json!({"name": "Names"})),
        label(json!("[POP]"), json!({"minimumScale": 100000.0, "maximumScale": 5000.0})),
        label(json!("!CODE!"), json!({"visibility": false})),
        label(json!("ID"), json!({})),
        label(json!("$feature.FIRST + \" \" + $feature.SECOND"), json!({})),
        label(json!("\"no field here\""), json!({})),
    ];
    let layer = feature_layer(
        "Places",
        standard_connection("Shapefile", "DATABASE=/d", "p.shp"),
        json!({"labelVisibility": false, "labelClasses": classes}),
    );
    let (map, report) = read_new_detail_map(&map_document(&[layer], &["CIMPATH=new_detail_map/places.json"]));
    let labels = map.layers[0].style.labels.as_ref().unwrap();
    assert!(!labels.enabled);
    let fields: Vec<&str> = labels.classes.iter().map(|c| c.field.as_str()).collect();
    assert_eq!(fields, ["NAME", "POP", "CODE", "ID", "FIRST"]);
    assert_eq!(labels.classes[0].name, "Names");
    assert_eq!(labels.classes[0].size, 12.0);
    let (min, max) = (labels.classes[1].min_scale.unwrap(), labels.classes[1].max_scale.unwrap());
    assert!((min - 100_000.0 * crate::qgis::QGIS_SCALE).abs() < 1e-6 && (max - 5000.0 * crate::qgis::QGIS_SCALE).abs() < 1e-6);
    assert_eq!((labels.classes[0].min_scale, labels.classes[0].max_scale), (None, None));
    assert!(!labels.classes[2].visible);
    // Several classes: the default is a hidden copy of the first.
    assert_eq!((labels.default.name.as_str(), labels.default.visible, labels.default.field.as_str()), ("Default", false, "NAME"));
    assert!(!labels.classes[0].halo);
    let n = notes(&report);
    assert!(n.iter().any(|t| t.contains("label expression $feature.FIRST") && t.contains("drawn as FIRST")), "{n:?}");
    assert!(n.iter().any(|t| t.starts_with("Places: ") && t.contains("no field here")), "{n:?}");
}

#[test]
fn unsupported_layers_are_noted() {
    use crate::symbology::Renderer;
    let child = feature_layer("Child", standard_connection("Shapefile", "DATABASE=/d", "child.shp"), json!({}));
    let group = json!({"type": "CIMGroupLayer", "name": "Boundaries", "uRI": "CIMPATH=new_detail_map/boundaries.json", "visibility": false,
        "layers": ["CIMPATH=new_detail_map/child.json", "CIMPATH=new_detail_map/gone.json"]});
    let raster = json!({"type": "CIMRasterLayer", "name": "Imagery", "uRI": "CIMPATH=new_detail_map/imagery.json", "visibility": true});
    let surface = json!({"type": "CIMElevationSurfaceLayer", "name": "Ground", "uRI": "CIMPATH=new_detail_map/ground.json"});
    let classed = feature_layer(
        "Classed",
        standard_connection("Shapefile", "DATABASE=/d", "classed.shp"),
        json!({"renderer": {"type": "CIMClassBreaksRenderer", "field": "X"}}),
    );
    let sql = feature_layer(
        "Query",
        json!({"type": "CIMSqlQueryDataConnection", "workspaceConnectionString": "DATABASE=/d", "sqlQuery": "select 1"}),
        json!({}),
    );
    let doc = map_document(
        &[group, child, raster, surface, classed, sql],
        &[
            "CIMPATH=new_detail_map/boundaries.json",
            "CIMPATH=new_detail_map/imagery.json",
            "CIMPATH=new_detail_map/ground.json",
            "CIMPATH=new_detail_map/classed.json",
            "CIMPATH=new_detail_map/query.json",
            "CIMPATH=new_detail_map/missing.json",
        ],
    );
    let (map, report) = read_new_detail_map(&doc);
    let got: Vec<(&str, bool, bool)> = map.layers.iter().map(|l| (l.name.as_str(), l.visible, l.path.is_some())).collect();
    assert_eq!(
        got,
        [
            ("Child", false, true), // a group's layers are listed one by one, hidden with it
            ("gone", false, false),
            ("Imagery", true, false),
            ("Ground", true, false),
            ("Classed", true, true),
            ("Query", true, false),
            ("missing", true, false),
        ]
    );
    assert!(matches!(map.layers[4].style.renderer, Some(Renderer::Single(_))));
    let n = notes(&report);
    for expected in [
        "Boundaries: group Boundaries, its layers listed one by one",
        "Imagery: CIMRasterLayer layers, left out",
        "Ground: CIMElevationSurfaceLayer layers, left out",
        "Classed: CIMClassBreaksRenderer renderers, drawn with a default symbol",
    ] {
        assert!(n.iter().any(|t| t == expected), "{expected}\n{n:?}");
    }
    assert!(n.iter().any(|t| t.starts_with("Query: ") && t.contains("CIMSqlQueryDataConnection")), "{n:?}");
    assert!(n.iter().any(|t| t.contains("missing.json")), "{n:?}");
    assert!(n.iter().any(|t| t.contains("gone.json")), "{n:?}");

    // Not a map: an error, and no map at all.
    let mut report = Report::default();
    assert!(read_map(&doc, "CIMPATH=new_detail_map/child.json", &mut report).is_err());
    assert!(read_map(&doc, "CIMPATH=nothing.json", &mut report).is_err());
}

#[test]
fn point_and_line_symbols_become_symbols() {
    use crate::symbology::{Marker, Renderer};
    let marker = json!({"type": "CIMVectorMarker", "enable": true, "size": 9.0, "markerGraphics": [{"type": "CIMMarkerGraphic",
        "geometry": {"curveRings": [[[0.0, 0.0]]]}, "symbol": polygon_symbol(vec![solid_fill(rgb(0, 128, 0))])}]});
    let gif = json!({"type": "CIMPictureMarker", "enable": true, "size": 8.0, "url": "data:image/gif;base64,R0lGODlhAQABAAAAACw="});
    let wells = feature_layer(
        "Wells",
        standard_connection("Shapefile", "DATABASE=/d", "wells.shp"),
        json!({"renderer": simple_renderer(json!({"type": "CIMPointSymbol", "symbolLayers": [gif, marker]}))}),
    );
    let pipes = feature_layer(
        "Pipes",
        standard_connection("Shapefile", "DATABASE=/d", "pipes.shp"),
        json!({"renderer": simple_renderer(json!({"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(rgb(1, 2, 3), 2.0)]}))}),
    );
    let (map, report) =
        read_new_detail_map(&map_document(&[wells, pipes], &["CIMPATH=new_detail_map/wells.json", "CIMPATH=new_detail_map/pipes.json"]));
    let Some(Renderer::Single(w)) = &map.layers[0].style.renderer else { panic!() };
    // The first marker (the picture, a circle for now); the second is noted.
    assert_eq!((w.marker, w.size), (Marker::Circle, 8.0 * 96.0 / 72.0));
    let n = notes(&report);
    assert!(n.iter().any(|t| t == "Wells: a GIF or BMP marker, drawn as a circle"), "{n:?}");
    assert!(n.iter().any(|t| t.starts_with("Wells: ") && t.contains("first marker")), "{n:?}");
    let Some(Renderer::Single(p)) = &map.layers[1].style.renderer else { panic!() };
    assert_eq!((p.stroke, p.fill, p.stroke_width, p.size), (Rgba::rgb(1, 2, 3), Rgba::rgb(1, 2, 3), 2.0 * 96.0 / 72.0, 0.0));
    assert_eq!(map.layers[1].style.line_strokes, Some(Vec::new()), "a single plain stroke is the symbol's own");
}

fn hostile_map_base() -> serde_json::Value {
    let halo = symbol_ref(polygon_symbol(vec![solid_fill(rgb(255, 255, 255))]));
    let states = feature_layer(
        "TAA_US_States",
        standard_connection("Shapefile", &format!("DATABASE={LANDGRID}"), "TAA_US_States.shp"),
        json!({"transparency": 30, "minScale": 1e7, "maxScale": 1e3, "labelVisibility": true,
            "renderer": simple_renderer(polygon_symbol(vec![solid_stroke(rgb(52, 52, 52), 1.0), solid_fill(rgb(1, 2, 3))])),
            "labelClasses": [{"type": "CIMLabelClass", "expression": "$feature.STATE_NAME", "visibility": true, "minimumScale": 5e6,
                "textSymbol": symbol_ref(text_symbol(json!({"haloSize": 1, "haloSymbol": halo})))}]}),
    );
    let zones = feature_layer(
        "Zones",
        standard_connection("FileGDB", r"DATABASE=C:\z.gdb", "Zones"),
        json!({"renderer": unique_renderer(
            &["ZONE"],
            vec![unique_class("Residential", &["R"], polygon_symbol(vec![solid_fill(rgb(255, 0, 0))]))],
            Some(polygon_symbol(vec![solid_fill(rgb(9, 9, 9))])),
        )}),
    );
    // (A group that lists itself must not loop.)
    let group = json!({"type": "CIMGroupLayer", "name": "G", "uRI": "CIMPATH=new_detail_map/g.json",
        "layers": ["CIMPATH=new_detail_map/zones.json", "CIMPATH=new_detail_map/g.json"]});
    json!({"type": "CIMLayoutDocument", "version": "3.7.0", "layerDefinitions": [states, zones, group],
        "mapDefinitions": [{"type": "CIMMap", "name": "M", "uRI": "CIMPATH=map/m.json",
            "layers": ["CIMPATH=new_detail_map/taa_us_states.json", "CIMPATH=new_detail_map/g.json"],
            "spatialReference": {"wkid": 4326, "latestWkid": 4326},
            "defaultExtent": {"xmin": -125.0, "ymin": 24.0, "xmax": -66.0, "ymax": 50.0}}]})
}

/// Reads `doc`'s map; when that works, what it carries must serialise (which refuses NaN and infinities).
fn read_map_and_check(doc: &serde_json::Value) {
    let Ok(doc) = Document::from_json(&doc.to_string()) else { return };
    let mut report = Report::default();
    if let Ok(map) = read_map(&doc, "CIMPATH=map/m.json", &mut report) {
        assert!(map.layers.len() <= 500);
        for l in &map.layers {
            let json = serde_json::to_string(&l.style.renderer);
            assert!(json.is_ok(), "{:?}", json.err());
            let json = serde_json::to_string(&l.style.labels);
            assert!(json.is_ok(), "{:?}", json.err());
        }
    }
}

#[test]
fn hostile_maps_never_panic() {
    let base = hostile_map_base();
    let mut report = Report::default();
    let doc = Document::from_json(&base.to_string()).unwrap();
    let map = read_map(&doc, "CIMPATH=map/m.json", &mut report).unwrap();
    // The fixture reads whole: its layer, then the group's one (the group's own loop is cut).
    assert_eq!(map.layers.first().map(|l| l.name.as_str()), Some("TAA_US_States"));
    assert!(map.layers.iter().any(|l| l.name == "Zones"));
    read_map_and_check(&base);

    let mut paths = Vec::new();
    json_paths(&base, &mut Vec::new(), &mut paths);
    let mut runs = 0;
    for path in &paths {
        let mut probe = base.clone();
        let Some(here) = node_at(&mut probe, path) else { continue };
        for junk in replacements(here) {
            let mut doc = base.clone();
            if let Some(node) = node_at(&mut doc, path) {
                *node = junk;
            }
            read_map_and_check(&doc);
            runs += 1;
        }
    }
    assert!(runs >= 200, "{runs} mutations");
}

#[test]
fn maps_and_groups_are_bounded() {
    // 600 layers listed: 500 read.
    let layers: Vec<serde_json::Value> =
        (0..600).map(|i| feature_layer(&format!("L{i}"), standard_connection("Shapefile", "DATABASE=/d", "x.shp"), json!({}))).collect();
    let uris: Vec<String> = (0..600).map(|i| format!("CIMPATH=new_detail_map/l{i}.json")).collect();
    let uris: Vec<&str> = uris.iter().map(String::as_str).collect();
    let (map, report) = read_new_detail_map(&map_document(&layers, &uris));
    assert_eq!(map.layers.len(), 500);
    assert!(notes(&report).iter().any(|n| n.contains("500")), "{:?}", notes(&report));
    let past: Vec<_> = report.notes.iter().filter(|n| n.text.contains("layers past")).collect();
    assert_eq!(past.iter().map(|n| n.count).collect::<Vec<_>>(), [1], "noted once, not per layer");

    // Groups nested in groups end somewhere.
    let groups: Vec<serde_json::Value> = (0..100)
        .map(|i| {
            json!({"type": "CIMGroupLayer", "name": format!("G{i}"), "uRI": format!("CIMPATH=new_detail_map/g{i}.json"),
                "layers": [format!("CIMPATH=new_detail_map/g{}.json", i + 1)]})
        })
        .collect();
    let (map, _) = read_new_detail_map(&map_document(&groups, &["CIMPATH=new_detail_map/g0.json"]));
    assert!(map.layers.len() <= 500);
}

#[test]
fn unreadable_symbols_empty_renderers_and_odd_datasets_are_noted() {
    use crate::symbology::Renderer;
    let shp = |dataset: &str| standard_connection("Shapefile", "DATABASE=/d", dataset);
    let no_symbol = feature_layer("NoSymbol", shp("a.shp"), json!({"renderer": {"type": "CIMSimpleRenderer"}}));
    let typeless = feature_layer("Typeless", shp("b.shp"), json!({"renderer": simple_renderer(json!({"symbolLayers": []}))}));
    let empty = feature_layer("Empty", shp("c.shp"), json!({"renderer": unique_renderer(&["F"], vec![], None)}));
    let up = feature_layer("Up", shp("../secret.shp"), json!({}));
    let deep = feature_layer("Deep", shp(r"sub\d.shp"), json!({}));
    let uris = ["nosymbol", "typeless", "empty", "up", "deep"].map(|n| format!("CIMPATH=new_detail_map/{n}.json"));
    let uris: Vec<&str> = uris.iter().map(String::as_str).collect();
    let (map, report) = read_new_detail_map(&map_document(&[no_symbol, typeless, empty, up, deep], &uris));
    // A symbol that can't be read still leaves a layer that draws.
    assert!(matches!(map.layers[0].style.renderer, Some(Renderer::Single(_))));
    assert!(matches!(map.layers[1].style.renderer, Some(Renderer::Single(_))));
    assert!(matches!(map.layers[2].style.renderer, Some(Renderer::Unique { .. })));
    assert_eq!((map.layers[3].path.as_deref(), map.layers[4].path.as_deref()), (None, None));
    let n = notes(&report);
    for expected in [
        "NoSymbol: simple renderers with no symbol, drawn with a default symbol",
        "Typeless: symbols with no type, left out",
        "Empty: unique value renderers with no class to read and no default symbol, nothing drawn",
    ] {
        assert!(
            n.iter().any(|t| t == expected),
            "{expected}
{n:?}"
        );
    }
    assert!(n.iter().any(|t| t.starts_with("Up: ") && t.contains("secret")), "{n:?}");
    assert!(n.iter().any(|t| t.starts_with("Deep: ")), "{n:?}");
}

/// `text` with a newline after every `every` characters, as Pro wraps the base64 it writes.
fn wrapped(text: &str, every: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars.chunks(every).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("\n")
}

#[test]
fn picture_markers_read_base64_wrapped_by_newlines() {
    use crate::symbology::Marker;
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let gif = "R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";
    let sym = json!({"type": "CIMPointSymbol", "symbolLayers": [
        {"type": "CIMPictureMarker", "enable": true, "size": 36.0, "url": format!("data:image/png;base64,{}", wrapped(png, 76))},
        {"type": "CIMPictureMarker", "enable": true, "size": 36.0, "url": format!("data:image/gif;base64,{}\r\n", wrapped(gif, 20))},
    ]});
    let mut report = Report::default();
    let p = symbol::point(&sym, "North arrow", &mut report);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    match p.symbols[0].marker {
        Marker::Image(image) => assert_eq!(image.as_str(), png),
        other => panic!("{other:?}"),
    }
    // The GIF is handed on as a clean data: URL.
    assert_eq!(p.pending_pictures, vec![(1, format!("data:image/gif;base64,{gif}"))]);
}

#[test]
fn layout_pictures_read_base64_wrapped_by_newlines() {
    use crate::layout::ElementKind::Picture;
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let picture = |name: &str, extra: serde_json::Value| {
        let mut g = extra;
        g["type"] = json!("CIMPictureGraphic");
        g["box"] = json!({"xmin": 0.76, "ymin": 9.0, "xmax": 1.83, "ymax": 9.41});
        json!({"type": "CIMGraphicElement", "name": name, "anchor": "CenterPoint", "graphic": g})
    };
    let doc = json!({"type": "CIMLayoutDocument", "version": "3.7.0",
    "binaryReferences": [
        {"type": "CIMBinaryReference", "uRI": "CIMPATH=Pictures/a.png", "data": wrapped(png, 76)},
        {"type": "CIMBinaryReference", "uRI": "CIMPATH=Pictures/b.png", "data": format!("data:image/png;base64,{}", wrapped(png, 76))},
    ],
    "layoutDefinition": {"type": "CIMLayout", "name": "P",
        "page": {"type": "CIMPage", "width": 17, "height": 11, "units": {"uwkid": 109008}, "guides": []},
        "elements": [
            picture("INLINE", json!({"sourceURL": format!("data:image/png;base64,{}", wrapped(png, 76))})),
            picture("EMBEDDED", json!({"referenceURI": "CIMPATH=Pictures/a.png"})),
            picture("EMBEDDED URL", json!({"referenceURI": "CIMPATH=Pictures/b.png"})),
        ]}});
    let mut report = Report::default();
    let imp = read_layout(&Document::from_json(&doc.to_string()).unwrap(), &mut report).unwrap();
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    assert_eq!(imp.layout.elements.len(), 3);
    for e in &imp.layout.elements {
        assert!(matches!(&e.kind, Picture { image, .. } if image == png), "{}: {:?}", e.name, e.kind);
    }
}

#[test]
fn an_extent_indicator_drawn_with_a_polygon_symbol_takes_its_outline_without_a_note() {
    // Pro's indicators are polygon symbols: an outline and a fill. RustGIS draws the outline.
    let sym =
        json!({"type": "CIMSymbolReference", "symbol": polygon_symbol(vec![solid_stroke(rgb(255, 0, 0), 1.5), solid_fill(rgb(0, 0, 0))])});
    let main = map_frame_element("MAIN MAP", (1.0, 1.0, 9.0, 8.0), json!({}));
    let mut small = map_frame_element("SMALL DETAIL MAP", (10.0, 1.0, 16.0, 4.0), json!({}));
    small["extentIndicators"] = json!([{"type": "CIMExtentIndicator", "sourceMapFrame": "MAIN MAP", "symbol": sym}]);
    let (imp, report) = read_with_maps(&[], &[main, small]);
    let main_id = frame_of(&imp, "MAIN MAP").0;
    let (_, ElementKind::MapFrame { extent_of, extent_color, extent_width, .. }) = frame_of(&imp, "SMALL DETAIL MAP") else { panic!() };
    assert_eq!((extent_of, extent_color), (Some(main_id), Rgba::rgb(255, 0, 0)));
    assert!(near(extent_width, 1.5 * 96.0 / 72.0, 1e-6), "{extent_width}");
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}
