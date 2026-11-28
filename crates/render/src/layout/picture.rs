//! A layout picture that draws no image: its grey box, named for its file, which pictures draw
//! one, and the notices that say so.

use super::*;
use rustgis_map::layout::{picture_file_name, picture_source_kept};

/// The most pixels a side a layout picture's image may declare and still be drawn.
pub const PICTURE_MAX_DECLARED: u32 = crate::picture::MAX_DECLARED;
/// The most memory, in MiB, a layout picture's image may take once decoded and still be drawn.
pub const PICTURE_MAX_DECODED_MB: u64 = crate::picture::MAX_ALLOC >> 20;

/// Layout pictures named in a notice, before "and N more"; and the characters of a picture's
/// name or file name a notice shows before cutting it with "…".
const NAMED: usize = 10;
const NAME_CHARS: usize = 100;

/// Whether a layout picture's image draws: the answer the page, its notices and the commands go
/// by. Decoded once while it is unchanged, through the cache the page draws from.
pub fn picture_draws(image: &str) -> bool {
    crate::picture::decoded(image).is_some()
}

/// What a picture's grey box says: the name of its file when it has one it keeps (never its
/// folders), else "Picture".
fn picture_label(source: Option<&str>) -> &str {
    source.filter(|s| picture_source_kept(s)).and_then(picture_file_name).unwrap_or("Picture")
}

/// `name` as it fits in `room`, measured by [`text::measure`] at `size`: whole when it fits, else
/// the longest start of it, whole characters, that fits with "…" after it, else "…" alone.
fn fitted_name(name: &str, room: f32, size: f32) -> String {
    let fits = |s: &str| text::measure(s, size, false) <= room;
    if fits(name) {
        return name.to_string();
    }
    // The byte offset each start of `n` characters ends at; found by halving over the count.
    let ends: Vec<usize> = name.char_indices().map(|(i, _)| i).collect();
    let with_dots = |n: usize| format!("{}…", ends.get(n).and_then(|&end| name.get(..end)).unwrap_or(name));
    let (mut lo, mut hi) = (0, ends.len().saturating_sub(1));
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(&with_dots(mid)) { lo = mid } else { hi = mid - 1 }
    }
    with_dots(lo)
}

/// A picture's grey box with no image to draw: "Picture" in it as [`placeholder`] draws it for a
/// picture with no file, else its file's name ([`picture_label`]) cut to the box, 2 pt in from
/// each side.
pub(super) fn draw_picture_box(pm: &mut Canvas, r: Rect, source: Option<&str>, ppp: f64) {
    let label = picture_label(source);
    if label == "Picture" {
        placeholder(pm, r, "Picture", ppp);
    } else {
        // `placeholder` draws its label 10 × 1.33 × `ppp` pixels tall, which text::measure
        // measures at 10 × `ppp`.
        let room = r.width() - 4.0 * ppp as f32;
        placeholder(pm, r, &fitted_name(label, room, 10.0 * ppp as f32), ppp);
    }
}

/// Why a layout picture is drawn as a grey box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PictureLoss {
    /// Its image is empty.
    NoImage,
    /// Its image doesn't draw: damaged, or too large (over [`PICTURE_MAX_DECLARED`] pixels a side,
    /// or over [`PICTURE_MAX_DECODED_MB`] MiB once decoded).
    Undrawable,
}

/// A layout picture [`pictures_not_drawn`] finds drawn as a grey box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureGap {
    pub id: u32,
    pub name: String,
    /// The name of the layout it is in, when a notice names it (a report's, which draws several).
    pub layout: Option<String>,
    /// The name of the file it was read from, when it keeps one; never its folders.
    pub file: Option<String>,
    pub why: PictureLoss,
}

/// The visible pictures of `layout` whose image doesn't draw ([`picture_draws`]), drawn as a grey
/// box, in the order of its elements. A hidden picture draws nothing and isn't listed, nor is one
/// with none of its box on the page ([`on_page`], the test a legend's pictures are waited by):
/// the page shows none of its grey box.
pub fn pictures_not_drawn(layout: &Layout) -> Vec<PictureGap> {
    let gaps = layout.elements.iter().filter(|e| e.visible && on_page(layout, e)).filter_map(|e| {
        let ElementKind::Picture { image, .. } = &e.kind else { return None };
        if picture_draws(image) {
            return None;
        }
        let why = if image.trim().is_empty() { PictureLoss::NoImage } else { PictureLoss::Undrawable };
        let file = e.kind.picture_source().and_then(picture_file_name).map(String::from);
        Some(PictureGap { id: e.id, name: e.name.clone(), layout: None, file, why })
    });
    gaps.collect()
}

