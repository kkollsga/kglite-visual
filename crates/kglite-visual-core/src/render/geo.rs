//! The geographic layout kernel: put a node where it actually is (plan E12).
//!
//! **Equirectangular, and that is a measurement rather than a default.** The
//! data this exists for is the Norwegian shelf: sodir's wellbores span
//! 56.1°N–80.7°N (measured 2026-08-30 against
//! `sodir_graph.kgl`, `min(w.latitude)` / `max(w.latitude)`). Mercator's
//! vertical scale is `1/cos φ`, so it draws a degree of latitude 1.8× its
//! equatorial size at the south end of that range and 6.2× at the north end —
//! the same shelf stretched more than three times as much at one end as at the
//! other, and every north–south distance in the picture a lie about a different
//! amount. Equirectangular has one vertical scale everywhere, so two wellbores
//! 100 km apart north–south are drawn the same distance apart wherever they
//! sit. That is the property a reader of *this* map needs; the property
//! Mercator buys instead — locally correct angles, for navigation — is not one
//! anybody asks of a graph view.
//!
//! **Longitude is corrected by `cos φ₁`, the bbox's mid-latitude.** A degree of
//! longitude is 111 km at the equator and 41 km at 68°N; plotting raw `lon`
//! against raw `lat` on equal axes draws Norway 2.7× too wide. Multiplying the
//! longitude offset by the cosine of the *standard parallel* (plate carrée's
//! own parameter, chosen here as the middle of the data rather than a fixed
//! 0°) makes a square kilometre near the middle of the picture come out square.
//! The distortion that remains is inherent to the projection and grows away
//! from φ₁ — at sodir's span it is under 30% at the extremes, against the 620%
//! Mercator would have introduced.
//!
//! **`cos` is the platform's libm, so its result is quantised** to a 1/4096
//! grid before it multiplies anything, exactly as `super::layout`'s `unit`
//! quantises its sine and cosine and for the same reason: IEEE-754 does not
//! require `cos` to be correctly rounded, two machines may disagree in the last
//! bit, and a golden SVG is an exact baseline. Everything downstream of the
//! quantisation is `+ - * /`.
//!
//! **Coincident nodes are jittered, because they are real.** 13 sodir wellbores
//! share the coordinate (60.83210631878465, 3.6022054034249433) to the last
//! bit, and three more groups of 11 do the same — a drilling pad reported once
//! per bore. Drawn honestly they are one dot standing for thirteen. Only
//! *exactly* equal coordinates are spread ([`jitter_offsets`]): two nearby-but-different
//! positions are two different places and moving them apart would be the map
//! inventing a distance.
//!
//! **A node with no coordinate is not hidden.** It goes into a labelled tray at
//! the foot of the picture, the same honesty the orphan island already carries
//! — sodir has 68 wellbores with a null latitude and 4 001 prospects with
//! neither a location nor a geometry, and a map that silently dropped them
//! would read as complete.

use std::collections::HashMap;

use kglite::api::DirGraph;

use crate::error::CoreError;

use super::layout::{self, Canvas, Island, LabelSide, LayoutNode, Positions};

/// Share of a scene's nodes that must be placeable before `auto` will draw a
/// map without being asked.
///
/// **A map with a big tray is a list with a map on top of it.** At this share
/// one node in five is in the tray, which is already a visible strip; below it
/// the picture stops being mostly a map and `auto` — which exists to make the
/// choice a caller had no opinion about — would be making the wrong one
/// silently. An explicit `kernel = geo` has no threshold at all: a caller that
/// asked for the map gets the map and the tray that comes with it, which is why
/// this number can be strict without taking anything away.
const AUTO_PLACEABLE_SHARE: f64 = 0.8;

/// Distinct positions `auto` needs before a map says anything.
///
/// Three points is a triangle; two is a line segment that carries no more
/// information than the two labels beside it, and one is a dot. Counted on
/// *distinct* coordinates rather than nodes, because thirteen wellbores on one
/// pad are one place.
const AUTO_MIN_DISTINCT: usize = 3;

/// How far apart [`jitter_offsets`] pushes two nodes that share a coordinate exactly,
/// in final canvas pixels.
///
/// One instance circle's diameter plus a hair
/// ([`super::encoding::INSTANCE_RADIUS_PX`] is 6), so two nodes on one pad read
/// as two nodes rather than as a figure of eight. Any larger and a pad starts
/// competing with the real distances around it.
const JITTER_STEP_PX: f64 = 14.0;

