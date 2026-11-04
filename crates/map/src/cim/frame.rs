//! A layout's map frames (`CIMMapFrame`): the box and outline, the map each shows and where its
//! camera is, the frame it follows, the frame whose extent it outlines. The walk in
//! [`layout`](super::layout) calls [`read`] per frame and hands what only a second pass can settle
//! to [`Frames`]: a link, an extent indicator, a text's frame, a surround's frame (see
//! [`surround`](super::surround)) and the map series' frame are all a frame's *name* in the
//! file, and the frame it names may come later in the list, so [`Frames::resolve`] turns the
//! names into ids once the whole page has been read. That is the one place names become ids.
//!
//! A frame's camera is in the spatial reference of the map it shows, which is read from the map's
//! embedded definition. Only longitude/latitude (4326) and Web Mercator (3857, 102100) are
//! converted; a frame in any other system takes the map's view, and says so. A frame without a
//! map the document holds has nothing to read the camera against, so it takes the map's view
//! too: centre (0, 0) and scale 0 stand for that, and its saved camera is kept in its
//! [`FrameBinding`].

use std::collections::{HashMap, HashSet};

use rustgis_geom::{Coord, merc_to_lonlat};
use serde_json::Value;

use super::layout::{FrameBinding, PageBox, geometry_box, is_ellipse, is_four_corner_ring, part_points, shown};
use super::surround::{PendingSeries, UNBOUND};
use super::{Document, Report, bearing_of, num, path_key, str_of, symbol};
use crate::Rgba;
use crate::layout::{ElementKind, FrameLink, FrameShape, INDICATOR_COLOR, INDICATOR_WIDTH, Layout, LinkMode, MapSeries};

/// The most points a polygon frame outline keeps.
const MAX_OUTLINE_POINTS: usize = 500;
/// The most distinct embedded maps listed.
const MAX_EMBEDDED_MAPS: usize = 500;
/// The largest scale denominator a camera may have and still be believed.
const MAX_CAMERA_SCALE: f64 = 1e9;

const NULL: Value = Value::Null;

/// What the walk keeps of a frame until the page is read: the names to turn into ids.
pub(crate) struct Pending {
    name: String,
    map_uri: Option<String>,
    saved_camera: Option<(f64, f64, f64)>,
    /// The frame followed (by name), how, and whether the rotation follows too.
    link: Option<(String, LinkMode, bool)>,
    /// The frame whose extent this one outlines (by name).
    indicator: Option<String>,
    /// The camera is "take the map's view": scale 0, which `Layout::clamp` would raise.
    scale_unset: bool,
}

/// The map frames read so far.
#[derive(Default)]
pub(crate) struct Frames {
    pub bindings: Vec<FrameBinding>,
    /// Map URIs the frames refer to, in order of first use.
    pub embedded: Vec<String>,
    seen: HashSet<String>,
    links: Vec<(u32, String, LinkMode, bool)>,
    indicators: Vec<(u32, String)>,
    /// Surrounds (legends, scale bars, north arrows, tables) and the frame name each reads.
    surrounds: Vec<(u32, String)>,
    series: Option<PendingSeries>,
    unset: Vec<u32>,
}

impl Frames {
    /// Keeps frame element `id`, read as `p`.
    pub fn add(&mut self, id: u32, p: Pending) {
        if let Some(uri) = &p.map_uri
            && self.embedded.len() < MAX_EMBEDDED_MAPS
            && self.seen.insert(path_key(uri))
        {
            self.embedded.push(uri.clone());
        }
        if let Some((name, mode, rotation)) = p.link {
            self.links.push((id, name, mode, rotation));
        }
        if let Some(name) = p.indicator {
            self.indicators.push((id, name));
        }
        if p.scale_unset {
            self.unset.push(id);
        }
        self.bindings.push(FrameBinding { element: id, name: p.name, map_uri: p.map_uri, saved_camera: p.saved_camera });
    }

    /// Keeps surround element `id`, which reads the frame called `frame`.
    pub fn add_surround(&mut self, id: u32, frame: String) {
        self.surrounds.push((id, frame));
    }

    /// Keeps the layout's map series.
    pub fn set_series(&mut self, series: Option<PendingSeries>) {
        self.series = series;
    }

