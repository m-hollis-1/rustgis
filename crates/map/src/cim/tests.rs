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

    // point() takes the graphic's fill and stroke, and the size in 96ths; a shape it can't tell is drawn.
    let mut report = Report::default();
    let p = symbol::point(&json!({"type": "CIMPointSymbol", "symbolLayers": [square, seven]}), "Pin", &mut report);
    assert_eq!((p.symbols.len(), p.paths.len()), (2, 2));
    let s = p.symbols[0];
    assert_eq!((s.marker, s.fill, s.stroke), (Marker::Square, Rgba::rgb(200, 0, 0), Rgba::rgb(0, 0, 0)));
    assert!((s.size - 8.0).abs() < 1e-5 && (s.stroke_width - 0.5 * 96.0 / 72.0).abs() < 1e-5);
    assert!(matches!(p.symbols[1].marker, Marker::Path(_)), "{:?}", p.symbols[1].marker);
    assert!(p.paths.iter().all(Option::is_some));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

/// A vector marker of `size` points drawing `geometry` in a 10 × 10 frame, filled `fill`.
fn vector_marker(size: f64, geometry: serde_json::Value, fill: serde_json::Value) -> serde_json::Value {
    json!({"type": "CIMVectorMarker", "enable": true, "size": size, "frame": {"xmin": -5, "ymin": -5, "xmax": 5, "ymax": 5},
        "markerGraphics": [{"type": "CIMMarkerGraphic", "geometry": geometry, "symbol": polygon_symbol(vec![solid_fill(fill)])}]})
}

fn square_ring() -> serde_json::Value {
    json!({"rings": [[[-5.0, -5.0], [-5.0, 5.0], [5.0, 5.0], [5.0, -5.0], [-5.0, -5.0]]]})
}

fn circle_rings() -> serde_json::Value {
    json!({"curveRings": [[[5.0, 0.0], {"a": [[5.0, 0.0], [0.0, 0.0], 0, 1]}]]})
}

fn path_of(m: crate::symbology::Marker) -> &'static crate::symbology::PathShape {
    match m {
        crate::symbology::Marker::Path(p) => p.shape(),
        other => panic!("not a path: {other:?}"),
    }
}