/// Extra degrees of lon/lat around the data's own bounding box, as a share of
/// its longer side.
///
/// The coastline is clipped to this box, so it exists to stop the shoreline
/// ending exactly at the outermost wellbore — a map whose land stops where the
/// data stops looks like a rendering bug.
const BBOX_MARGIN_SHARE: f64 = 0.12;

/// …and a floor for it, in degrees, so a scene whose nodes are all on one pad
/// still gets a box with a coast in it rather than a point.
const BBOX_MARGIN_MIN_DEG: f64 = 0.5;

/// Where a node is, in degrees: `(longitude, latitude)`.
pub(crate) type LonLat = (f64, f64);

/// How a longitude/latitude pair becomes a canvas pixel.
///
/// Carried out of the kernel because the coastline and the graticule are drawn
/// by the emitter and have to land in the **same** frame the nodes did — a
/// second projection there would be a second answer to a question with one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoFrame {
    /// `cos φ₁`, quantised. Multiplies a longitude offset before scaling.
    pub lon_scale: f64,
    /// The projection's origin, in degrees.
    pub lon0: f64,
    pub lat0: f64,
    /// Map-space bounding box the fit was solved against.
    pub min_x: f64,
    pub min_y: f64,
    /// Pixels per map-space unit, and where the box's corner landed.
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    /// The data's own lon/lat box, widened by [`BBOX_MARGIN_SHARE`]:
    /// `(west, south, east, north)`. What the coastline is clipped to.
    pub bbox: (f64, f64, f64, f64),
    /// The canvas rectangle the map occupies, `(x, y, width, height)`. The
    /// coastline is clipped to this too — the tray at the foot is not part of
    /// the map and a coast drawn through it would say it was.
    pub clip: (f64, f64, f64, f64),
}

impl GeoFrame {
    /// One coordinate, in canvas pixels.
    pub fn project(&self, lon: f64, lat: f64) -> (f64, f64) {
        let (x, y) = map_space(lon, lat, self.lon0, self.lat0, self.lon_scale);
        (
            (x - self.min_x) * self.scale + self.offset_x,
            (y - self.min_y) * self.scale + self.offset_y,
        )
    }
}

/// Longitude/latitude into the projection's own flat space.
///
/// `y` is negated because north is up and canvas `y` grows downward; doing it
/// here rather than at the emitter keeps every consumer of the frame agreeing
/// about which way up the world is.
fn map_space(lon: f64, lat: f64, lon0: f64, lat0: f64, lon_scale: f64) -> (f64, f64) {
    ((lon - lon0) * lon_scale, -(lat - lat0))
}

/// `cos` of a latitude in degrees, on a 1/4096 grid.
///
/// The determinism guard — see the module doc. Floored at a small positive
/// value so a scene centred on a pole cannot collapse the longitude axis to
/// zero and divide the fit by it; at 89.99° the correction is already telling
/// the reader that longitudes are meaningless there.
fn quantised_cos_deg(degrees: f64) -> f64 {
    let radians = degrees * std::f64::consts::PI / 180.0;
    let cosine = (radians.cos() * 4_096.0).round() / 4_096.0;
    cosine.max(1.0 / 4_096.0)
}

/// Read one node's position out of the graph, honouring its type's spatial
/// declaration.
///
/// **Both shapes, location first** — which is what sodir actually needs. 37 of
/// its 38 spatially-configured types declare a `location` lat/lon pair *and* a
/// WKT `geometry` field; one (`Prospect`) declares geometry alone. The
/// meta-graph badge cannot tell them apart, because kglite's own
/// `flags_csv` rule suppresses `loc` wherever `geo` is present — so a reader
/// written from the badge would have parsed a polygon per node to recover a
/// number already stored beside it.
///
/// A type with neither, or a node whose declared fields are null, has no
/// position. That is returned as `None` and drawn in the tray; it is never
/// guessed.
pub(crate) fn position_of(graph: &DirGraph, node_type: &str, node_id: u32) -> Option<LonLat> {
    let config = graph.spatial_configs.get(node_type)?;
    let view = graph.node_view(kglite::api::NodeIndex::new(node_id as usize))?;

    if let Some((lat_field, lon_field)) = &config.location {
        let lat = view.get_property(lat_field).and_then(|v| as_f64(&v));
        let lon = view.get_property(lon_field).and_then(|v| as_f64(&v));
        if let (Some(lat), Some(lon)) = (lat, lon) {
            if in_range(lon, lat) {
                return Some((lon, lat));
            }
        }
    }
    // The fallback the WKT-only types need. `wkt_centroid` returns `(lat, lon)`
    // — geographic order, the opposite of this module's — and swapping it here
    // once is cheaper than every caller remembering.
    let geometry = config.geometry.as_ref()?;
    let wkt = view.get_property(geometry)?;
    let kglite::api::Value::String(text) = wkt.as_ref() else {
        return None;
    };
    let (lat, lon) = kglite::api::fluent::wkt_centroid(text).ok()?;
    in_range(lon, lat).then_some((lon, lat))
}

