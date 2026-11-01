//! Character markers ([`Marker::Char`](rustgis_map::Marker::Char)): a character of an installed
//! family as a marker's outline, placed as QGIS places a font marker: the em is the marker's size,
//! the character's advance is centred on the point and its baseline is half the font's ascent
//! below it. A family the machine lacks, or one without the character, draws it in the label font,
//! except an Esri symbol font's (ESRI North), whose characters are pictures: those draw nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use ab_glyph::{Font, OutlineCurve};
use rustgis_map::Glyph;
use tiny_skia::{Path, PathBuilder};

/// The most characters kept, and the most curves read from one (a crafted font's glyph).
const MAX_KEPT: usize = 4096;
const MAX_CURVES: usize = 20_000;

/// A character's outline one em across, the point at the origin (y down), and the furthest it
/// reaches from the point, in ems.
#[derive(Debug)]
pub(crate) struct CharShape {
    pub path: Path,
    pub reach: f32,
}

/// `g`'s outline (see the module), read once and kept; `None` for a character with no outline
/// (a space) in its family and in the label font.
pub(crate) fn shape(g: Glyph) -> Option<Arc<CharShape>> {
    static KEPT: OnceLock<Mutex<HashMap<Glyph, Option<Arc<CharShape>>>>> = OnceLock::new();
    let kept = KEPT.get_or_init(Default::default);
    if let Some(s) = kept.lock().ok()?.get(&g) {
        return s.clone();
    }
    let (s, settled) = read(g);
    // Not kept while the installed families are still being indexed: drawn in the label font for
    // now, in its own family once they are.
    if settled
        && let Ok(mut kept) = kept.lock()
        && kept.len() < MAX_KEPT
    {
        kept.insert(g, s.clone());
    }
    s
}

/// Whether `family` is one of Esri's symbol fonts ("ESRI North", "ESRI Default Marker", in any
/// case): its characters are pictures, so the label font's character of the same code ("²" for
/// ESRI North's arrow 178) would be a wrong mark, not a stand-in.
pub(crate) fn symbol_family(family: &str) -> bool {
    family.trim_start().get(..5).is_some_and(|p| p.eq_ignore_ascii_case("ESRI "))
}

/// Whether `g` is a character of an Esri symbol font (see [`symbol_family`]) this machine lacks,
/// once the installed families are indexed: it draws nothing, and a north arrow draws its Classic
/// arrow in its place (see `layout::symbol_arrow`).
pub(crate) fn symbol_font_missing(g: Glyph) -> bool {
    symbol_family(g.font.as_str()) && matches!(crate::text::marker_font(g.font.as_str()), (None, true))
}

/// `g`'s outline, and whether it is final (its family's face, or the label font's because the
/// family isn't installed rather than not indexed yet). An Esri symbol font's character is never
/// drawn in the label font.
fn read(g: Glyph) -> (Option<Arc<CharShape>>, bool) {
    let (named, settled) = crate::text::marker_font(g.font.as_str());
    let from = |f: &ab_glyph::FontRef<'static>| {
        let id = f.glyph_id(g.ch);
        (id.0 != 0).then(|| outline(f, id)).flatten()
    };
    let s = named.and_then(from);
    let s = if symbol_family(g.font.as_str()) { s } else { s.or_else(|| crate::text::label_font().and_then(from)) };
    (s.map(Arc::new), settled)
}

