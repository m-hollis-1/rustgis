//! Layout pictures and the files they were read from: what they draw, say, save and require.

#[cfg(not(target_arch = "wasm32"))]
use crate::Session;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::{Value, json};

/// A `w` x `h` PNG, each pixel the opaque colour `colours` gives it, as base64.
#[cfg(not(target_arch = "wasm32"))]
fn png_b64(w: u32, h: u32, colours: impl Fn(u32, u32) -> [u8; 3]) -> String {
    use base64::Engine as _;
    let mut pm = tiny_skia::Pixmap::new(w, h).unwrap();
    for (i, px) in pm.pixels_mut().iter_mut().enumerate() {
        let i = i as u32;
        let [r, g, b] = colours(i % w, i / w);
        *px = tiny_skia::ColorU8::from_rgba(r, g, b, 255).premultiply();
    }
    base64::engine::general_purpose::STANDARD.encode(pm.encode_png().unwrap())
}

/// A 32 x 32 RGB JPEG of a diagonal gradient at quality 80, as base64.
#[cfg(not(target_arch = "wasm32"))]
fn jpeg_b64() -> String {
    use base64::Engine as _;
    let rgb: Vec<u8> = (0..32u32 * 32).flat_map(|i| [((i % 32 + i / 32) * 4) as u8, 120, (255 - (i % 32 + i / 32) * 4) as u8]).collect();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80).encode(&rgb, 32, 32, image::ExtendedColorType::Rgb8).unwrap();
    base64::engine::general_purpose::STANDARD.encode(jpeg)
}

/// The PNG the render pins draw: 40 x 20, its left half `#1F5C84`, its right half `#E8B04A`, a
/// white line one pixel high across the middle.
#[cfg(not(target_arch = "wasm32"))]
fn pin_png() -> String {
    png_b64(40, 20, |x, y| match (x, y) {
        (_, 10) => [0xFF, 0xFF, 0xFF],
        (0..20, _) => [0x1F, 0x5C, 0x84],
        _ => [0xE8, 0xB0, 0x4A],
    })
}

/// What [`pictures_save_and_say_as_before`] saved before pictures remembered their file, in turn.
#[cfg(not(target_arch = "wasm32"))]
const PINNED_PICTURE_SAVES: [u64; 2] = [
    0xc7c7_77d9_64d6_2abc, // the project's layouts
    0x5d70_a4b1_1043_607d, // the layout's own file
];
/// What each export of its layout said, in the order of the routes: where it wrote the file, and
/// nothing of its pictures.
#[cfg(not(target_arch = "wasm32"))]
const PINNED_PICTURE_NOTICES: [u64; 6] = [
    0x91fe_c13a_e3f0_46f7, // layout.export png: "Exported <dir>/p.png"
    0x91dc_c03a_e3d3_5ede, // layout.export pdf: "Exported <dir>/p.pdf"
    0xaba9_5c3a_f262_802e, // layout.export svg: "Exported <dir>/p.svg"
    0xcbf2_9ce4_8422_2325, // the Assistant's view: nothing at all
    0x39f5_2536_3dfd_3cbd, // report.export png: "Exported <dir>/r.png"
    0x39e0_3436_3dea_fa30, // report.export pdf: "Exported <dir>/r.pdf"
];