/// A coordinate outside this is a unit error or a sentinel, not a place.
///
/// Refusing it is what stops one `(0, 0)` "null island" row from setting a
/// bounding box that squeezes the real data into a corner — the classic way a
/// geographic plot goes wrong, and a failure mode a tray entry states honestly.
fn in_range(lon: f64, lat: f64) -> bool {
    lon.is_finite()
        && lat.is_finite()
        && (-180.0..=180.0).contains(&lon)
        && (-90.0..=90.0).contains(&lat)
}

/// kglite `Value` → `f64`, for the two numeric shapes a lat/lon column takes.
fn as_f64(value: &kglite::api::Value) -> Option<f64> {
    match value {
        kglite::api::Value::Float64(v) => Some(*v),
        kglite::api::Value::Int64(v) => Some(*v as f64),
        _ => None,
    }
}

/// True when `auto` should draw this scene as a map without being asked.
///
/// Two conditions, and both are about the *picture* rather than about the data
/// being geographic: enough of the scene has to land on the map for the map to
/// be the answer ([`AUTO_PLACEABLE_SHARE`]), and the points have to be in
/// enough different places for there to be a shape to see
/// ([`AUTO_MIN_DISTINCT`]).
pub(crate) fn auto_eligible(points: &[Option<LonLat>]) -> bool {
    if points.is_empty() {
        return false;
    }
    let placed: Vec<LonLat> = points.iter().flatten().copied().collect();
    if (placed.len() as f64) < AUTO_PLACEABLE_SHARE * points.len() as f64 {
        return false;
    }
    let mut keys: Vec<(u64, u64)> = placed
        .iter()
        .map(|(lon, lat)| (lon.to_bits(), lat.to_bits()))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys.len() >= AUTO_MIN_DISTINCT
}

