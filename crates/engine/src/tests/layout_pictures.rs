//! Layout pictures and the files they were read from: what they draw, say, save and require.

#[cfg(not(target_arch = "wasm32"))]
use crate::Session;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::{Value, json};

/// A `w` x `h` PNG, each pixel the opaque colour `colours` gives it, as base64: made when the test
/// runs, for a picture whose declared size alone decides what it is answered.
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

/// The PNG the render pins draw (a copy of `rustgis_render`'s), frozen so an encoder's new output
/// can't move a pin: 40 x 20, its left half (columns 0 to 19) `#1F5C84`, its right half
/// `#E8B04A`, row 10 white; written by `tiny_skia::Pixmap::encode_png` (8-bit RGBA, opaque) and
/// base64 (standard) once.
#[cfg(not(target_arch = "wasm32"))]
const PIN_PNG: &str = concat!(
    "iVBORw0KGgoAAAANSUhEUgAAACgAAAAUCAYAAAD/Rn+7AAAAWElEQVR4nO3VMQ2AUBAE0WHzfYAoSnAD1khQdUig",
    "uYQpeAIm2+2Yt6NodK9XZ44gF+SCXJALckEuyAW5IDdVVeuTdBvLfrYG7//qZIJckAtyQS7I5esBbx4rIwruG4R9",
    "JwAAAABJRU5ErkJggg==",
);

/// The JPEG the render pins draw (a copy of `rustgis_render`'s), frozen as the PNG is: 32 x 32,
/// sixteen 8 x 8 blocks each of one colour, block (bx, by) RGB (40 + 60 bx, 40 + 60 by, 200 - 20
/// (bx + by)); written once by the `image` crate's `JpegEncoder` at quality 80, which writes it
/// 4:4:4 (every component sampled 1 x 1).
#[cfg(not(target_arch = "wasm32"))]
const PIN_JPEG: &str = concat!(
    "/9j/4AAQSkZJRgABAgAAAQABAAD/wAARCAAgACADAREAAhEBAxEB/9sAQwAGBAUGBQQGBgUGBwcGCAoQCgoJCQoU",
    "Dg8MEBcUGBgXFBYWGh0lHxobIxwWFiAsICMmJykqKRkfLTAtKDAlKCko/9sAQwEHBwcKCAoTCgoTKBoWGigoKCgo",
    "KCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgo/8QAHwAAAQUBAQEBAQEAAAAAAAAA",
    "AAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAk",
    "M2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKT",
    "lJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QA",
    "HwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdh",
    "cRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hp",
    "anN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk",
    "5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDyKv1s4DVryT6U1a8g+jNSvIPpTNr6I/nc1a8g+kNSvIPpTVry",
    "T6Mza+iP54NWvIPpTUryD6M1a8g+lM2voz+eDVryD6Q1K8g+kNWvIPpD/9k=",
);

/// What [`pictures_save_and_say_as_before`] saved before pictures remembered their file, in turn.
#[cfg(not(target_arch = "wasm32"))]
const PINNED_PICTURE_SAVES: [u64; 3] = [
    0xa9fd_f881_3487_56d3, // the project's layouts
    0x9595_ade9_9651_60a0, // the layout's own file
    0xdf18_dc46_0b16_30e7, // what the project requires: ["layout-element-background"]
];
/// What each export of its layout said, in the order of the routes: where it wrote the file, and
/// nothing of its pictures.
#[cfg(not(target_arch = "wasm32"))]
const PINNED_PICTURE_NOTICES: [u64; 9] = [
    0x91fe_c13a_e3f0_46f7, // layout.export png: "Exported <dir>/p.png"
    0x91dc_c03a_e3d3_5ede, // layout.export pdf: "Exported <dir>/p.pdf"
    0xaba9_5c3a_f262_802e, // layout.export svg: "Exported <dir>/p.svg"
    0xcbf2_9ce4_8422_2325, // the Assistant's view: nothing at all
    0x39f5_2536_3dfd_3cbd, // report.export png: "Exported <dir>/r.png"
    0x39e0_3436_3dea_fa30, // report.export pdf: "Exported <dir>/r.pdf"
    0xcbf2_9ce4_8422_2325, // the Assistant's view of a region with a picture: nothing
    0xcbf2_9ce4_8422_2325, // the Assistant's view of a region without one: nothing
    0xa900_a1bc_27c1_480f, // the series' PDF of every page: "Exported <dir>/series.pdf"
];

