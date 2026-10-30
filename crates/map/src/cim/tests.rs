//! Small CIM documents, written by hand in the shapes ArcGIS Pro writes them.

use super::*;

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
