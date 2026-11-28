//! A layout picture's file: which path it keeps, and what of it it shows.

use super::*;

/// The most bytes of a picture's file path kept: a longer one is dropped, never cut (a cut path
/// names another file).
pub const MAX_PICTURE_SOURCE: usize = 4096;

/// Whether a picture keeps `path` as its file: when it names a file ([`picture_file_name`]; a path
/// ending in a folder doesn't), isn't an address ([`picture_path_is_address`]; one could carry a
/// login), holds no character [`rustgis_data::io::dbf_notice_char`] flattens in a notice (a control
/// character, U+2028 or U+2029, which break a line, or U+061C, U+200E, U+200F, U+202A–U+202E or
/// U+2066–U+2069, which turn the direction of text), and is at most [`MAX_PICTURE_SOURCE`] bytes.
/// Zero-width characters (U+200B–U+200D) and U+FEFF are kept: they are part of real file names,
/// and none of them breaks a line or reorders text. A kept path is kept exactly as given, neither
/// trimmed nor resolved. A name holding a colon (`a:b.png`) is kept.
pub fn picture_source_kept(path: &str) -> bool {
    path.len() <= MAX_PICTURE_SOURCE
        && picture_file_name(path).is_some()
        && !picture_path_is_address(path)
        && path.chars().all(|c| rustgis_data::io::dbf_notice_char(c) == c)
}

/// Whether `path` reads as an address, not a file: it holds `://` anywhere, or begins (after
/// leading blanks) with a scheme of two or more characters (a letter, then letters, digits, `+`,
/// `-` or `.`), a colon and a `/` or `\`. One letter is a drive (`C:\a.png`), and a name with a
/// colon and no separator after it (`a:b.png`) is a file. A relative path whose first folder
/// ends in a colon reads as an address: give it as `./folder:/a.png`. `mailto:a@b`,
/// `data:image/png;base64,…` and `user:pw@host:/a.png` are left as paths: no rule tells them from
/// a file name holding a colon, and none opens as a file unless it is one.
pub fn picture_path_is_address(path: &str) -> bool {
    if path.contains("://") {
        return true;
    }
    let Some((scheme, rest)) = path.trim_start().split_once(':') else { return false };
    let mut chars = scheme.chars();
    scheme.len() >= 2
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && rest.starts_with(['/', '\\'])
}

/// The name a picture shows of its file `source`: its last part after `/` or `\`, trimmed; none
/// when that is blank, `.` or `..` (the path ends in a folder). The folders are never shown.
pub fn picture_file_name(source: &str) -> Option<&str> {
    source.rsplit(['/', '\\']).next().map(str::trim).filter(|n| !matches!(*n, "" | "." | ".."))
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

    /// The paths a picture keeps as its file: each with a file name; a POSIX name with a colon and
    /// a zero-width character or byte-order mark are kept; a blank, an address, a folder, a control
    /// or direction character, or more than MAX_PICTURE_SOURCE bytes isn't.
    #[test]
    fn a_pictures_file_is_kept_only_when_it_is_a_plain_path() {
        let kept = [
            "/data/logos/north.png",
            r"\\server\share\logo.jpg",
            "logo.png",
            "a:b.png",
            " spaced .png",
            "logo@2x.png",
            "\u{fc}.png",
            "a\u{200D}b.png",
            "a\u{200C}b.png",
            "a\u{200B}b.png",
            "\u{FEFF}logo.png",
        ];
        for path in kept.into_iter().chain(NAMED.iter().map(|(path, _)| *path)) {
            assert!(picture_source_kept(path), "{path:?}");
        }
        let long = format!("/{}", "d".repeat(MAX_PICTURE_SOURCE));
        let not = [
            "https://user:secret@example.invalid/logo.png",
            "file:///data/x.png",
            "a\u{7}b.png",
            "a\nb.png",
            "x\u{202E}gnp.png",
            "a\u{061C}b.png",
            "a\u{200E}b.png",
            "a\u{200F}b.png",
            "logo\u{2028}b.png",
            "logo\u{2029}b.png",
            "a\u{2066}b.png",
            long.as_str(),
        ];
        for path in not.into_iter().chain(UNNAMED).chain(ADDRESSES) {
            assert!(!picture_source_kept(path), "{path:?}");
        }
        assert!(picture_source_kept(&format!("/{}", "d".repeat(MAX_PICTURE_SOURCE - 1))));
    }

    /// Paths, and the name a picture shows of each.
    const NAMED: [(&str, &str); 9] = [
        ("/data/logos/north.png", "north.png"),
        (r"C:\maps\inset.png", "inset.png"),
        (r"\\server\share\logo.jpg", "logo.jpg"),
        ("logo.png", "logo.png"),
        ("  /x/ name.png ", "name.png"),
        ("a:b.png", "a:b.png"),
        ("C:inset.png", "C:inset.png"),
        ("a.png:stream", "a.png:stream"),
        ("...", "..."),
    ];
    /// Paths that end in a folder or in nothing: no name is shown, and none is kept.
    const UNNAMED: [&str; 10] = ["/home/someone/", r"C:\maps\", "dir/", "///", "", "   ", "/data/logos/..", "x/.", "/x/ ", ".."];

    /// The name a picture shows of its file: the last part, never the folders, and none when the
    /// last part is blank, `.` or `..`.
    #[test]
    fn picture_file_names_hide_their_folders() {
        for (source, name) in NAMED {
            assert_eq!(picture_file_name(source), Some(name), "{source:?}");
        }
        for source in UNNAMED {
            assert_eq!(picture_file_name(source), None, "{source:?}");
        }
    }

    /// Paths that read as an address.
    const ADDRESSES: [&str; 9] = [
        "https://user:pw@host/a.png",
        "file:///data/x.png",
        "/vsicurl/https://host/a.png",
        r"https:\\user:pw@host\x.png",
        r"https:\user:pw@host\x",
        "https:/user:pw@host/x.png",
        "file:/data/x.png",
        " s3:/bucket/a.png",
        "ab:/c.png",
    ];

    /// An address is `://` anywhere, or a scheme of two or more characters, a colon and a
    /// separator; a drive, a share, a name with a colon, and the forms no rule can tell from a
    /// file name, are paths.
    #[test]
    fn an_address_is_told_from_a_path() {
        for path in ADDRESSES {
            assert!(picture_path_is_address(path), "{path:?}");
        }
        for path in [
            r"C:\a.png",
            "C:/a.png",
            r"c:\\maps\\a.png",
            r"\\server\share\a.png",
            "//server/share/a.png",
            r"\\?\C:\a.png",
            "/data/a:b/c.png",
            "logos/x:y/a.png",
            "a:b.png",
            "ab:c.png",
            "logo@2x.png",
            "./folder:/a.png",
            "mailto:a@b",
            "data:image/png;base64,AAAA",
            "user:pw@host:/a.png",
            "",
        ] {
            assert!(!picture_path_is_address(path), "{path:?}");
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