/// Lay the scene out geographically.
///
/// `points` is one entry per node, parallel to `nodes`: `Some` for a node with
/// a coordinate, `None` for one without. Refuses a scene with no placeable node
/// at all rather than drawing an empty map with everything in the tray — at
/// that point the caller asked for a picture this data cannot be.
pub(crate) fn layout(
    points: &[Option<LonLat>],
    nodes: &[LayoutNode],
    canvas: Canvas,
) -> Result<(Positions, GeoFrame), CoreError> {
    let count = nodes.len().min(points.len());
    if count > layout::MAX_LAYOUT_NODES {
        return Err(CoreError::Request(format!(
            "a geographic layout of {count} nodes is past the {} the renderer will place",
            layout::MAX_LAYOUT_NODES
        )));
    }
    let placed: Vec<usize> = (0..count).filter(|i| points[*i].is_some()).collect();
    let tray: Vec<usize> = (0..count).filter(|i| points[*i].is_none()).collect();
    if placed.is_empty() {
        return Err(CoreError::Request(
            "nothing in this view has a coordinate, so there is no map to draw. \
             A node is placeable when its type declares a lat/lon location or a \
             WKT geometry (`GET /api/describe` lists the types) and the node's \
             own fields are not null."
                .to_string(),
        ));
    }

    // The tray strip is solved before the map is fitted, because the map is
    // fitted into what the tray leaves — a tray placed afterwards would be
    // drawn over the southern half of the picture.
    let tray_plan = TrayPlan::solve(&tray, nodes, canvas);
    let map_height = (canvas.height - tray_plan.height).max(canvas.reserved_top + 1.0);

    let (west, south, east, north) = degree_bbox(points);
    let lon_scale = quantised_cos_deg((south + north) / 2.0);
    let (lon0, lat0) = (west, north);

    // Map space, and the jitter budget the fit has to leave room for: the
    // spread happens in FINAL pixels (a pad is the same size at every zoom), so
    // an allowance folded into the pre-fit extent would arrive scaled by
    // exactly the factor it was there to survive — the same argument
    // `layout::fit`'s `side_room` makes.
    let mut map_xy = vec![(0.0f64, 0.0f64); count];
    for index in &placed {
        let (lon, lat) = points[*index].expect("filtered to Some above");
        map_xy[*index] = map_space(lon, lat, lon0, lat0, lon_scale);
    }
    let spread = jitter_offsets(points, &placed);
    let widest_jitter = spread
        .iter()
        .map(|(dx, dy): &(f64, f64)| dx.abs().max(dy.abs()))
        .fold(0.0f64, f64::max);

    // **Map space is pre-scaled to roughly pixels before the fit, and that is a
    // bug fix rather than a tidy-up.** [`layout::fit_detail`] adds each node's
    // radius and a 26 px label allowance to the extent it solves against —
    // constants in *pixels*, which is right for every other kernel because
    // every other kernel already works in pixels. Geographic map space is
    // degrees. Measured on the sodir wellbore render (2026-08-30): a slice
    // spanning 15.9° of latitude was fitted as though it spanned 15.9 + 38, so
    // the scale came out 3.4× too small and the Norwegian shelf was drawn as a
    // 130 px smudge on a map of Europe and North Africa. Pre-scaling so the
    // extent is already near the canvas makes those allowances the small
    // corrections they were written to be. It cannot change the *shape*: it is
    // one factor applied to both axes, and `fit` is affine.
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for index in &placed {
        let (x, y) = map_xy[*index];
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    // A degenerate extent — every node on one pad — has no scale to derive, and
    // 1.0 is as good as any: `fit` centres a zero-extent cloud whatever it is
    // multiplied by, and the jitter that spreads it is in final pixels already.
    let pre_scale = {
        // A tenth of a millimetre on the ground, as an extent floor. Below it
        // the points are one place and the picture is the jitter, so there is
        // no scale to solve — and dividing by the real extent would produce a
        // factor large enough to overflow the fit.
        const DEGENERATE_DEG: f64 = 1e-9;
        let by_width = canvas.width / (max_x - min_x).max(DEGENERATE_DEG);
        let by_height = map_height / (max_y - min_y).max(DEGENERATE_DEG);
        let solved = by_width.min(by_height);
        if solved.is_finite() && solved > 0.0 && (max_x - min_x).max(max_y - min_y) > DEGENERATE_DEG
        {
            solved
        } else {
            1.0
        }
    };
    for point in map_xy.iter_mut() {
        *point = (point.0 * pre_scale, point.1 * pre_scale);
    }

    let map_nodes: Vec<LayoutNode> = placed.iter().map(|i| nodes[*i]).collect();
    let map_points: Vec<(f64, f64)> = placed.iter().map(|i| map_xy[*i]).collect();
    let fitted = layout::fit_detail(
        &map_points,
        &map_nodes,
        canvas.width,
        map_height,
        canvas.reserved_top,
        widest_jitter,
    );

    let mut xy = vec![(0.0f64, 0.0f64); count];
    for (slot, index) in placed.iter().enumerate() {
        let (x, y) = fitted.xy[slot];
        let (dx, dy) = spread[*index];
        xy[*index] = (x + dx, y + dy);
    }
    tray_plan.place(&tray, nodes, canvas, &mut xy);

    let frame = GeoFrame {
        lon_scale,
        lon0,
        lat0,
        // Back out of the pre-scale, so the frame is expressed in the map space
        // `map_space` produces and `GeoFrame::project` can be the one function
        // that turns a coordinate into a pixel. The two forms are algebraically
        // the same map; `the_frame_reprojects_a_node_to_where_the_node_was_drawn`
        // is the test that says so rather than the comment.
        min_x: fitted.min_x / pre_scale,
        min_y: fitted.min_y / pre_scale,
        scale: fitted.scale * pre_scale,
        offset_x: fitted.offset_x,
        offset_y: fitted.offset_y,
        bbox: (west, south, east, north),
        clip: (
            0.0,
            canvas.reserved_top,
            canvas.width,
            map_height - canvas.reserved_top,
        ),
    };

    let islands = if tray.is_empty() {
        Vec::new()
    } else {
        vec![Island {
            members: tray,
            orphans: true,
            caption: Some(TRAY_CAPTION.to_string()),
        }]
    };
    Ok((
        Positions {
            xy,
            label_side: vec![LabelSide::Below; count],
            islands,
            geo: Some(frame),
        },
        frame,
    ))
}

/// What the tray's own boundary says it is.
///
/// Written on the picture rather than only in the status block: the status
/// block is top-left and the tray is at the foot, and a reader who has to
/// connect the two has been asked to do the honesty work themselves.
pub const TRAY_CAPTION: &str = "no coordinate";

/// The status line the tray earns in the picture's own block.
pub(crate) fn tray_line(tray: usize, total: usize) -> Option<String> {
    (tray > 0).then(|| {
        format!(
            "{} of {} have no coordinate — in the tray at the foot",
            super::encoding::group_thousands(tray as u64),
            super::encoding::group_thousands(total as u64),
        )
    })
}

/// The lon/lat box the data occupies, widened by [`BBOX_MARGIN_SHARE`].
fn degree_bbox(points: &[Option<LonLat>]) -> (f64, f64, f64, f64) {
    let (mut west, mut south) = (f64::INFINITY, f64::INFINITY);
    let (mut east, mut north) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (lon, lat) in points.iter().flatten() {
        west = west.min(*lon);
        east = east.max(*lon);
        south = south.min(*lat);
        north = north.max(*lat);
    }
    let margin = ((east - west).max(north - south) * BBOX_MARGIN_SHARE).max(BBOX_MARGIN_MIN_DEG);
    (
        (west - margin).max(-180.0),
        (south - margin).max(-90.0),
        (east + margin).min(180.0),
        (north + margin).min(90.0),
    )
}

/// Per-node pixel offsets that spread exactly-coincident coordinates.
///
/// **Exact equality only** — see the module doc. The key is the pair's bit
/// pattern, so the grouping is a fact about the stored numbers and does not
/// move when the canvas does. Members of a group are visited in scene-index
/// order and take successive cells of a square spiral, which is integer
/// arithmetic end to end: no trigonometry, no random source, no hash iteration
/// order, and therefore the same offsets on every machine forever.
fn jitter_offsets(points: &[Option<LonLat>], placed: &[usize]) -> Vec<(f64, f64)> {
    let mut spread = vec![(0.0f64, 0.0f64); points.len()];
    let mut seen: HashMap<(u64, u64), usize> = HashMap::new();
    for index in placed {
        let (lon, lat) = points[*index].expect("placed indices carry a coordinate");
        let rank = seen.entry((lon.to_bits(), lat.to_bits())).or_insert(0);
        if *rank > 0 {
            let (dx, dy) = spiral_cell(*rank);
            spread[*index] = (dx as f64 * JITTER_STEP_PX, dy as f64 * JITTER_STEP_PX);
        }
        *rank += 1;
    }
    spread
}

/// The `n`-th cell of a square spiral around the origin, `n = 0` at the centre.
///
/// Integer arithmetic so the jitter is bit-identical everywhere; a spiral
/// rather than a row so a pad of thirteen stays compact instead of becoming a
/// line pointing at nothing.
fn spiral_cell(n: usize) -> (i64, i64) {
    if n == 0 {
        return (0, 0);
    }
    // Ring `r` is the square shell `max(|x|, |y|) == r`. It holds the cells
    // numbered `(2r-1)^2 ..= (2r+1)^2 - 1`, which is `8r` of them.
    let mut ring = 1i64;
    while ((2 * ring + 1) * (2 * ring + 1)) as usize <= n {
        ring += 1;
    }
    let side = 2 * ring;
    let offset = (n - ((2 * ring - 1) * (2 * ring - 1)) as usize) as i64;
    // Four sides, each `2r` cells, starting at the ring's top-right corner and
    // going clockwise on screen (y grows downward). Written as four closed
    // forms rather than as a walk: a walk accumulates, and an accumulator with
    // an off-by-one lands two nodes of a pad on the same pixel.
    if offset < side {
        (ring, -ring + offset)
    } else if offset < 2 * side {
        (ring - (offset - side), ring)
    } else if offset < 3 * side {
        (-ring, ring - (offset - 2 * side))
    } else {
        (-ring + (offset - 3 * side), -ring)
    }
}

/// The strip at the foot of the canvas that holds what has no coordinate.
struct TrayPlan {
    /// Height of the strip, in pixels. Zero when the tray is empty.
    height: f64,
    columns: usize,
    cell: f64,
}

/// Keep-out between the tray strip and the canvas edges.
const TRAY_MARGIN_PX: f64 = 26.0;
/// Room above the tray's first row for its caption.
const TRAY_CAPTION_ROOM_PX: f64 = 20.0;
/// Most of the canvas the tray may take before it stops being a tray.
///
/// Past this the strip is competing with the map for the frame. It does not
/// grow further; the cells get tighter and the picture is honest about being
/// full, which is the same rule the label grid follows.
const TRAY_MAX_SHARE: f64 = 0.3;

impl TrayPlan {
    fn solve(tray: &[usize], nodes: &[LayoutNode], canvas: Canvas) -> Self {
        if tray.is_empty() {
            return Self {
                height: 0.0,
                columns: 1,
                cell: 1.0,
            };
        }
        let widest = tray.iter().map(|i| nodes[*i].radius).fold(0.0f64, f64::max);
        // A grid, never a force pass, for the reason the orphan island already
        // gives: nodes with nothing in common have no shape, and a blob shaped
        // by a seed lattice invites a reader to find meaning in an artefact.
        let mut cell = 2.0 * widest + 8.0;
        let usable = (canvas.width - 2.0 * TRAY_MARGIN_PX).max(1.0);
        let mut columns = ((usable / cell).floor() as usize).max(1);
        let mut rows = tray.len().div_ceil(columns);
        let ceiling = canvas.height * TRAY_MAX_SHARE;
        let mut height = rows as f64 * cell + TRAY_CAPTION_ROOM_PX + 2.0 * TRAY_MARGIN_PX;
        if height > ceiling {
            // Tighten rather than grow. Solve the cell size the ceiling can
            // wear: `rows * cell` is what has to fit, and `rows` and `columns`
            // are both functions of `cell`, so one pass of the same arithmetic
            // at the shrunken size is the fixed point in practice.
            let room = (ceiling - TRAY_CAPTION_ROOM_PX - 2.0 * TRAY_MARGIN_PX).max(cell);
            // `columns * rows >= n` and `rows * cell <= room` and
            // `columns * cell <= usable` give `cell <= sqrt(room * usable / n)`.
            cell = ((room * usable / tray.len() as f64).sqrt()).max(2.0);
            columns = ((usable / cell).floor() as usize).max(1);
            rows = tray.len().div_ceil(columns);
            height =
                (rows as f64 * cell + TRAY_CAPTION_ROOM_PX + 2.0 * TRAY_MARGIN_PX).min(ceiling);
        }
        Self {
            height,
            columns,
            cell,
        }
    }

    /// Write the tray members' canvas positions into `xy`.
    fn place(&self, tray: &[usize], nodes: &[LayoutNode], canvas: Canvas, xy: &mut [(f64, f64)]) {
        if tray.is_empty() {
            return;
        }
        let top = canvas.height - self.height + TRAY_MARGIN_PX + TRAY_CAPTION_ROOM_PX;
        let grid_width = (self.columns.min(tray.len()) as f64) * self.cell;
        let left = (canvas.width - grid_width) / 2.0 + self.cell / 2.0;
        for (position, index) in tray.iter().enumerate() {
            let column = (position % self.columns) as f64;
            let row = (position / self.columns) as f64;
            let radius = nodes[*index].radius;
            xy[*index] = (
                (left + column * self.cell).clamp(radius, (canvas.width - radius).max(radius)),
                (top + row * self.cell + self.cell / 2.0)
                    .min(canvas.height - radius)
                    .max(radius),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(count: usize) -> Vec<LayoutNode> {
        vec![LayoutNode { radius: 6.0 }; count]
    }

    fn canvas() -> Canvas {
        Canvas {
            width: 1000.0,
            height: 800.0,
            reserved_top: 0.0,
        }
    }

    /// The projection's own claim, on coordinates whose answer is arithmetic.
    ///
    /// Four corners of a lon/lat box: north must land above south, east right
    /// of west, and — the part that is the whole reason this kernel is not a
    /// scatter plot — the *shape* must be corrected. At 60°N a 4°-wide,
    /// 2°-tall box is 222 km x 222 km on the ground, so it must come out
    /// roughly square rather than twice as wide as it is tall.
    #[test]
    fn a_box_at_sixty_north_comes_out_square() {
        let points = vec![
            Some((0.0, 59.0)),
            Some((4.0, 59.0)),
            Some((0.0, 61.0)),
            Some((4.0, 61.0)),
        ];
        let (positions, frame) = layout(&points, &nodes(4), canvas()).expect("four corners");
        let (sw, se, nw, _ne) = (
            positions.xy[0],
            positions.xy[1],
            positions.xy[2],
            positions.xy[3],
        );
        assert!(nw.1 < sw.1, "north is up: {nw:?} vs {sw:?}");
        assert!(se.0 > sw.0, "east is right: {se:?} vs {sw:?}");
        let width = se.0 - sw.0;
        let height = sw.1 - nw.1;
        let ratio = width / height;
        assert!(
            (0.9..1.1).contains(&ratio),
            "cos(60 deg) correction should make this square; got w/h = {ratio}"
        );
        // Uncorrected, the same box would be twice as wide as tall.
        assert!(
            frame.lon_scale > 0.49 && frame.lon_scale < 0.51,
            "{frame:?}"
        );
    }

    /// The falsification of the correction: remove it and the assertion above
    /// must stop holding. Written as a test so the aspect claim is not resting
    /// on a constant nobody exercises.
    #[test]
    fn without_the_correction_the_same_box_is_twice_as_wide() {
        let (x_west, _) = map_space(0.0, 59.0, 0.0, 61.0, 1.0);
        let (x_east, _) = map_space(4.0, 59.0, 0.0, 61.0, 1.0);
        let (_, y_south) = map_space(0.0, 59.0, 0.0, 61.0, 1.0);
        let (_, y_north) = map_space(0.0, 61.0, 0.0, 61.0, 1.0);
        let ratio = (x_east - x_west) / (y_south - y_north);
        assert!(
            (1.9..2.1).contains(&ratio),
            "an uncorrected plate carree draws this box 2:1; got {ratio}"
        );
    }

    /// Sodir's real coincidence, in the shape the graph actually holds it.
    #[test]
    fn thirteen_nodes_on_one_pad_become_thirteen_dots() {
        let pad = Some((3.6022054034249433, 60.83210631878465));
        let mut points = vec![pad; 13];
        // Two other places, so the fit has an extent to work with.
        points.push(Some((2.0, 58.0)));
        points.push(Some((5.0, 62.0)));
        let (positions, _) = layout(&points, &nodes(points.len()), canvas()).expect("a pad");
        let mut seen: Vec<(u64, u64)> = positions.xy[..13]
            .iter()
            .map(|(x, y)| (x.to_bits(), y.to_bits()))
            .collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 13, "every node on the pad got its own position");
        for (index, (x, y)) in positions.xy[..13].iter().enumerate() {
            let (cx, cy) = positions.xy[0];
            let distance = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            assert!(
                distance < 4.0 * JITTER_STEP_PX,
                "node {index} drifted {distance} px off the pad"
            );
        }
    }

    /// The regression for the bug the first sodir render found (2026-08-30).
    ///
    /// `layout::fit_detail` adds a node's radius and a 26 px label allowance to
    /// the extent it scales — pixel constants, right for every kernel that
    /// already works in pixels and three orders of magnitude wrong for one
    /// working in degrees. A 15° x 20° slice was fitted as though it spanned
    /// 15 + 38, so the shelf came out at a third of the size the canvas could
    /// hold. Asserted as "the map fills the frame it was given", which is the
    /// property a reader notices, and which fails by a factor of three with the
    /// pre-scale removed.
    #[test]
    fn a_slice_measured_in_degrees_still_fills_the_canvas() {
        let canvas = Canvas {
            width: 1600.0,
            height: 1100.0,
            reserved_top: 0.0,
        };
        // A grid over the Norwegian shelf's own span: 20 deg of longitude,
        // 15 deg of latitude, at the latitudes the data is actually at.
        let points: Vec<Option<LonLat>> = (0..5)
            .flat_map(|row| {
                (0..5).map(move |column| {
                    Some((-1.0 + f64::from(column) * 5.0, 58.0 + f64::from(row) * 3.75))
                })
            })
            .collect();
        let (positions, _) = layout(&points, &nodes(points.len()), canvas).expect("a shelf");
        let height = positions
            .xy
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max)
            - positions
                .xy
                .iter()
                .map(|(_, y)| *y)
                .fold(f64::INFINITY, f64::min);
        assert!(
            height > 0.85 * canvas.height,
            "the map should fill the frame; it spans {height} px of {}",
            canvas.height
        );
    }

    #[test]
    fn the_jitter_is_the_same_on_every_run() {
        let pad = Some((3.6, 60.8));
        let points = vec![pad, pad, pad, Some((2.0, 58.0)), Some((5.0, 62.0))];
        let first = layout(&points, &nodes(5), canvas()).expect("a pad").0;
        let second = layout(&points, &nodes(5), canvas()).expect("a pad").0;
        assert_eq!(first.xy, second.xy);
    }

    /// The spiral is what makes the jitter compact; assert the cells rather
    /// than the picture, because a wrong spiral is a subtle drift.
    #[test]
    fn the_spiral_walks_the_ring_it_says_it_does() {
        let cells: Vec<(i64, i64)> = (0..9).map(spiral_cell).collect();
        let mut unique = cells.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 9, "no cell is used twice: {cells:?}");
        assert_eq!(cells[0], (0, 0));
        for (index, (x, y)) in cells.iter().enumerate().skip(1) {
            assert!(
                x.abs() <= 1 && y.abs() <= 1,
                "cell {index} = {x},{y} left the first ring"
            );
        }
    }

    #[test]
    fn a_node_with_no_coordinate_goes_to_the_tray_and_is_counted() {
        let points = vec![
            Some((2.0, 58.0)),
            Some((5.0, 62.0)),
            Some((3.0, 60.0)),
            None,
            None,
        ];
        let (positions, frame) = layout(&points, &nodes(5), canvas()).expect("three placed");
        assert_eq!(positions.islands.len(), 1);
        assert_eq!(positions.islands[0].members, vec![3, 4]);
        assert!(positions.islands[0].orphans);
        assert_eq!(
            positions.islands[0].caption.as_deref(),
            Some(TRAY_CAPTION),
            "the tray says what it is, on the picture"
        );
        // The tray is below the map, and the map's clip stops above it.
        let map_bottom = frame.clip.1 + frame.clip.3;
        for placed in &positions.xy[..3] {
            assert!(placed.1 < map_bottom, "{placed:?} is not on the map");
        }
        for trayed in &positions.xy[3..] {
            assert!(trayed.1 > map_bottom, "{trayed:?} is not in the tray");
        }
        assert_eq!(
            tray_line(2, 5).as_deref(),
            Some("2 of 5 have no coordinate — in the tray at the foot")
        );
        assert_eq!(tray_line(0, 5), None);
    }

    /// The tray is bounded, and every one of its members is *on* the canvas.
    ///
    /// The size is the sodir prospect render's own — 1 765 nodes with neither a
    /// location nor a parseable geometry, which is what a WKT-only type with
    /// mostly-null geometries produces. The two things asserted are the two the
    /// picture depends on: the strip never grows past [`TRAY_MAX_SHARE`] of the
    /// frame (without the ceiling it wants 532 px of 1 100 and the map is a
    /// footnote), and every member is drawn inside it rather than clamped onto
    /// the canvas edge.
    #[test]
    fn a_large_tray_stays_inside_the_strip_it_was_given() {
        let canvas = Canvas {
            width: 1600.0,
            height: 1100.0,
            reserved_top: 0.0,
        };
        let mut points: Vec<Option<LonLat>> =
            vec![Some((2.0, 58.0)), Some((5.0, 62.0)), Some((3.0, 60.0))];
        points.extend(std::iter::repeat_n(None, 1_765));
        let (positions, frame) = layout(&points, &nodes(points.len()), canvas).expect("a big tray");
        let map_bottom = frame.clip.1 + frame.clip.3;
        let tray_height = canvas.height - map_bottom;
        assert!(
            tray_height <= canvas.height * TRAY_MAX_SHARE + 1.0,
            "the tray took {tray_height} px of {}, past its share",
            canvas.height
        );
        for (index, (_, y)) in positions.xy[3..].iter().enumerate() {
            assert!(
                *y > map_bottom && *y <= canvas.height - 6.0,
                "tray node {index} at y={y} is outside the strip {map_bottom}..{}",
                canvas.height
            );
        }
    }

    #[test]
    fn a_scene_with_no_coordinate_at_all_is_refused_rather_than_drawn_empty() {
        let error = layout(&[None, None, None], &nodes(3), canvas())
            .expect_err("an empty map is not a map");
        assert!(
            error.to_string().contains("no map to draw"),
            "{error} must say why"
        );
    }

    #[test]
    fn auto_needs_most_of_the_scene_and_more_than_two_places() {
        let three_places = [
            Some((2.0, 58.0)),
            Some((5.0, 62.0)),
            Some((3.0, 60.0)),
            Some((3.0, 60.0)),
        ];
        assert!(auto_eligible(&three_places));

        let mut mostly_unplaced = three_places.to_vec();
        mostly_unplaced.extend([None, None]);
        assert!(
            !auto_eligible(&mostly_unplaced),
            "4 of 6 is under the {AUTO_PLACEABLE_SHARE} share"
        );

        let one_pad = [Some((3.0, 60.0)); 40];
        assert!(
            !auto_eligible(&one_pad),
            "forty nodes in one place is a dot, not a map"
        );
        assert!(!auto_eligible(&[]));
    }

    /// The frame the emitter draws the coastline in is the frame the nodes are
    /// in — asserted directly, because the two are computed in different files
    /// and a drift between them is a coastline in the wrong place.
    #[test]
    fn the_frame_reprojects_a_node_to_where_the_node_was_drawn() {
        let points = vec![Some((2.0, 58.0)), Some((5.0, 62.0)), Some((3.5, 60.0))];
        let (positions, frame) = layout(&points, &nodes(3), canvas()).expect("three points");
        for (index, point) in points.iter().enumerate() {
            let (lon, lat) = point.expect("all placed");
            let (x, y) = frame.project(lon, lat);
            let (px, py) = positions.xy[index];
            assert!(
                (x - px).abs() < 1e-9 && (y - py).abs() < 1e-9,
                "node {index}: frame said {x},{y}; layout drew {px},{py}"
            );
        }
    }

    #[test]
    fn a_coordinate_outside_the_world_is_not_a_place() {
        assert!(in_range(3.6, 60.8));
        assert!(!in_range(361.0, 60.8));
        assert!(!in_range(3.6, 91.0));
        assert!(!in_range(f64::NAN, 60.8));
    }
}
