//! Picture graphics: PNG or JPEG images (base64 in the graphic's `image` attribute) stretched over
//! the graphic's rectangle.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine;
use rustgis_data::{Dataset, Value};
use rustgis_map::ImageRef;
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

/// Pictures are drawn from at most this many pixels per side (bigger ones are scaled down).
const MAX_SIDE: u32 = 4096;
/// Pictures declaring more pixels per side than this aren't decoded at all.
const MAX_DECLARED: u32 = 20_000;
/// Most memory a picture's decoder may allocate.
const MAX_ALLOC: u64 = 512 << 20;
/// Decoded pictures kept between frames, and the most pixel memory they may hold together.
const CACHE: usize = 32;
const CACHE_BYTES: usize = 256 << 20;

/// Decoded pictures (with the base64 they came from, compared on every hit so pictures whose
/// cheap keys collide never swap), and their keys oldest first (for eviction).
#[derive(Default)]
struct Cache {
    images: HashMap<u64, (String, Option<Pixmap>)>,
    order: std::collections::VecDeque<u64>,
}

thread_local! {
    static DECODED: RefCell<Cache> = RefCell::new(Cache::default());
}

/// A cheap key for a picture: its length and its first and last bytes (hashing megabytes of
/// base64 every frame would be slow).
fn key(b64: &str) -> u64 {
    let bytes = b64.as_bytes();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut h);
    bytes.get(..bytes.len().min(512)).hash(&mut h);
    bytes.get(bytes.len().saturating_sub(512)..).hash(&mut h);
    h.finish()
}

/// The pixel size of base64 PNG or JPEG bytes, read from the header without decoding the image.
pub fn size(b64: &str) -> Option<(u32, u32)> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

/// Decodes base64 PNG or JPEG bytes into a premultiplied pixmap. Pictures come from project
/// files, so their declared size is checked before decoding and the decoder's memory is capped.
pub fn decode(b64: &str) -> Option<Pixmap> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let reader = || image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok();
    let (w, h) = reader()?.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > MAX_DECLARED || h > MAX_DECLARED {
        return None;
    }
    let mut r = reader()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_ALLOC);
    r.limits(limits);
    let img = r.decode().ok()?;
    let img = if img.width() > MAX_SIDE || img.height() > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut pm = Pixmap::new(w, h)?;
    for (dst, src) in pm.pixels_mut().iter_mut().zip(rgba.pixels()) {
        let [r, g, b, a] = src.0;
        let c = tiny_skia::ColorU8::from_rgba(r, g, b, a);
        *dst = c.premultiply();
    }
    Some(pm)
}

/// Picture markers are kept at most this many pixels per side (a marker is drawn small).
const MARKER_SIDE: u32 = 256;

/// Decoded picture markers by the address of their interned text, which never moves or goes: a
/// picture that doesn't decode is kept as `None` so it isn't tried again for every point. The
/// table holds at most [`ImageRef::MAX_IMAGES`] pictures.
type MarkerCache = Mutex<HashMap<usize, Option<Arc<Pixmap>>>>;

/// A picture marker's pixels (premultiplied, at most [`MARKER_SIDE`] a side), `None` when its
/// picture can't be read.
pub(crate) fn marker_image(r: &ImageRef) -> Option<Arc<Pixmap>> {
    static CACHE: OnceLock<MarkerCache> = OnceLock::new();
    let key = r.as_str().as_ptr() as usize;
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return hit.clone();
    }
    let made = decode(r.as_str()).map(|pm| Arc::new(small(pm)));
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, made.clone());
    made
}

/// `pm` scaled down to [`MARKER_SIDE`] a side, when it is larger.
fn small(pm: Pixmap) -> Pixmap {
    let k = MARKER_SIDE as f32 / pm.width().max(pm.height()) as f32;
    if k >= 1.0 {
        return pm;
    }
    let (w, h) = (((pm.width() as f32 * k).round() as u32).max(1), ((pm.height() as f32 * k).round() as u32).max(1));
    let Some(mut out) = Pixmap::new(w, h) else { return pm };
    let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
    out.draw_pixmap(0, 0, pm.as_ref(), &paint, Transform::from_scale(w as f32 / pm.width() as f32, h as f32 / pm.height() as f32), None);
    out
}

/// Draws row `row`'s picture into the pixel rectangle (x0, y0)–(x1, y1).
pub fn draw(pm: &mut Pixmap, ds: &Dataset, row: usize, (x0, y0, x1, y1): (f32, f32, f32, f32), opacity: f32) {
    let Some(Value::Text(b64)) = ds.field_index("image").map(|fi| ds.value(row, fi)) else { return };
    let (w, h) = ((x1 - x0).abs(), (y1 - y0).abs());
    if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 || w > 1e5 || h > 1e5 {
        return;
    }
    let key = key(b64);
    DECODED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.images.get(&key).is_none_or(|(src, _)| src.as_str() != b64.as_str()) {
            // Evict the oldest pictures (not all of them) to stay within the count and memory budget.
            let held = |c: &Cache| c.images.values().filter_map(|(_, p)| p.as_ref()).map(|p| p.data().len()).sum::<usize>();
            while cache.images.len() >= CACHE || (!cache.images.is_empty() && held(&cache) > CACHE_BYTES) {
                let Some(old) = cache.order.pop_front() else { break };
                cache.images.remove(&old);
            }
            if cache.images.insert(key, (b64.clone(), decode(b64))).is_none() {
                cache.order.push_back(key);
            }
        }
        let Some((_, Some(img))) = cache.images.get(&key) else { return };
        let t = Transform::from_row(w / img.width() as f32, 0.0, 0.0, h / img.height() as f32, x0.min(x1), y0.min(y1));
        let paint = PixmapPaint { opacity, quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
        pm.draw_pixmap(0, 0, img.as_ref(), &paint, t, None);
    });
}
