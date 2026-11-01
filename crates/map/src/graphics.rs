//! Graphics layers: shapes, text and pictures drawn on the map, each carrying its own style.
//!
//! A graphics layer is a layer of mixed geometry whose features keep their look in attributes
//! ([`FIELDS`]), so every graphic can differ. Text graphics are drawn by the label engine (centred
//! on their point), and dynamic text expands tokens such as `{scale}` when drawn.

use rustgis_data::{Dataset, Field, FieldType, GeometryType, Value};

use crate::layout::{MAX_LETTER_SPACING, MIN_LETTER_SPACING, TextAlign};
use crate::{Marker, Rgba, Symbol};

/// The graphic kinds.
pub const SHAPES: &[&str] =
    &["point", "text", "line", "curve", "freehand", "polygon", "lasso", "rectangle", "circle", "ellipse", "picture"];

/// The attribute fields of a graphics layer: (name, type).
pub const FIELDS: &[(&str, FieldType)] = &[
    ("kind", FieldType::Text),
    ("text", FieldType::Text),
    ("fill", FieldType::Text),
    ("outline", FieldType::Text),
    ("width", FieldType::Double),
    ("size", FieldType::Double),
    ("font_size", FieldType::Double),
    ("image", FieldType::Text),
    // Last: graphics layers made before them lack them, and read them as plain text (#516, #535).
    ("bold", FieldType::Boolean),
    ("italic", FieldType::Boolean),
    ("align", FieldType::Text),
    ("letter_spacing", FieldType::Double),
];

/// Tokens dynamic text expands: (token, description).
pub const TOKENS: &[(&str, &str)] = &[
    ("{map}", "Map name"),
    ("{scale}", "Map scale"),
    ("{date}", "Current date"),
    ("{time}", "Current time"),
    ("{crs}", "Coordinate system"),
    ("{layers}", "Number of layers"),
    ("{center}", "Map centre"),
    ("{project}", "Project name"),
    ("{credits}", "Map sources' credits"),
    ("{crsName}", "Coordinate system name"),
    ("{crsProjection}", "Coordinate system projection"),
    ("{crsUnits}", "Coordinate system units"),
    ("{crsWkid}", "Coordinate system code"),
    ("{extent}", "Map extent"),
];

/// An empty graphics dataset.
pub fn dataset(name: &str) -> Dataset {
    Dataset::new(name, GeometryType::None, FIELDS.iter().map(|(n, t)| Field::new(*n, *t)).collect())
}

fn text<'a>(ds: &'a Dataset, row: usize, field: &str) -> Option<&'a str> {
    match ds.field_index(field).map(|fi| ds.value(row, fi)) {
        Some(Value::Text(t)) => Some(t.as_str()),
        _ => None,
    }
}

fn num(ds: &Dataset, row: usize, field: &str) -> Option<f64> {
    ds.field_index(field).and_then(|fi| ds.value(row, fi).as_f64()).filter(|v| v.is_finite())
}

/// The graphic kind of a row.
pub fn kind(ds: &Dataset, row: usize) -> &str {
    text(ds, row, "kind").unwrap_or("")
}

/// The text a row shows (text graphics only).
pub fn label(ds: &Dataset, row: usize) -> Option<&str> {
    if kind(ds, row) == "text" { text(ds, row, "text").filter(|t| !t.is_empty()) } else { None }
}

/// The font sizes text is drawn at, in points: a text graphic (`graphics.add`, `graphics.edit`,
/// Add Text) and a layout's text (`layout.addElement`, `layout.update`), so the same text can
/// look the same on the map and on the page (#516).
pub const FONT_SIZE: std::ops::RangeInclusive<f64> = 2.0..=400.0;

/// The font size of a text graphic, in points.
pub fn font_size(ds: &Dataset, row: usize) -> f32 {
    num(ds, row, "font_size").unwrap_or(14.0).clamp(*FONT_SIZE.start(), *FONT_SIZE.end()) as f32
}

