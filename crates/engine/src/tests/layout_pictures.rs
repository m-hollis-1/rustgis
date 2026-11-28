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
    let images =
        [(PIN_PNG, "the PNG"), (PIN_JPEG, "the JPEG"), (huge.as_str(), "the 30000 x 1 PNG"), (damaged.as_str(), "the damaged PNG")];
    const UNDRAWN: &str =
        "the picture can't be drawn: its image is damaged, or too large (over 20000 pixels a side, or over 512 MB once decoded)";
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
        ("an image that is a number", picture("image", json!(5)), "image is a PNG or JPEG image in base64"),
        ("a path to a missing file", picture("path", json!(at("missing.png"))), "<dir>/missing.png: <the system's error>"),
        ("an empty path", picture("path", json!("")), "path is the path of a PNG or JPEG file"),
        // Never tried as a file, and not repeated.
        ("a path that is an address", picture("path", json!(address)), "a picture is read from a file on this computer, not a web address"),
        ("a path to a folder", picture("path", json!(at("folder.png"))), "<dir>/folder.png: not a regular file"),
        ("a path to a text file", picture("path", json!(note)), "the picture isn't a PNG or JPEG image"),
        ("a path to a file of 16 MiB and a byte", picture("path", json!(over)), "the picture is too large"),
        ("a path that is a number", picture("path", json!(5)), "path is the path of a PNG or JPEG file"),
        // Given both, neither is read.
        (
            "both an image and a path",
            json!({"kind": "picture", "image": PIN_PNG, "path": note}),
            "give a picture an image or a path, not both",
        ),
        // These four are refused: their size reads, but their pixels can't be drawn (declared over
        // 20000 pixels a side, or cut short).
        ("an image over 20000 pixels a side", picture("image", json!(huge)), UNDRAWN),
        ("an image whose pixels don't read", picture("image", json!(damaged)), UNDRAWN),
        ("a path to a PNG over 20000 pixels a side", picture("path", json!(huge_file)), UNDRAWN),
        ("a path to a PNG whose pixels don't read", picture("path", json!(damaged_file)), UNDRAWN),
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
    // answer, the picture afterwards, and the Undo steps it leaves. Each is taken as one Undo step;
    // a file it can't read is refused, the picture left as it was and no step left.
    let png_picture = &added[0];
    for (case, key, v, pinned) in [
        (
            "a path to the JPEG's file",
            "path",
            json!(file("jpeg.jpg", &b64(PIN_JPEG))),
            "answered null; a picture of the JPEG after; 1 Undo step(s)",
        ),
        ("the JPEG as an image", "image", json!(PIN_JPEG), "answered null; a picture of the JPEG after; 1 Undo step(s)"),
        (
            "a path to a missing file",
            "path",
            json!(at("missing.png")),
            "refused \"<dir>/missing.png: <the system's error>\"; a picture of the JPEG after; 0 Undo step(s)",
        ),
    ] {
        let depth = s.undo_depth();
        let path = v.as_str().filter(|_| key == "path").unwrap_or("").to_string();
        let mut p = json!({"layout": li, "element": png_picture});
        p[key] = v;
        let answer = match s.run("layout.update", &p) {
            Ok(r) => format!("answered {r}"),
            Err(e) => format!("refused {:?}", marked(e, &path).replace(&folder, "<dir>/")),
        };
        let got = format!("{answer}; {} after; {} Undo step(s)", which(&s, png_picture), s.undo_depth() - depth);
        if got != pinned {
            wrong.push(format!("layout.update with {case}: got {got:?}, pinned {pinned:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A folder of the test's own, removed however it ends, and a session with a layout "L".
#[cfg(not(target_arch = "wasm32"))]
fn picture_world(test: &str) -> (Dir, Session) {
    let dir = Dir(std::env::temp_dir().join(format!("rustgis-{test}-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let mut s = super::world();
    s.run("layout.new", &json!({"name": "L"})).unwrap();
    (dir, s)
}

/// A `w` x `h` PNG of one colour written to `name` in `dir`: its path, and its image as base64.
#[cfg(not(target_arch = "wasm32"))]
fn png_file(dir: &Dir, name: &str, (w, h): (u32, u32), colour: [u8; 3]) -> (String, String) {
    use base64::Engine as _;
    let b64 = png_b64(w, h, |_, _| colour);
    let path = dir.0.join(name);
    std::fs::write(&path, base64::engine::general_purpose::STANDARD.decode(&b64).unwrap()).unwrap();
    (path.display().to_string(), b64)
}

/// The image and file of picture `id` in the layout named `layout`.
#[cfg(not(target_arch = "wasm32"))]
fn picture_in(s: &Session, layout: &str, id: &Value) -> (String, Option<String>) {
    let l = s.project.layouts.iter().find(|l| l.name == layout).unwrap();
    match &l.element(id.as_u64().unwrap() as u32).unwrap().kind {
        rustgis_map::layout::ElementKind::Picture { image, source } => (image.clone(), source.clone()),
        k => panic!("not a picture: {k:?}"),
    }
}

/// A picture added from a file remembers it as given; one added from an image has none; no answer
/// carries the path.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_picture_added_from_a_file_remembers_it() {
    let (dir, mut s) = picture_world("a-picture-added-from-a-file-remembers-it");
    let (first, first_b64) = png_file(&dir, "first.png", (40, 20), [0x1F, 0x5C, 0x84]);
    let added = s.run("layout.addElement", &json!({"kind": "picture", "path": first})).unwrap();
    assert!(!added.to_string().contains("first.png"), "{added}");
    let id = added["element"].clone();
    assert_eq!(picture_in(&s, "L", &id), (first_b64.clone(), Some(first.clone())));
    let bare = s.run("layout.addElement", &json!({"kind": "picture", "image": first_b64})).unwrap()["element"].clone();
    assert_eq!(picture_in(&s, "L", &bare), (first_b64, None));
    let l = s.project.layouts.iter().find(|l| l.name == "L").unwrap();
    let listed = json!(crate::cmd::layout::element_list(l)).to_string();
    assert!(!listed.contains("first.png"), "{listed}");
}

/// `layout.update` with a path reads the file again and remembers it; with an image, takes it and
/// forgets the file. Each is one undo step, undone as one.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_picture_is_read_again_from_a_file_and_relinked() {
    let (dir, mut s) = picture_world("a-picture-is-read-again-from-a-file-and-relinked");
    let (first, first_b64) = png_file(&dir, "first.png", (40, 20), [0x1F, 0x5C, 0x84]);
    let (second, second_b64) = png_file(&dir, "second.png", (30, 30), [0xE8, 0xB0, 0x4A]);
    let third_b64 = png_b64(10, 20, |_, _| [0x22, 0x88, 0x22]);
    let id = s.run("layout.addElement", &json!({"kind": "picture", "path": first})).unwrap()["element"].clone();
    let depth = s.undo_depth();
    assert_eq!(s.run("layout.update", &json!({"element": id, "path": second})).unwrap(), Value::Null);
    assert_eq!(picture_in(&s, "L", &id), (second_b64.clone(), Some(second.clone())));
    assert_eq!(s.undo_depth(), depth + 1);
    s.run("edit.undo", &Value::Null).unwrap();
    assert_eq!(picture_in(&s, "L", &id), (first_b64, Some(first)));
    s.run("layout.update", &json!({"element": id, "image": third_b64})).unwrap();
    assert_eq!(picture_in(&s, "L", &id), (third_b64.clone(), None));
    let (before, depth) = (serde_json::to_value(&s.project).unwrap(), s.undo_depth());
    s.run("layout.update", &json!({"element": id, "path": null, "image": null})).unwrap();
    // Null changes nothing, and leaves its Undo step as every update does.
    assert_eq!((serde_json::to_value(&s.project).unwrap(), s.undo_depth()), (before, depth + 1), "null changes nothing");
    assert_eq!(picture_in(&s, "L", &id), (third_b64, None));
    // Any other update keeps its Undo step, as it always has, even one that changes nothing.
    let x = |s: &Session| s.project.layouts.iter().find(|l| l.name == "L").unwrap().element(id.as_u64().unwrap() as u32).unwrap().x;
    for p in [json!({"element": id}), json!({"element": id, "text": null}), json!({"element": id, "path": null, "x": 12.0})] {
        let depth = s.undo_depth();
        s.run("layout.update", &p).unwrap();
        assert_eq!(s.undo_depth(), depth + 1, "{p}");
    }
    assert_eq!(x(&s), 12.0);
    // A picture an older file holds with no image, only its file, relinked: it draws again.
    let imported = json!({"name": "Imported", "width": 210.0, "height": 297.0, "elements": [
        {"id": 1, "name": "Logo", "x": 10.0, "y": 10.0, "w": 40.0, "h": 20.0, "kind": {"Picture": {"image": "", "source": "/data/logos/north.png"}}}
    ]});
    s.run("layout.import", &json!({"text": imported.to_string()})).unwrap();
    let layout = |s: &Session| s.project.layouts.iter().find(|l| l.name == "Imported").unwrap().clone();
    assert_eq!(rustgis_render::layout::pictures_not_drawn(&layout(&s)).len(), 1);
    s.run("layout.update", &json!({"layout": "Imported", "element": 1, "path": second})).unwrap();
    assert_eq!(picture_in(&s, "Imported", &json!(1)), (second_b64, Some(second)));
    assert!(rustgis_render::layout::pictures_not_drawn(&layout(&s)).is_empty());
}

/// A missing, unreadable, too large, undrawable or not-a-picture file, or a path of the wrong kind,
/// is refused, even beside a valid key, and the picture is left as it was, with no undo step.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_bad_file_is_refused_and_the_picture_left_as_it_was() {
    let (dir, mut s) = picture_world("a-bad-file-is-refused-and-the-picture-left-as-it-was");
    let (first, first_b64) = png_file(&dir, "first.png", (40, 20), [0x1F, 0x5C, 0x84]);
    let (second, _) = png_file(&dir, "second.png", (30, 30), [0xE8, 0xB0, 0x4A]);
    let (huge, _) = png_file(&dir, "huge.png", (30_000, 1), [0x1F, 0x5C, 0x84]);
    let id = s.run("layout.addElement", &json!({"kind": "picture", "path": first})).unwrap()["element"].clone();
    s.run("layout.update", &json!({"element": id, "path": second})).unwrap();
    let at = |name: &str| dir.0.join(name).display().to_string();
    std::fs::write(at("note.png"), b"not a picture").unwrap();
    std::fs::write(at("big.png"), vec![0u8; (16 << 20) + 1]).unwrap();
    std::fs::create_dir_all(at("sub")).unwrap();
    // The picture, its place, the project, and the Undo steps and unsaved state.
    let state = |s: &Session| {
        let l = s.project.layouts.iter().find(|l| l.name == "L").unwrap();
        let x = l.element(id.as_u64().unwrap() as u32).unwrap().x;
        (picture_in(s, "L", &id), x, serde_json::to_value(&s.project).unwrap(), s.undo_depth(), s.dirty())
    };
    let before = state(&s);
    let undrawn = "the picture can't be drawn: its image is damaged, or too large (over 20000 pixels a side, or over 512 MB once decoded)";
    let not_a_path = "path is the path of a PNG or JPEG file";
    for (path, said) in [
        // `read_picture_file`'s own "{path}: …".
        (json!(at("gone.png")), "gone.png"),
        (json!(at("note.png")), "the picture isn't a PNG or JPEG image"),
        (json!(huge), undrawn),
        (json!(at("big.png")), "the picture is too large"),
        (json!(at("sub")), "not a regular file"),
        (json!("https://user:secret@example.invalid/logo.png"), "a picture is read from a file on this computer, not a web address"),
        (json!(5), not_a_path),
        (json!(""), not_a_path),
        (json!("  "), not_a_path),
        (json!(["a.png"]), not_a_path),
    ] {
        let e = s.run("layout.update", &json!({"element": id, "path": path, "x": 5})).unwrap_err();
        assert!(e.contains(said) && !e.contains("secret") && !e.contains("example.invalid"), "{path}: {e}");
        assert!(state(&s) == before, "{path} changed the picture");
    }
    for (p, said) in [
        (json!({"element": id, "image": 5, "x": 5}), "image is a PNG or JPEG image in base64"),
        (json!({"element": id, "image": first_b64, "path": first, "x": 5}), "give a picture an image or a path, not both"),
        (json!({"element": id, "image": "bm90IGFuIGltYWdl", "x": 5}), "the picture isn't a PNG or JPEG image"),
        (json!({"element": id, "image": png_b64(30_000, 1, |_, _| [9, 9, 9]), "x": 5}), undrawn),
    ] {
        let e = s.run("layout.update", &p).unwrap_err();
        assert_eq!(e, said);
        assert!(state(&s) == before, "{said}: the picture changed");
    }
    // An address with one slash or with backslashes is refused by both commands, never echoed.
    for path in [r"https:\\user:pw@host\x.png", "https:/user:pw@host/x.png"] {
        for (command, p) in
            [("layout.update", json!({"element": id, "path": path})), ("layout.addElement", json!({"kind": "picture", "path": path}))]
        {
            let e = s.run(command, &p).unwrap_err();
            assert_eq!(e, "a picture is read from a file on this computer, not a web address", "{command} {path}");
            assert!(!e.contains("pw") && !e.contains("host"), "{command}: {e}");
            assert!(state(&s) == before, "{command} {path} changed the layout");
        }
    }
    // On another kind of element, refused before any file is read; null is not given.
    let l = s.project.layouts.iter().find(|l| l.name == "L").unwrap();
    let text = l.elements.iter().find(|e| e.kind.code() == "text").unwrap().id;
    let before = state(&s);
    for key in ["path", "image"] {
        let e = s.run("layout.update", &json!({"element": text, key: first})).unwrap_err();
        assert_eq!(e, "only pictures take an image or a path");
        assert!(state(&s) == before, "{key} on a text changed it");
    }
    let depth = s.undo_depth();
    s.run("layout.update", &json!({"element": text, "path": null, "image": null})).unwrap();
    let after = state(&s);
    assert!((&after.0, after.1, &after.2) == (&before.0, before.1, &before.2), "null picture keys on a text changed it");
    assert_eq!(s.undo_depth(), depth + 1);
    // Locked, it is refused before its file is read.
    s.run("layout.update", &json!({"element": id, "locked": true})).unwrap();
    let before = state(&s);
    let e = s.run("layout.update", &json!({"element": id, "path": first})).unwrap_err();
    assert!(e.contains("locked"), "{e}");
    assert!(state(&s) == before, "a locked picture changed");
    // Adding: refused, and no element added.
    let count = |s: &Session| s.project.layouts.iter().find(|l| l.name == "L").unwrap().elements.len();
    let (n, depth, dirty) = (count(&s), s.undo_depth(), s.dirty());
    for (p, said) in [
        (json!({"kind": "picture", "path": huge}), undrawn),
        (json!({"kind": "picture", "image": first_b64, "path": first}), "give a picture an image or a path, not both"),
        (
            json!({"kind": "picture", "path": "https://user:secret@example.invalid/logo.png"}),
            "a picture is read from a file on this computer, not a web address",
        ),
        (json!({"kind": "picture"}), "a picture needs an image or a path"),
        (json!({"kind": "picture", "path": null, "image": null}), "a picture needs an image or a path"),
    ] {
        assert_eq!(s.run("layout.addElement", &p).unwrap_err(), said, "{p}");
        assert_eq!((count(&s), s.undo_depth(), s.dirty()), (n, depth, dirty), "{p}");
    }
}

/// A path the rule doesn't keep: the picture is read, its file not remembered, and that is said.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_path_that_is_not_kept_is_read_and_said() {
    let (dir, mut s) = picture_world("a-path-that-is-not-kept-is-read-and-said");
    // A direction override is a file name every system allows; a bell, only some.
    let names: &[&str] = if cfg!(unix) { &["logo\u{202E}.png", "logo\u{7}.png"] } else { &["logo\u{202E}.png"] };
    let unremembered = |s: &Session, mark| s.notices_since(mark).iter().filter(|n| n.text.contains("isn't remembered")).count();
    for (i, name) in names.iter().enumerate() {
        let (first, _) = png_file(&dir, &format!("first{i}.png"), (40, 20), [0x1F, 0x5C, 0x84]);
        let (odd, odd_b64) = png_file(&dir, name, (20, 20), [0x22, 0x88, 0x22]);
        let mark = s.notice_mark();
        let id = s.run("layout.addElement", &json!({"kind": "picture", "path": odd})).unwrap()["element"].clone();
        assert_eq!(picture_in(&s, "L", &id), (odd_b64.clone(), None), "{name:?}");
        assert_eq!(unremembered(&s, mark), 1, "{name:?}");
        let kept = s.run("layout.addElement", &json!({"kind": "picture", "path": first})).unwrap()["element"].clone();
        let mark = s.notice_mark();
        s.run("layout.update", &json!({"element": kept, "path": odd})).unwrap();
        assert_eq!(picture_in(&s, "L", &kept), (odd_b64, None), "{name:?}");
        assert_eq!(unremembered(&s, mark), 1, "{name:?}");
    }
}

/// A duplicated picture keeps its file, and a project holding only the copy still requires the
/// capability.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_duplicated_picture_keeps_its_file() {
    let (dir, mut s) = picture_world("a-duplicated-picture-keeps-its-file");
    let (first, first_b64) = png_file(&dir, "first.png", (40, 20), [0x1F, 0x5C, 0x84]);
    let id = s.run("layout.addElement", &json!({"kind": "picture", "path": first})).unwrap()["element"].clone();
    let copy = s.run("layout.duplicateElement", &json!({"element": id})).unwrap()["element"].clone();
    assert_ne!(copy, id);
    assert_eq!(picture_in(&s, "L", &copy), (first_b64, Some(first)));
    s.run("layout.removeElement", &json!({"element": id})).unwrap();
    let requires = serde_json::to_value(&s.project).unwrap()["requires"].clone();
    assert!(requires.as_array().is_some_and(|r| r.contains(&json!("layout-picture-source"))), "{requires}");
}

/// A project whose pictures' files are of the wrong kind, blank, an address, too long or with
/// control characters opens, says it repaired them, and saves without them and without the
/// capability.
#[test]
fn a_project_with_a_damaged_picture_file_opens_and_says_so() {
    let sources = [
        serde_json::json!(5),
        serde_json::json!(""),
        serde_json::json!("https://user:secret@example.invalid/a.png"),
        serde_json::json!(format!("/{}", "d".repeat(5000))),
        serde_json::json!("a\u{7}b.png"),
    ];
    let elements: Vec<serde_json::Value> = sources
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let picture = serde_json::json!({"Picture": {"image": "", "source": source}});
            serde_json::json!({"id": i + 1, "name": "p", "x": 0, "y": 0, "w": 50, "h": 50, "kind": picture})
        })
        .collect();
    let document = serde_json::json!({
        "format": 2,
        "name": "Damaged pictures",
        "active_map": 0,
        "maps": [{"name": "Map", "layers": [], "basemap": "Paper", "camera": {"center": {"x": 0, "y": 0}, "res": 30000}}],
        "layouts": [{"name": "Pictures", "width": 297, "height": 210, "elements": elements}]
    });
    let mut p = crate::Project::from_json(&document.to_string()).unwrap();
    let said: Vec<String> = p.take_read_notices(&crate::project::ALL_PARTS).iter().map(|n| n.english()).collect();
    assert_eq!(said.iter().filter(|n| n.contains("5 layout element(s) with a value out of range")).count(), 1, "{said:?}");
    assert!(p.layouts[0].elements.iter().all(|e| e.kind.picture_source().is_none()));
    let saved = serde_json::to_string(&p).unwrap();
    for absent in ["\"source\"", "secret", "layout-picture-source"] {
        assert!(!saved.contains(absent), "{absent} in {saved}");
    }
}