fn outline(f: &ab_glyph::FontRef<'static>, id: ab_glyph::GlyphId) -> Option<CharShape> {
    let upm = f.units_per_em().filter(|u| u.is_finite() && *u > 0.0)?;
    let advance = f.h_advance_unscaled(id);
    let ascent = f.ascent_unscaled();
    let (dx, dy) = (if advance.is_finite() { advance / 2.0 } else { 0.0 }, if ascent.is_finite() { ascent / 2.0 } else { 0.0 });
    // Font units (y up, from the origin of the advance and the baseline) to ems about the point.
    let at = |p: ab_glyph::Point| ((p.x - dx) / upm, (dy - p.y) / upm);
    let o = f.outline(id)?;
    let mut pb = PathBuilder::new();
    let mut last: Option<ab_glyph::Point> = None;
    for c in o.curves.iter().take(MAX_CURVES) {
        let (start, end) = match c {
            OutlineCurve::Line(a, b) | OutlineCurve::Quad(a, _, b) | OutlineCurve::Cubic(a, _, _, b) => (*a, *b),
        };
        if last != Some(start) {
            if last.is_some() {
                pb.close();
            }
            let (x, y) = at(start);
            pb.move_to(x, y);
        }
        match c {
            OutlineCurve::Line(_, b) => {
                let (x, y) = at(*b);
                pb.line_to(x, y);
            }
            OutlineCurve::Quad(_, k, b) => {
                let ((kx, ky), (x, y)) = (at(*k), at(*b));
                pb.quad_to(kx, ky, x, y);
            }
            OutlineCurve::Cubic(_, k1, k2, b) => {
                let ((ax, ay), (bx, by), (x, y)) = (at(*k1), at(*k2), at(*b));
                pb.cubic_to(ax, ay, bx, by, x, y);
            }
        }
        last = Some(end);
    }
    if last.is_some() {
        pb.close();
    }
    let path = pb.finish()?;
    let b = path.bounds();
    let reach = b.left().abs().max(b.right().abs()).max(b.top().abs()).max(b.bottom().abs());
    reach.is_finite().then_some(CharShape { path, reach })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_is_centred_on_its_advance_with_its_baseline_below_the_point() {
        // The label font's "H": as wide either side of the point, its foot on the baseline half
        // the ascent below it, standing a cap height tall.
        let f = crate::text::label_font().unwrap();
        let s = shape(Glyph::new('H', "")).unwrap();
        let b = s.path.bounds();
        let upm = f.units_per_em().unwrap();
        let id = f.glyph_id('H');
        let ink = f.outline(id).unwrap().bounds;
        let advance = f.h_advance_unscaled(id);
        assert!((b.left() - (ink.min.x - advance / 2.0) / upm).abs() < 1e-3, "{b:?}");
        assert!((b.bottom() - f.ascent_unscaled() / 2.0 / upm).abs() < 0.02, "{b:?}");
        assert!(b.height() > 0.5 && b.height() < 0.8, "a cap height: {b:?}");
        assert!(s.reach >= b.bottom() && s.reach < 1.0);
    }

    #[test]
    fn a_missing_family_or_character_draws_in_the_label_font_and_a_space_draws_nothing() {
        let label = shape(Glyph::new('H', "")).unwrap();
        let missing = shape(Glyph::new('H', "No Such Family Anywhere")).unwrap();
        assert_eq!(label.path.bounds(), missing.path.bounds());
        assert!(shape(Glyph::new(' ', "")).is_none());
        assert!(shape(Glyph::new('\u{10FFFD}', "")).is_none(), "a character no font has");
    }

    #[test]
    fn a_missing_esri_symbol_font_draws_nothing_rather_than_a_text_glyph() {
        // Char 178 of ESRI North is an arrow; in a text font it is "²". A symbol font the machine
        // lacks draws nothing (not the label font's character, not a stand-in circle).
        let missing = Glyph::new('\u{B2}', "ESRI No Such Symbols");
        assert!(symbol_font_missing(missing));
        assert!(shape(missing).is_none());
        let mut pm = tiny_skia::Pixmap::new(60, 60).unwrap();
        let s = rustgis_map::Symbol {
            fill: rustgis_map::Rgba::rgb(0, 0, 0),
            stroke: rustgis_map::Rgba::rgb(0, 0, 0),
            stroke_width: 1.0,
            size: 40.0,
            marker: rustgis_map::Marker::Char(missing),
        };
        crate::draw_marker(&mut pm, 30.0, 30.0, &s, 1.0, None, 1.0);
        assert!(pm.pixels().iter().all(|p| p.alpha() == 0), "nothing drawn");
        // In any case; a text family, or none, still falls back to the label font.
        assert!(symbol_font_missing(Glyph::new('A', "esri no such symbols")));
        assert!(!symbol_font_missing(Glyph::new('\u{B2}', "No Such Family Anywhere")));
        assert!(!symbol_font_missing(Glyph::new('\u{B2}', "")));
        assert!(!symbol_font_missing(Glyph::new('\u{B2}', "ESRIOID")));
        // ESRI North itself: drawn from its own face where installed (ArcGIS Pro's folder),
        // nothing where it isn't.
        let north = Glyph::new('\u{B2}', "ESRI North");
        let label = shape(Glyph::new('\u{B2}', "")).map(|s| s.path.bounds());
        match crate::families::find("ESRI North") {
            Some(_) => assert_ne!(shape(north).map(|s| s.path.bounds()), label, "its own arrow"),
            None => assert!(symbol_font_missing(north) && shape(north).is_none()),
        }
    }
}