    /// Turns the frame names the page used into ids: the links, the extent indicators, the frame
    /// each text reads (`text_frames`), the frame each surround reads and the map series'. A name
    /// no frame has leaves the field unset, and says so; of two frames with one name the first
    /// is found. A surround that reads a frame the page lacks reads the first frame instead, and
    /// is removed when the page has none (a surround must have a frame). A map series without
    /// its frame is left out.
    pub fn resolve(&self, layout: &mut Layout, text_frames: &[(u32, String)], report: &mut Report) {
        let mut ids: HashMap<&str, u32> = HashMap::new();
        for b in &self.bindings {
            if ids.contains_key(b.name.as_str()) {
                report.approximated(&b.name, "another map frame has this name; the first is the one links and text find");
            } else {
                ids.insert(b.name.as_str(), b.element);
            }
        }
        let find = |name: &str| ids.get(name.trim()).copied();
        for (id, name, mode, rotation) in &self.links {
            let what = layout.element(*id).map(|e| e.name.clone()).unwrap_or_default();
            let target = match find(name) {
                Some(t) if t == *id => {
                    report.skipped(&what, "follows itself; the link is left out");
                    None
                }
                Some(t) => Some(t),
                None => {
                    report.skipped(&what, format!("follows the frame {}, which isn't on the page; the link is left out", shown(name)));
                    None
                }
            };
            if let (Some(frame), Some(ElementKind::MapFrame { link, .. })) = (target, layout.element_mut(*id).map(|e| &mut e.kind)) {
                *link = Some(FrameLink { frame, mode: *mode, rotation: *rotation });
            }
        }
        for (id, name) in &self.indicators {
            let what = layout.element(*id).map(|e| e.name.clone()).unwrap_or_default();
            let target = match find(name) {
                Some(t) if t == *id => {
                    report.skipped(&what, "outlines its own extent; the indicator is left out");
                    None
                }
                Some(t) => Some(t),
                None => {
                    report
                        .skipped(&what, format!("outlines the frame {}, which isn't on the page; the indicator is left out", shown(name)));
                    None
                }
            };
            if let (Some(frame), Some(ElementKind::MapFrame { extent_of, .. })) = (target, layout.element_mut(*id).map(|e| &mut e.kind)) {
                *extent_of = Some(frame);
            }
        }
        for (id, name) in text_frames {
            let target = find(name);
            let Some(el) = layout.element_mut(*id) else { continue };
            if target.is_none() {
                report.approximated(&el.name, format!("reads frame {}, which isn't on the page; the first frame is used", shown(name)));
            }
            if let ElementKind::Text { frame, .. } = &mut el.kind {
                *frame = target;
            }
        }
        let first = self.bindings.iter().min_by_key(|b| b.element);
        for (id, name) in &self.surrounds {
            let what = layout.element(*id).map(|e| e.name.clone()).unwrap_or_default();
            let unnamed = name.trim().is_empty();
            let target = match (find(name), first) {
                (Some(t), _) => Some(t),
                (None, Some(f)) => {
                    let reads = if unnamed {
                        "names no map frame".to_string()
                    } else {
                        format!("reads frame {}, which isn't on the page", shown(name))
                    };
                    report.approximated(&what, format!("{reads}; {} is used", shown(&f.name)));
                    Some(f.element)
                }
                (None, None) => {
                    let reads = if unnamed { "names no map frame".to_string() } else { format!("reads frame {}", shown(name)) };
                    report.skipped(&what, format!("{reads}, and the page has no map frame; left out"));
                    None
                }
            };
            match target {
                // A surround has one frame, which still holds `UNBOUND`.
                Some(frame) => {
                    if let Some(el) = layout.element_mut(*id) {
                        el.kind.remap(|old| (old == UNBOUND).then_some(frame));
                    }
                }
                None => {
                    layout.remove(*id);
                }
            }
        }
        if let Some(s) = &self.series {
            match find(&s.frame) {
                Some(frame) => {
                    layout.series = Some(MapSeries::new(frame, s.layer.clone(), s.name_field.clone()));
                    // The series' frame follows the pages, never another frame.
                    if let Some(el) = layout.element_mut(frame)
                        && let ElementKind::MapFrame { link, .. } = &mut el.kind
                        && link.take().is_some()
                    {
                        report.skipped(&el.name, "drives the map series; its link is left out");
                    }
                }
                None if s.frame.is_empty() => report.skipped(&s.what, "the map series names no frame; left out"),
                None => {
                    report.skipped(&s.what, format!("the map series reads frame {}, which isn't on the page; left out", shown(&s.frame)))
                }
            }
        }
    }