/// Whether a text graphic is drawn bold; false where the layer has no `bold` field (graphics
/// layers made before it).
pub fn bold(ds: &Dataset, row: usize) -> bool {
    num(ds, row, "bold").is_some_and(|v| v != 0.0)
}

/// Whether a text graphic is drawn italic (#535); false where the layer has no `italic` field.
pub fn italic(ds: &Dataset, row: usize) -> bool {
    num(ds, row, "italic").is_some_and(|v| v != 0.0)
}

/// How a text graphic's lines are aligned in their block, which stays centred on its point
/// (#535): left where the layer has no `align` field or it holds something else (typed in the
/// attribute table), as text graphics have always been laid out.
pub fn align(ds: &Dataset, row: usize) -> TextAlign {
    text(ds, row, "align").and_then(TextAlign::from_name).unwrap_or_default()
}

/// A text graphic's letter spacing in points (#535), within a layout text's range; 0 where the
/// layer has no `letter_spacing` field.
pub fn letter_spacing(ds: &Dataset, row: usize) -> f32 {
    num(ds, row, "letter_spacing").unwrap_or(0.0).clamp(MIN_LETTER_SPACING, MAX_LETTER_SPACING) as f32
}

/// The colour text graphics are drawn in when their `fill` doesn't say.
pub const TEXT_COLOR: Rgba = Rgba::rgb(0x20, 0x20, 0x20);

/// The text colour of a text graphic.
pub fn text_color(ds: &Dataset, row: usize) -> Rgba {
    text(ds, row, "fill").and_then(Rgba::from_hex).unwrap_or(TEXT_COLOR)
}

/// The symbol a graphic draws with.
pub fn symbol(ds: &Dataset, row: usize) -> Symbol {
    let fill = text(ds, row, "fill").and_then(Rgba::from_hex).unwrap_or(Rgba::new(0x4A, 0x90, 0xD9, 90));
    let stroke = text(ds, row, "outline").and_then(Rgba::from_hex).unwrap_or(Rgba::rgb(0x1F, 0x4E, 0x8C));
    let width = num(ds, row, "width").unwrap_or(1.5).clamp(0.0, 50.0) as f32;
    let size = num(ds, row, "size").unwrap_or(10.0).clamp(0.0, 200.0) as f32;
    match kind(ds, row) {
        // Text and pictures draw themselves; their geometry stays invisible.
        "text" | "picture" => {
            Symbol { fill: Rgba::new(0, 0, 0, 0), stroke: Rgba::new(0, 0, 0, 0), stroke_width: 0.0, size: 0.0, marker: Marker::Circle }
        }
        // A line's colour is its outline.
        "line" | "curve" | "freehand" => Symbol { fill: stroke, stroke, stroke_width: width, size, marker: Marker::Circle },
        _ => Symbol { fill, stroke, stroke_width: width, size, marker: Marker::Circle },
    }
}

/// What dynamic text tokens stand for when a map or a layout page is drawn.
pub struct TokenValues {
    pub map: String,
    pub scale: f64,
    pub crs: String,
    pub layers: usize,
    pub center: rustgis_geom::Coord,
    /// Seconds since the Unix epoch.
    pub now: u64,
    /// A layout page's 1-based number and count, its name (a map series has many) and its
    /// layout's name; 0 pages and empty names stand for nothing.
    pub page: usize,
    pub pages: usize,
    pub page_name: String,
    pub layout: String,
    /// Whether numbers print with a decimal comma (the scale's points and commas swap).
    pub decimal_comma: bool,
    pub project: String,
    /// The map's web layers' attributions.
    pub credits: String,
    /// The coordinate system's name, projection, linear units and code.
    pub crs_name: String,
    pub crs_projection: String,
    pub crs_units: String,
    pub crs_wkid: String,
    /// The map's extent in degrees.
    pub extent: Option<rustgis_geom::Extent>,
}