/// Each hash that isn't its pinned one, named, with both values and all of them to pin.
#[cfg(not(target_arch = "wasm32"))]
fn mismatches(what: &str, got: &[(String, u64)], pinned: &[u64]) -> Vec<String> {
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
}

/// Removed however the test ends.
#[cfg(not(target_arch = "wasm32"))]
struct Dir(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// What main saves for pictures not read from a file, and says when a layout of pictures that draw
/// is exported every way, before pictures remembered their file; in the built-in face, in a process
/// of its own. The layout is an older file's, imported (a hidden picture with no image, which only
/// a file holds), with a PNG, a JPEG and the PNG on a background added to it. The saves, each
/// pinned on its own: the project's layouts as JSON, the layout's own file (header and all), and
/// what the saved project requires. The exports, each pinned on its own: PNG, PDF and SVG by
/// `layout.export`, the Assistant's view of the layout whole, of a region holding a picture and of
/// one holding none, a report of it as PNG and as PDF, and every page of a map series' PDF of a
/// layout with a picture. The test's folder is taken out of what is said before it is hashed. Then
/// what `layout.addElement` answers for a picture's `image` and `path`, and what `layout.update`
/// does with them on a picture, case by case, word for word.
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
    let backed = json!({"fill": "#FFF3D6", "outline": "#222222", "width": 1, "padding": 2});
    let mut added = Vec::new();
    for p in [
        json!({"kind": "picture", "image": PIN_PNG, "x": 20, "y": 40, "w": 60, "h": 30}),
        json!({"kind": "picture", "image": PIN_JPEG, "x": 100, "y": 40, "w": 40, "h": 40}),
        json!({"kind": "picture", "image": PIN_PNG, "x": 20, "y": 100, "w": 60, "h": 30, "background": backed}),
    ] {
        added.push(s.run("layout.addElement", &p).unwrap()["element"].clone());
    }
    let li = s.project.layouts.len() - 1;
    let name = s.project.layouts[li].name.clone();
    assert_eq!((s.project.layouts.len(), name.as_str(), s.project.layouts[li].elements.len()), (1, "Pictures", 4));
    let requires = serde_json::to_value(&s.project).unwrap()["requires"].to_string();
    let saves = [
        ("layouts".to_string(), super::fnv_bytes(serde_json::to_string(&s.project.layouts).unwrap().as_bytes())),
        ("layout file".to_string(), super::fnv_bytes(&crate::project::layout_file_bytes(&s.project.layouts[li]).unwrap())),
        (format!("project requires {requires}"), super::fnv_bytes(requires.as_bytes())),
    ];
    s.run("report.new", &json!({"layer": "Countries", "name": "R", "fields": ["name"], "where": "name LIKE 'F%'"})).unwrap();
    s.run("report.existingTemplate", &json!({"report": "R", "layout": name})).unwrap();
    // A map series of a layout with a picture, in a session of its own: its frame drawn at each
    // lake in turn.
    let mut series = super::world();
    series.run("layout.new", &json!({"name": "Series", "page": "a4", "orientation": "landscape"})).unwrap();
    series.run("layout.addElement", &json!({"kind": "picture", "image": PIN_PNG, "x": 200, "y": 20, "w": 40, "h": 20})).unwrap();
    series.run("layout.series", &json!({"layer": "Lakes"})).unwrap();
    let export = |path: String, dpi: Option<f64>| {
        move |s: &mut Session| {
            let mut p = json!({"path": path});
            if let Some(dpi) = dpi {
                p["dpi"] = json!(dpi);
            }
            s.run("layout.export", &p).unwrap();
        }
    };
    let report =
        |path: String| move |s: &mut Session| drop(s.run("report.export", &json!({"report": "R", "path": path, "dpi": 36})).unwrap());
    let view = |region: Option<[f64; 4]>| move |s: &mut Session| drop(crate::cmd::layout::layout_png(s, li, 72.0, region).unwrap());
    let mut notices = Vec::new();
    let mut said = |route: &str, s: &mut Session, run: &dyn Fn(&mut Session)| {
        let mark = s.notice_mark();
        run(s);
        let texts: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.replace(&folder, "<dir>/")).collect();
        let text = texts.join("\n");
        assert!(!text.contains(&*dir.0.to_string_lossy()), "{route}: the folder is said in another form: {text}");
        notices.push((format!("{route}: {text:?}"), super::fnv_bytes(text.as_bytes())));
    };
    // route: layout.export as PNG
    said("layout.export png", &mut s, &export(at("p.png"), Some(72.0)));
    // route: layout.export as PDF
    said("layout.export pdf", &mut s, &export(at("p.pdf"), None));
    // route: layout.export as SVG
    said("layout.export svg", &mut s, &export(at("p.svg"), None));
    // route: the Assistant's view of the layout
    said("the Assistant's view", &mut s, &view(None));
    // route: report.export as PNG
    said("report.export png", &mut s, &report(at("r.png")));
    // route: report.export as PDF
    said("report.export pdf", &mut s, &report(at("r.pdf")));
    // route: the Assistant's view of a region holding the PNG at (20, 40)
    said("the Assistant's view of a region with a picture", &mut s, &view(Some([15.0, 35.0, 70.0, 40.0])));
    // route: the Assistant's view of a region holding no picture
    said("the Assistant's view of a region without one", &mut s, &view(Some([150.0, 200.0, 40.0, 40.0])));
    // route: layout.export of the map series as a PDF of every page
    said("the series' PDF of every page", &mut series, &export(at("series.pdf"), Some(36.0)));
    let mut wrong = mismatches("save", &saves, &PINNED_PICTURE_SAVES);
    wrong.extend(mismatches("notice", &notices, &PINNED_PICTURE_NOTICES));

    // What `layout.addElement` answers for each picture given, word for word, the folder taken out
    // and the system's own words for a file it can't find marked: a refusal, or what it added.
    let huge = png_b64(30_000, 1, |_, _| [0x1F, 0x5C, 0x84]);
    // The PNG cut four bytes into its pixels: its size reads, its pixels don't.
    let damaged = {
        let bytes = base64::engine::general_purpose::STANDARD.decode(PIN_PNG).unwrap();
        let pixels = bytes.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
        base64::engine::general_purpose::STANDARD.encode(&bytes[..pixels + 4])
    };
    let file = |name: &str, bytes: &[u8]| {
        std::fs::write(dir.0.join(name), bytes).unwrap();
        at(name)
    };
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.decode(s).unwrap();
    let note = file("note.png", b"not a picture");
    let huge_file = file("huge.png", &b64(&huge));
    let damaged_file = file("damaged.png", &b64(&damaged));
    let over = file("over.png", &vec![0u8; (16 << 20) + 1]);
    std::fs::create_dir_all(dir.0.join("folder.png")).unwrap();
    let address = "http://example.invalid/a.png";
    // What the system says of a path it can't open, which differs between systems: marked.
    let marked = |text: String, path: &str| match std::fs::metadata(path) {
        Err(e) => text.replace(&e.to_string(), "<the system's error>"),
        Ok(_) => text,
    };
    let images = [(PIN_PNG, "the PNG"), (huge.as_str(), "the 30000 x 1 PNG"), (damaged.as_str(), "the damaged PNG")];
    let rectangle = |key: &str, v: Value| {
        let mut p = json!({"kind": "rectangle", "x": 10, "y": 10, "w": 20, "h": 20});
        p[key] = v;
        p
    };
    let picture = |key: &str, v: Value| {
        let mut p = json!({"kind": "picture"});
        p[key] = v;
        p
    };
    let cases: Vec<(&str, Value, &str)> = vec![
        ("neither an image nor a path", json!({"kind": "picture"}), "a picture needs an image or a path"),
        ("an image that isn't one", picture("image", json!("bm90IGFuIGltYWdl")), "the picture isn't a PNG or JPEG image"),
        ("an empty image", picture("image", json!("")), "the picture isn't a PNG or JPEG image"),
        ("an image over 16 MiB", picture("image", json!("A".repeat((16 << 20) / 3 * 4 + 8))), "the picture is too large"),
        // An image that isn't text is read as no image.
        ("an image that is a number", picture("image", json!(5)), "a picture needs an image or a path"),
        ("a path to a missing file", picture("path", json!(at("missing.png"))), "<dir>/missing.png: <the system's error>"),
        ("an empty path", picture("path", json!("")), ": <the system's error>"),
        ("a path that is an address", picture("path", json!(address)), "http://example.invalid/a.png: <the system's error>"),
        ("a path to a folder", picture("path", json!(at("folder.png"))), "<dir>/folder.png: not a regular file"),
        ("a path to a text file", picture("path", json!(note)), "the picture isn't a PNG or JPEG image"),
        ("a path to a file of 16 MiB and a byte", picture("path", json!(over)), "the picture is too large"),
        // A path that isn't text is read as no path.
        ("a path that is a number", picture("path", json!(5)), "a picture needs an image or a path"),
        // Given both, the image is used and the path is never read.
        ("both an image and a path", json!({"kind": "picture", "image": PIN_PNG, "path": note}), "added a picture of the PNG"),
        // These four are added, and each draws as the grey box: their size reads, but their pixels
        // can't be drawn (declared over 20000 pixels a side, or cut short).
        ("an image over 20000 pixels a side", picture("image", json!(huge)), "added a picture of the 30000 x 1 PNG"),
        ("an image whose pixels don't read", picture("image", json!(damaged)), "added a picture of the damaged PNG"),
        ("a path to a PNG over 20000 pixels a side", picture("path", json!(huge_file)), "added a picture of the 30000 x 1 PNG"),
        ("a path to a PNG whose pixels don't read", picture("path", json!(damaged_file)), "added a picture of the damaged PNG"),
        // Other kinds of element ignore `image` and `path`.
        ("an image on a rectangle", rectangle("image", json!(PIN_PNG)), "added a rectangle"),
        ("a path on a rectangle", rectangle("path", json!(at("missing.png"))), "added a rectangle"),
    ];
    let which = |s: &Session, id: &Value| match s.project.layouts[li].element(id.as_u64().unwrap() as u32).map(|e| &e.kind) {
        Some(rustgis_map::layout::ElementKind::Picture { image, .. }) => {
            format!("a picture of {}", images.iter().find(|(i, _)| *i == image.as_str()).map_or("another image", |(_, n)| n))
        }
        Some(k) => format!("a {}", k.code()),
        None => "nothing".into(),
    };
    for (case, p, pinned) in &cases {
        let path = p.get("path").and_then(Value::as_str).unwrap_or("");
        let got = match s.run("layout.addElement", p) {
            Err(e) => marked(e, path).replace(&folder, "<dir>/"),
            Ok(r) => format!("added {}", which(&s, &r["element"])),
        };
        if got != *pinned {
            wrong.push(format!("layout.addElement with {case}: got {got:?}, pinned {pinned:?}"));
        }
    }
    // What `layout.update` does with `path` and `image` on a picture (the PNG first added): its
    // answer, the picture afterwards, and the Undo steps it leaves. It takes neither key: the
    // picture is left as it was, the file never read, and one Undo step is left all the same.
    let png_picture = &added[0];
    for (case, key, v, pinned) in [
        (
            "a path to the JPEG's file",
            "path",
            json!(file("jpeg.jpg", &b64(PIN_JPEG))),
            "answered null; a picture of the PNG after; 1 Undo step(s)",
        ),
        ("the JPEG as an image", "image", json!(PIN_JPEG), "answered null; a picture of the PNG after; 1 Undo step(s)"),
        ("a path to a missing file", "path", json!(at("missing.png")), "answered null; a picture of the PNG after; 1 Undo step(s)"),
    ] {
        let depth = s.undo_depth();
        let mut p = json!({"layout": li, "element": png_picture});
        p[key] = v;
        let answer = match s.run("layout.update", &p) {
            Ok(r) => format!("answered {r}"),
            Err(e) => format!("refused {:?}", e.replace(&folder, "<dir>/")),
        };
        let got = format!("{answer}; {} after; {} Undo step(s)", which(&s, png_picture), s.undo_depth() - depth);
        if got != pinned {
            wrong.push(format!("layout.update with {case}: got {got:?}, pinned {pinned:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