    /// Each map frame's link target, to compare across `Layout::clamp` with [`Frames::note_cut_links`].
    pub fn links_of(layout: &Layout) -> Vec<(u32, u32)> {
        layout
            .elements
            .iter()
            .filter_map(|e| match &e.kind {
                ElementKind::MapFrame { link: Some(l), .. } => Some((e.id, l.frame)),
                _ => None,
            })
            .collect()
    }

    /// Notes the links `Layout::clamp` cut (`before` is [`Frames::links_of`] taken before it):
    /// after `resolve`, every link names a map frame on the page, so a cut one closed a loop.
    pub fn note_cut_links(before: &[(u32, u32)], layout: &Layout, report: &mut Report) {
        for (id, target) in before {
            let Some(el) = layout.element(*id) else { continue };
            if matches!(&el.kind, ElementKind::MapFrame { link: None, .. }) {
                let other = layout.element(*target).map(|t| t.name.clone()).unwrap_or_default();
                report.skipped(&el.name, format!("follows {}, which follows it back; the link is left out", shown(&other)));
            }
        }
    }

    /// Puts back the scale 0 of the frames that take the map's view, which `Layout::clamp` raised
    /// to the smallest scale a frame has. Run after it.
    pub fn restore_unset_scales(&self, layout: &mut Layout) {
        for id in &self.unset {
            if let Some(ElementKind::MapFrame { scale, .. }) = layout.element_mut(*id).map(|e| &mut e.kind) {
                *scale = 0.0;
            }
        }
    }
}

/// Reads the `CIMMapFrame` `e` called `name`: its box, the element, and the names left to resolve.
/// `None` (and a note) for a frame with no outline to place.
pub(crate) fn read(doc: &Document, e: &Value, name: &str, report: &mut Report) -> Option<(PageBox, ElementKind, Pending)> {
    let Some((bounds, shape)) = outline(e, name, report) else {
        report.skipped(name, "a map frame with no shape, left out");
        return None;
    };
    let map_uri = str_of(e, "uRI").map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    let def = map_uri.as_deref().and_then(|u| doc.resolve(u));
    if let (Some(uri), None) = (&map_uri, def) {
        report.approximated(name, format!("the map {} isn't in the document; the map's view is used", shown(uri)));
    }
    let cam = e.get("view").and_then(|v| v.get("camera")).unwrap_or(&NULL);
    let saved_camera = match (num(cam, "x"), num(cam, "y"), num(cam, "scale")) {
        (Some(x), Some(y), Some(scale)) => Some((x, y, scale)),
        _ => None,
    };
    let (center, scale) = def.map_or((Coord::new(0.0, 0.0), 0.0), |d| camera(d, e, cam, name, report));
    // The camera's heading is the frame's bearing (see `bearing_of` for the sign). An unbound
    // frame's camera belongs to some other project: its rotation is not believed either.
    let bearing = def.and_then(|_| bearing_of(cam, "heading", false, name, report)).unwrap_or(0.0);
    let (extent_color, extent_width, indicator) = indicators(e, name, report);
    // Graticules and measured grids aren't read yet: noted, so the frame's missing grid is told.
    if e.get("grids").and_then(Value::as_array).is_some_and(|g| !g.is_empty()) {
        report.skipped(name, "map grids, left out");
    }
    let kind = ElementKind::MapFrame {
        map: name.to_string(),
        center,
        scale,
        shape,
        grid: None,
        extent_of: None, // by name, in `Frames::resolve`
        extent_color,
        extent_width,
        bearing,
        pitch: 0.0,
        link: None,
    };
    let pending =
        Pending { name: name.to_string(), map_uri, saved_camera, link: link(e, name, report), indicator, scale_unset: scale == 0.0 };
    Some((bounds, kind, pending))
}