impl Default for TokenValues {
    fn default() -> Self {
        TokenValues {
            map: String::new(),
            scale: 0.0,
            crs: String::new(),
            layers: 0,
            center: rustgis_geom::Coord::default(),
            now: 0,
            page: 0,
            pages: 0,
            page_name: String::new(),
            layout: String::new(),
            decimal_comma: false,
            project: String::new(),
            credits: String::new(),
            crs_name: String::new(),
            crs_projection: String::new(),
            crs_units: String::new(),
            crs_wkid: String::new(),
            extent: None,
        }
    }
}

impl TokenValues {
    /// The values a map's dynamic text shows at `scale`, now.
    pub fn of(map: &crate::Map, scale: f64) -> Self {
        let crs = map.projection.info();
        TokenValues {
            map: map.name.clone(),
            scale,
            crs: map.projection.label(),
            crs_name: crs.name,
            crs_projection: crs.method,
            crs_units: crs.units,
            crs_wkid: crs.wkid,
            layers: map.layers.iter().filter(|l| !l.graphics).count(),
            center: map.center_lonlat(),
            now: web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            ..Default::default()
        }
    }
}

/// The longest token body (between the braces) read; a longer one is plain text.
const MAX_TOKEN: usize = 200;

/// Expands dynamic text tokens: `{name}`, `{name|prefix}` and `{name|prefix|suffix}`, where the
/// prefix and suffix are text (a backslash and `n` for a line break) printed only when the token
/// has a value. Unknown names are left as they are, braces and all.
pub fn expand(text: &str, v: &TokenValues) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let (date, time) = civil(v.now);
    // One pass, so a value containing a token (a map named "{date}") is left as it is.
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let tail = rest.get(i..).unwrap_or("");
        // The closing brace within the longest body, so a stray "{" doesn't scan the whole text.
        let close = tail.char_indices().skip(1).take(MAX_TOKEN + 1).find(|(_, c)| *c == '}').map(|(j, _)| j);
        let body = close.and_then(|j| tail.get(1..j));
        let mut parts = body.unwrap_or("").split('|');
        let name = parts.next().unwrap_or("");
        match (body.and_then(|_| token_value(name, v, &date, &time)), close) {
            (Some(val), Some(j)) => {
                if !val.is_empty() {
                    push_literal(&mut out, parts.next().unwrap_or(""));
                    out.push_str(&val);
                    push_literal(&mut out, parts.next().unwrap_or(""));
                }
                rest = tail.get(j + 1..).unwrap_or("");
            }
            _ => {
                out.push('{');
                rest = tail.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// What the token `name` stands for; empty when it has no value, `None` for no such token.
fn token_value(name: &str, v: &TokenValues, date: &str, time: &str) -> Option<String> {
    let number = |n: usize| if n == 0 { String::new() } else { n.to_string() };
    Some(match name {
        "map" => v.map.clone(),
        "scale" if v.scale > 0.0 => decimal(&scale_ratio(v.scale), v.decimal_comma),
        "scale" => String::new(),
        "date" => date.to_string(),
        "time" => time.to_string(),
        "crs" => v.crs.clone(),
        "layers" => number(v.layers),
        "center" => rustgis_geom::format_lonlat(v.center),
        "page" => number(v.page),
        "pages" => number(v.pages),
        "pageName" => v.page_name.clone(),
        "layout" => v.layout.clone(),
        "project" => v.project.clone(),
        "credits" => v.credits.clone(),
        "crsName" => v.crs_name.clone(),
        "crsProjection" => v.crs_projection.clone(),
        "crsUnits" => v.crs_units.clone(),
        "crsWkid" => v.crs_wkid.clone(),
        "extent" => match v.extent {
            Some(e) if !e.is_empty() && [e.min_x, e.min_y, e.max_x, e.max_y].iter().all(|c| c.is_finite()) => {
                format!("{:.4}, {:.4}, {:.4}, {:.4}", e.min_x, e.min_y, e.max_x, e.max_y)
            }
            _ => String::new(),
        },
        _ => return None,
    })
}

/// Adds a token's prefix or suffix, a backslash and `n` in it a line break.
fn push_literal(out: &mut String, text: &str) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x5c' && chars.peek() == Some(&'n') {
            chars.next();
            out.push('\n');
        } else {
            out.push(c);
        }
    }
}