#[test]
fn a_vector_marker_rustgis_lacks_becomes_a_path() {
    let hexagon: Vec<[f64; 2]> = (0..=6)
        .map(|i| {
            let a = f64::from(i) * std::f64::consts::PI / 3.0;
            [5.0 * a.cos(), 5.0 * a.sin()]
        })
        .collect();
    let sym = json!({"type": "CIMPointSymbol", "symbolLayers": [vector_marker(8.0, json!({"rings": [hexagon]}), rgb(255, 0, 0))]});
    let mut report = Report::default();
    let p = symbol::point(&sym, "Hex", &mut report);
    let shape = path_of(p.symbols[0].marker);
    assert_eq!(shape.parts.len(), 1);
    let part = &shape.parts[0];
    assert!(part.closed);
    assert_eq!(part.points.len(), 7);
    assert!(part.points.iter().all(|[x, y]| x.abs() <= 0.5 + 1e-6 && y.abs() <= 0.5 + 1e-6), "{:?}", part.points);
    assert!((part.points[0][0] - 0.5).abs() < 1e-6 && part.points[0][1].abs() < 1e-6, "{:?}", part.points[0]);
    assert_eq!((part.fill, part.stroke), (Some(Rgba::rgb(255, 0, 0)), None));
    assert!((p.symbols[0].size - 8.0 * 96.0 / 72.0).abs() < 1e-5);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn a_rotated_square_becomes_a_turned_path() {
    let mut square = vector_marker(8.0, square_ring(), rgb(0, 0, 255));
    square["rotation"] = json!(30);
    let mut report = Report::default();
    let p = symbol::point(&json!({"type": "CIMPointSymbol", "symbolLayers": [square]}), "Turned", &mut report);
    let part = &path_of(p.symbols[0].marker).parts[0];
    assert_eq!(part.points.len(), 5);
    // (−0.5, −0.5) turned 30 degrees anticlockwise: no longer on the square's axes.
    let [x, y] = part.points[0];
    let (s, c) = 30f32.to_radians().sin_cos();
    assert!((x - (-0.5 * c + 0.5 * s)).abs() < 1e-5 && (y - (-0.5 * s - 0.5 * c)).abs() < 1e-5, "{x} {y}");
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn stacked_marker_layers_become_one_path_sized_by_the_largest() {
    let sym = json!({"type": "CIMPointSymbol", "symbolLayers": [
        vector_marker(12.0, circle_rings(), rgb(255, 255, 255)),
        vector_marker(6.0, square_ring(), rgb(0, 0, 0)),
    ]});
    let mut report = Report::default();
    let p = symbol::point(&sym, "Stack", &mut report);
    assert_eq!(p.symbols[0].marker, crate::symbology::Marker::Circle);
    let layers: Vec<_> = p.symbols.iter().copied().zip(p.paths).collect();
    let s = symbol::stacked(&layers, "Stack", &mut report);
    assert!((s.size - 16.0).abs() < 1e-5, "{}", s.size);
    let shape = path_of(s.marker);
    assert_eq!(shape.parts.len(), 2);
    let (circle, square) = (&shape.parts[0], &shape.parts[1]);
    assert_eq!((circle.points.len(), circle.fill), (32, Some(Rgba::rgb(255, 255, 255))));
    assert!(circle.points.iter().all(|[x, y]| (x.hypot(*y) - 0.5).abs() < 1e-5), "{:?}", circle.points);
    assert_eq!(square.fill, Some(Rgba::rgb(0, 0, 0)));
    assert!(square.points.iter().all(|[x, y]| x.abs() <= 0.25 + 1e-6 && y.abs() <= 0.25 + 1e-6), "{:?}", square.points);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    // Through a layer's renderer, the same.
    let marks =
        feature_layer("Marks", standard_connection("Shapefile", "DATABASE=/d", "marks.shp"), json!({"renderer": simple_renderer(sym)}));
    let (map, report) = read_new_detail_map(&map_document(&[marks], &["CIMPATH=new_detail_map/marks.json"]));
    let Some(crate::symbology::Renderer::Single(w)) = &nth(&map, 0).style.renderer else { panic!() };
    assert_eq!(path_of(w.marker).parts.len(), 2);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn a_stack_with_a_picture_keeps_its_first_marker_and_says_so() {
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let sym = json!({"type": "CIMPointSymbol", "symbolLayers": [
        {"type": "CIMPictureMarker", "enable": true, "size": 10.0, "url": png},
        vector_marker(6.0, square_ring(), rgb(0, 0, 0)),
    ]});
    let mut report = Report::default();
    let p = symbol::point(&sym, "Pic", &mut report);
    assert!(p.paths[0].is_none() && p.paths[1].is_some());
    let layers: Vec<_> = p.symbols.iter().copied().zip(p.paths).collect();
    let s = symbol::stacked(&layers, "Pic", &mut report);
    assert!(matches!(s.marker, crate::symbology::Marker::Image(_)), "{:?}", s.marker);
    let n = notes(&report);
    assert!(n.len() == 1 && n[0].starts_with("Pic: ") && n[0].contains("first marker"), "{n:?}");
}

/// A line symbol of a 1 pt black stroke under a red square marker (outlined 0.5 pt black) placed by `placement`.
fn marker_line(placement: serde_json::Value) -> serde_json::Value {
    let mut marker = vector_marker(6.0, square_ring(), rgb(255, 0, 0));
    marker["markerGraphics"][0]["symbol"] = polygon_symbol(vec![solid_stroke(rgb(0, 0, 0), 0.5), solid_fill(rgb(255, 0, 0))]);
    marker["markerPlacement"] = placement;
    json!({"type": "CIMLineSymbol", "symbolLayers": [solid_stroke(rgb(0, 0, 0), 1.0), marker]})
}

#[test]
fn markers_along_a_line_become_line_markers() {
    use crate::symbology::{LineMark, Marker, Placements};
    let mut report = Report::default();
    let end = symbol::line(
        &marker_line(json!({"type": "CIMMarkerPlacementAtExtremities", "extremityPlacement": "JustEnd", "angleToLine": true, "offset": 3})),
        "Flow",
        &mut report,
    );
    assert_eq!(end.strokes.len(), 2);
    // The line's own stroke stays the symbol's.
    assert_eq!(end.stroke, Some((Rgba::rgb(0, 0, 0), 96.0 / 72.0)));
    let st = &end.strokes[1];
    let m = st.markers.unwrap();
    assert_eq!((m.at, m.rotate, m.mark), (Placements::LAST, true, LineMark::Marker(Marker::Square)));
    assert!((m.size - 8.0).abs() < 1e-5);
    assert_eq!((st.color, m.outline), (Some(Rgba::rgb(255, 0, 0)), Some(Rgba::rgb(0, 0, 0))));
    assert!((st.width.unwrap() - 0.5 * 96.0 / 72.0).abs() < 1e-5 && (st.offset - 4.0).abs() < 1e-5);
    let both = symbol::line(&marker_line(json!({"type": "CIMMarkerPlacementAtExtremities"})), "Flow", &mut report);
    assert_eq!(both.strokes[1].markers.unwrap().at, Placements::FIRST.with(Placements::LAST));
    let middle =
        symbol::line(&marker_line(json!({"type": "CIMMarkerPlacementAtRatioPositions", "positionArray": [0.5]})), "Flow", &mut report);
    assert_eq!(middle.strokes[1].markers.unwrap().at, Placements::MIDDLE);
    let along = symbol::line(
        &marker_line(json!({"type": "CIMMarkerPlacementAlongLineSameSize", "placementTemplate": [20], "angleToLine": false})),
        "Flow",
        &mut report,
    );
    let m = along.strokes[1].markers.unwrap();
    assert_eq!((m.at, m.rotate), (Placements::INTERVAL, false));
    assert!((m.interval - 26.67).abs() < 1e-2, "{}", m.interval);
    let vertices = symbol::line(&marker_line(json!({"type": "CIMMarkerPlacementOnVertices"})), "Flow", &mut report);
    assert_eq!(vertices.strokes[1].markers.unwrap().at, Placements::VERTICES);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    // Ratios other than the middle are at intervals, noted.
    let ratios =
        symbol::line(&marker_line(json!({"type": "CIMMarkerPlacementAtRatioPositions", "positionArray": [0.25]})), "Flow", &mut report);
    assert_eq!(ratios.strokes[1].markers.unwrap().at, Placements::INTERVAL);
    assert_eq!(report.notes.len(), 1, "{:?}", notes(&report));

    // Through a layer's renderer, the markers are among its strokes.
    let flow = feature_layer(
        "Flow",
        standard_connection("Shapefile", "DATABASE=/d", "flow.shp"),
        json!({"renderer": simple_renderer(marker_line(
            json!({"type": "CIMMarkerPlacementAtExtremities", "extremityPlacement": "JustEnd", "angleToLine": true})
        ))}),
    );
    let (map, _) = read_new_detail_map(&map_document(&[flow], &["CIMPATH=new_detail_map/flow.json"]));
    let strokes = nth(&map, 0).style.line_strokes.as_ref().unwrap();
    assert_eq!(strokes.len(), 2);
    assert_eq!(strokes[1].markers.unwrap().at, Placements::LAST);
}

#[test]
fn dots_inside_polygons_become_a_dots_fill() {
    use crate::fills::FillPattern;
    use crate::symbology::Marker;
    let mut marker = vector_marker(4.0, circle_rings(), rgb(0, 128, 0));
    marker["markerPlacement"] = json!({"type": "CIMMarkerPlacementInsidePolygon", "stepX": 10, "stepY": 10, "gridType": "Fixed"});
    let sym = polygon_symbol(vec![solid_fill(rgb(255, 255, 255)), marker]);
    let mut report = Report::default();
    let p = symbol::polygon(&sym, "Park", &mut report);
    assert_eq!(p.fills.len(), 2);
    assert_eq!(p.fills[1].color, Some(Rgba::rgb(0, 128, 0)));
    let FillPattern::Dots { marker, size, spacing, .. } = p.fills[1].pattern else { panic!("{:?}", p.fills[1]) };
    assert_eq!(marker, Marker::Circle);
    assert!((size - 4.0 * 96.0 / 72.0).abs() < 1e-5);
    assert!(spacing.iter().all(|s| (s - 13.33).abs() < 1e-2), "{spacing:?}");
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn hostile_marker_graphics_are_bounded() {
    use crate::symbology::{LineMark, MAX_PATH_PARTS, MAX_PATH_POINTS, Marker, PathShape};
    let long: Vec<[f64; 2]> = (0..5000).map(|i| [f64::from(i % 97) - 48.0, f64::from(i % 89) - 44.0]).collect();
    let graphic = |geometry: serde_json::Value| json!({"type": "CIMMarkerGraphic", "geometry": geometry, "symbol": polygon_symbol(vec![solid_fill(rgb(1, 2, 3))])});
    // (JSON has no NaN: serde_json writes one as null, which is what a damaged file holds.)
    let mut graphics = vec![
        graphic(json!({"rings": [long]})),
        graphic(json!({"rings": [[[f64::NAN, f64::NAN], [f64::NAN, 1.0], [2.0, f64::NAN]]]})),
        graphic(json!({"curveRings": [[[0, 0], {"a": "x"}, {"b": [[1, 1]]}, {"c": null}, [1e308, -1e308]]]})),
        graphic(json!({"paths": 5})),
        graphic(json!({"rings": [[[1, 2], [3]], "x", [[0, 0], [0, 0]]]})),
    ];
    graphics.extend((0..200).map(|i| graphic(json!({"paths": [[[0, 0], [i, 1]]]}))));
    let hostile = [
        json!({"type": "CIMVectorMarker", "size": 10, "rotation": 1e308, "offsetX": "x",
            "frame": {"xmin": -50, "ymin": -50, "xmax": 50, "ymax": 50}, "markerGraphics": graphics}),
        json!({"type": "CIMVectorMarker", "size": 0, "frame": {"xmin": 0, "ymin": 0, "xmax": 0, "ymax": 0}, "markerGraphics": graphics}),
        json!({"type": "CIMVectorMarker", "size": -3, "markerGraphics": [graphic(json!({"rings": [[[0, 0], [0, 0], [0, 0]]]}))]}),
        json!({"type": "CIMVectorMarker", "markerGraphics": "x"}),
    ];
    let bounded = |shape: &PathShape| {
        assert!(shape.parts.len() <= MAX_PATH_PARTS);
        for part in &shape.parts {
            assert!(part.points.len() <= MAX_PATH_POINTS);
            assert!(part.points.iter().flatten().all(|v| v.is_finite()), "{:?}", part.points);
        }
    };
    let check = |m: Marker| {
        if let Marker::Path(p) = m {
            bounded(p.shape());
        }
    };
    // The long ring is cut to the most points kept, and the graphics to the most parts (two of the first
    // 64 draw nothing), noted.
    let mut report = Report::default();
    let shape = symbol::path_marker(&hostile[0], 10.0, "H", &mut report).unwrap();
    assert_eq!((shape.parts.len(), shape.parts[0].points.len()), (MAX_PATH_PARTS - 2, MAX_PATH_POINTS));
    bounded(&shape);
    assert!(notes(&report).iter().any(|n| n.contains("cut short")), "{:?}", notes(&report));
    for marker in &hostile {
        let mut report = Report::default();
        if let Some(shape) = symbol::path_marker(marker, 10.0, "H", &mut report) {
            bounded(&shape);
        }
        let point = json!({"type": "CIMPointSymbol", "symbolLayers": [marker, marker, {"type": "CIMCharacterMarker"}]});
        let p = symbol::point(&point, "H", &mut report);
        p.symbols.iter().for_each(|s| check(s.marker));
        let layers: Vec<_> = p.symbols.iter().copied().zip(p.paths).collect();
        if !layers.is_empty() {
            check(symbol::stacked(&layers, "H", &mut report).marker);
        }
        for placement in [
            json!({"type": "CIMMarkerPlacementAlongLineSameSize", "placementTemplate": "x"}),
            json!({"type": "CIMMarkerPlacementAtRatioPositions", "positionArray": [f64::NAN]}),
            json!({"type": "CIMMarkerPlacementInsidePolygon", "stepX": -1, "stepY": 1e308, "offsetX": "x"}),
            json!({"type": "CIMMarkerPlacementSomethingNew"}),
            json!(7),
        ] {
            let mut m = marker.clone();
            m["markerPlacement"] = placement;
            let line = symbol::line(&json!({"type": "CIMLineSymbol", "symbolLayers": [m]}), "H", &mut report);
            for s in line.strokes.iter().filter_map(|s| s.markers) {
                if let LineMark::Marker(m) = s.mark {
                    check(m);
                }
            }
            let poly = symbol::polygon(&json!({"type": "CIMPolygonSymbol", "symbolLayers": [m]}), "H", &mut report);
            for f in &poly.fills {
                if let crate::fills::FillPattern::Dots { marker, .. } = f.pattern {
                    check(marker);
                }
            }
        }
    }
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
        picture("COMPANY LOGO", json!({"referenceURI": "CIMPATH=Pictures/logo.png", "sourceURL": r"D:\logos\Company Logo.png"})),
        picture("FILE", json!({"sourceURL": r"D:\logos\Other.png"})),
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
    // Every picture is left for the engine to check and convert, embedded ones included: the
    // elements are empty until it does.
    assert!(
        matches!(&els[0].kind, Picture { image, source } if image.is_empty() && source.as_deref() == Some(r"D:\logos\Company Logo.png"))
    );
    assert!(matches!(&els[1].kind, Picture { image, source } if image.is_empty() && source.as_deref() == Some(r"D:\logos\Other.png")));
    assert!(matches!(&els[2].kind, Picture { image, source } if image.is_empty() && source.is_none()));
    assert_eq!(
        imp.pictures,
        vec![
            (els[0].id, PictureSource::Embedded("iVBORw0KGgo=".into())),
            (els[1].id, PictureSource::Path(r"D:\logos\Other.png".into())),
            (els[2].id, PictureSource::Embedded("AAAA".into())),
        ]
    );
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
        "graphic": {"type": "CIMPictureGraphic", "referenceURI": "CIMPATH=Pictures/logo.png", "sourceURL": r"D:\logos\Logo.png",
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

const LANDGRID: &str = r"D:\data\landgrid";

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
    map_document_with(layers, listed, json!({}), json!({}))
}

/// `map_document`, with `map` added to the map's keys and `root` to the document's.
fn map_document_with(layers: &[serde_json::Value], listed: &[&str], map: serde_json::Value, root: serde_json::Value) -> Document {
    let mut m = json!({"type": "CIMMap", "name": "NEW DETAIL MAP", "uRI": "CIMPATH=map/new_detail_map.json", "layers": listed,
        "spatialReference": {"wkid": 4326, "latestWkid": 4326},
        "defaultExtent": {"xmin": -125.0, "ymin": 24.0, "xmax": -66.0, "ymax": 50.0, "spatialReference": {"wkid": 4326}}});
    if let (Some(o), Some(e)) = (m.as_object_mut(), map.as_object()) {
        o.extend(e.clone());
    }
    let mut doc = json!({"type": "CIMLayoutDocument", "version": "3.7.0", "layerDefinitions": layers, "mapDefinitions": [m]});
    if let (Some(o), Some(e)) = (doc.as_object_mut(), root.as_object()) {
        o.extend(e.clone());
    }
    Document::from_json(&doc.to_string()).unwrap()
}

/// A map's layers, its groups opened up, top first.
fn flat(map: &MapImport) -> Vec<&LayerImport> {
    fn walk<'a>(entries: &'a [Entry], out: &mut Vec<&'a LayerImport>) {
        for e in entries {
            match e {
                Entry::Layer(l) => out.push(l),
                Entry::Group(g) => walk(&g.children, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(&map.entries, &mut out);
    out
}

/// The `i`th of a map's layers, its groups opened up.
fn nth(map: &MapImport, i: usize) -> &LayerImport {
    flat(map)[i]
}

/// A layer's file and table, when it reads from a file.
fn file_of(l: &LayerImport) -> (Option<&str>, Option<&str>) {
    match &l.source {
        LayerSource::File { path, table } => (Some(path.as_str()), table.as_deref()),
        _ => (None, None),
    }
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
        "Sample_US_States",
        standard_connection("Shapefile", &format!("DATABASE={LANDGRID}"), "Sample_US_States.shp"),
        json!({"transparency": 30, "renderer": simple_renderer(states_symbol), "labelVisibility": true,
            "labelClasses": [{"type": "CIMLabelClass", "expression": "$feature.STATE_NAME", "expressionEngine": "Arcade",
                "visibility": true, "textSymbol": label_symbol, "name": "Class 1"}]}),
    );
    let counties = feature_layer(
        "Counties",
        standard_connection("Shapefile", &format!("DATABASE={LANDGRID}"), "Counties.shp"),
        json!({"visibility": false, "minScale": 5000000.0, "renderer": simple_renderer(polygon_symbol(vec![solid_fill(rgb(200, 100, 0))]))}),
    );
    let doc = map_document(&[states, counties], &["CIMPATH=new_detail_map/sample_us_states.json", "CIMPATH=new_detail_map/counties.json"]);
    let (map, report) = read_new_detail_map(&doc);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    assert_eq!(map.name, "NEW DETAIL MAP");
    assert_eq!(map.wkid, Some(4326));
    assert_eq!(map.extent, Some([-125.0, 24.0, -66.0, 50.0]));
    let names: Vec<&str> = flat(&map).into_iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Sample_US_States", "Counties"]);
    assert_eq!(flat(&map).into_iter().map(|l| l.visible).collect::<Vec<_>>(), [true, false]);

    assert!(map.entries.iter().all(|e| matches!(e, Entry::Layer(_))));
    assert!(map.tables.is_empty());
    assert_eq!(map.reference_scale, None);

    let s = nth(&map, 0);
    assert_eq!(s.source, LayerSource::File { path: r"D:\data\landgrid\Sample_US_States.shp".into(), table: None });
    assert_eq!(s.origin, r"Shapefile Sample_US_States.shp in D:\data\landgrid");
    assert_eq!((s.kind, s.expanded, s.scale_symbols, s.definition_query.as_deref()), (LayerKind::Feature, false, true, None));
    assert_eq!(s.style.blend, None);
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
    assert_eq!(c.halo_width, Some(1.0), "points, as RustGIS keeps them");
    // Only what the file sets is set.
    assert_eq!((s.style.min_scale, s.style.max_scale), (None, None));

    let c = nth(&map, 1);
    assert_eq!(file_of(c), (Some(r"D:\data\landgrid\Counties.shp"), None));
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
    let got: Vec<(Option<&str>, Option<&str>)> = flat(&map).into_iter().map(file_of).collect();
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
    assert_eq!(nth(&map, 1).origin, r"FileGDB Parcels in C:\data\parcels.gdb");
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
        &nth(&map, 0).style.renderer
    else {
        panic!("{:?}", nth(&map, 0).style.renderer)
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

    let Some(Renderer::Unique { field, other, other_label, .. }) = &nth(&map, 1).style.renderer else { panic!() };
    assert_eq!(field, "A");
    assert_eq!(*other_label, None, "the usual label is not kept");
    assert_eq!(other.fill.a(), 0);
    assert_eq!(other.stroke.a(), 0);
    assert!(notes(&report).iter().any(|n| n.starts_with("Two: ") && n.contains("2 fields")), "{:?}", notes(&report));

    let r = &nth(&map, 2).style;
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
        label(json!("Upper($feature.FIRST) + \" \" + $feature.SECOND"), json!({})),
        label(json!("\"no field here\""), json!({})),
    ];
    let layer = feature_layer(
        "Places",
        standard_connection("Shapefile", "DATABASE=/d", "p.shp"),
        json!({"labelVisibility": false, "labelClasses": classes}),
    );
    let (map, report) = read_new_detail_map(&map_document(&[layer], &["CIMPATH=new_detail_map/places.json"]));
    let labels = nth(&map, 0).style.labels.as_ref().unwrap();
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
    assert!(n.iter().any(|t| t.contains("label expression Upper($feature.FIRST)") && t.contains("drawn as FIRST")), "{n:?}");
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
        json!({"renderer": {"type": "CIMDotDensityRenderer", "attributes": []}}),
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
    // The group is kept, hidden on its own: its layers keep their own visibility.
    let [Entry::Group(g), ..] = map.entries.as_slice() else { panic!("{:?}", map.entries) };
    assert_eq!((g.name.as_str(), g.visible, g.children.len()), ("Boundaries", false, 2));
    let got: Vec<(&str, bool, bool, LayerKind)> =
        flat(&map).into_iter().map(|l| (l.name.as_str(), l.visible, l.source != LayerSource::Unopenable, l.kind)).collect();
    assert_eq!(
        got,
        [
            ("Child", true, true, LayerKind::Feature),
            ("gone", true, false, LayerKind::Other),
            ("Imagery", true, false, LayerKind::Raster), // a raster with no data connection
            ("Ground", true, false, LayerKind::Other),
            ("Classed", true, true, LayerKind::Feature),
            ("Query", true, false, LayerKind::Feature),
            ("missing", true, false, LayerKind::Other),
        ]
    );
    assert!(matches!(nth(&map, 4).style.renderer, Some(Renderer::Single(_))));
    let n = notes(&report);
    for expected in [
        "Imagery: a layer with no data connection, nothing to open",
        "Ground: CIMElevationSurfaceLayer layers, which RustGIS doesn't draw",
        "Classed: CIMDotDensityRenderer renderers, drawn with a default symbol",
        "Query: SQL query layers, which RustGIS can't open",
    ] {
        assert!(
            n.iter().any(|t| t == expected),
            "{expected}
{n:?}"
        );
    }
    assert!(!n.iter().any(|t| t.contains("one by one")), "{n:?}");
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
    let Some(Renderer::Single(w)) = &nth(&map, 0).style.renderer else { panic!() };
    // The first marker (the picture, a circle for now); the second is noted.
    assert_eq!((w.marker, w.size), (Marker::Circle, 8.0 * 96.0 / 72.0));
    let n = notes(&report);
    assert!(n.iter().any(|t| t == "Wells: a GIF or BMP marker, drawn as a circle"), "{n:?}");
    assert!(n.iter().any(|t| t.starts_with("Wells: ") && t.contains("first marker")), "{n:?}");
    let Some(Renderer::Single(p)) = &nth(&map, 1).style.renderer else { panic!() };
    assert_eq!((p.stroke, p.fill, p.stroke_width, p.size), (Rgba::rgb(1, 2, 3), Rgba::rgb(1, 2, 3), 2.0 * 96.0 / 72.0, 0.0));
    assert_eq!(nth(&map, 1).style.line_strokes, Some(Vec::new()), "a single plain stroke is the symbol's own");
}

fn hostile_map_base() -> serde_json::Value {
    let halo = symbol_ref(polygon_symbol(vec![solid_fill(rgb(255, 255, 255))]));
    let states = feature_layer(
        "Sample_US_States",
        standard_connection("Shapefile", &format!("DATABASE={LANDGRID}"), "Sample_US_States.shp"),
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
        ),
        "blendingMode": "Multiply", "expanded": true, "scaleSymbols": false, "enableDisplayFilters": true,
        "displayFilters": [{"type": "CIMDisplayFilter", "name": "f"}], "layerMasks": ["CIMPATH=new_detail_map/g.json"],
        "featureReduction": {"type": "CIMFeatureReductionCluster"}, "symbolLayerDrawing": {"useSymbolLayerDrawing": true},
        "charts": [{"type": "CIMChart"}], "popupInfo": {"type": "CIMPopupInfo"}, "timeFilter": {"type": "CIMTimeFilter"},
        "selectionSymbol": symbol_ref(polygon_symbol(vec![solid_fill(rgb(0, 255, 255))]))}),
    );
    let mut zones = zones;
    if let Some(t) = zones.pointer_mut("/featureTable") {
        t["definitionExpression"] = json!("ZONE = 'R'");
    }
    let gdb = r"DATABASE=..\data\z.gdb";
    let joined = feature_layer(
        "Joined",
        json!({"type": "CIMRelQueryTableDataConnection", "sourceTable": standard_connection("FileGDB", gdb, "parcels"),
            "destinationTable": standard_connection("FileGDB", gdb, "owners"), "primaryKey": "PID", "foreignKey": "PID"}),
        json!({}),
    );
    let sde = feature_layer(
        "Sde",
        standard_connection("SDE", "ENCRYPTED_PASSWORD=00ff;SERVER=s;DATABASE=d;USER=u;PASSWORD=p", "d.dbo.t"),
        json!({}),
    );
    let raster = json!({"type": "CIMRasterLayer", "name": "Photo", "uRI": "CIMPATH=new_detail_map/photo.json",
        "dataConnection": standard_connection("Raster", r"DATABASE=.\IMG", "photo.tif")});
    let tiles = json!({"type": "CIMTiledServiceLayer", "name": "Base", "uRI": "CIMPATH=new_detail_map/base.json",
        "serviceConnection": {"type": "CIMAGSServiceConnection", "objectType": "MapServer",
            "url": "https://example.org/arcgis/rest/services/Base/MapServer"}});
    let wms = json!({"type": "CIMWMSLayer", "name": "Wms", "uRI": "CIMPATH=new_detail_map/wms.json",
        "serviceConnection": {"type": "CIMWMSServiceConnection",
            "serverConnection": {"type": "CIMInternetServerConnection", "url": "https://example.org/wms"}},
        "subLayers": [{"type": "CIMWMSSubLayer", "name": "roads", "visibility": true}]});
    let vector = json!({"type": "CIMVectorTileLayer", "name": "Streets", "uRI": "CIMPATH=new_detail_map/streets.json",
        "dataConnection": {"type": "CIMVectorTileDataConnection", "uRI": "https://example.org/arcgis/rest/services/Streets/VectorTileServer"}});
    let annotation = json!({"type": "CIMAnnotationLayer", "name": "Notes", "uRI": "CIMPATH=new_detail_map/notes.json"});
    // (A group that lists itself must not loop.)
    let group = json!({"type": "CIMGroupLayer", "name": "G", "uRI": "CIMPATH=new_detail_map/g.json", "visibility": false, "expanded": true,
        "transparency": 20, "minScale": 1e6, "maxScale": 100,
        "layers": ["CIMPATH=new_detail_map/zones.json", "CIMPATH=new_detail_map/g.json", "CIMPATH=new_detail_map/joined.json"]});
    let table = json!({"type": "CIMStandaloneTable", "name": "Owners", "uRI": "CIMPATH=new_detail_map/owners.json",
        "dataConnection": standard_connection("FileGDB", gdb, "owners")});
    json!({"type": "CIMLayoutDocument", "version": "3.7.0",
        "layerDefinitions": [states, zones, group, joined, sde, raster, tiles, wms, vector, annotation], "tableDefinitions": [table],
        "mapDefinitions": [{"type": "CIMMap", "name": "M", "uRI": "CIMPATH=map/m.json", "referenceScale": 24000,
            "layers": ["CIMPATH=new_detail_map/sample_us_states.json", "CIMPATH=new_detail_map/g.json", "CIMPATH=new_detail_map/sde.json",
                "CIMPATH=new_detail_map/photo.json", "CIMPATH=new_detail_map/base.json", "CIMPATH=new_detail_map/wms.json",
                "CIMPATH=new_detail_map/streets.json", "CIMPATH=new_detail_map/notes.json"],
            "standaloneTables": ["CIMPATH=new_detail_map/owners.json"],
            "spatialReference": {"wkid": 4326, "latestWkid": 4326},
            "defaultExtent": {"xmin": -125.0, "ymin": 24.0, "xmax": -66.0, "ymax": 50.0}}]})
}

/// How many groups deep `entries` go.
fn group_depth(entries: &[Entry]) -> usize {
    entries.iter().map(|e| if let Entry::Group(g) = e { 1 + group_depth(&g.children) } else { 0 }).max().unwrap_or(0)
}

/// Reads `doc`'s map; when that works, what it carries must serialise (which refuses NaN and infinities).
fn read_map_and_check(doc: &serde_json::Value) {
    let Ok(doc) = Document::from_json(&doc.to_string()) else { return };
    let mut report = Report::default();
    if let Ok(map) = read_map(&doc, "CIMPATH=map/m.json", &mut report) {
        assert!(flat(&map).len() <= 500);
        assert!(group_depth(&map.entries) <= 8);
        assert!(map.tables.len() <= 500);
        assert!(map.reference_scale.is_none_or(f64::is_finite));
        for l in flat(&map) {
            let json = serde_json::to_string(&l.style.renderer);
            assert!(json.is_ok(), "{:?}", json.err());
            let json = serde_json::to_string(&l.style.labels);
            assert!(json.is_ok(), "{:?}", json.err());
            assert!(!l.origin.contains("PASSWORD") && !l.origin.contains("USER"), "{}", l.origin);
        }
        for t in &map.tables {
            assert!(!t.origin.contains("PASSWORD") && !t.origin.contains("USER"), "{}", t.origin);
        }
    }
}

#[test]
fn hostile_maps_never_panic() {
    let base = hostile_map_base();
    let mut report = Report::default();
    let doc = Document::from_json(&base.to_string()).unwrap();
    let map = read_map(&doc, "CIMPATH=map/m.json", &mut report).unwrap();
    // The fixture reads whole: its layer, then the group's (the group's own loop is cut), then the rest.
    assert_eq!(flat(&map).first().map(|l| l.name.as_str()), Some("Sample_US_States"));
    for name in ["Zones", "Joined", "Sde", "Photo", "Base", "Wms", "Streets", "Notes"] {
        assert!(flat(&map).into_iter().any(|l| l.name == name), "{name}");
    }
    assert!(matches!(map.entries.get(1), Some(Entry::Group(g)) if g.name == "G" && !g.visible));
    assert_eq!(map.tables.len(), 1);
    assert!(map.reference_scale.is_some());
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
    assert_eq!(flat(&map).len(), 500);
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
    let (map, report) = read_new_detail_map(&map_document(&groups, &["CIMPATH=new_detail_map/g0.json"]));
    assert!(flat(&map).is_empty());
    // Kept nested, as deep as groups are followed; the one past that is noted and left out.
    assert_eq!(group_depth(&map.entries), 8);
    assert!(notes(&report).iter().any(|n| n == "G8: groups inside groups past 8 levels"), "{:?}", notes(&report));
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
    assert!(matches!(nth(&map, 0).style.renderer, Some(Renderer::Single(_))));
    assert!(matches!(nth(&map, 1).style.renderer, Some(Renderer::Single(_))));
    assert!(matches!(nth(&map, 2).style.renderer, Some(Renderer::Unique { .. })));
    assert_eq!((&nth(&map, 3).source, &nth(&map, 4).source), (&LayerSource::Unopenable, &LayerSource::Unopenable));
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
    // Handed to the engine to check, the whitespace gone; the elements wait empty.
    for e in &imp.layout.elements {
        assert!(matches!(&e.kind, Picture { image, .. } if image.is_empty()), "{}: {:?}", e.name, e.kind);
        assert!(imp.pictures.contains(&(e.id, PictureSource::Embedded(png.to_string()))), "{}: {:?}", e.name, imp.pictures);
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

#[test]
fn a_frames_map_grids_are_noted_as_left_out() {
    let graticule = json!({"grids": [{"type": "CIMGraticule", "name": "Graticule", "visible": true}]});
    let (imp, report) = read_with_maps(&[], &[map_frame_element("GRID", (1.0, 1.0, 9.0, 8.0), graticule), main_frame()]);
    assert!(matches!(frame_of(&imp, "GRID").1, ElementKind::MapFrame { grid: None, .. }));
    assert_eq!(notes(&report), ["GRID: map grids, left out"]);
    // An empty list says nothing.
    let (_, report) = read_with_maps(&[], &[map_frame_element("NONE", (1.0, 1.0, 9.0, 8.0), json!({"grids": []}))]);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
}

#[test]
fn the_series_frame_drops_its_link_with_a_note() {
    let layers = [layer_def("Index", "CIMPATH=map/index.json")];
    let mut linked = main_frame();
    linked["autoCamera"] = json!({"source": "MapFrameLink", "mapFrame": "OVERVIEW", "autoCameraType": "Center"});
    let overview = map_frame_element("OVERVIEW", (10.0, 6.0, 16.0, 10.0), json!({}));
    let series = json!({"type": "CIMSpatialMapSeries", "indexLayerURI": "CIMPATH=map/index.json", "mapFrameName": "MAIN MAP"});
    let (imp, report) = read_surrounds(&layers, series, &[linked, overview]);
    let (main, kind) = frame_of(&imp, "MAIN MAP");
    assert!(matches!(kind, ElementKind::MapFrame { link: None, .. }), "{kind:?}");
    assert_eq!(imp.layout.series.as_ref().map(|s| s.frame), Some(main));
    let n = notes(&report);
    assert!(n.iter().any(|s| s == "MAIN MAP: drives the map series; its link is left out"), "{n:?}");
}

#[test]
fn a_link_cut_from_a_cycle_is_noted() {
    let linking = |name: &str, target: &str, at: f64| {
        let mut f = map_frame_element(name, (at, 1.0, at + 3.0, 5.0), json!({}));
        f["autoCamera"] = json!({"source": "MapFrameLink", "mapFrame": target, "autoCameraType": "Center"});
        f
    };
    let (imp, report) = read_with_maps(&[], &[linking("A", "B", 1.0), linking("B", "A", 5.0)]);
    let link = |n: &str| match frame_of(&imp, n).1 {
        ElementKind::MapFrame { link, .. } => link.map(|l| l.frame),
        _ => panic!(),
    };
    assert_eq!((link("A"), link("B")), (Some(frame_of(&imp, "B").0), None));
    assert_eq!(notes(&report), ["B: follows A, which follows it back; the link is left out"]);
}

#[test]
fn path_keys_ignore_the_case_of_the_cimpath_prefix() {
    assert_eq!(path_key("cimpath=Map/M.json"), "map/m.json");
    assert_eq!(path_key("CIMPATH=Map/M.json"), "map/m.json");
    assert_eq!(path_key(r" CimPath=\Map\M.json"), "map/m.json");
    let d =
        Document::from_json(r#"{"type":"CIMLayoutDocument","mapDefinitions":[{"type":"CIMMap","name":"M","uRI":"cimpath=map/m.json"}]}"#)
            .unwrap();
    assert_eq!(d.resolve("CIMPATH=Map/M.json").unwrap()["name"], "M");
}

#[test]
fn a_project_lists_its_maps_layouts_open_views_and_pictures() {
    use base64::Engine;
    let project = json!({
        "type": "CIMGISProject",
        "projectItems": [
            {"type": "CIMProjectItem", "itemType": "Map", "name": "First", "catalogPath": "CIMPATH=first/first.json"},
            {"type": "CIMProjectItem", "itemType": "Map", "name": "Second", "catalogPath": "CIMPATH=second/second.json"},
            {"type": "CIMProjectItem", "itemType": "Layout", "name": "Sheet", "catalogPath": "CIMPATH=sheet/sheet.json"},
            {"type": "CIMProjectItem", "itemType": "Map", "name": "Old", "catalogPath": "CIMPATH=old/old.xml"},
            {"type": "CIMProjectItem", "itemType": "Folder", "name": "Data", "catalogPath": "CIMPATH=data"},
        ],
        "views": [
            {"type": "CIMMapView", "viewableObjectPath": "CIMPATH=second/second.json", "camera": {"x": 1.0, "y": 2.0, "scale": 5000.0, "heading": 0.0}},
            {"type": "CIMLayoutView", "viewableObjectPath": "CIMPATH=sheet/sheet.json"},
            {"type": "CIMMapTableView", "viewableObjectPath": "CIMPATH=first/first.json"},
        ],
    });
    let png = b"\x89PNG\r\n\x1a\n".to_vec();
    let zip =
        rustgis_data::zip::write(&[("GISProject.json".into(), project.to_string().into_bytes()), ("Pictures/a.png".into(), png.clone())])
            .unwrap();
    let d = Document::from_zip(&zip).unwrap();

    let items = d.items();
    assert_eq!(items.len(), 4);
    assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["First", "Second", "Sheet", "Old"]);
    assert_eq!(items.iter().map(|i| i.kind.clone()).collect::<Vec<_>>(), [ItemKind::Map, ItemKind::Map, ItemKind::Layout, ItemKind::Map]);
    assert_eq!(items.iter().map(|i| i.arcmap).collect::<Vec<_>>(), [false, false, false, true]);
    assert_eq!(items.get(1).map(|i| i.uri.as_str()), Some("CIMPATH=second/second.json"));

    assert_eq!(d.open_maps(), ["CIMPATH=second/second.json"]);
    assert_eq!(d.view_camera("CIMPATH=second/second.json"), Some((1.0, 2.0, 5000.0)));
    assert_eq!(d.view_camera("CIMPATH=first/first.json"), None);

    let want = base64::engine::general_purpose::STANDARD.encode(&png);
    assert_eq!(d.binary("CIMPATH=Pictures/A.PNG"), Some(want.as_str()));
    assert_eq!(d.binary("CIMPATH=Pictures/b.png"), None);
}

#[test]
fn xml_named_entries_that_hold_json_are_read_and_are_not_arcmap() {
    // Pro keeps the `.xml` name of a map, layout or layer that came over from ArcMap after it
    // has rewritten it as CIM JSON; one that is still XML can't be read.
    let project = json!({
        "type": "CIMGISProject",
        "projectItems": [
            {"type": "CIMProjectItem", "itemType": "Map", "name": "Converted", "catalogPath": "CIMPATH=conv/conv.xml"},
            {"type": "CIMProjectItem", "itemType": "Layout", "name": "Converted Sheet", "catalogPath": "CIMPATH=layout/sheet.xml"},
            {"type": "CIMProjectItem", "itemType": "Map", "name": "Still Xml", "catalogPath": "CIMPATH=old/old.xml"},
            {"type": "CIMProjectItem", "itemType": "Map", "name": "Missing", "catalogPath": "CIMPATH=gone/gone.xml"},
        ],
    });
    let map = json!({"type": "CIMMap", "name": "Converted", "uRI": "CIMPATH=conv/conv.xml", "layers": ["CIMPATH=conv/roads.xml"]});
    let layer = json!({"type": "CIMFeatureLayer", "name": "Roads", "uRI": "CIMPATH=conv/roads.xml"});
    let sheet = json!({"type": "CIMLayout", "name": "Converted Sheet", "uRI": "CIMPATH=layout/sheet.xml"});
    let zip = rustgis_data::zip::write(&[
        ("GISProject.json".into(), project.to_string().into_bytes()),
        ("conv/conv.xml".into(), map.to_string().into_bytes()),
        ("conv/roads.xml".into(), layer.to_string().into_bytes()),
        ("layout/sheet.xml".into(), sheet.to_string().into_bytes()),
        ("old/old.xml".into(), b"<?xml version=\"1.0\"?><CIMMap/>".to_vec()),
    ])
    .unwrap();
    let d = Document::from_zip(&zip).unwrap();
    let items = d.items();
    assert_eq!(items.iter().map(|i| (i.name.as_str(), i.arcmap)).collect::<Vec<_>>(), [
        ("Converted", false),
        ("Converted Sheet", false),
        ("Still Xml", true),
        ("Missing", true)
    ]);
    assert_eq!(d.resolve("CIMPATH=conv/conv.xml").unwrap()["name"], "Converted");
    assert_eq!(d.resolve("CIMPATH=CONV/Roads.xml").unwrap()["name"], "Roads");
    assert!(d.resolve("CIMPATH=old/old.xml").is_none());
}

#[test]
fn a_document_that_is_not_a_project_has_no_items_or_views() {
    let d = Document::from_json(r#"{"type":"CIMLayoutDocument"}"#).unwrap();
    assert!(d.items().is_empty());
    assert!(d.open_maps().is_empty());
    assert_eq!(d.view_camera("CIMPATH=x.json"), None);
}

#[test]
fn project_items_are_bounded() {
    let items: Vec<_> = (0..MAX_ITEMS + 50)
        .map(|i| json!({"itemType": "Map", "name": format!("M{i}"), "catalogPath": format!("CIMPATH=m{i}/m{i}.json")}))
        .collect();
    let d = Document::from_json(&json!({"type": "CIMGISProject", "projectItems": items}).to_string()).unwrap();
    assert_eq!(d.items().len(), MAX_ITEMS);
}

#[test]
fn a_mapx_and_a_lyrx_expose_their_definitions() {
    let mapx =
        Document::from_json(r#"{"type":"CIMMapDocument","mapDefinition":{"type":"CIMMap","name":"M","layers":[]},"layerDefinitions":[]}"#)
            .unwrap();
    assert_eq!(mapx.map_definition().and_then(|m| m.get("name")).and_then(|n| n.as_str()), Some("M"));
    assert!(mapx.layer_uris().is_empty());

    let lyrx = Document::from_json(r#"{"type":"CIMLayerDocument","layers":["CIMPATH=a/a.json","CIMPATH=b/b.json"],"layerDefinitions":[]}"#)
        .unwrap();
    assert_eq!(lyrx.layer_uris(), ["CIMPATH=a/a.json", "CIMPATH=b/b.json"]);
    assert!(lyrx.map_definition().is_none());

    let pagx = Document::from_json(r#"{"type":"CIMLayoutDocument","layoutDefinition":{"type":"CIMLayout"}}"#).unwrap();
    assert!(pagx.map_definition().is_none());
    assert!(pagx.layer_uris().is_empty());
}

#[test]
fn oversize_picture_entries_are_left_out() {
    let project = br#"{"type":"CIMGISProject"}"#.to_vec();
    let zip = rustgis_data::zip::write(&[
        ("GISProject.json".into(), project),
        ("Pictures/big.png".into(), vec![7u8; 17 << 20]),
        ("Pictures/small.jpg".into(), vec![1, 2, 3]),
    ])
    .unwrap();
    let d = Document::from_zip(&zip).unwrap();
    assert_eq!(d.binary("CIMPATH=Pictures/big.png"), None);
    assert_eq!(d.binary("CIMPATH=Pictures/small.jpg"), Some("AQID"));
}

#[test]
fn a_layout_definition_and_a_map_definition_read_like_their_documents() {
    let text = r#"{"type":"CIMLayoutDocument","layoutDefinition":{"type":"CIMLayout","name":"P",
      "page":{"type":"CIMPage","width":11,"height":8.5,"units":{"uwkid":109008}},"elements":[]}}"#;
    let d = Document::from_json(text).unwrap();
    let by_doc = read_layout(&d, &mut Report::default()).unwrap();
    let def = d.root().get("layoutDefinition").unwrap();
    let by_def = read_layout_def(&d, def, &mut Report::default()).unwrap();
    assert_eq!(by_doc.layout.name, by_def.layout.name);

    let m = Document::from_json(r#"{"type":"CIMMapDocument","mapDefinition":{"type":"CIMMap","name":"Solo","layers":[]}}"#).unwrap();
    let imp = read_map_def(&m, m.map_definition().unwrap(), &mut Report::default()).unwrap();
    assert_eq!(imp.name, "Solo");
}

// ---- renderers: class breaks, proportional, the rest ----

fn class_break(upper: f64, label: Option<&str>, symbol: serde_json::Value) -> serde_json::Value {
    let mut b = json!({"type": "CIMClassBreak", "upperBound": upper, "symbol": symbol_ref(symbol)});
    if let (Some(o), Some(l)) = (b.as_object_mut(), label) {
        o.insert("label".into(), json!(l));
    }
    b
}

fn class_breaks_renderer(field: &str, breaks: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"type": "CIMClassBreaksRenderer", "field": field, "classificationMethod": "NaturalBreaks", "minimumBreak": 0.0,
        "normalizationType": "Nothing", "classBreakType": "GraduatedColor", "breaks": breaks})
}

/// A point symbol of one character marker, `size` points across.
fn point_symbol(size: f64) -> serde_json::Value {
    json!({"type": "CIMPointSymbol", "symbolLayers": [
        {"type": "CIMCharacterMarker", "enable": true, "size": size, "characterIndex": 33, "fontFamilyName": "ESRI Default Marker",
         "fontStyleName": "Regular", "symbol": polygon_symbol(vec![solid_fill(rgb(0, 128, 0))])}]})
}

fn size_variable(expression: &str) -> serde_json::Value {
    json!({"type": "CIMSizeVisualVariable", "valueExpressionInfo": {"type": "CIMExpressionInfo", "expression": expression},
        "minSize": 4.0, "maxSize": 20.0, "minValue": 0.0, "maxValue": 1000.0})
}

/// A layer of `renderer`, read back as the only layer of its map.
fn layer_with_renderer(renderer: serde_json::Value) -> (crate::qgis::Style, Report) {
    let layer = feature_layer("L", standard_connection("Shapefile", "DATABASE=/d", "l.shp"), json!({"renderer": renderer}));
    let (map, report) = read_new_detail_map(&map_document(&[layer], &["CIMPATH=new_detail_map/l.json"]));
    (nth(&map, 0).style.clone(), report)
}

#[test]
fn class_breaks_read_as_graduated() {
    use crate::symbology::{ClassMethod, Renderer};
    let fill = |r, g, b| polygon_symbol(vec![solid_fill(rgb(r, g, b))]);
    // Listed out of order: they are sorted by their upper bounds, each keeping its own symbol.
    let (style, report) = layer_with_renderer(class_breaks_renderer(
        "POP",
        vec![
            class_break(30.0, Some("20 to 30"), fill(0, 0, 255)),
            class_break(10.0, Some("0 to 10"), fill(255, 0, 0)),
            class_break(20.0, None, fill(0, 255, 0)),
        ],
    ));
    let Some(Renderer::Graduated { field, breaks, symbols, method, min, classes, class_labels, class_strokes, class_fills, .. }) =
        &style.renderer
    else {
        panic!("{:?}", style.renderer)
    };
    assert_eq!(field, "POP");
    assert_eq!(breaks, &[10.0, 20.0, 30.0]);
    assert_eq!(*method, ClassMethod::NaturalBreaks);
    assert_eq!((*min, *classes), (0.0, 3));
    let fills: Vec<_> = symbols.iter().map(|s| s.fill).collect();
    assert_eq!(fills, [Rgba::rgb(255, 0, 0), Rgba::rgb(0, 255, 0), Rgba::rgb(0, 0, 255)]);
    assert_eq!(class_labels.get(&0).map(String::as_str), Some("0 to 10"));
    assert_eq!(class_labels.get(&2).map(String::as_str), Some("20 to 30"));
    assert!(!class_labels.contains_key(&1));
    assert!(class_strokes.is_empty() && class_fills.is_empty());
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    // No field, or no break to read: a default symbol and a note.
    let (style, report) = layer_with_renderer(json!({"type": "CIMClassBreaksRenderer", "breaks": []}));
    assert!(matches!(style.renderer, Some(Renderer::Single(_))), "{:?}", style.renderer);
    assert!(notes(&report).iter().any(|n| n.starts_with("L: ") && n.contains("class breaks")), "{:?}", notes(&report));
    let (style, _) = layer_with_renderer(class_breaks_renderer("POP", vec![json!({"type": "CIMClassBreak", "upperBound": 5.0})]));
    assert!(matches!(style.renderer, Some(Renderer::Single(_))), "{:?}", style.renderer);
}

#[test]
fn unclassed_colour_is_graduated_and_noted() {
    use crate::symbology::{ClassMethod, Renderer};
    let fill = |r| polygon_symbol(vec![solid_fill(rgb(r, 0, 0))]);
    let mut r = class_breaks_renderer("DENSITY", vec![class_break(50.0, None, fill(10)), class_break(100.0, None, fill(200))]);
    r["classBreakType"] = json!("UnclassedColor");
    r["normalizationType"] = json!("Field");
    r["normalizationField"] = json!("AREA");
    r["classificationMethod"] = json!("Something New");
    r.as_object_mut().unwrap().remove("minimumBreak");
    let (style, report) = layer_with_renderer(r);
    let Some(Renderer::Graduated { breaks, method, min, classes, .. }) = &style.renderer else { panic!("{:?}", style.renderer) };
    assert_eq!(breaks, &[50.0, 100.0]);
    assert_eq!(*method, ClassMethod::Manual);
    assert_eq!((*min, *classes), (50.0, 2), "without a minimum, the first bound");
    let n = notes(&report);
    assert!(n.iter().any(|t| t.starts_with("L: ") && t.contains("unclassed colour ramp, drawn as its classes")), "{n:?}");
    assert!(n.iter().any(|t| t.starts_with("L: ") && t.contains("normalized")), "{n:?}");
}

#[test]
fn proportional_and_size_variables_read() {
    use crate::symbology::Renderer;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;

    let proportional = json!({"type": "CIMProportionalRenderer", "field": "TOTAL", "minDataValue": 0.0, "maxDataValue": 500.0,
        "minSymbol": symbol_ref(point_symbol(4.0)), "maxSymbol": symbol_ref(point_symbol(20.0)), "symbol": symbol_ref(point_symbol(8.0))});
    let (style, report) = layer_with_renderer(proportional);
    let Some(Renderer::Proportional { field, base, min_size, max_size, min, max }) = &style.renderer else {
        panic!("{:?}", style.renderer)
    };
    assert_eq!(field, "TOTAL");
    assert!(near(*min_size, 5.33) && near(*max_size, 26.67), "{min_size} {max_size}");
    assert!(near(base.size, 10.67), "{}", base.size);
    assert_eq!((*min, *max), (0.0, 500.0));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    let mut by_field = simple_renderer(point_symbol(8.0));
    by_field["visualVariables"] = json!([{"type": "CIMColorVisualVariable", "expression": "POP"}, size_variable("$feature.POP")]);
    let (style, report) = layer_with_renderer(by_field);
    let Some(Renderer::Proportional { field, base, min_size, max_size, min, max }) = &style.renderer else {
        panic!("{:?}", style.renderer)
    };
    assert_eq!(field, "POP");
    assert!(near(*min_size, 5.33) && near(*max_size, 26.67), "{min_size} {max_size}");
    assert!(near(base.size, 10.67), "{}", base.size);
    assert_eq!((*min, *max), (0.0, 1000.0));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    // By the map scale: drawn at one size, noted.
    let mut by_scale = simple_renderer(point_symbol(8.0));
    by_scale["visualVariables"] = json!([size_variable("$view.scale")]);
    let (style, report) = layer_with_renderer(by_scale);
    let Some(Renderer::Single(s)) = &style.renderer else { panic!("{:?}", style.renderer) };
    assert!(near(s.size, 10.67), "{}", s.size);
    let n = notes(&report);
    assert!(n.iter().any(|t| t.starts_with("L: ") && t.contains("symbol sizes that change") && t.contains("drawn at one size")), "{n:?}");

    // On another renderer: the same note, the renderer as it was.
    let mut on_classes = class_breaks_renderer("POP", vec![class_break(10.0, None, point_symbol(6.0))]);
    on_classes["visualVariables"] = json!([size_variable("$feature.POP")]);
    let (style, report) = layer_with_renderer(on_classes);
    assert!(matches!(style.renderer, Some(Renderer::Graduated { .. })), "{:?}", style.renderer);
    assert!(notes(&report).iter().any(|t| t.contains("symbol sizes that change")), "{:?}", notes(&report));
}

#[test]
fn other_renderers_draw_their_first_symbol() {
    use crate::symbology::Renderer;
    // The symbol is found anywhere in the renderer, past things that are not symbols.
    let heat = json!({"type": "CIMHeatMapRenderer", "colorRamp": {"type": "CIMMultipartColorRamp", "colorRamps": []}, "radius": 10,
        "dots": {"layers": [{"deep": [symbol_ref(polygon_symbol(vec![solid_fill(rgb(1, 2, 3))])), symbol_ref(point_symbol(9.0))]}]}});
    let (style, report) = layer_with_renderer(heat);
    let Some(Renderer::Single(s)) = &style.renderer else { panic!("{:?}", style.renderer) };
    assert_eq!(s.fill, Rgba::rgb(1, 2, 3));
    let n = notes(&report);
    assert!(n.iter().any(|t| t.starts_with("L: ") && t.contains("CIMHeatMapRenderer renderers, drawn with its first symbol")), "{n:?}");

    // Nothing to draw with: the default symbol.
    let (style, report) = layer_with_renderer(json!({"type": "CIMDotDensityRenderer", "attributes": []}));
    assert!(matches!(style.renderer, Some(Renderer::Single(_))), "{:?}", style.renderer);
    assert!(notes(&report).iter().any(|t| t.contains("CIMDotDensityRenderer")), "{:?}", notes(&report));
}

// ---- label expressions ------------------------------------------------------------------------

/// `expression` in `engine` as `(template, exact, fields)`.
fn translated(expression: &str, engine: &str) -> Option<(String, bool, Vec<String>)> {
    expr::template(expression, engine).map(|t| (t.template, t.exact, t.fields))
}

fn exact(template: &str, fields: &[&str]) -> Option<(String, bool, Vec<String>)> {
    Some((template.to_string(), true, fields.iter().map(|f| f.to_string()).collect()))
}

fn inexact(field: &str) -> Option<(String, bool, Vec<String>)> {
    Some((field.to_string(), false, vec![field.to_string()]))
}

#[test]
fn arcade_concatenations_become_templates() {
    assert_eq!(translated("$feature.A + \"\\n \" + $feature.B", "Arcade"), exact("{A}\n {B}", &["A", "B"]));
    assert_eq!(translated("\"SEC. \"+$feature.SECT_NO", "Arcade"), exact("SEC. {SECT_NO}", &["SECT_NO"]));
    // A field of a join keeps the part after the last dot; alone it is a plain field.
    let t = expr::template("$feature['T.Name']", "Arcade").unwrap();
    assert_eq!((t.template.as_str(), t.exact, t.fields.as_slice()), ("Name", true, &["Name".to_string()][..]));
    assert_eq!(t.joins, [("T.Name".to_string(), "Name".to_string())]);
    assert_eq!(translated("\"(\" + $feature[\"T.Name\"] + \")\"", "Arcade"), exact("({Name})", &["Name"]));
    // One field alone stays a plain field name, as do a bare name and a leading return.
    assert_eq!(translated("$feature.A", "Arcade"), exact("A", &["A"]));
    assert_eq!(translated(" return $feature.A; ", "Arcade"), exact("A", &["A"]));
    assert_eq!(translated("RETURN \"x\" + $feature.A;", "Arcade"), exact("x{A}", &["A"]));
    assert_eq!(translated("StreetName", "Arcade"), exact("StreetName", &["StreetName"]));
    // A bare dotted name and a dotted `$feature.` name are fields of a join.
    let t = expr::template("BM_Parcel.StreetName", "Arcade").unwrap();
    assert_eq!((t.template.as_str(), t.exact), ("StreetName", true));
    assert_eq!(t.joins, [("BM_Parcel.StreetName".to_string(), "StreetName".to_string())]);
    assert_eq!(translated("$feature.T.Name", "Arcade"), exact("Name", &["Name"]));
    assert_eq!(translated("$feature.T.Name + \" \" + $feature.A", "Arcade"), exact("{Name} {A}", &["Name", "A"]));
    // A quoted name can hold any character but its quote: alone it is the field; in a template
    // only a plain name fits a placeholder, so the expression falls back to its first field.
    assert_eq!(translated("$feature[\"Lot Number\"]", "Arcade"), exact("Lot Number", &["Lot Number"]));
    assert_eq!(translated("$feature['A-B']", "Arcade"), exact("A-B", &["A-B"]));
    assert_eq!(translated("\"Lot \" + $feature[\"Lot Number\"]", "Arcade"), inexact("Lot Number"));
    assert_eq!(translated("$feature.A + \" \" + $feature['A-B']", "Arcade"), inexact("A"));
    // Newlines, each field once in order, escapes and a literal holding a `+`.
    assert_eq!(
        translated("$feature.A + TextFormatting.NewLine + $feature.B + \"\\r\\n\" + $feature.A", "Arcade"),
        exact("{A}\n{B}\n{A}", &["A", "B"])
    );
    assert_eq!(translated("'it\\'s ' + $feature.A + \" a+b \\\"q\\\" \\\\\"", "Arcade"), exact("it's {A} a+b \"q\" \\", &["A"]));
    assert_eq!(translated("$feature.A + \"<br>\" + $feature.B + \"<BR>x\\ry\"", "Arcade"), exact("{A}\n{B}\nx\ny", &["A", "B"]));
}

#[test]
fn vbscript_and_python_concatenations_become_templates() {
    assert_eq!(translated("\"BLK \" & [BLOCK]", "VBScript"), exact("BLK {BLOCK}", &["BLOCK"]));
    assert_eq!(translated("[rods]&\" RD\"", "VBScript"), exact("{rods} RD", &["rods"]));
    assert_eq!(translated("\"A: \" + !A! + vbNewLine + !B!", "Python"), exact("A: {A}\n{B}", &["A", "B"]));
    assert_eq!(translated("\"%s\" % !X!", "Python"), inexact("X"));
    // VBScript doubles a quote, keeps a backslash, and has its own newlines.
    assert_eq!(
        translated("\"say \"\"hi\"\" C:\\n\" & [A] & vbCrLf & [B] & vbLf & [A]", "VBScript"),
        exact("say \"hi\" C:\\n{A}\n{B}\n{A}", &["A", "B"])
    );
    assert_eq!(
        translated("[A] & chr(13) & chr(10) & [B] & CHR(10) & [OWNER]", "VBScript"),
        exact("{A}\n{B}\n{OWNER}", &["A", "B", "OWNER"])
    );
    assert_eq!(translated("[A]", "VBScript"), exact("A", &["A"]));
    assert_eq!(translated("!A!", "Python"), exact("A", &["A"]));
    assert_eq!(translated("[A] + \" \" + [B]", "JScript"), exact("{A} {B}", &["A", "B"]));
}

#[test]
fn function_calls_fall_back_to_the_first_field() {
    assert_eq!(translated("Split($feature.OWNER, ' ')[0]", "Arcade"), inexact("OWNER"));
    assert_eq!(translated("Return Concatenate([$feature.a, $feature.b], ' ')", "Arcade"), inexact("a"));
    assert_eq!(translated("1 + 2", "Arcade"), None);
    assert_eq!(translated("IIf([A] > 0, [A], [B])", "VBScript"), inexact("A"));
    assert_eq!(translated("$feature.A * 2", "Arcade"), inexact("A"));
    assert_eq!(translated("$feature.A $feature.B", "Arcade"), inexact("A"), "two fields with nothing joining them");
    assert_eq!(translated("$feature.A +", "Arcade"), inexact("A"));
    assert_eq!(translated("\"unterminated + $feature.A", "Arcade"), inexact("A"));
    // Nothing but text has no field to draw.
    assert_eq!(translated("\"no field here\"", "Arcade"), None);
    assert_eq!(translated("", "Arcade"), None);
    assert_eq!(translated("return", "Arcade"), None);
    assert_eq!(translated("return;", "Arcade"), None);
    assert_eq!(translated("$feature.", "Arcade"), None);
    assert_eq!(translated("[ ]", "VBScript"), None);
}

#[test]
fn formatting_tags_and_braces() {
    let t = expr::template("\"<BOL>\" + $feature.X + \"</BOL>\"", "Arcade").unwrap();
    assert_eq!((t.template.as_str(), t.exact, t.tags_dropped), ("{X}", true, true));
    assert_eq!(
        translated("\"<CLR red='255'><ITA>\" + $feature.X + \"</ITA></CLR> <UND>u</UND><FNT name='A' size='9'>f</FNT>\"", "Arcade"),
        exact("{X} uf", &["X"])
    );
    // Not a tag: kept as text.
    assert_eq!(translated("\"a < b <i> <CLR\" + $feature.X", "Arcade"), exact("a < b <i> <CLR{X}", &["X"]));
    assert!(!expr::template("\"a < b\" + $feature.X", "Arcade").unwrap().tags_dropped);
    // Braces in text are doubled, so they stay text.
    assert_eq!(translated("\"{x}\" + [A] + \"}\"", "VBScript"), exact("{{x}}{A}}}", &["A"]));
    let t = expr::template("\"{x}\" + [A]", "VBScript").unwrap().template;
    assert_eq!(crate::symbology::template_fields(&t), ["A"]);
}

#[test]
fn hostile_expressions_are_bounded() {
    let long = "x".repeat(100_000);
    let expressions = [
        format!("\"{long}"),
        format!("'{long}"),
        "[".repeat(100_000),
        "!".repeat(100_000),
        "$feature[\"".repeat(10_000),
        "<CLR".repeat(25_000),
        format!("\"{}\"", "<CLR".repeat(25_000)),
        "[[[[$feature.A]]]]".repeat(5_000),
        format!("$feature.A + \"{long}\""),
        "$feature.A + ".repeat(8_000) + "$feature.A",
        format!("\"{}\" + $feature.A", "{".repeat(100_000)),
        "\"\\".repeat(50_000),
        "chr(".repeat(25_000),
        // Thousands of different field names.
        (0..8_000).map(|i| format!("$feature.f{i}")).collect::<Vec<_>>().join(" + "),
        (0..8_000).map(|i| format!("[f{i}]")).collect::<Vec<_>>().join("&"),
    ];
    for e in &expressions {
        for engine in ["Arcade", "VBScript", "Python", "JScript"] {
            // The best of five runs, so a busy machine doesn't fail it.
            let took = (0..5)
                .map(|_| {
                    let start = std::time::Instant::now();
                    std::hint::black_box(expr::template(e, engine));
                    start.elapsed()
                })
                .min()
                .unwrap();
            assert!(took.as_millis() < 20, "{} chars took {took:?}", e.len());
            if let Some(t) = expr::template(e, engine) {
                assert!(t.template.chars().count() <= crate::symbology::MAX_TEMPLATE);
                assert!(!t.fields.is_empty());
            }
        }
    }
    // Too long a template falls back to the first field.
    assert_eq!(translated(&format!("$feature.A + \"{long}\""), "Arcade"), inexact("A"));
    let many = (0..8_000).map(|i| format!("$feature.f{i}")).collect::<Vec<_>>().join(" + ");
    assert_eq!(translated(&many, "Arcade"), inexact("f0"));
    // No field's name is longer than a template can be.
    let name = "N".repeat(2_000);
    assert_eq!(translated(&name, "Arcade"), None);
    assert_eq!(translated(&format!("[{name}]"), "VBScript"), None);
    assert_eq!(translated(&format!("\"a\" + [{name}]"), "VBScript"), None);
    assert_eq!(translated(&format!("Upper($feature.{name})"), "Arcade"), None);
}

#[test]
fn label_expressions_come_through_as_templates() {
    let class = |expression: &str, engine: &str| {
        json!({"type": "CIMLabelClass", "expression": expression, "expressionEngine": engine,
            "textSymbol": symbol_ref(text_symbol(json!({"height": 10})))})
    };
    let classes = vec![
        class("\"SEC. \" + $feature.SECT_NO + TextFormatting.NewLine + $feature['T.BLOCK']", "Arcade"),
        class("$feature.OWNER", "Arcade"),
        class("\"<BOL>\" & [StreetName] & \"</BOL>\"", "VBScript"),
    ];
    let layer = feature_layer("Lots", standard_connection("Shapefile", "DATABASE=/d", "l.shp"), json!({"labelClasses": classes}));
    let (map, report) = read_new_detail_map(&map_document(&[layer], &["CIMPATH=new_detail_map/lots.json"]));
    let labels = nth(&map, 0).style.labels.as_ref().unwrap();
    let fields: Vec<&str> = labels.classes.iter().map(|c| c.field.as_str()).collect();
    assert_eq!(fields, ["SEC. {SECT_NO}\n{BLOCK}", "OWNER", "{StreetName}"]);
    let n = notes(&report);
    assert!(n.iter().any(|t| t == "Lots: field T.BLOCK of a join, read as BLOCK"), "{n:?}");
    assert_eq!(n.iter().filter(|t| t.contains("text formatting tags, left out")).count(), 1, "{n:?}");
    assert!(!n.iter().any(|t| t.contains("drawn as")), "{n:?}");
}

// ---- label placement, callouts and stacking ---------------------------------------------------

/// The default label class and the report of a layer "Roads" with one label class: `class` is merged
/// into the class, `text` into its text symbol.
fn read_class(class: serde_json::Value, text: serde_json::Value) -> (crate::symbology::LabelClass, Report) {
    let mut c = json!({"type": "CIMLabelClass", "expression": "$feature.NAME", "visibility": true,
        "textSymbol": symbol_ref(text_symbol(text))});
    if let (Some(o), Some(e)) = (c.as_object_mut(), class.as_object()) {
        o.extend(e.clone());
    }
    let layer = feature_layer("Roads", standard_connection("Shapefile", "DATABASE=/d", "r.shp"), json!({"labelClasses": [c]}));
    let (map, report) = read_new_detail_map(&map_document(&[layer], &["CIMPATH=new_detail_map/roads.json"]));
    (nth(&map, 0).style.labels.clone().unwrap().default, report)
}

fn maplex(props: serde_json::Value) -> serde_json::Value {
    let mut p = json!({"type": "CIMMaplexLabelPlacementProperties"});
    if let (Some(o), Some(e)) = (p.as_object_mut(), props.as_object()) {
        o.extend(e.clone());
    }
    json!({"maplexLabelPlacementProperties": p})
}

fn standard(props: serde_json::Value) -> serde_json::Value {
    let mut p = json!({"type": "CIMStandardLabelPlacementProperties"});
    if let (Some(o), Some(e)) = (p.as_object_mut(), props.as_object()) {
        o.extend(e.clone());
    }
    json!({"standardLabelPlacementProperties": p})
}

#[test]
fn maplex_placements_map_by_feature_type() {
    use crate::symbology::LabelPlacement::{self as P, *};
    let rows: [(&str, &str, &str, P); 25] = [
        ("Point", "pointPlacementMethod", "AroundPoint", Auto),
        ("Point", "pointPlacementMethod", "CenteredOnPoint", Center),
        ("Point", "pointPlacementMethod", "NorthOfPoint", Above),
        ("Point", "pointPlacementMethod", "SouthOfPoint", Below),
        ("Point", "pointPlacementMethod", "EastOfPoint", Right),
        ("Point", "pointPlacementMethod", "WestOfPoint", Left),
        ("Point", "pointPlacementMethod", "NorthEastOfPoint", AboveRight),
        ("Point", "pointPlacementMethod", "NorthWestOfPoint", AboveLeft),
        ("Point", "pointPlacementMethod", "SouthEastOfPoint", BelowRight),
        ("Point", "pointPlacementMethod", "SouthWestOfPoint", BelowLeft),
        ("Point", "pointPlacementMethod", "RotationField", Auto),
        ("Line", "linePlacementMethod", "OffsetStraightFromLine", Parallel),
        ("Line", "linePlacementMethod", "CenteredStraightOnLine", Parallel),
        ("Line", "linePlacementMethod", "CenteredHorizontalOnLine", Parallel),
        ("Line", "linePlacementMethod", "OffsetHorizontalFromLine", Parallel),
        ("Line", "linePlacementMethod", "OffsetCurvedFromLine", Curved),
        ("Line", "linePlacementMethod", "CenteredCurvedOnLine", Curved),
        ("Polygon", "polygonPlacementMethod", "HorizontalInPolygon", Center),
        ("Polygon", "polygonPlacementMethod", "AlwaysHorizontal", Center),
        ("Polygon", "polygonPlacementMethod", "BoundaryPlacement", Center),
        ("Polygon", "polygonPlacementMethod", "StraightInPolygon", Parallel),
        ("Polygon", "polygonPlacementMethod", "CurvedInPolygon", Curved),
        ("Polygon", "polygonPlacementMethod", "RepeatAlongBoundary", Curved),
        // A method this reader does not know is Auto, noted.
        ("Polygon", "polygonPlacementMethod", "SpiralInPolygon", Auto),
        // A method of another feature type is none of this one's.
        ("Line", "pointPlacementMethod", "NorthOfPoint", Auto),
    ];
    for (feature, key, method, want) in rows {
        let mut props = json!({"featureType": feature, "primaryOffset": 0});
        props[key] = json!(method);
        let (c, report) = read_class(maplex(props), json!({}));
        assert_eq!(c.placement, want, "{feature} {method}");
        assert_eq!(c.repeat, None, "{feature} {method}");
        let n = notes(&report);
        let odd = matches!(method, "RotationField" | "SpiralInPolygon");
        assert_eq!(n.len(), usize::from(odd), "{method}: {n:?}");
        if odd {
            assert_eq!(n[0], format!("Roads: label placement {method}, drawn as Auto"));
        }
    }
}

#[test]
fn maplex_offsets_repeats_and_missing_feature_types() {
    use crate::symbology::LabelPlacement::*;
    let point = |method: &str, offset: f64| {
        let (c, _) =
            read_class(maplex(json!({"featureType": "Point", "pointPlacementMethod": method, "primaryOffset": offset})), json!({}));
        (c.placement, c.offset)
    };
    // Points, above up and below down; around a point is above.
    assert_eq!(point("NorthOfPoint", 4.0), (Above, [0.0, -4.0]));
    assert_eq!(point("NorthWestOfPoint", 4.0), (AboveLeft, [0.0, -4.0]));
    assert_eq!(point("SouthOfPoint", 4.0), (Below, [0.0, 4.0]));
    assert_eq!(point("SouthEastOfPoint", 2.5), (BelowRight, [0.0, 2.5]));
    assert_eq!(point("AroundPoint", 3.0), (Auto, [0.0, -3.0]));
    assert_eq!(point("EastOfPoint", 3.0), (Right, [3.0, 0.0]));
    assert_eq!(point("WestOfPoint", 3.0), (Left, [-3.0, 0.0]));
    assert_eq!(point("CenteredOnPoint", 3.0), (Center, [0.0, 0.0]));
    assert_eq!(point("NorthOfPoint", 500.0), (Above, [0.0, -200.0]));
    assert_eq!(point("SouthOfPoint", 1e300).1, [0.0, 200.0]);
    assert_eq!(point("NorthOfPoint", -4.0), (Above, [0.0, 0.0]), "a negative distance is none");

    // Repeat along a boundary: points, within 10 to 10,000; none for a size that is not positive.
    let boundary = |interval: serde_json::Value| {
        let (c, _) = read_class(
            maplex(json!({"featureType": "Polygon", "polygonPlacementMethod": "RepeatAlongBoundary", "repetitionInterval": interval})),
            json!({}),
        );
        (c.placement, c.repeat)
    };
    assert_eq!(boundary(json!(120)), (Curved, Some(120.0)));
    assert_eq!(boundary(json!(2)), (Curved, Some(10.0)));
    assert_eq!(boundary(json!(1e9)), (Curved, Some(10_000.0)));
    assert_eq!(boundary(json!(0)), (Curved, None));
    assert_eq!(boundary(json!(-5)), (Curved, None));
    assert_eq!(boundary(json!("far")), (Curved, None));
    // The repeat belongs to that method alone.
    let (c, _) = read_class(
        maplex(json!({"featureType": "Polygon", "polygonPlacementMethod": "AlwaysHorizontal", "repetitionInterval": 50})),
        json!({}),
    );
    assert_eq!((c.placement, c.repeat), (Center, None));

    // Without a feature type, the method the block has is the one read.
    let (c, report) = read_class(maplex(json!({"linePlacementMethod": "CenteredCurvedOnLine"})), json!({}));
    assert_eq!(c.placement, Curved);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    // No placement properties at all is Auto, unnoted; so is an empty block.
    let (c, report) = read_class(json!({}), json!({}));
    assert_eq!((c.placement, c.offset, c.repeat), (Auto, [0.0; 2], None));
    assert!(report.notes.is_empty());
    let (c, _) = read_class(maplex(json!({})), json!({}));
    assert_eq!(c.placement, Auto);
}

#[test]
fn maplex_settings_without_a_counterpart_are_noted() {
    let (_, report) = read_class(
        maplex(json!({"featureType": "Line", "linePlacementMethod": "CenteredStraightOnLine",
            "canOverrunFeature": true, "labelPriority": 3, "featureWeight": 5, "keyNumberMethod": "GetUniqueNumbers",
            "polygonInternalZonePriorities": {"center": 1}, "pointPlacementPriorities": {"aboveLeft": 2},
            "offsetAlongLineProperties": {"placementMethod": "OffsetFromStart", "distance": 0}, "thinDuplicateLabels": true,
            "removeDuplicates": "ByLabel"})),
        json!({}),
    );
    let n = notes(&report);
    assert_eq!(n.len(), 7, "{n:?}");
    assert!(!n.iter().any(|t| t.contains("zone")), "zone priorities are never noted: {n:?}");
    assert!(n.iter().all(|t| t.starts_with("Roads: ") && t.ends_with(", left out")), "{n:?}");
    // The defaults are not worth saying.
    let (_, report) = read_class(
        maplex(json!({"featureType": "Line", "linePlacementMethod": "CenteredStraightOnLine",
            "canOverrunFeature": false, "labelPriority": -1, "featureWeight": 0, "keyNumberMethod": "NoKey",
            "polygonInternalZonePriorities": {}, "offsetAlongLineProperties": {}, "thinDuplicateLabels": false,
            "removeDuplicates": "None"})),
        json!({}),
    );
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    // Populated but default, as Pro writes them on every class: priorities for the zones, and an
    // offset along a line that stays at the best position, no distance.
    let (_, report) = read_class(
        maplex(json!({"featureType": "Line", "linePlacementMethod": "CenteredStraightOnLine",
            "pointPlacementPriorities": {"aboveLeft": 2, "aboveCenter": 1}, "polygonInternalZonePriorities": {"center": 1},
            "polygonExternalZonePriorities": {"aboveLeft": 1},
            "offsetAlongLineProperties": {"placementMethod": "BestPositionAlongLine", "distance": 0, "distanceUnit": "Percentage"}})),
        json!({}),
    );
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    // A distance along the line moves the label; so does a place other than the best one.
    for offset in [json!({"placementMethod": "BestPositionAlongLine", "distance": 15}), json!({"placementMethod": "OffsetFromEnd"})] {
        let (_, report) = read_class(
            maplex(json!({"featureType": "Line", "linePlacementMethod": "CenteredStraightOnLine", "offsetAlongLineProperties": offset})),
            json!({}),
        );
        assert_eq!(notes(&report), ["Roads: label offsets along a line, left out"]);
    }
}

#[test]
fn standard_placements_map_too() {
    use crate::symbology::LabelPlacement::{self as P, *};
    let rows: Vec<(serde_json::Value, P)> = vec![
        (json!({"featureType": "Point", "pointPlacementMethod": "AroundPoint"}), Auto),
        (json!({"featureType": "Point", "pointPlacementMethod": "NorthEastOfPoint"}), AboveRight),
        (json!({"featureType": "Point", "pointPlacementMethod": "WestOfPoint"}), Left),
        (json!({"featureType": "Point", "pointPlacementMethod": "CenteredOnPoint"}), Center),
        (json!({"featureType": "Line", "lineLabelPosition": {"parallel": true, "above": true}}), Parallel),
        (json!({"featureType": "Line", "lineLabelPosition": {"horizontal": true}}), Parallel),
        (json!({"featureType": "Line", "lineLabelPosition": {"perpendicular": true}}), Parallel),
        (json!({"featureType": "Line", "lineLabelPosition": {"below": true}}), Parallel),
        (json!({"featureType": "Line", "lineLabelPosition": {"online": true}}), Parallel),
        (json!({"featureType": "Line", "lineLabelPosition": {"inLine": true}}), Parallel),
        (json!({"featureType": "Line", "lineLabelPosition": {"parallel": true}, "produceCurvedLabels": true}), Curved),
        (json!({"featureType": "Line", "lineLabelPosition": {"parallel": false}}), Auto),
        (json!({"featureType": "Line"}), Auto),
        (json!({"featureType": "Polygon", "polygonPlacementMethod": "AlwaysHorizontal"}), Center),
        (json!({"featureType": "Polygon", "polygonPlacementMethod": "MixedStrategy"}), Center),
        (json!({"featureType": "Polygon", "polygonPlacementMethod": "AlwaysStraight"}), Parallel),
        // No feature type: the method the block has.
        (json!({"polygonPlacementMethod": "AlwaysStraight"}), Parallel),
        (json!({"lineLabelPosition": {"parallel": true}, "produceCurvedLabels": true}), Curved),
        // A line setting on a point block is none of its business.
        (json!({"featureType": "Point", "lineLabelPosition": {"parallel": true}}), Auto),
    ];
    for (props, want) in rows {
        let (c, report) = read_class(standard(props.clone()), json!({}));
        assert_eq!(c.placement, want, "{props}");
        assert!(report.notes.is_empty(), "{props}: {:?}", notes(&report));
    }
    let (c, report) = read_class(standard(json!({"featureType": "Polygon", "polygonPlacementMethod": "Whirl"})), json!({}));
    assert_eq!(c.placement, Auto);
    assert_eq!(notes(&report), ["Roads: label placement Whirl, drawn as Auto"]);

    // Maplex wins when both are there.
    let mut both = standard(json!({"featureType": "Point", "pointPlacementMethod": "NorthOfPoint"}));
    both["maplexLabelPlacementProperties"] = json!({"featureType": "Point", "pointPlacementMethod": "SouthOfPoint"});
    let (c, _) = read_class(both, json!({}));
    assert_eq!(c.placement, crate::symbology::LabelPlacement::Below);
}

fn background_callout(extra: serde_json::Value) -> serde_json::Value {
    let mut c = json!({"type": "CIMBackgroundCallout",
        "backgroundSymbol": polygon_symbol(vec![solid_stroke(rgb(128, 128, 128), 0.5), solid_fill(rgb(255, 255, 255))]),
        "margin": {"left": 2, "right": 2, "top": 1, "bottom": 1}, "cornerRadius": 1});
    if let (Some(o), Some(e)) = (c.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    c
}

#[test]
fn background_callouts_become_label_backgrounds() {
    let halo = symbol_ref(polygon_symbol(vec![solid_fill(rgb(255, 255, 0))]));
    let (c, report) = read_class(json!({}), json!({"callout": background_callout(json!({})), "haloSize": 1, "haloSymbol": halo.clone()}));
    let b = c.background.expect("a background");
    assert_eq!((b.fill, b.stroke), (Some(Rgba::rgb(255, 255, 255)), Some(Rgba::rgb(128, 128, 128))));
    // Points, as they are.
    assert!((b.stroke_width - 0.5).abs() < 1e-4, "{}", b.stroke_width);
    assert_eq!((b.padding, b.radius), ([2.0, 1.0], 1.0));
    // The background stands where the halo would: the halo is not kept.
    assert!(!c.halo);
    assert_eq!((c.halo_color, c.halo_width), (None, None));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));

    // The wider margin of each pair; margins that are missing are 0; huge ones are kept within range.
    let wide = background_callout(json!({"margin": {"left": 3, "right": 1, "top": 0.5, "bottom": 2}}));
    let (c, _) = read_class(json!({}), json!({"callout": wide}));
    assert_eq!(c.background.unwrap().padding, [3.0, 2.0]);
    let (c, _) = read_class(json!({}), json!({"callout": background_callout(json!({"margin": null, "cornerRadius": null}))}));
    let b = c.background.unwrap();
    assert_eq!((b.padding, b.radius), ([0.0, 0.0], 0.0));
    let huge = background_callout(json!({"margin": {"left": 9999, "top": -4}, "cornerRadius": 1e9}));
    let (c, _) = read_class(json!({}), json!({"callout": huge}));
    let b = c.background.unwrap();
    assert_eq!((b.padding, b.radius), ([50.0, 0.0], 50.0));
    // A fill alone has no outline.
    let only_fill = polygon_symbol(vec![solid_fill(rgb(0, 0, 0))]);
    let (c, _) = read_class(json!({}), json!({"callout": background_callout(json!({"backgroundSymbol": only_fill}))}));
    let b = c.background.unwrap();
    assert_eq!((b.fill, b.stroke, b.stroke_width), (Some(Rgba::rgb(0, 0, 0)), None, 0.0));
    // An outline of no width is no outline, not a hairline.
    let hairline = polygon_symbol(vec![solid_stroke(rgb(128, 128, 128), 0.0), solid_fill(rgb(255, 255, 255))]);
    let (c, _) = read_class(json!({}), json!({"callout": background_callout(json!({"backgroundSymbol": hairline}))}));
    let b = c.background.unwrap();
    assert_eq!((b.fill, b.stroke, b.stroke_width), (Some(Rgba::rgb(255, 255, 255)), None, 0.0));

    // Nothing to draw (no symbol, or one that draws nothing) is no background, and the halo stays.
    for callout in [
        json!({"type": "CIMBackgroundCallout", "margin": {"left": 2}}),
        json!({"type": "CIMBackgroundCallout", "backgroundSymbol": polygon_symbol(vec![])}),
    ] {
        let (c, _) = read_class(json!({}), json!({"callout": callout, "haloSize": 1, "haloSymbol": halo.clone()}));
        assert_eq!(c.background, None);
        assert!(c.halo);
        assert_eq!(c.halo_width, Some(1.0));
    }
}

#[test]
fn line_callouts_and_shadows_are_noted() {
    let halo = symbol_ref(polygon_symbol(vec![solid_fill(rgb(255, 255, 255))]));
    let leader = json!({"type": "CIMSimpleLineCallout", "leaderLineSymbol": {"type": "CIMLineSymbol", "symbolLayers": []}});
    let (c, report) = read_class(json!({}), json!({"callout": leader, "haloSize": 1, "haloSymbol": halo}));
    assert_eq!(c.background, None);
    assert!(c.halo, "a leader line leaves the halo");
    assert_eq!(notes(&report), ["Roads: a leader line, left out"]);

    let composite = json!({"type": "CIMCompositeCallout", "callouts": [{"type": "CIMSimpleLineCallout"}]});
    let (c, report) = read_class(json!({}), json!({"callout": composite}));
    assert_eq!(c.background, None);
    assert_eq!(notes(&report), ["Roads: a leader line, left out"]);

    let (_, report) = read_class(json!({}), json!({"callout": {"type": "CIMFancyCallout"}}));
    assert_eq!(notes(&report), ["Roads: callouts of type CIMFancyCallout, left out"]);

    // A shadow shows when it is offset (`shadowOffsetX`/`Y`, as Pro writes them) in a colour that
    // shows (black when none is given). Pro writes an opaque black `shadowColor` on most text
    // symbols with no offset: no shadow, nothing to note.
    let shadow = |extra: serde_json::Value| notes(&read_class(json!({}), extra).1);
    let black = json!({"type": "CIMRGBColor", "values": [0, 0, 0, 100]});
    let clear = json!({"type": "CIMRGBColor", "values": [0, 0, 0, 0]});
    assert!(shadow(json!({"shadowColor": black})).is_empty(), "Pro's default: a colour and no offset");
    assert_eq!(shadow(json!({"shadowColor": black, "shadowOffsetX": 1, "shadowOffsetY": -1})), ["Roads: a text shadow, left out"]);
    assert_eq!(shadow(json!({"shadowOffsetX": 1.5})), ["Roads: a text shadow, left out"]);
    assert_eq!(shadow(json!({"shadowOffsetY": -1})), ["Roads: a text shadow, left out"]);
    assert!(shadow(json!({"shadowColor": clear, "shadowOffsetX": 2})).is_empty(), "a clear shadow doesn't show");
    assert!(shadow(json!({"shadowColor": black, "shadowOffsetX": 0, "shadowOffsetY": 0})).is_empty());
    assert!(shadow(json!({})).is_empty());
}

#[test]
fn a_hidden_stacking_separator_becomes_the_wrap_character() {
    let separators = json!([{"separator": " ", "splitAfter": true}, {"separator": ",", "visible": true, "splitAfter": true}]);
    // On the class, and inside the Maplex block (where Pro writes them).
    let (c, report) = read_class(json!({"canStackLabel": true, "labelStackingProperties": {"separators": separators}}), json!({}));
    assert_eq!(c.wrap_char.as_deref(), Some(" "));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    let (c, _) = read_class(maplex(json!({"canStackLabel": true, "labelStackingProperties": {"separators": separators}})), json!({}));
    assert_eq!(c.wrap_char.as_deref(), Some(" "));

    // One that stays visible comes first: noted, and the next hidden one is the one.
    let ordered = json!([{"separator": ",", "visible": true}, {"separator": ";", "visible": false}]);
    let (c, report) = read_class(json!({"canStackLabel": true, "labelStackingProperties": {"separators": ordered}}), json!({}));
    assert_eq!(c.wrap_char.as_deref(), Some(";"));
    assert_eq!(notes(&report), ["Roads: stacking at , keeping it, drawn without"]);
    // Only visible ones: no wrap character.
    let visible = json!([{"separator": ",", "visible": true}]);
    let (c, report) = read_class(json!({"canStackLabel": true, "labelStackingProperties": {"separators": visible}}), json!({}));
    assert_eq!(c.wrap_char, None);
    assert_eq!(notes(&report), ["Roads: stacking at , keeping it, drawn without"]);

    // No stacking: nothing, nothing noted.
    for class in [
        json!({"canStackLabel": false, "labelStackingProperties": {"separators": separators}}),
        json!({"labelStackingProperties": {"separators": separators}}),
        json!({"canStackLabel": true}),
    ] {
        let (c, report) = read_class(class, json!({}));
        assert_eq!(c.wrap_char, None);
        assert!(report.notes.is_empty(), "{:?}", notes(&report));
    }
    // An empty separator is no character to break at; a long one is cut; a line break is no character.
    let odd = json!([{"separator": ""}, {"separator": "\n"}, {"separator": 5}, {"separator": "0123456789"}]);
    let (c, _) = read_class(json!({"canStackLabel": true, "labelStackingProperties": {"separators": odd}}), json!({}));
    assert_eq!(c.wrap_char.as_deref(), Some("01234567"));
    // Cut by characters, not bytes: ten of a three-byte character leave eight whole ones.
    let wide = json!([{"separator": "\u{2192}".repeat(10)}]);
    let (c, _) = read_class(json!({"canStackLabel": true, "labelStackingProperties": {"separators": wide}}), json!({}));
    assert_eq!(c.wrap_char, Some("\u{2192}".repeat(8)));
}

#[test]
fn a_label_whose_expression_fills_the_stacking_lines_is_not_stacked_further() {
    // Pro stacks a label into at most `maximumNumberOfLines` lines, its own line breaks
    // counted: an expression that already makes that many lines is not broken at a separator.
    let stacking = |max: serde_json::Value| {
        maplex(json!({"canStackLabel": true, "labelStackingProperties": {"maximumNumberOfLines": max,
            "separators": [{"separator": " ", "splitAfter": true}, {"separator": ",", "visible": true, "splitAfter": true}]}}))
    };
    let class = |expression: &str, max: serde_json::Value| {
        let mut c = stacking(max);
        if let Some(o) = c.as_object_mut() {
            o.insert("expression".into(), json!(expression));
        }
        read_class(c, json!({})).0
    };
    let three = "$feature.A + TextFormatting.NewLine + $feature.B + \" \" + $feature.C + TextFormatting.NewLine + $feature.D";
    assert_eq!(class(three, json!(3)).wrap_char, None, "three lines of three: nothing to stack");
    assert_eq!(class(three, json!(2)).wrap_char, None, "more lines than the limit");
    assert_eq!(class(three, json!(4)).wrap_char.as_deref(), Some(" "), "room for one more line");
    assert_eq!(class("$feature.A + \" \" + $feature.B", json!(3)).wrap_char.as_deref(), Some(" "));
    // No limit written: stacked as before.
    let mut c = maplex(json!({"canStackLabel": true, "labelStackingProperties": {"separators": [{"separator": " "}]}}));
    if let Some(o) = c.as_object_mut() {
        o.insert("expression".into(), json!(three));
    }
    assert_eq!(read_class(c, json!({})).0.wrap_char.as_deref(), Some(" "));
}

// ---- the layer tree, sources and layer properties ----

const CITY_GDB: &str = r"D:\data\city.gdb";

/// A layer definition of `ty` named `name`, at `CIMPATH=new_detail_map/<name>.json`, with `extra` keys.
fn layer_of(ty: &str, name: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut l = json!({"type": ty, "name": name, "uRI": format!("CIMPATH=new_detail_map/{}.json", name.to_lowercase())});
    if let (Some(o), Some(e)) = (l.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    l
}

/// The `CIMPATH=…` of the layers `layer_of` and `feature_layer` name `names`.
fn uris_of(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| format!("CIMPATH=new_detail_map/{}.json", n.to_lowercase())).collect()
}

/// A map listing `layers` (all of them, in order).
fn read_listed(layers: &[serde_json::Value]) -> (MapImport, Report) {
    let names: Vec<&str> = layers.iter().filter_map(|l| l["name"].as_str()).collect();
    let uris = uris_of(&names);
    let uris: Vec<&str> = uris.iter().map(String::as_str).collect();
    read_new_detail_map(&map_document(layers, &uris))
}

#[test]
fn groups_nest_with_their_own_visibility_and_settings() {
    let shp = |name: &str| feature_layer(name, standard_connection("Shapefile", r"DATABASE=D:\data", &format!("{name}.shp")), json!({}));
    let roads = shp("roads");
    let parcels = shp("parcels");
    let inner = layer_of("CIMGroupLayer", "Inner", json!({"visibility": true, "layers": ["CIMPATH=new_detail_map/parcels.json"]}));
    let outer = layer_of(
        "CIMGroupLayer",
        "Outer",
        json!({"visibility": false, "expanded": true, "transparency": 20, "minScale": 1e6, "maxScale": 1000,
            "layers": ["CIMPATH=new_detail_map/roads.json", "CIMPATH=new_detail_map/inner.json"]}),
    );
    let (map, report) = read_new_detail_map(&map_document(&[outer, inner, roads, parcels], &["CIMPATH=new_detail_map/outer.json"]));
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    let [Entry::Group(g)] = map.entries.as_slice() else { panic!("{:?}", map.entries) };
    assert_eq!((g.name.as_str(), g.visible, g.expanded, g.transparency), ("Outer", false, true, Some(20.0)));
    let scale = crate::qgis::QGIS_SCALE;
    assert!(g.min_scale.is_some_and(|s| (s - 1e6 * scale).abs() < 1e-6), "{:?}", g.min_scale);
    assert!(g.max_scale.is_some_and(|s| (s - 1000.0 * scale).abs() < 1e-6), "{:?}", g.max_scale);
    let [Entry::Layer(l), Entry::Group(sub)] = g.children.as_slice() else { panic!("{:?}", g.children) };
    // A layer's visibility is its own: the hidden group hides it, it isn't hidden itself.
    assert_eq!((l.name.as_str(), l.visible), ("roads", true));
    assert_eq!((sub.name.as_str(), sub.visible, sub.expanded, sub.transparency, sub.min_scale), ("Inner", true, false, None, None));
    let [Entry::Layer(p)] = sub.children.as_slice() else { panic!("{:?}", sub.children) };
    assert_eq!(p.name, "parcels");
}

#[test]
fn every_connection_kind_has_a_source_and_an_origin() {
    use LayerSource::*;
    let fl = |name: &str, c: serde_json::Value| feature_layer(name, c, json!({}));
    let gdb = format!("DATABASE={CITY_GDB}");
    let cases: Vec<(serde_json::Value, LayerSource, &str, LayerKind)> = vec![
        (
            fl("Shp", standard_connection("Shapefile", r"DATABASE=D:\data", "roads.shp")),
            File { path: r"D:\data\roads.shp".into(), table: None },
            r"Shapefile roads.shp in D:\data",
            LayerKind::Feature,
        ),
        (
            fl("Gdb", standard_connection("FileGDB", &gdb, "parcels")),
            File { path: CITY_GDB.into(), table: Some("parcels".into()) },
            r"FileGDB parcels in D:\data\city.gdb",
            LayerKind::Feature,
        ),
        (
            fl(
                "InDataset",
                json!({"type": "CIMFeatureDatasetDataConnection", "workspaceConnectionString": gdb, "workspaceFactory": "FileGDB",
                    "featureDataset": "transport", "dataset": "rails", "datasetType": "esriDTFeatureClass"}),
            ),
            File { path: CITY_GDB.into(), table: Some("rails".into()) },
            r"FileGDB rails in D:\data\city.gdb",
            LayerKind::Feature,
        ),
        (
            layer_of("CIMRasterLayer", "Photo", json!({"dataConnection": standard_connection("Raster", r"DATABASE=.\IMG", "photo.tif")})),
            File { path: r".\IMG\photo.tif".into(), table: None },
            r"Raster photo.tif in .\IMG",
            LayerKind::Raster,
        ),
        (
            fl(
                "Cad",
                json!({"type": "CIMFeatureDatasetDataConnection", "workspaceConnectionString": r"DATABASE=D:\data\cad", "workspaceFactory": "Cad",
                    "featureDataset": "site.dwg", "dataset": "Polyline", "datasetType": "esriDTFeatureClass"}),
            ),
            File { path: r"D:\data\cad\site.dwg".into(), table: Some("Polyline".into()) },
            r"Cad site.dwg:Polyline in D:\data\cad",
            LayerKind::Feature,
        ),
        (
            layer_of("CIMKMLLayer", "Sites", json!({"dataConnection": {"type": "CIMKMLDataConnection", "uRI": r"D:\data\sites.kmz"}})),
            File { path: r"D:\data\sites.kmz".into(), table: None },
            r"KML D:\data\sites.kmz",
            LayerKind::Feature,
        ),
        (
            fl("Service", standard_connection("FeatureService", "URL=https://example.org/arcgis/rest/services/Parcels/FeatureServer", "0")),
            FeatureService { url: "https://example.org/arcgis/rest/services/Parcels/FeatureServer".into(), layer: "0".into() },
            "ArcGIS feature service https://example.org/arcgis/rest/services/Parcels/FeatureServer layer 0",
            LayerKind::Feature,
        ),
        (
            layer_of(
                "CIMTiledServiceLayer",
                "Base",
                json!({"serviceConnection": {"type": "CIMAGSServiceConnection", "objectName": "Base", "objectType": "MapServer",
                    "url": "https://example.org/arcgis/rest/services/Base/MapServer"}}),
            ),
            MapService { url: "https://example.org/arcgis/rest/services/Base/MapServer".into() },
            "ArcGIS map service https://example.org/arcgis/rest/services/Base/MapServer",
            LayerKind::Tiles,
        ),
        (
            layer_of(
                "CIMImageServiceLayer",
                "Elevation",
                json!({"serviceConnection": {"type": "CIMAGSServiceConnection", "objectName": "Elevation", "objectType": "ImageServer",
                    "serverConnection": {"type": "CIMInternetServerConnection", "url": "https://example.org/arcgis/rest/services/"}}}),
            ),
            MapService { url: "https://example.org/arcgis/rest/services/Elevation/ImageServer".into() },
            "ArcGIS image service https://example.org/arcgis/rest/services/Elevation/ImageServer",
            LayerKind::Tiles,
        ),
        (
            layer_of(
                "CIMWMSLayer",
                "Wms",
                json!({"serviceConnection": {"type": "CIMWMSServiceConnection",
                        "serverConnection": {"type": "CIMInternetServerConnection", "url": "https://example.org/wms"}},
                    "subLayers": [{"type": "CIMWMSSubLayer", "name": "hidden", "visibility": false},
                        {"type": "CIMWMSSubLayer", "name": "roads", "visibility": true}]}),
            ),
            Wms { url: "https://example.org/wms".into(), layer: "roads".into() },
            "WMS https://example.org/wms layer roads",
            LayerKind::Tiles,
        ),
        (
            // A root with no name holds the layers: the first that shows is looked for inside it.
            layer_of(
                "CIMWMSLayer",
                "WmsNested",
                json!({"serviceConnection": {"type": "CIMWMSServiceConnection",
                        "serverConnection": {"type": "CIMInternetServerConnection", "url": "https://example.org/wms"}},
                    "subLayers": [{"type": "CIMWMSSubLayer", "visibility": true, "subLayers": [
                        {"type": "CIMWMSSubLayer", "name": "off", "visibility": false},
                        {"type": "CIMWMSSubLayer", "name": "rivers", "visibility": true}]}]}),
            ),
            Wms { url: "https://example.org/wms".into(), layer: "rivers".into() },
            "WMS https://example.org/wms layer rivers",
            LayerKind::Tiles,
        ),
        (
            // As Pro writes it: a named composite sublayer (the service's root, not a layer the
            // server draws) holding the sublayers, each with its title as `name` and the WMS
            // layer name as `subLayerID`.
            layer_of(
                "CIMWMSLayer",
                "WmsPro",
                json!({"serviceConnection": {"type": "CIMWMSServiceConnection", "version": "1.3.0",
                        "serverConnection": {"type": "CIMInternetServerConnection", "anonymous": true, "url": "https://example.org/wms"}},
                    "subLayers": [{"type": "CIMServiceCompositeSubLayer", "name": "service_root", "visibility": true, "subLayers": [
                        {"type": "CIMWMSSubLayer", "name": "Hidden Title", "subLayerID": "0", "visibility": false},
                        {"type": "CIMWMSSubLayer", "name": "Parcels Title", "subLayerID": "1", "visibility": true}]}]}),
            ),
            Wms { url: "https://example.org/wms".into(), layer: "1".into() },
            "WMS https://example.org/wms layer 1",
            LayerKind::Tiles,
        ),
        (
            layer_of(
                "CIMWMSLayer",
                "WmsHidden",
                json!({"serviceConnection": {"type": "CIMWMSServiceConnection",
                        "serverConnection": {"type": "CIMInternetServerConnection", "url": "https://example.org/wms"}},
                    "subLayers": [{"type": "CIMWMSSubLayer", "name": "off", "visibility": false},
                        {"type": "CIMWMSSubLayer", "visibility": false, "subLayers": [{"type": "CIMWMSSubLayer", "name": "in"}]}]}),
            ),
            Unopenable,
            "WMS https://example.org/wms",
            LayerKind::Tiles,
        ),
        (
            layer_of(
                "CIMVectorTileLayer",
                "Streets",
                json!({"dataConnection": {"type": "CIMVectorTileDataConnection",
                    "uRI": "https://example.org/arcgis/rest/services/Streets/VectorTileServer"}}),
            ),
            VectorTiles { url: "https://example.org/arcgis/rest/services/Streets/VectorTileServer".into() },
            "vector tiles https://example.org/arcgis/rest/services/Streets/VectorTileServer",
            LayerKind::Tiles,
        ),
        (
            fl("Sde", standard_connection("SDE", "SERVER=s;INSTANCE=sde:sqlserver:s;DATABASE=d;VERSION=sde.DEFAULT", "d.dbo.parcels")),
            Unopenable,
            "an SDE geodatabase on s, database d, table d.dbo.parcels",
            LayerKind::Feature,
        ),
        (
            fl("Excel", standard_connection("Excel", r"DATABASE=D:\data\book.xlsx", "Sheet1$")),
            Unopenable,
            r"Excel Sheet1$ in D:\data\book.xlsx",
            LayerKind::Feature,
        ),
        (
            fl(
                "Sql",
                json!({"type": "CIMSqlQueryDataConnection", "workspaceFactory": "SDE", "workspaceConnectionString": "SERVER=s;DATABASE=d",
                    "dataset": "d.dbo.q", "sqlQuery": "select * from t"}),
            ),
            Unopenable,
            "SQL query d.dbo.q on an SDE geodatabase on s, database d",
            LayerKind::Feature,
        ),
        (
            fl(
                "Joined",
                json!({"type": "CIMRelQueryTableDataConnection", "sourceTable": standard_connection("FileGDB", &gdb, "parcels"),
                    "destinationTable": standard_connection("FileGDB", &gdb, "owners"), "primaryKey": "PID", "foreignKey": "PID"}),
            ),
            File { path: CITY_GDB.into(), table: Some("parcels".into()) },
            r"FileGDB parcels in D:\data\city.gdb",
            LayerKind::Feature,
        ),
    ];
    let layers: Vec<serde_json::Value> = cases.iter().map(|c| c.0.clone()).collect();
    let (map, report) = read_listed(&layers);
    let got = flat(&map);
    assert_eq!(got.len(), cases.len());
    for (l, (_, source, origin, kind)) in got.into_iter().zip(&cases) {
        assert_eq!((&l.source, l.origin.as_str(), l.kind), (source, *origin, *kind), "{}", l.name);
    }
    let n = notes(&report);
    for expected in [
        "Sde: data in SDE workspaces, which RustGIS can't open",
        "Excel: data in Excel workspaces, which RustGIS can't open",
        "Sql: SQL query layers, which RustGIS can't open",
        "Joined: a join to owners, drawn without it",
        "WmsHidden: a WMS layer that names no visible sublayer, nothing to draw",
    ] {
        assert!(n.iter().any(|t| t == expected), "{expected}\n{n:?}");
    }
    // Each of the others opens as it is, without a word.
    assert_eq!(n.len(), 5, "{n:?}");
}

#[test]
fn origins_never_carry_passwords() {
    let hex = "00022e684d4a6b5a4e6a4d4f6f3d2a";
    let secret = format!(
        "ENCRYPTED_PASSWORD_UTF8={hex};ENCRYPTED_PASSWORD={hex};SERVER=s;INSTANCE=sde:sqlserver:s;DATABASE=d;USER=sa;VERSION=sde.DEFAULT"
    );
    let scrubbed = map::scrub_connection(&format!("{secret};PASSWORD=pw;TOKEN=tk;password=lower"));
    assert_eq!(scrubbed, "SERVER=s;INSTANCE=sde:sqlserver:s;DATABASE=d;VERSION=sde.DEFAULT");

    let sde = feature_layer("Sde", standard_connection("SDE", &secret, "d.dbo.parcels"), json!({}));
    let sql = feature_layer(
        "Sql",
        json!({"type": "CIMSqlQueryDataConnection", "workspaceFactory": "SDE", "workspaceConnectionString": secret, "dataset": "d.dbo.q",
            "sqlQuery": "select 1"}),
        json!({}),
    );
    let service = feature_layer(
        "Service",
        standard_connection(
            "FeatureService",
            "URL=https://example.org/arcgis/rest/services/P/FeatureServer?token=tk123&f=json;TOKEN=tk123",
            "3",
        ),
        json!({}),
    );
    let kml = layer_of(
        "CIMKMLLayer",
        "Kml",
        json!({"dataConnection": {"type": "CIMKMLDataConnection", "uRI": "https://example.org/sites.kmz?token=tk123"}}),
    );
    let (map, report) = read_listed(&[sde, sql, service, kml]);
    let layers = flat(&map);
    let origin = &layers[0].origin;
    for part in ["s", "d", "d.dbo.parcels"] {
        assert!(origin.contains(part), "{origin}");
    }
    let n = notes(&report);
    for text in layers.iter().map(|l| &l.origin).chain(&n) {
        for leak in ["PASSWORD", "USER", "sa", hex, "tk123"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
    }
    let url = "https://example.org/arcgis/rest/services/P/FeatureServer?f=json";
    assert_eq!(layers[2].source, LayerSource::FeatureService { url: url.into(), layer: "3".into() });
    assert_eq!(layers[3].source, LayerSource::File { path: "https://example.org/sites.kmz".into(), table: None });
}

#[test]
fn layer_properties_and_the_map_reference_scale_carry_over() {
    use crate::effects::Blend;
    let shp = |name: &str| standard_connection("Shapefile", r"DATABASE=D:\data", &format!("{name}.shp"));
    let mut roads = feature_layer("roads", shp("roads"), json!({"blendingMode": "Multiply", "expanded": false, "scaleSymbols": false}));
    roads["featureTable"]["definitionExpression"] = json!("LANES > 2");
    let plain = feature_layer("plain", shp("plain"), json!({"blendingMode": "Alpha", "expanded": true}));
    let burnt = feature_layer("burnt", shp("burnt"), json!({"blendingMode": "ColorBurn"}));
    let busy = feature_layer(
        "busy",
        shp("busy"),
        json!({"enableDisplayFilters": true, "displayFilters": [{"type": "CIMDisplayFilter", "name": "f"}],
            "layerMasks": ["CIMPATH=new_detail_map/roads.json"], "featureReduction": {"type": "CIMFeatureReductionCluster"},
            "symbolLayerDrawing": {"type": "CIMSymbolLayerDrawing", "useSymbolLayerDrawing": true}, "charts": [{"type": "CIMChart"}],
            "popupInfo": {"type": "CIMPopupInfo"}, "timeFilter": {"type": "CIMTimeFilter"},
            "selectionSymbol": symbol_ref(polygon_symbol(vec![solid_fill(rgb(0, 255, 255))]))}),
    );
    // Display filters that are off, an empty list and a symbol layer drawing that isn't used say nothing.
    let quiet = feature_layer(
        "quiet",
        shp("quiet"),
        json!({"displayFilters": [{"type": "CIMDisplayFilter", "name": "f"}], "layerMasks": [],
            "symbolLayerDrawing": {"type": "CIMSymbolLayerDrawing", "useSymbolLayerDrawing": false}}),
    );
    let layers = [roads, plain, burnt, busy, quiet];
    let uris = uris_of(&["roads", "plain", "burnt", "busy", "quiet"]);
    let uris: Vec<&str> = uris.iter().map(String::as_str).collect();
    let (map, report) = read_new_detail_map(&map_document_with(&layers, &uris, json!({"referenceScale": 24000}), json!({})));
    assert_eq!(map.reference_scale, Some(24000.0 * crate::qgis::QGIS_SCALE));
    let l = flat(&map);
    assert_eq!(
        (l[0].style.blend, l[0].definition_query.as_deref(), l[0].expanded, l[0].scale_symbols),
        (Some(Blend::Multiply), Some("LANES > 2"), false, false)
    );
    assert_eq!(
        (l[1].style.blend, l[1].definition_query.as_deref(), l[1].expanded, l[1].scale_symbols),
        (Some(Blend::Normal), None, true, true)
    );
    assert_eq!(l[2].style.blend, None);
    let n = notes(&report);
    assert!(n.iter().any(|t| t == "burnt: its blending mode ColorBurn, drawn as normal"), "{n:?}");
    let busy: Vec<&crate::qgis::Note> = report.notes.iter().filter(|n| n.layer == "busy").collect();
    assert_eq!(busy.len(), 8, "{n:?}");
    assert!(busy.iter().all(|n| n.count == 1), "{n:?}");
    for part in [
        "display filters",
        "layer masks",
        "feature reduction",
        "symbol layer drawing",
        "charts",
        "pop-up",
        "time filter",
        "selection symbol",
    ] {
        assert!(busy.iter().any(|n| n.text.contains(part)), "{part}\n{n:?}");
    }
    assert!(!n.iter().any(|t| t.starts_with("quiet: ") || t.starts_with("roads: ") || t.starts_with("plain: ")), "{n:?}");
}

#[test]
fn other_layer_types_and_tables_are_read() {
    let gdb = format!("DATABASE={CITY_GDB}");
    let annotation = layer_of(
        "CIMAnnotationLayer",
        "Notes",
        json!({"featureTable": {"type": "CIMAnnotationTable", "dataConnection": standard_connection("FileGDB", &gdb, "notes_anno")},
            "popupInfo": {"type": "CIMPopupInfo"}, "blendingMode": "ColorBurn"}),
    );
    let dimension = layer_of("CIMDimensionLayer", "Dims", json!({}));
    let graph = layer_of("CIMKnowledgeGraphLayer", "Graph", json!({}));
    let owners = json!({"type": "CIMStandaloneTable", "name": "Owners", "uRI": "CIMPATH=new_detail_map/owners.json",
        "dataConnection": standard_connection("FileGDB", &gdb, "owners")});
    let layers = [annotation, dimension, graph];
    let uris = uris_of(&["Notes", "Dims", "Graph"]);
    let uris: Vec<&str> = uris.iter().map(String::as_str).collect();
    let doc = map_document_with(
        &layers,
        &uris,
        json!({"standaloneTables": ["CIMPATH=new_detail_map/owners.json", "CIMPATH=new_detail_map/gone.json"]}),
        json!({"tableDefinitions": [owners]}),
    );
    let (map, report) = read_new_detail_map(&doc);
    let l = flat(&map);
    assert_eq!(l.len(), 3);
    assert!(l.iter().all(|l| l.source == LayerSource::Unopenable && l.kind == LayerKind::Other));
    assert_eq!(l[0].origin, r"FileGDB notes_anno in D:\data\city.gdb", "where it was, for people");
    assert_eq!(l[1].origin, "CIMDimensionLayer");
    let n = notes(&report);
    for expected in [
        "Notes: CIMAnnotationLayer layers, which RustGIS doesn't draw",
        "Dims: CIMDimensionLayer layers, which RustGIS doesn't draw",
        "Graph: CIMKnowledgeGraphLayer layers, which RustGIS doesn't draw",
    ] {
        assert!(n.iter().any(|t| t == expected), "{expected}\n{n:?}");
    }
    assert_eq!(
        map.tables,
        [TableImport {
            name: "Owners".into(),
            source: LayerSource::File { path: CITY_GDB.into(), table: Some("owners".into()) },
            origin: r"FileGDB owners in D:\data\city.gdb".into(),
        }]
    );
    assert!(n.iter().any(|t| t.contains("gone.json")), "{n:?}");
    // A layer RustGIS doesn't draw has its one note, nothing about its pop-up or blending.
    assert_eq!(n.iter().filter(|t| t.starts_with("Notes: ")).count(), 1, "{n:?}");
}

#[test]
fn a_lyrx_reads_its_layers() {
    let shp = |name: &str| {
        let mut l = feature_layer(name, standard_connection("Shapefile", r"DATABASE=D:\data", &format!("{name}.shp")), json!({}));
        l["uRI"] = json!(format!("CIMPATH=lyr/{name}.json"));
        l
    };
    let group = json!({"type": "CIMGroupLayer", "name": "Transport", "uRI": "CIMPATH=lyr/transport.json", "visibility": true,
        "layers": ["CIMPATH=lyr/roads.json", "CIMPATH=lyr/rails.json"]});
    let text = json!({"type": "CIMLayerDocument", "version": "3.7.0", "layers": ["CIMPATH=lyr/transport.json"],
        "layerDefinitions": [group, shp("roads"), shp("rails")]})
    .to_string();
    let doc = Document::from_json(&text).unwrap();
    let mut report = Report::default();
    let map = read_layers(&doc, &doc.layer_uris(), &mut report);
    assert!(report.notes.is_empty(), "{:?}", notes(&report));
    assert_eq!((map.name.as_str(), map.wkid, map.extent, map.reference_scale), ("", None, None, None));
    assert!(map.tables.is_empty());
    let [Entry::Group(g)] = map.entries.as_slice() else { panic!("{:?}", map.entries) };
    assert_eq!(g.name, "Transport");
    let names: Vec<&str> = g.children.iter().filter_map(|e| if let Entry::Layer(l) = e { Some(l.name.as_str()) } else { None }).collect();
    assert_eq!(names, ["roads", "rails"]);
    assert_eq!(file_of(nth(&map, 1)), (Some(r"D:\data\rails.shp"), None));
}
