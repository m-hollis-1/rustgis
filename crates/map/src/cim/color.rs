//! A CIM colour as an [`Rgba`]. CIM has six kinds, each `{"type": "CIM…Color", "values": […]}`
//! with the alpha last and as a percentage (0–100, a missing one is opaque). A colour that can't
//! be read gives `None`, so a reader skips that one symbol layer and notes it instead of drawing
//! a wrong colour.

use serde_json::Value;

use super::{str_of, typed};
use crate::Rgba;

/// The colour `v` (a `CIM…Color` object) stands for.
#[allow(dead_code)] // used by the symbol and layout readers built on this module
pub(crate) fn color(v: &Value) -> Option<Rgba> {
    let values = v.get("values")?.as_array()?;
    let at = |i: usize| values.get(i).and_then(Value::as_f64).filter(|n| n.is_finite());
    // The channel at `i` (required), as a fraction of `scale`, clamped to 0..=1.
    let frac = |i: usize, scale: f64| at(i).map(|n| (n / scale).clamp(0.0, 1.0));
    let alpha = |i: usize| at(i).map_or(1.0, |n| (n / 100.0).clamp(0.0, 1.0));
    let (rgb, a) = match str_of(v, "type")? {
        "CIMRGBColor" => ([frac(0, 255.0)?, frac(1, 255.0)?, frac(2, 255.0)?], alpha(3)),
        "CIMGrayColor" => {
            let g = frac(0, 100.0)?;
            ([g, g, g], alpha(1))
        }
        "CIMCMYKColor" => {
            let (c, m, y, k) = (frac(0, 100.0)?, frac(1, 100.0)?, frac(2, 100.0)?, frac(3, 100.0)?);
            ([(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)], alpha(4))
        }
        "CIMHSVColor" => (hsv(hue(at(0)?), frac(1, 100.0)?, frac(2, 100.0)?), alpha(3)),
        "CIMHSLColor" => (hsl(hue(at(0)?), frac(1, 100.0)?, frac(2, 100.0)?), alpha(3)),
        "CIMLABColor" => (lab(at(0)?, at(1)?, at(2)?), alpha(3)),
        _ => return None,
    };
    let byte = |f: f64| (f * 255.0).round().clamp(0.0, 255.0) as u8;
    Some(Rgba::new(byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(a)))
}

/// The symbol a `CIMSymbolReference` holds, or `v` itself when it is a symbol already (it has
/// `symbolLayers`). Layers and renderers hold either.
#[allow(dead_code)]
pub(crate) fn symbol_of(v: &Value) -> Option<&Value> {
    if typed(v, "CIMSymbolReference").is_some() {
        return v.get("symbol");
    }
    v.get("symbolLayers").is_some().then_some(v)
}

/// A hue in degrees, wrapped into 0..360.
fn hue(h: f64) -> f64 {
    h.rem_euclid(360.0)
}

/// RGB (0..=1) of a hue (degrees), saturation and value (0..=1).
fn hsv(h: f64, s: f64, v: f64) -> [f64; 3] {
    let c = v * s;
    chroma_to_rgb(h, c, v - c)
}

/// RGB (0..=1) of a hue (degrees), saturation and lightness (0..=1).
fn hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    chroma_to_rgb(h, c, l - c / 2.0)
}

/// The shared last step of HSV and HSL: the sector of the hue circle picks which channels get the
/// chroma `c` and the intermediate value, and `m` lifts all three.
fn chroma_to_rgb(h: f64, c: f64, m: f64) -> [f64; 3] {
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// sRGB (0..=1) of a CIE L*a*b* colour, through XYZ with the D65 white point.
fn lab(l: f64, a: f64, b: f64) -> [f64; 3] {
    const WHITE: [f64; 3] = [0.95047, 1.0, 1.08883];
    let fy = (l.clamp(0.0, 100.0) + 16.0) / 116.0;
    let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
    let inv = |t: f64| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f64 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let (x, y, z) = (inv(fx) * WHITE[0], inv(fy) * WHITE[1], inv(fz) * WHITE[2]);
    let linear = [
        3.2404542 * x - 1.5371385 * y - 0.4985314 * z,
        -0.969266 * x + 1.8760108 * y + 0.041556 * z,
        0.0556434 * x - 0.2040259 * y + 1.0572252 * z,
    ];
    linear.map(|c| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
    })
}