/// A number as printed with a decimal comma when `comma` (`0.05` to `0,05`, `6,300,000` to
/// `6.300.000`): its points and commas swapped.
pub fn decimal(s: &str, comma: bool) -> String {
    if !comma {
        return s.to_string();
    }
    s.chars()
        .map(|c| match c {
            '.' => ',',
            ',' => '.',
            c => c,
        })
        .collect()
}

/// A scale denominator as a ratio: `1:12,345`, or `2:1` and `2.5:1` once the map is larger than
/// life.
pub fn scale_ratio(scale: f64) -> String {
    // `max` reads NaN as 0: no scale is 1:0, as `thousands` shows nothing below it.
    if scale.is_nan() || scale <= 0.0 || scale >= 1.0 {
        return format!("1:{}", thousands(scale.max(0.0)));
    }
    let times = 1.0 / scale;
    let times = if times < 10.0 { format!("{times:.1}") } else { thousands(times) };
    format!("{}:1", times.strip_suffix(".0").unwrap_or(&times))
}

/// `v` rounded, its thousands apart with commas (`6,300,000`).
pub fn thousands(v: f64) -> String {
    let s = format!("{:.0}", v.clamp(0.0, 1e15));
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// (YYYY-MM-DD, HH:MM) in UTC, of `secs` since 1970.
pub fn civil(secs: u64) -> (String, String) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Days to civil date (proleptic Gregorian), after Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}:{:02}", rem / 3600, rem % 3600 / 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_expand() {
        let v = TokenValues {
            map: "World".into(),
            scale: 25_000_000.0,
            crs: "Web Mercator".into(),
            layers: 4,
            center: rustgis_geom::Coord::new(0.0, 0.0),
            now: 1_760_140_800, // 2025-10-11 00:00 UTC
            ..Default::default()
        };
        assert_eq!(expand("{map} at {scale}", &v), "World at 1:25,000,000");
        assert_eq!(expand("{date} {time} · {layers}", &v), "2025-10-11 00:00 · 4");
        assert_eq!(expand("plain", &v), "plain");
        let named = TokenValues { map: "{date}".into(), ..v };
        assert_eq!(expand("{map} {nope} {", &named), "{date} {nope} {");
    }

    #[test]
    fn tokens_take_prefix_and_suffix_dropped_when_empty() {
        let mut v = TokenValues { crs_name: "NAD 1983 StatePlane Texas Central FIPS 4203 (US Feet)".into(), ..Default::default() };
        assert_eq!(expand("{crsName|Name: |\\n}{crsWkid|WKID: |\\n}", &v), "Name: NAD 1983 StatePlane Texas Central FIPS 4203 (US Feet)\n");
        assert_eq!(expand("{scale|Scale }", &v), "");
        v.scale = 24_000.0;
        assert_eq!(expand("{scale|Scale }", &v), "Scale 1:24,000");
        assert_eq!(expand("{nothing|x|y}", &v), "{nothing|x|y}");
        // A prefix that starts with a line break (Pro's newLine), only when there is a value.
        let v = TokenValues { map: "World".into(), crs_name: "WGS 1984".into(), ..Default::default() };
        assert_eq!(
            expand("Spatial Reference{map|\\nName: }{crsWkid|\\nWKID: }{crsName|\\n}", &v),
            "Spatial Reference\nName: World\nWGS 1984"
        );
    }

    #[test]
    fn page_tokens_expand_here_now() {
        let v = TokenValues { page: 2, pages: 5, page_name: "North".into(), layout: "Atlas".into(), ..Default::default() };
        assert_eq!(expand("{page} of {pages}, {pageName} · {layout}", &v), "2 of 5, North · Atlas");
    }

    #[test]
    fn empty_values_leave_nothing_and_odd_tokens_stay_as_written() {
        let v = TokenValues { layers: 0, project: "Parcels".into(), ..Default::default() };
        // Counts of 0, an empty string and an unset extent print nothing, prefix and suffix included.
        assert_eq!(expand("[{layers|n=}{map|m=|;}{extent|e=}{page|p=}]", &v), "[]");
        assert_eq!(expand("{project|<|>}", &v), "<Parcels>");
        // The prefix alone; a third `|` ends the suffix.
        assert_eq!(expand("{project|by }", &v), "by Parcels");
        assert_eq!(expand("{project||!|ignored}", &v), "Parcels!");
        // No closing brace, or a body over 200 characters: plain text.
        assert_eq!(expand("{project", &v), "{project");
        let pad = "x".repeat(200 - "project|".len());
        assert_eq!(expand(&format!("{{project|{pad}}}"), &v), format!("{pad}Parcels"));
        let long = format!("{{project|{pad}x}}");
        assert_eq!(expand(&long, &v), long);
        // A stray brace before a token doesn't hide it.
        assert_eq!(expand("{ {project}", &v), "{ Parcels");
    }

    #[test]
    fn the_extent_prints_in_degrees_and_the_scale_honours_the_decimal_comma() {
        let v = TokenValues {
            extent: Some(rustgis_geom::Extent::new(-97.5, 30.25, -97.0, 30.75)),
            scale: 6_300_000.0,
            decimal_comma: true,
            ..Default::default()
        };
        assert_eq!(expand("{extent}", &v), "-97.5000, 30.2500, -97.0000, 30.7500");
        assert_eq!(expand("{scale} {extent|x}", &v), "1:6.300.000 x-97.5000, 30.2500, -97.0000, 30.7500");
        let empty = TokenValues { extent: Some(rustgis_geom::Extent::EMPTY), ..Default::default() };
        assert_eq!(expand("{extent|e}", &empty), "");
        assert_eq!(decimal("0.05", true), "0,05");
        assert_eq!(decimal("6,300,000", true), "6.300.000");
        assert_eq!(decimal("0.05", false), "0.05");
    }

    #[test]
    fn scales_larger_than_life_read_as_ratios() {
        assert_eq!(scale_ratio(20.0), "1:20");
        assert_eq!(scale_ratio(1.4), "1:1");
        // Just under 1:1 (a scale set to 1, back from metres per pixel), still 1:1.
        assert_eq!(scale_ratio(0.999_999), "1:1");
        assert_eq!(scale_ratio(0.7), "1.4:1");
        assert_eq!(scale_ratio(0.5), "2:1");
        assert_eq!(scale_ratio(0.04), "25:1");
        assert_eq!(scale_ratio(0.0005), "2,000:1");
        for nothing in [0.0, -3.0, f64::NAN, f64::NEG_INFINITY] {
            assert_eq!(scale_ratio(nothing), "1:0", "{nothing}");
        }
        assert_eq!(scale_ratio(f64::INFINITY), "1:1,000,000,000,000,000");
    }

    /// A map's own dynamic text names its coordinate system too.
    #[test]
    fn a_maps_text_names_its_coordinate_system() {
        let mut map = crate::Map::new("m");
        map.projection = crate::Projection::from_code("EPSG:2277").unwrap();
        let v = TokenValues::of(&map, 1000.0);
        assert_eq!(
            expand("{crsName} / {crsProjection} / {crsUnits} / {crsWkid}", &v),
            "NAD83 / Texas Central (ftUS) / Lambert Conformal Conic / US survey foot / 2277"
        );
    }
}
