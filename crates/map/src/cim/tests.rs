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