/// A session holding layout "L": the sample's map in a frame, and the pictures "Logo" (no image,
/// from `/home/someone/logos/north.png`), "Blank" (no image, no file), "Broken" (an image that
/// isn't one), "Hidden" (no image, not visible) and "Fine" (a PNG), as an older file holds them.
#[cfg(not(target_arch = "wasm32"))]
fn grey_pictures() -> Session {
    let mut s = super::world();
    s.run("layout.new", &json!({"name": "L", "page": "a4", "orientation": "portrait", "template": "map"})).unwrap();
    let mut layout = serde_json::to_value(&s.project.layouts[0]).unwrap();
    s.project.layouts.clear();
    let picture = |id: u32, name: &str, x: f64, y: f64, kind: Value, visible: bool| json!({"id": id, "name": name, "x": x, "y": y, "w": 30.0, "h": 15.0, "visible": visible, "kind": {"Picture": kind}});
    let fine = png_b64(20, 10, |_, _| [0x1F, 0x5C, 0x84]);
    let elements = layout["elements"].as_array_mut().unwrap();
    elements.extend([
        picture(101, "Logo", 10.0, 10.0, json!({"image": "", "source": "/home/someone/logos/north.png"}), true),
        picture(102, "Blank", 50.0, 10.0, json!({"image": ""}), true),
        picture(103, "Broken", 90.0, 10.0, json!({"image": "bm90IGFuIGltYWdl"}), true),
        picture(104, "Hidden", 130.0, 10.0, json!({"image": ""}), false),
        picture(105, "Fine", 150.0, 250.0, json!({"image": fine}), true),
    ]);
    s.run("layout.import", &json!({"text": layout.to_string()})).unwrap();
    assert_eq!((s.project.layouts.len(), s.project.layouts[0].name.as_str()), (1, "L"));
    s
}