/// What main saves for pictures not read from a file, and says when a layout of pictures that draw
/// is exported every way, before pictures remembered their file; in the built-in face, in a process
/// of its own. The layout is an older file's, imported (a hidden picture with no image, which only
/// a file holds), with a PNG, a JPEG and the PNG on a background added to it. The saves, each
/// pinned on its own: the project's layouts as JSON, and the layout's own file (header and all).
/// The exports, each pinned on its own: PNG, PDF and SVG by `layout.export`, the Assistant's view
/// of the layout, and a report of it as PNG and as PDF. The test's folder is taken out of what is
/// said before it is hashed. Then what `layout.addElement` answers for a picture's `image` and
/// `path`, case by case, word for word.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn pictures_save_and_say_as_before() {
    use base64::Engine as _;
    if std::env::var_os("RUSTGIS_PICTURE_NOTICE_PIN_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::layout_pictures::pictures_save_and_say_as_before", "--nocapture"])
            .env("RUSTGIS_PICTURE_NOTICE_PIN_CHILD", "1")
            .env("RUSTGIS_BUILTIN_FONTS", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "the test didn't run in the child process");
        return;
    }
    // Each hash that isn't its pinned one, named, with both values and all of them to pin.
    let mismatches = |what: &str, got: &[(String, u64)], pinned: &[u64]| {
        let mut wrong: Vec<String> = got
            .iter()
            .zip(pinned)
            .filter(|((_, g), p)| g != *p)
            .map(|((name, g), p)| format!("{what} {name}: got {g:#018x}, pinned {p:#018x}"))
            .collect();
        if got.len() != pinned.len() || !wrong.is_empty() {
            let all: Vec<String> = got.iter().map(|(name, g)| format!("{g:#018x}, // {name}")).collect();
            wrong.push(format!("{what}s, {} of {} pinned, all:\n{}", got.len(), pinned.len(), all.join("\n")));
        }
        wrong
    };
    /// Removed however the test ends.
    struct Dir(std::path::PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("rustgis-pictures-save-and-say-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let at = |file: &str| dir.0.join(file).display().to_string();
    // The folder, as said before a file's name in it: taken out of what is said, so the same
    // text is pinned on every system.
    let folder = format!("{}{}", dir.0.display(), std::path::MAIN_SEPARATOR);
    let mut s = super::world();
    // An older file's layout, A4 upright: a hidden picture with no image.
    let old = json!({"name": "Pictures", "width": 210.0, "height": 297.0, "elements": [
        {"id": 1, "name": "Old picture", "x": 10.0, "y": 10.0, "w": 40.0, "h": 20.0, "visible": false, "kind": {"Picture": {"image": ""}}}
    ]});
    s.run("layout.import", &json!({"text": old.to_string()})).unwrap();
    let (png, jpeg) = (pin_png(), jpeg_b64());
    let backed = json!({"fill": "#FFF3D6", "outline": "#222222", "width": 1, "padding": 2});
    for p in [
        json!({"kind": "picture", "image": png, "x": 20, "y": 40, "w": 60, "h": 30}),
        json!({"kind": "picture", "image": jpeg, "x": 100, "y": 40, "w": 40, "h": 40}),
        json!({"kind": "picture", "image": png, "x": 20, "y": 100, "w": 60, "h": 30, "background": backed}),
    ] {
        s.run("layout.addElement", &p).unwrap();
    }
    let li = s.project.layouts.len() - 1;
    let name = s.project.layouts[li].name.clone();
    assert_eq!((s.project.layouts.len(), name.as_str(), s.project.layouts[li].elements.len()), (1, "Pictures", 4));
    let saves = [
        ("layouts".to_string(), super::fnv_bytes(serde_json::to_string(&s.project.layouts).unwrap().as_bytes())),
        ("layout file".to_string(), super::fnv_bytes(&crate::project::layout_file_bytes(&s.project.layouts[li]).unwrap())),
    ];
    s.run("report.new", &json!({"layer": "Countries", "name": "R", "fields": ["name"], "where": "name LIKE 'F%'"})).unwrap();
    s.run("report.existingTemplate", &json!({"report": "R", "layout": name})).unwrap();
    let export = |path: String, dpi: Option<f64>| {
        move |s: &mut Session| {
            let mut p = json!({"layout": li, "path": path});
            if let Some(dpi) = dpi {
                p["dpi"] = json!(dpi);
            }
            s.run("layout.export", &p).unwrap();
        }
    };
    let report =
        |path: String| move |s: &mut Session| drop(s.run("report.export", &json!({"report": "R", "path": path, "dpi": 36})).unwrap());
    let routes: Vec<(&str, Box<dyn Fn(&mut Session)>)> = vec![
        // route: layout.export as PNG
        ("layout.export png", Box::new(export(at("p.png"), Some(72.0)))),
        // route: layout.export as PDF
        ("layout.export pdf", Box::new(export(at("p.pdf"), None))),
        // route: layout.export as SVG
        ("layout.export svg", Box::new(export(at("p.svg"), None))),
        // route: the Assistant's view of the layout
        ("the Assistant's view", Box::new(move |s: &mut Session| drop(crate::cmd::layout::layout_png(s, li, 72.0, None).unwrap()))),
        // route: report.export as PNG
        ("report.export png", Box::new(report(at("r.png")))),
        // route: report.export as PDF
        ("report.export pdf", Box::new(report(at("r.pdf")))),
    ];
    let mut notices = Vec::new();
    for (route, run) in &routes {
        let mark = s.notice_mark();
        run(&mut s);
        let texts: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.replace(&folder, "<dir>/")).collect();
        let text = texts.join("\n");
        assert!(!text.contains(&*dir.0.to_string_lossy()), "{route}: the folder is said in another form: {text}");
        notices.push((format!("{route}: {text:?}"), super::fnv_bytes(text.as_bytes())));
    }
    let mut wrong = mismatches("save", &saves, &PINNED_PICTURE_SAVES);
    wrong.extend(mismatches("notice", &notices, &PINNED_PICTURE_NOTICES));
    // What `layout.addElement` answers for each picture given, word for word, the folder taken out:
    // a refusal, or what it added.
    let huge = png_b64(30_000, 1, |_, _| [0x1F, 0x5C, 0x84]);
    // The PNG cut four bytes into its pixels: its size reads, its pixels don't.
    let damaged = {
        let bytes = base64::engine::general_purpose::STANDARD.decode(&png).unwrap();
        let pixels = bytes.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
        base64::engine::general_purpose::STANDARD.encode(&bytes[..pixels + 4])
    };
    std::fs::write(dir.0.join("note.png"), "not a picture").unwrap();
    std::fs::create_dir_all(dir.0.join("folder.png")).unwrap();
    // What the system says of a file that isn't there, which differs between systems.
    let not_found = std::fs::metadata(dir.0.join("missing.png")).unwrap_err().to_string();
    let too_large = "A".repeat((16 << 20) / 3 * 4 + 8);
    let images = [(&png, "the PNG"), (&huge, "the 30000 x 1 PNG"), (&damaged, "the damaged PNG")];
    let rectangle = |key: &str, v: Value| {
        let mut p = json!({"kind": "rectangle", "x": 10, "y": 10, "w": 20, "h": 20});
        p[key] = v;
        p
    };
    let cases: Vec<(&str, Value, String)> = vec![
        ("neither an image nor a path", json!({"kind": "picture"}), "a picture needs an image or a path".into()),
        (
            "an image that isn't one",
            json!({"kind": "picture", "image": "bm90IGFuIGltYWdl"}),
            "the picture isn't a PNG or JPEG image".into(),
        ),
        ("an empty image", json!({"kind": "picture", "image": ""}), "the picture isn't a PNG or JPEG image".into()),
        ("an image over 16 MiB", json!({"kind": "picture", "image": too_large}), "the picture is too large".into()),
        ("a path to a missing file", json!({"kind": "picture", "path": at("missing.png")}), format!("<dir>/missing.png: {not_found}")),
        ("a path to a folder", json!({"kind": "picture", "path": at("folder.png")}), "<dir>/folder.png: not a regular file".into()),
        ("a path to a text file", json!({"kind": "picture", "path": at("note.png")}), "the picture isn't a PNG or JPEG image".into()),
        // Task 4 changes this to "path is the path of a PNG or JPEG file".
        ("a path that isn't text", json!({"kind": "picture", "path": 5}), "a picture needs an image or a path".into()),
        // Task 4 refuses this ("give a picture an image or a path, not both"); main uses the image.
        ("both an image and a path", json!({"kind": "picture", "image": png, "path": at("note.png")}), "added a picture of the PNG".into()),
        // Task 4 refuses these two (their image can't be drawn); main adds each as a grey box.
        ("an image over 20000 pixels a side", json!({"kind": "picture", "image": huge}), "added a picture of the 30000 x 1 PNG".into()),
        ("an image whose pixels don't read", json!({"kind": "picture", "image": damaged}), "added a picture of the damaged PNG".into()),
        // Task 4 refuses these two ("only pictures take an image or a path"); main ignores the key.
        ("an image on a rectangle", rectangle("image", json!(png)), "added a rectangle".into()),
        ("a path on a rectangle", rectangle("path", json!(at("missing.png"))), "added a rectangle".into()),
    ];
    for (case, p, pinned) in &cases {
        let got = match s.run("layout.addElement", p) {
            Err(e) => e.replace(&folder, "<dir>/"),
            Ok(r) => match s.project.layouts[li].element(r["element"].as_u64().unwrap() as u32).map(|e| &e.kind) {
                Some(rustgis_map::layout::ElementKind::Picture { image }) => {
                    let which = images.iter().find(|(i, _)| *i == image).map_or("another image", |(_, n)| n);
                    format!("added a picture of {which}")
                }
                Some(k) => format!("added a {}", k.code()),
                None => "added nothing".into(),
            },
        };
        if got != *pinned {
            wrong.push(format!("layout.addElement with {case}: got {got:?}, pinned {pinned:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