/// `s` as a notice shows it: its first [`NAME_CHARS`] characters, with "…" after them when it
/// has more.
fn notice_cut(s: &str) -> String {
    match s.char_indices().nth(NAME_CHARS) {
        Some((end, _)) => format!("{}…", s.get(..end).unwrap_or(s)),
        None => s.to_string(),
    }
}

/// How a notice says to give a picture its image again: in the browser, which has no file paths,
/// by its image; elsewhere, from its file.
const WAY_BACK: &str = if cfg!(target_arch = "wasm32") {
    "give one its image again with layout.update and \"image\""
} else {
    "read one from its file again with layout.update and \"path\" (Choose File… in the Format pane)"
};

/// What is said of `gaps`: one notice for each [`PictureLoss`] among them, in its order, with the
/// pictures counted and the first ten named, each with its layout's name and its file's name
/// when it has them.
pub fn pictures_notices(gaps: &[PictureGap]) -> Vec<String> {
    [PictureLoss::NoImage, PictureLoss::Undrawable]
        .into_iter()
        .filter_map(|why| {
            let these: Vec<&PictureGap> = gaps.iter().filter(|g| g.why == why).collect();
            if these.is_empty() {
                return None;
            }
            let named = these.iter().take(NAMED).map(|g| {
                let name = match &g.layout {
                    Some(layout) => format!("{} in layout '{}'", notice_cut(&g.name), notice_cut(layout)),
                    None => notice_cut(&g.name),
                };
                match &g.file {
                    Some(file) => format!("{name} ({}, {})", g.id, notice_cut(file)),
                    None => format!("{name} ({})", g.id),
                }
            });
            let mut named = named.collect::<Vec<_>>().join("; ");
            if these.len() > NAMED {
                named.push_str(&format!("; and {} more", these.len() - NAMED));
            }
            let n = these.len();
            let what = match why {
                PictureLoss::NoImage => "have no image".to_string(),
                PictureLoss::Undrawable => {
                    format!(
                        "have an image that can't be drawn (damaged, or too large: over {PICTURE_MAX_DECLARED} pixels a side \
                         or over {PICTURE_MAX_DECODED_MB} MB once decoded)"
                    )
                }
            };
            Some(format!("{n} layout picture(s) {what} and are drawn as a grey box: {named}; {WAY_BACK}"))
        })
        .collect()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A 100 x 80 mm page with one picture, of `image` and from file `source`, in (10, 10, 60, 20).
    fn one_picture_page(image: &str, source: Option<&str>) -> Layout {
        let mut l = Layout::new("Pictures", 100.0, 80.0);
        l.add("Picture", (10.0, 10.0, 60.0, 20.0), ElementKind::Picture { image: image.to_string(), source: source.map(String::from) });
        l
    }

    /// A picture without an image draws its grey box with its file's name in it; without a file,
    /// the word Picture, as before.
    #[test]
    fn a_picture_without_an_image_is_labelled_with_its_file_name() {
        assert_eq!(picture_label(None), "Picture");
        assert_eq!(picture_label(Some("")), "Picture");
        assert_eq!(picture_label(Some("/data/logos/north.png")), "north.png");
        assert_eq!(picture_label(Some(r"\\server\share\logo.jpg")), "logo.jpg");
        assert_eq!(picture_label(Some("https://example.invalid/north.png")), "Picture");
        // Drawn: a named box differs from the plain one, and from one named otherwise.
        let page = |source: Option<&str>| one_picture_page("", source);
        let draw = |l: Layout| render(&l, &mut Vec::new(), 96.0, &PageInfo::default()).unwrap();
        let (plain, north, south) =
            (draw(page(None)), draw(page(Some("/data/logos/north.png"))), draw(page(Some("/data/logos/south.png"))));
        assert_ne!(plain.data(), north.data());
        assert_ne!(north.data(), south.data());
        // A source the rule doesn't keep draws as none.
        assert_eq!(draw(page(Some("https://example.invalid/north.png"))).data(), plain.data());
    }

    /// Only the file's name reaches the page, the SVG and the PDF: two folders, the same drawing.
    #[test]
    fn the_grey_box_names_the_file_not_its_folders() {
        let (a, b) = (one_picture_page("", Some("/home/someone/logos/north.png")), one_picture_page("", Some(r"\\server\share\north.png")));
        let info = PageInfo::default();
        assert_eq!(render(&a, &mut Vec::new(), 96.0, &info).unwrap().data(), render(&b, &mut Vec::new(), 96.0, &info).unwrap().data());
        let svg = |l: &Layout| crate::svg::layout_svg(l, &mut Vec::new(), 72.0, &info).unwrap().0;
        let (sa, sb) = (svg(&a), svg(&b));
        assert_eq!(sa, sb);
        assert!(!sa.contains("someone") && !sa.contains("server"));
        let pdf = |l: &Layout| {
            let (drawing, _) = render_drawing(l, &mut Vec::new(), 150.0, &info).unwrap();
            crate::pdf::write(&[crate::pdf::Page::drawing(drawing, l.width, l.height)], "P")
        };
        assert_eq!(pdf(&a), pdf(&b));
    }

    /// A name too long for its box is cut to the box with "…"; one that fits is kept whole.
    #[test]
    fn a_long_file_name_is_cut_to_its_box() {
        let px = 13.3;
        let name = "a-very-long-picture-file-name-from-the-survey-office.png";
        let whole = crate::text::measure(name, px, false);
        assert_eq!(fitted_name(name, whole + 1.0, px), name);
        let cut = fitted_name(name, whole / 2.0, px);
        assert!(cut.ends_with('…') && name.starts_with(cut.trim_end_matches('…')), "{cut}");
        assert!(crate::text::measure(&cut, px, false) <= whole / 2.0, "{cut}");
        assert_eq!(fitted_name(name, 0.0, px), "…");
        assert_eq!(fitted_name(&"ü".repeat(300), 40.0, px).chars().last(), Some('…'));
    }

    /// The visible pictures drawn as a grey box, each with why and only its file's name; a
    /// picture that draws, or a hidden one, isn't listed.
    #[test]
    fn pictures_not_drawn_lists_each_picture_drawn_as_a_grey_box() {
        let huge = png_b64(30_000, 1, |_, _| [0x1F, 0x5C, 0x84]);
        let mut l = Layout::new("Pictures", 100.0, 80.0);
        let pictures: [(&str, Option<&str>); 6] = [
            (PIN_PNG, None),
            ("", Some("/data/logos/north.png")),
            ("", None),
            ("bm90IGFuIGltYWdl", None),
            (&huge, None),
            ("", Some("/data/logos/south.png")),
        ];
        for (image, source) in pictures {
            let kind = ElementKind::Picture { image: image.to_string(), source: source.map(String::from) };
            l.add("Picture", (10.0, 10.0, 40.0, 20.0), kind);
        }
        if let Some(e) = l.elements.last_mut() {
            e.visible = false;
        }
        assert_eq!(l.elements.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
        let gaps = pictures_not_drawn(&l);
        let listed: Vec<(u32, PictureLoss, Option<String>)> = gaps.iter().map(|g| (g.id, g.why, g.file.clone())).collect();
        assert_eq!(
            listed,
            [
                (2, PictureLoss::NoImage, Some("north.png".to_string())),
                (3, PictureLoss::NoImage, None),
                (4, PictureLoss::Undrawable, None),
                (5, PictureLoss::Undrawable, None),
            ]
        );
        assert!(gaps.iter().all(|g| g.name == "Picture"));
    }

    /// Only a picture whose box has some of the page under it is listed: one partly on the page
    /// draws part of its grey box; one wholly off it, or only touching its edge, draws none.
    #[test]
    fn a_picture_off_the_page_is_not_listed() {
        // Each box, whether it is listed, and whether it is far enough from the page's edge for
        // the page to show its grey box just when it is listed (the box's outline is drawn
        // across its edge, so half of it reaches the page from a box only touching it).
        let boxes: [((f64, f64, f64, f64), bool, bool); 15] = [
            ((10.0, 10.0, 20.0, 10.0), true, true),
            ((90.0, 10.0, 20.0, 10.0), true, true),
            ((-10.0, -5.0, 20.0, 10.0), true, true),
            ((10.0, 75.0, 20.0, 10.0), true, true),
            ((100.0, 10.0, 20.0, 10.0), false, false),
            ((-20.0, 10.0, 20.0, 10.0), false, false),
            ((10.0, 80.0, 20.0, 10.0), false, false),
            ((10.0, -10.0, 20.0, 10.0), false, false),
            ((110.0, 10.0, 20.0, 10.0), false, true),
            ((-40.0, 10.0, 20.0, 10.0), false, true),
            ((10.0, 90.0, 20.0, 10.0), false, true),
            ((10.0, -30.0, 20.0, 10.0), false, true),
            // Sizes only a hand-edited file has, before `Layout::clamp`: none, or turned back.
            ((10.0, 10.0, 0.0, 0.0), true, false),
            ((10.0, 10.0, -5.0, 10.0), true, false),
            ((10.0, 10.0, f64::NAN, 10.0), false, false),
        ];
        let page = |at: (f64, f64, f64, f64)| {
            let mut l = Layout::new("Pictures", 100.0, 80.0);
            l.add("Picture", (10.0, 10.0, 20.0, 10.0), ElementKind::Picture { image: String::new(), source: None });
            if let Some(e) = l.elements.first_mut() {
                (e.x, e.y, e.w, e.h) = at;
            }
            l
        };
        let draw = |l: &Layout| render(l, &mut Vec::new(), 96.0, &PageInfo::default()).unwrap();
        let blank = draw(&Layout::new("Pictures", 100.0, 80.0));
        for (at, listed, clear) in boxes {
            let l = page(at);
            assert_eq!(pictures_not_drawn(&l).len(), usize::from(listed), "{at:?}");
            if clear {
                assert_eq!(draw(&l).data() != blank.data(), listed, "{at:?}");
            }
        }
        // `Layout::clamp`, which every layout read or edited goes through, gives a box a size.
        let mut l = page((10.0, 10.0, f64::NAN, 10.0));
        l.clamp();
        assert_eq!(pictures_not_drawn(&l).len(), 1);
    }

    /// One notice per kind, counted, the first ten named with their file's name, never a folder.
    #[test]
    fn the_picture_notices_count_and_name_each_kind() {
        let mut gaps: Vec<PictureGap> = (1..=12)
            .map(|id| PictureGap {
                id,
                name: format!("Logo {id}"),
                layout: None,
                file: Some("north.png".to_string()),
                why: PictureLoss::NoImage,
            })
            .collect();
        gaps.push(PictureGap { id: 13, name: "Inset".to_string(), layout: None, file: None, why: PictureLoss::Undrawable });
        let said = pictures_notices(&gaps);
        assert_eq!(said.len(), 2, "{said:?}");
        let head = "12 layout picture(s) have no image and are drawn as a grey box: Logo 1 (1, north.png); ";
        assert!(said[0].starts_with(head), "{}", said[0]);
        assert!(said[0].contains("Logo 10 (10, north.png); and 2 more; read one"), "{}", said[0]);
        assert!(!said[0].contains("Logo 11"), "{}", said[0]);
        assert!(said[0].contains("; and 2 more; read one from its file again with layout.update and \"path\""), "{}", said[0]);
        let undrawable = "1 layout picture(s) have an image that can't be drawn (damaged, or too large: over 20000 pixels a side or over 512 MB once decoded)";
        assert!(said[1].starts_with(undrawable), "{}", said[1]);
        assert!(said[1].contains("drawn as a grey box: Inset (13); read one"), "{}", said[1]);
        assert!(pictures_notices(&[]).is_empty());
    }

    /// A long name is cut to 100 characters and "…"; in a report, the name and its layout's are
    /// each cut, so the layout is still named.
    #[test]
    fn a_long_picture_name_is_cut_and_keeps_its_layout() {
        let long = "n".repeat(150);
        let gap = |layout: Option<&str>| PictureGap {
            id: 4,
            name: long.clone(),
            layout: layout.map(String::from),
            file: Some("north.png".to_string()),
            why: PictureLoss::NoImage,
        };
        let alone = pictures_notices(&[gap(None)]);
        let cut = format!(": {}… (4, north.png);", "n".repeat(100));
        assert!(alone[0].contains(&cut), "{}", alone[0]);
        let in_report = pictures_notices(&[gap(Some("L"))]);
        assert!(in_report[0].contains(&format!("{}… in layout 'L' (4, north.png);", "n".repeat(100))), "{}", in_report[0]);
        let long_layout = pictures_notices(&[PictureGap { layout: Some("l".repeat(150)), ..gap(None) }]);
        assert!(long_layout[0].contains(&format!(" in layout '{}…' (4, north.png);", "l".repeat(100))), "{}", long_layout[0]);
    }

    /// A picture is decoded once while it is unchanged, however often its page is drawn and
    /// checked.
    #[test]
    fn a_layout_picture_is_decoded_once_while_it_is_unchanged() {
        // Colours no other test draws, so no other picture is this one.
        let first = png_b64(23, 17, |x, y| [0x3A, (x * 7) as u8, (y * 11) as u8]);
        let mut l = one_picture_page(&first, None);
        let info = PageInfo::default();
        let before = crate::picture::decodes();
        for _ in 0..3 {
            render(&l, &mut Vec::new(), 96.0, &info).unwrap();
        }
        assert!(pictures_not_drawn(&l).is_empty());
        assert!(pictures_not_drawn(&l).is_empty());
        assert!(picture_draws(&first));
        assert_eq!(crate::picture::decodes() - before, 1);
        // The image changed to another PNG: one more.
        let second = png_b64(23, 17, |x, y| [0x3B, (y * 5) as u8, (x * 9) as u8]);
        if let Some(e) = l.elements.first_mut() {
            e.kind = ElementKind::Picture { image: second, source: None };
        }
        render(&l, &mut Vec::new(), 96.0, &info).unwrap();
        assert!(pictures_not_drawn(&l).is_empty());
        assert_eq!(crate::picture::decodes() - before, 2);
    }
}
