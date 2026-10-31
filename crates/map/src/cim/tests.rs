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