/// Pictures drawn as a grey box are said by every export, once a kind, named by their file's name,
/// never its folders; a hidden one or one that draws isn't; a map series says them once; after a
/// relink, nothing is said. (A layout whose pictures all draw says nothing new at any export:
/// `pictures_save_and_say_as_before` pins that.)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_picture_drawn_as_a_grey_box_is_said_by_every_export() {
    let dir = Dir(std::env::temp_dir().join(format!("rustgis-a-picture-drawn-as-a-grey-box-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let at = |file: &str| dir.0.join(file).display().to_string();
    let mut s = grey_pictures();
    // The report places the layout on two pages: it is said once all the same.
    s.run("report.new", &json!({"layer": "Countries", "name": "R", "fields": ["name"], "where": "name LIKE 'F%'"})).unwrap();
    s.run("report.existingTemplate", &json!({"report": "R", "layout": "L"})).unwrap();
    s.run("report.existingTemplate", &json!({"report": "R", "layout": "L", "end": true})).unwrap();
    // What a route says of layout pictures.
    let said = |s: &mut Session, run: &dyn Fn(&mut Session)| -> Vec<String> {
        let mark = s.notice_mark();
        run(s);
        s.notices_since(mark).iter().map(|n| n.text.clone()).filter(|t| t.contains("layout picture(s)")).collect()
    };
    let export = |file: &'static str| {
        let path = at(file);
        Box::new(move |s: &mut Session| drop(s.run("layout.export", &json!({"path": path, "dpi": 36})).unwrap()))
            as Box<dyn Fn(&mut Session)>
    };
    let report = |file: &'static str| {
        let path = at(file);
        Box::new(move |s: &mut Session| drop(s.run("report.export", &json!({"report": "R", "path": path, "dpi": 36})).unwrap()))
            as Box<dyn Fn(&mut Session)>
    };
    let view: Box<dyn Fn(&mut Session)> = Box::new(|s| drop(crate::cmd::layout::layout_png(s, 0, 36.0, None).unwrap()));
    let routes: Vec<(&str, bool, Box<dyn Fn(&mut Session)>)> = vec![
        ("layout.export png", false, export("p.png")),
        ("layout.export pdf", false, export("p.pdf")),
        ("layout.export svg", false, export("p.svg")),
        ("the Assistant's view", false, view),
        ("report.export png", true, report("r.png")),
        ("report.export pdf", true, report("r.pdf")),
    ];
    for (route, in_report, run) in &routes {
        let said = said(&mut s, run.as_ref());
        assert_eq!(said.len(), 2, "{route}: {said:?}");
        let (logo, blank, broken) = if *in_report {
            ("Logo in layout 'L' (", "Blank in layout 'L' (", "Broken in layout 'L' (")
        } else {
            ("Logo (", "Blank (", "Broken (")
        };
        assert!(
            said[0].contains("2 layout picture(s) have no image")
                && said[0].contains(logo)
                && said[0].contains("north.png")
                && said[0].contains(blank),
            "{route}: {said:?}"
        );
        assert!(said[1].contains("1 layout picture(s) have an image that can't be drawn") && said[1].contains(broken), "{route}: {said:?}");
        assert!(
            said.iter().all(|n| !n.contains("someone") && !n.contains("logos") && !n.contains("Hidden") && !n.contains("Fine")),
            "{route}: {said:?}"
        );
        assert!(said.iter().all(|n| n.starts_with("The report: ") == *in_report), "{route}: {said:?}");
    }
    // The Assistant's view of a region holding only "Logo" says it, and only it.
    let logo = said(&mut s, &|s| drop(crate::cmd::layout::layout_png(s, 0, 36.0, Some([5.0, 5.0, 40.0, 25.0])).unwrap()));
    assert_eq!(logo.len(), 1, "{logo:?}");
    assert!(logo[0].starts_with("1 layout picture(s) have no image") && logo[0].contains("Logo ("), "{logo:?}");
    assert!(!logo[0].contains("Blank") && !logo[0].contains("Broken"), "{logo:?}");
    // The Assistant's view of a region holding only "Fine" says nothing of pictures.
    let fine = said(&mut s, &|s| drop(crate::cmd::layout::layout_png(s, 0, 36.0, Some([145.0, 245.0, 40.0, 25.0])).unwrap()));
    assert!(fine.is_empty(), "{fine:?}");
    // A map series over three countries: the PDF of all its pages says each kind once.
    let mut series = grey_pictures();
    for layer in &mut series.map_mut().layers {
        if layer.name == "Countries" {
            layer.dataset.features.truncate(3);
        }
    }
    series.run("layout.series", &json!({"layout": "L", "layer": "Countries"})).unwrap();
    assert_eq!(crate::cmd::layout::series_pages(&series, 0).map(|p| p.len()), Some(3));
    let pages = said(&mut series, export("series.pdf").as_ref());
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert!(pages[0].contains("2 layout picture(s) have no image"), "{pages:?}");
    assert!(pages[1].contains("1 layout picture(s) have an image that can't be drawn"), "{pages:?}");
    // Relinked, or removed: no route says anything of pictures.
    let png = dir.0.join("north.png");
    std::fs::write(&png, {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.decode(png_b64(20, 10, |_, _| [0xE8, 0xB0, 0x4A])).unwrap()
    })
    .unwrap();
    for id in [101, 103] {
        s.run("layout.update", &json!({"layout": "L", "element": id, "path": png.display().to_string()})).unwrap();
    }
    s.run("layout.removeElement", &json!({"layout": "L", "element": 102})).unwrap();
    assert!(rustgis_render::layout::pictures_not_drawn(&s.project.layouts[0]).is_empty());
    for (route, _, run) in &routes {
        let said = said(&mut s, run.as_ref());
        assert!(said.is_empty(), "{route}: {said:?}");
    }
}