/// The box and shape of the frame's outline (`frame`): a rectangle for four corners, an ellipse
/// for a closed arc, else the polygon of its points in the box's unit square.
fn outline(e: &Value, name: &str, report: &mut Report) -> Option<(PageBox, FrameShape)> {
    let g = e.get("frame").filter(|g| g.is_object())?;
    let b = geometry_box(g)?;
    let shape = if is_ellipse(g) {
        FrameShape::Ellipse
    } else if is_four_corner_ring(g) {
        FrameShape::Rectangle
    } else {
        polygon(g, b, name, report)
    };
    Some((b, shape))
}

/// The first ring of `g` as a polygon in box `b`'s unit square (x from the left, y from the
/// top); a rectangle when it has no three points or the box has no area.
fn polygon(g: &Value, b: PageBox, name: &str, report: &mut Report) -> FrameShape {
    let rings = ["rings", "curveRings"].iter().filter_map(|k| g.get(*k).and_then(Value::as_array)).find(|r| !r.is_empty());
    let Some(rings) = rings else { return FrameShape::Rectangle };
    if rings.len() > 1 {
        report.approximated(name, "an outline of more than one ring, drawn as the first");
    }
    let (w, h) = (b.xmax - b.xmin, b.ymax - b.ymin);
    let Some(ring) = rings.first().filter(|_| w > 0.0 && h > 0.0) else { return FrameShape::Rectangle };
    let mut budget = MAX_OUTLINE_POINTS + 1;
    let mut pts = part_points(ring, &mut budget);
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    pts.truncate(MAX_OUTLINE_POINTS);
    if pts.len() < 3 {
        return FrameShape::Rectangle;
    }
    FrameShape::Polygon(pts.into_iter().map(|(x, y)| (((x - b.xmin) / w).clamp(0.0, 1.0), ((b.ymax - y) / h).clamp(0.0, 1.0))).collect())
}

/// A spatial reference's well-known id: `latestWkid`, else `wkid`, of `holder.spatialReference`.
fn wkid_of(holder: &Value) -> Option<i64> {
    let sr = holder.get("spatialReference")?;
    sr.get("latestWkid").and_then(Value::as_i64).or_else(|| sr.get("wkid").and_then(Value::as_i64))
}

fn wkid_text(w: Option<i64>) -> String {
    w.map_or_else(|| "unknown".to_string(), |w| w.to_string())
}

/// Point (`x`, `y`) of system `wkid` as longitude/latitude, when RustGIS converts that system and
/// the result is a place on the earth.
fn lonlat(wkid: Option<i64>, x: f64, y: f64) -> Option<Coord> {
    let c = match wkid? {
        4326 => Coord::new(x, y),
        3857 | 102100 => merc_to_lonlat(Coord::new(x, y)),
        w => match projector(w)? {
            None => Coord::new(x, y),
            Some(p) => p.inverse(Coord::new(x, y))?,
        },
    };
    (c.x.is_finite() && c.y.is_finite() && c.x.abs() <= 180.0 && c.y.abs() <= 90.0).then_some(c)
}

/// The projection of system `wkid`, by its EPSG code or else Esri's (Pro writes Esri's own codes,
/// a state plane in feet say, as the EPSG ones), when RustGIS knows it: `Some(None)` for one in
/// degrees (NAD83's, say), whose points are read as longitude/latitude.
fn projector(wkid: i64) -> Option<Option<crate::projection::Projector>> {
    // (The projection lookup refuses a system in degrees, so they are found here first.)
    let epsg = u16::try_from(wkid).ok().and_then(rustgis_data::raster::crs::epsg_proj4);
    if epsg.is_some_and(|def| def.split_whitespace().any(|t| t == "+proj=longlat" || t == "+proj=latlong")) {
        return Some(None);
    }
    ["EPSG", "ESRI"].iter().find_map(|authority| crate::projection::Projector::new(&format!("{authority}:{wkid}")).ok().map(Some))
}

/// Whether RustGIS converts system `wkid`.
fn supported(wkid: Option<i64>) -> bool {
    matches!(wkid, Some(4326 | 3857 | 102100)) || wkid.and_then(projector).is_some()
}

/// A camera scale worth believing.
fn believable(scale: f64) -> Option<f64> {
    (scale > 0.0 && scale <= MAX_CAMERA_SCALE).then_some(scale)
}

