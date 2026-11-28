//! A layout picture's file: which path it keeps, and what of it it shows.

use super::*;

/// The most bytes of a picture's file path kept: a longer one is dropped, never cut (a cut path
/// names another file).
pub const MAX_PICTURE_SOURCE: usize = 4096;

/// Whether a picture keeps `path` as its file: when it isn't blank, holds no `://` (an address,
/// which could carry a login), no control character and none of U+202A–U+202E or U+2066–U+2069
/// (which turn the direction of text), and is at most [`MAX_PICTURE_SOURCE`] bytes. A kept path is
/// kept exactly as given, neither trimmed nor resolved. A name holding a colon (`a:b.png`) is kept.
pub fn picture_source_kept(path: &str) -> bool {
    !path.trim().is_empty()
        && path.len() <= MAX_PICTURE_SOURCE
        && !path.contains("://")
        && !path.chars().any(|c| c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
}

/// The name a picture shows of its file `source`: its last part after `/` or `\` that isn't blank,
/// trimmed; none when every part is blank. The folders are never shown.
pub fn picture_file_name(source: &str) -> Option<&str> {
    source.rsplit(['/', '\\']).map(str::trim).find(|part| !part.is_empty())
}

/// A picture's file as a file holds it: text as it is (whether it is kept is for `Layout::clamp`),
/// null as none, anything else (a number, a flag, a list, an object) as a blank, which
/// [`picture_source_kept`] never keeps.
pub(crate) fn de_picture_source<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v: Option<serde_json::Value> = Deserialize::deserialize(d)?;
    Ok(match v {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s),
        Some(_) => Some(String::new()),
    })
}

/// Whether a save leaves a picture's file out: none, or one [`picture_source_kept`] doesn't keep.
pub(crate) fn picture_source_unkept(s: &Option<String>) -> bool {
    !s.as_deref().is_some_and(picture_source_kept)
}

impl ElementKind {
    /// A picture's file, when it has one [`picture_source_kept`] keeps; none for anything else.
    pub fn picture_source(&self) -> Option<&str> {
        match self {
            ElementKind::Picture { source, .. } => source.as_deref().filter(|s| picture_source_kept(s)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A picture saved before keeps its bytes; one with a file writes it after its image and reads
    /// it back.
    #[test]
    fn pictures_without_a_file_save_as_before() {
        for plain in [r#"{"Picture":{"image":""}}"#, r#"{"Picture":{"image":"iVBORw0KGgo="}}"#] {
            let k: ElementKind = serde_json::from_str(plain).unwrap();
            assert_eq!(serde_json::to_string(&k).unwrap(), plain);
            assert_eq!(k.picture_source(), None);
        }
        let with = r#"{"Picture":{"image":"","source":"/data/logos/north.png"}}"#;
        let k: ElementKind = serde_json::from_str(with).unwrap();
        assert_eq!(k.picture_source(), Some("/data/logos/north.png"));
        assert_eq!(serde_json::to_string(&k).unwrap(), with);
    }

    /// What of a path is kept: not blank, not an address, no control or direction-override
    /// characters, at most MAX_PICTURE_SOURCE bytes; a POSIX name with a colon is kept.
    #[test]
    fn a_pictures_file_is_kept_only_when_it_is_a_plain_path() {
        for kept in ["/data/logos/north.png", r"\\server\share\logo.jpg", "logo.png", "a:b.png", " spaced .png", "logo@2x.png"] {
            assert!(picture_source_kept(kept), "{kept}");
        }
        let long = format!("/{}", "d".repeat(MAX_PICTURE_SOURCE));
        for not in [
            "",
            "   ",
            "https://user:secret@example.invalid/logo.png",
            "file:///data/x.png",
            "a\u{7}b.png",
            "a\nb.png",
            "x\u{202E}gnp.png",
            long.as_str(),
        ] {
            assert!(!picture_source_kept(not), "{not:?}");
        }
        assert!(picture_source_kept(&format!("/{}", "d".repeat(MAX_PICTURE_SOURCE - 1))));
    }

    /// The name a picture shows of its file: the last part, never the folders.
    #[test]
    fn picture_file_names_hide_their_folders() {
        for (source, name) in [
            ("/data/logos/north.png", Some("north.png")),
            (r"\\server\share\logo.jpg", Some("logo.jpg")),
            (r"C:\maps\inset.png", Some("inset.png")),
            ("logo.png", Some("logo.png")),
            ("dir/", Some("dir")),
            ("  /x/ name.png ", Some("name.png")),
            ("///", None),
            ("", None),
        ] {
            assert_eq!(picture_file_name(source), name, "{source}");
        }
    }

    /// A hand-edited file's source of the wrong kind, blank, an address, too long or with control
    /// characters loads as none, counted once per element; the others are kept or none and not
    /// counted; nothing dropped is written back.
    #[test]
    fn a_pictures_file_is_saved_when_kept_and_read_into_range() {
        let long = format!("/{}", "d".repeat(5000));
        let sources = [
            Some(json!("/data/logos/north.png")),                     // 1: kept
            Some(json!(5)),                                           // 2: the wrong kind, dropped and counted
            Some(json!(["a.png"])),                                   // 3: the wrong kind, dropped and counted
            Some(json!("")),                                          // 4: blank, dropped and counted
            Some(json!("https://user:secret@example.invalid/a.png")), // 5: an address
            Some(json!(long)),                                        // 6: too long, dropped and counted
            Some(json!("a\u{7}b.png")),                               // 7: a control character, dropped and counted
            Some(serde_json::Value::Null),                            // 8: none, not counted
            None,                                                     // 9: absent, none, not counted
        ];
        let elements: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(i, source)| {
                let mut picture = json!({"image": ""});
                if let Some(s) = source {
                    picture.as_object_mut().unwrap().insert("source".into(), s.clone());
                }
                json!({"id": i + 1, "name": "p", "x": 0, "y": 0, "w": 50, "h": 50, "kind": {"Picture": picture}})
            })
            .collect();
        let mut l: Layout = serde_json::from_value(json!({"name": "x", "width": 297, "height": 210, "elements": elements})).unwrap();
        let repairs = l.clamp();
        let source = |id| match &l.element(id).unwrap().kind {
            ElementKind::Picture { source, .. } => source.clone(),
            k => panic!("{k:?}"),
        };
        assert_eq!(source(1).as_deref(), Some("/data/logos/north.png"));
        for id in 2..=9 {
            assert_eq!(source(id), None, "{id}");
        }
        assert_eq!((repairs.elements_adjusted, repairs.elements_cut), (6, 0));
        let saved = serde_json::to_string(&l).unwrap();
        assert!(saved.matches("\"source\"").count() == 1 && !saved.contains("secret"), "{saved}");
        // An unclamped junk source writes nothing.
        let junk: ElementKind = serde_json::from_str(r#"{"Picture":{"image":"","source":7}}"#).unwrap();
        assert_eq!(serde_json::to_string(&junk).unwrap(), r#"{"Picture":{"image":""}}"#);
        assert_eq!(junk.picture_source(), None);
    }
}