/// The centre of an envelope and its system.
fn extent_centre(ext: &Value) -> Option<(f64, f64, Option<i64>)> {
    let (x0, y0, x1, y1) = (num(ext, "xmin")?, num(ext, "ymin")?, num(ext, "xmax")?, num(ext, "ymax")?);
    Some(((x0 + x1) / 2.0, (y0 + y1) / 2.0, wkid_of(ext)))
}

/// Where a frame showing map `def` looks: the saved camera when the map's system is one RustGIS
/// converts, else the centre of the frame's auto-camera extent, else of the map's default
/// extent (the scale then the camera's, if it has one). Nothing usable is the map's view
/// (centre (0, 0), scale 0).
fn camera(def: &Value, e: &Value, cam: &Value, name: &str, report: &mut Report) -> (Coord, f64) {
    let scale = num(cam, "scale").and_then(believable);
    let map_wkid = wkid_of(def);
    let mut unsupported = None;
    if let (Some(x), Some(y), Some(scale)) = (num(cam, "x"), num(cam, "y"), scale) {
        if let Some(c) = lonlat(map_wkid, x, y) {
            return (c, scale);
        }
        if !supported(map_wkid) {
            unsupported = Some(map_wkid);
        }
    }
    let auto = e.get("autoCamera").and_then(|a| a.get("extent"));
    for ext in [auto, def.get("defaultExtent")].into_iter().flatten() {
        let Some((x, y, wkid)) = extent_centre(ext) else { continue };
        if let Some(c) = lonlat(wkid, x, y) {
            return (c, scale.unwrap_or(0.0));
        }
        if !supported(wkid) {
            unsupported.get_or_insert(wkid);
        }
    }
    if let Some(w) = unsupported {
        report.approximated(name, format!("frame camera in an unsupported coordinate system ({}); the map's view is used", wkid_text(w)));
    }
    (Coord::new(0.0, 0.0), 0.0)
}

/// The frame this one follows (`autoCamera`, `source` `MapFrameLink`): its name, how it follows,
/// and whether the rotation follows too.
fn link(e: &Value, name: &str, report: &mut Report) -> Option<(String, LinkMode, bool)> {
    let auto = e.get("autoCamera")?;
    if str_of(auto, "source") != Some("MapFrameLink") {
        return None;
    }
    let target = str_of(auto, "mapFrameLinkName").or_else(|| str_of(auto, "mapFrame")).map(str::trim).filter(|n| !n.is_empty());
    let Some(target) = target else {
        report.skipped(name, "follows a frame it doesn't name; the link is left out");
        return None;
    };
    let mode = match str_of(auto, "autoCameraType").unwrap_or("Center") {
        "Extent" => LinkMode::Extent,
        "Scale" => LinkMode::Scale,
        "Center" => LinkMode::Center,
        other => {
            report.approximated(name, format!("link type {}, taken as centre", shown(other)));
            LinkMode::Center
        }
    };
    Some((target.to_string(), mode, super::flag(auto, "syncRotation", false)))
}

/// The first extent indicator: its outline's colour and width (the defaults when its symbol draws no
/// stroke; its fill is not drawn, and not noted) and the name of the frame it outlines. Further
/// ones are noted.
fn indicators(e: &Value, name: &str, report: &mut Report) -> (Rgba, f64, Option<String>) {
    let list = e.get("extentIndicators").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let Some(first) = list.first() else { return (INDICATOR_COLOR, INDICATOR_WIDTH, None) };
    if list.len() > 1 {
        report.skipped(name, format!("{} more extent indicators, left out", list.len() - 1));
    }
    let (color, width) = match first.get("symbol").and_then(|s| symbol::polygon(s, name, report).stroke) {
        Some((c, w)) => (c, f64::from(w)),
        None => (INDICATOR_COLOR, INDICATOR_WIDTH),
    };
    let target = str_of(first, "sourceMapFrame").or_else(|| str_of(first, "mapFrame")).map(str::trim).filter(|n| !n.is_empty());
    if target.is_none() {
        report.skipped(name, "an extent indicator naming no frame, left out");
    }
    (color, width, target.map(str::to_string))
}
