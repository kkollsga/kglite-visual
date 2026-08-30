//! The world's coastline, decoded from vendored TopoJSON (plan E12).
//!
//! **A quiet stroke under the graph, and nothing else.** The map this project
//! draws is a *graph* view whose positions happen to be places; the coast is
//! there so a reader can tell the North Sea from a scatter plot, not so they can
//! navigate. That decides the styling and never moves: a land outline, no
//! borders, no labels, no fill, drawn first so every node and every link is on
//! top of it.
//!
//! **The resolution, though, follows the frame** — three vendored scales and
//! [`resolution_for_span`] picking between them by how many degrees the render
//! covers. This is the label budget's honesty rule applied to the background:
//! detail the picture cannot resolve is not detail, it is bytes. A world map
//! drawn from the 1:10 000 000 outline is three megabytes of path describing
//! sub-pixel wiggles; a 5°-wide North Sea crop drawn from the 1:110 000 000
//! outline is a coast made of straight lines tens of kilometres long, which is
//! what the first version of this module shipped and what a user correctly
//! called ugly. Neither is a styling choice — both are the wrong source data
//! for the frame.
//!
//! **An interactive tile basemap is a recorded anti-goal** (plan E12). Network
//! tiles contradict the offline localhost model this whole app is built on — the
//! CLI serves from `127.0.0.1` with the frontend embedded in the binary — and a
//! tile pyramid's camera contradicts a layout whose zoom is derived from the
//! data's own bounding box. Three frozen files totalling 964 KB are the whole of
//! the ambition, on purpose.
//!
//! **Hand-rolled decoder, no crate.** TopoJSON has one shape that matters here:
//! quantised integer deltas per arc, plus a `transform` that maps them back to
//! degrees. `topojson` (the Rust crate) pulls in `geojson` and `serde_json`
//! types this crate does not otherwise want, to read three files whose structure
//! is known and frozen. Their provenance, licence and freeze are recorded in
//! `crates/kglite-visual-core/assets/land/README.md`.

use std::io::Read;
use std::sync::OnceLock;

/// The vendored outlines, gzipped. See the README beside them: frozen data
/// drops from `world-atlas@2.0.2` (ISC package, public-domain Natural Earth
/// data), not a dependency and not something a refresh job updates.
///
/// Stored compressed because the 10m file is 3.1 MB of JSON and 774 KB of
/// gzip, and it is embedded in every binary and every wheel this project ships
/// whether or not a map is ever drawn. `flate2` is already in the tree via
/// kglite, so the saving costs no new crate.
const LAND_110M_GZ: &[u8] = include_bytes!("../../assets/land/land-110m.json.gz");
const LAND_50M_GZ: &[u8] = include_bytes!("../../assets/land/land-50m.json.gz");
const LAND_10M_GZ: &[u8] = include_bytes!("../../assets/land/land-10m.json.gz");

/// One closed ring of the land outline, in `(longitude, latitude)` degrees.
pub type Ring = Vec<(f64, f64)>;

/// Which vendored outline a render draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// 1:110 000 000 — 130 arcs, 5 129 points. Continents.
    Land110m,
    /// 1:50 000 000 — 1 425 arcs, 60 635 points. Countries and large fjords.
    Land50m,
    /// 1:10 000 000 — 4 075 arcs, 408 957 points. Individual islands and skerries.
    Land10m,
}

/// Degrees of span at or above which a coarser outline is the honest one.
///
/// **Chosen by eye, 2026-08-30**, by drawing the Norwegian coast at 900×600 at
/// every one of 4°, 8°, 12°, 16°, 20°, 30°, 40°, 55°, 60°, 90°, 120°, 180° and
/// 360° of span, in all three scales, and looking at all of them. What the
/// comparison showed, and where each line sits:
///
/// - **Below 25°, 10m is the only one that draws a coast.** At 4° the 50m
///   outline is three smooth blobs where western Norway has a fjord system;
///   10m has the fjords and the skerries, for 25 KB of path. At 20° the fjords
///   are still structure a reader uses. At 30° they have become texture: 50m
///   (43 KB) and 10m (367 KB) differ by fuzz along a line of the same shape.
///   25° is between the last frame where the detail is structure and the first
///   where it is noise.
/// - **Between 25° and 120°, 50m.** At 55° — sodir's whole shelf, the frame
///   this project actually renders — 110m draws Norway as a lump and Svalbard
///   as a pebble, which is the picture a user called ugly and the reason G4b
///   exists; 50m draws both, for 72 KB against 4 KB. 10m at the same frame
///   costs 583 KB to add speckle.
/// - **At 120° and above, 110m.** At 180° the 50m outline's extra 22 000 points
///   are Arctic archipelago speckle a 900 px frame cannot separate; the two
///   pictures are indistinguishable at a glance and one is 12× the other. At
///   90° they are still distinguishable (Denmark's islands, Scotland's west
///   coast), which is why the line is at 120 and not at 90.
///
/// Both lines sit between observations rather than on one, so a render a degree
/// either side of a boundary does not swing between visibly different
/// backgrounds. Worst-case coastline path bytes per band, at 900×600: 250 KB
/// just under 25°, 190 KB just under 120°, 66 KB for a whole world.
const COARSE_SPAN_DEG: f64 = 120.0;
const MEDIUM_SPAN_DEG: f64 = 25.0;

/// The coarsest outline whose detail the frame can still resolve.
///
/// `span_deg` is the longer side of the map's lon/lat box. Deliberately *not* a
/// degrees-per-pixel figure: the canvas size is a caller's argument, and tying
/// the source data to it would mean the same query at 900 px and at 1800 px
/// drew backgrounds from different files — a golden whose data tier depends on
/// its dimensions is a golden that pins less than it looks like it does.
pub fn resolution_for_span(span_deg: f64) -> Resolution {
    if span_deg >= COARSE_SPAN_DEG {
        Resolution::Land110m
    } else if span_deg >= MEDIUM_SPAN_DEG {
        Resolution::Land50m
    } else {
        Resolution::Land10m
    }
}

/// Every ring of the world's land at one scale, decoded once per process.
///
/// `OnceLock` per tier rather than a decode per render: the bytes are constant,
/// the answer is constant, and a render that re-inflated and re-parsed 3 MB of
/// JSON each time would put the coastline on the critical path of a picture it
/// is only the background of. Per tier, not one shared slot, because a session
/// that renders both a world map and a fjord crop needs both and neither should
/// evict the other.
///
/// A decode failure yields an empty outline rather than an error — a map
/// without a coast is still a map, and a render is not the place to discover
/// that a compiled-in constant is malformed. The unit tests below are.
pub fn land(resolution: Resolution) -> &'static [Ring] {
    static LAND_110M: OnceLock<Vec<Ring>> = OnceLock::new();
    static LAND_50M: OnceLock<Vec<Ring>> = OnceLock::new();
    static LAND_10M: OnceLock<Vec<Ring>> = OnceLock::new();
    let (slot, bytes) = match resolution {
        Resolution::Land110m => (&LAND_110M, LAND_110M_GZ),
        Resolution::Land50m => (&LAND_50M, LAND_50M_GZ),
        Resolution::Land10m => (&LAND_10M, LAND_10M_GZ),
    };
    slot.get_or_init(|| {
        inflate(bytes)
            .as_deref()
            .and_then(decode)
            .unwrap_or_default()
    })
}

/// The gzip member, back to JSON.
fn inflate(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// The rings of one outline, cut down to the segments a box can see.
///
/// **Per-segment, not per-ring, and that is the whole cost model.** Rejecting a
/// ring only when *every* point is outside the box is enough at 110m, where the
/// largest ring is 1 100 points. At 10m the Eurasian landmass is one ring of
/// ~90 000 points, and a North Sea crop touches it — so a per-ring test emits
/// all 90 000, of which a few hundred are on screen. That is not a slow render;
/// it is a 5 MB SVG of a 5° box.
///
/// A segment is kept when its own bounding box overlaps the clip box, which is
/// a superset of the segments that truly cross it — cheap, and wrong only in
/// the direction that keeps a line the reader would have seen. Consecutive kept
/// segments join into one polyline; a gap starts a new one. The result is open
/// polylines rather than closed rings, which is identical on screen because the
/// coast is stroked and never filled.
pub fn clipped(resolution: Resolution, bbox: (f64, f64, f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    for ring in land(resolution) {
        clip_ring(ring, bbox, &mut out);
    }
    out
}

fn clip_ring(ring: &Ring, bbox: (f64, f64, f64, f64), out: &mut Vec<Vec<(f64, f64)>>) {
    let (west, south, east, north) = bbox;
    let mut current: Vec<(f64, f64)> = Vec::new();
    for pair in ring.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let visible = a.0.min(b.0) <= east
            && a.0.max(b.0) >= west
            && a.1.min(b.1) <= north
            && a.1.max(b.1) >= south;
        if visible {
            if current.is_empty() {
                current.push(a);
            }
            current.push(b);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    // Two points is a line and one is nothing; a coast made of those is noise.
    if current.len() >= 2 {
        out.push(current);
    }
}

/// Decode a TopoJSON topology's `land` object into closed rings.
///
/// Returns `None` for anything that is not the shape this file is: the
/// alternative is a coastline silently missing half its arcs, which is a
/// picture that looks like a rendering bug and is not one.
pub fn decode(bytes: &[u8]) -> Option<Vec<Ring>> {
    let topology: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let transform = topology.get("transform")?;
    let scale = pair(transform.get("scale")?)?;
    let translate = pair(transform.get("translate")?)?;

    // **Arcs are delta-encoded and are decoded once, not once per use.** An arc
    // is shared by every ring that borders along it (that is the whole point of
    // the format), and the same arc appears reversed on the other side.
    let arcs: Vec<Vec<(f64, f64)>> = topology
        .get("arcs")?
        .as_array()?
        .iter()
        .map(|arc| decode_arc(arc, scale, translate))
        .collect::<Option<_>>()?;

    let land = topology.get("objects")?.get("land")?;
    let mut rings: Vec<Ring> = Vec::new();
    // `land` is a GeometryCollection of MultiPolygons in the vendored file, but
    // a bare Polygon/MultiPolygon is equally valid TopoJSON; walking both keeps
    // the decoder a fact about the format rather than about one file.
    collect(land, &arcs, &mut rings);
    (!rings.is_empty()).then_some(rings)
}

/// One arc: an integer starting point followed by integer deltas, mapped
/// through `scale`/`translate` into degrees.
fn decode_arc(
    arc: &serde_json::Value,
    scale: (f64, f64),
    translate: (f64, f64),
) -> Option<Vec<(f64, f64)>> {
    let points = arc.as_array()?;
    let mut out = Vec::with_capacity(points.len());
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for point in points {
        let (dx, dy) = pair(point)?;
        x += dx;
        y += dy;
        out.push((x * scale.0 + translate.0, y * scale.1 + translate.1));
    }
    Some(out)
}

/// Walk a TopoJSON geometry and push every ring it names.
fn collect(geometry: &serde_json::Value, arcs: &[Vec<(f64, f64)>], out: &mut Vec<Ring>) {
    match geometry.get("type").and_then(|t| t.as_str()) {
        Some("GeometryCollection") => {
            for child in geometry
                .get("geometries")
                .and_then(|g| g.as_array())
                .map(|v| v.as_slice())
                .unwrap_or(&[])
            {
                collect(child, arcs, out);
            }
        }
        Some("MultiPolygon") => {
            for polygon in as_array(geometry.get("arcs")) {
                for ring in as_array(Some(polygon)) {
                    push_ring(ring, arcs, out);
                }
            }
        }
        Some("Polygon") => {
            for ring in as_array(geometry.get("arcs")) {
                push_ring(ring, arcs, out);
            }
        }
        // LineString / Point / anything else: the land outline has none, and
        // inventing a ring from one would draw a shape that is not a coast.
        _ => {}
    }
}

/// One ring: a list of arc indices, each joined to the last.
///
/// A **negative** index `i` means arc `-i - 1` traversed backwards — TopoJSON's
/// way of saying "the same shared boundary, from the other side". The first
/// point of every arc after the first is dropped, because it is the previous
/// arc's last point and drawing it twice puts a zero-length segment in the path.
fn push_ring(ring: &serde_json::Value, arcs: &[Vec<(f64, f64)>], out: &mut Vec<Ring>) {
    let mut points: Ring = Vec::new();
    for index in as_array(Some(ring)) {
        let Some(index) = index.as_i64() else {
            continue;
        };
        let (slot, reversed) = if index < 0 {
            ((-index - 1) as usize, true)
        } else {
            (index as usize, false)
        };
        let Some(arc) = arcs.get(slot) else { continue };
        let mut segment: Vec<(f64, f64)> = arc.clone();
        if reversed {
            segment.reverse();
        }
        if points.is_empty() {
            points = segment;
        } else {
            points.extend(segment.into_iter().skip(1));
        }
    }
    // Two points is a line, not a ring; a coast made of those is noise.
    if points.len() >= 3 {
        out.push(points);
    }
}

fn as_array(value: Option<&serde_json::Value>) -> &[serde_json::Value] {
    value
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

fn pair(value: &serde_json::Value) -> Option<(f64, f64)> {
    let array = value.as_array()?;
    Some((array.first()?.as_f64()?, array.get(1)?.as_f64()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A four-point square in one arc, with the transform doing real work.
    ///
    /// The golden this file's decoder is really checked against is the vendored
    /// world, which no assertion can read by eye. This fixture is the one whose
    /// every coordinate is arithmetic a reader can do in their head, so a
    /// decoder that got the delta accumulation or the transform backwards fails
    /// here with an obvious number rather than a slightly wrong continent.
    const SQUARE: &str = r#"{
      "type": "Topology",
      "transform": {"scale": [0.5, 0.25], "translate": [-10.0, 40.0]},
      "arcs": [[[0, 0], [4, 0], [0, 4], [-4, 0], [0, -4]]],
      "objects": {"land": {"type": "GeometryCollection",
        "geometries": [{"type": "MultiPolygon", "arcs": [[[0]]]}]}}
    }"#;

    #[test]
    fn the_decoder_accumulates_deltas_and_applies_the_transform() {
        let rings = decode(SQUARE.as_bytes()).expect("the fixture decodes");
        assert_eq!(rings.len(), 1);
        assert_eq!(
            rings[0],
            vec![
                (-10.0, 40.0),
                (-8.0, 40.0),
                (-8.0, 41.0),
                (-10.0, 41.0),
                (-10.0, 40.0),
            ],
            "x accumulates 0,4,4,0,0 at scale 0.5 from -10; y accumulates \
             0,0,4,4,0 at scale 0.25 from 40"
        );
    }

    /// The negative-index rule, which is the one place a plausible decoder gets
    /// a coastline visibly wrong: the shared arc comes back mirrored.
    #[test]
    fn a_negative_arc_index_is_the_same_arc_backwards() {
        let mirrored = SQUARE.replace(r#""arcs": [[[0]]]"#, r#""arcs": [[[-1]]]"#);
        let forward = decode(SQUARE.as_bytes()).expect("forward");
        let backward = decode(mirrored.as_bytes()).expect("backward");
        let mut expected = forward[0].clone();
        expected.reverse();
        assert_eq!(backward[0], expected);
    }

    #[test]
    fn two_arcs_join_without_repeating_the_shared_point() {
        // Arc 0 ends where arc 1 begins; the joined ring must carry that point
        // once. A duplicate is invisible in a picture and is exactly the kind of
        // thing that doubles a path's size.
        let two = r#"{
          "type": "Topology",
          "transform": {"scale": [1.0, 1.0], "translate": [0.0, 0.0]},
          "arcs": [[[0, 0], [2, 0]], [[2, 0], [0, 2]]],
          "objects": {"land": {"type": "Polygon", "arcs": [[0, 1]]}}
        }"#;
        let rings = decode(two.as_bytes()).expect("two arcs");
        assert_eq!(rings[0], vec![(0.0, 0.0), (2.0, 0.0), (2.0, 2.0)]);
    }

    #[test]
    fn a_topology_of_the_wrong_shape_is_refused_rather_than_half_decoded() {
        assert!(decode(b"not json").is_none());
        assert!(decode(br#"{"type":"Topology"}"#).is_none());
        // A topology with a `land` object that names no ring: an empty
        // coastline is not a coastline, and returning `Some(vec![])` would let
        // a silently blank map pass for a drawn one.
        let empty = r#"{"type":"Topology","transform":{"scale":[1,1],"translate":[0,0]},
                        "arcs":[],"objects":{"land":{"type":"Polygon","arcs":[]}}}"#;
        assert!(decode(empty.as_bytes()).is_none());
    }

    /// The vendored files are what actually ship, so all three are asserted —
    /// against numbers a wrong decode or a wrong gzip member cannot produce.
    ///
    /// Each tier is checked for its *own* extent rather than a shared one: the
    /// 10m file's southern edge is −85.2°, not −90°, because upstream clips
    /// Antarctica there. A single shared bound would have to be the loosest of
    /// the three and would stop being a check.
    #[test]
    fn every_vendored_world_decodes_into_a_world() {
        for (resolution, rings_at_least, south_range) in [
            (Resolution::Land110m, 90usize, -90.0..=-85.0),
            (Resolution::Land50m, 800, -90.0..=-85.0),
            (Resolution::Land10m, 3_000, -86.0..=-85.0),
        ] {
            let rings = land(resolution);
            assert!(
                rings.len() >= rings_at_least,
                "{resolution:?} should carry at least {rings_at_least} rings; got {}",
                rings.len()
            );
            let (mut west, mut south) = (f64::INFINITY, f64::INFINITY);
            let (mut east, mut north) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for ring in rings {
                for (lon, lat) in ring {
                    west = west.min(*lon);
                    east = east.max(*lon);
                    south = south.min(*lat);
                    north = north.max(*lat);
                }
            }
            assert!(
                (-180.0..=-179.0).contains(&west) && (179.0..=180.0).contains(&east),
                "{resolution:?}: the world spans the whole longitude range; got {west}..{east}"
            );
            assert!(
                south_range.contains(&south) && (83.0..=84.0).contains(&north),
                "{resolution:?}: expected {south_range:?} to northern Greenland; \
                 got {south}..{north}"
            );
            // Norway is the case this exists for: some land in the box the sodir
            // acceptance renders sit in.
            assert!(
                rings.iter().any(|ring| ring
                    .iter()
                    .any(|(lon, lat)| (4.0..12.0).contains(lon) && (58.0..66.0).contains(lat))),
                "{resolution:?}: no coastline inside southern Norway's box"
            );
        }
    }

    /// Finer is finer — the property the whole tiering rests on, asserted
    /// rather than assumed from three file names.
    #[test]
    fn each_tier_carries_more_detail_than_the_one_above_it() {
        let points = |resolution| -> usize { land(resolution).iter().map(|ring| ring.len()).sum() };
        let coarse = points(Resolution::Land110m);
        let medium = points(Resolution::Land50m);
        let fine = points(Resolution::Land10m);
        assert!(
            coarse < medium && medium < fine,
            "110m {coarse} < 50m {medium} < 10m {fine}"
        );
    }

    #[test]
    fn the_span_picks_the_scale_the_frame_can_resolve() {
        // A world view, sodir's whole shelf, and a North Sea crop — the three
        // frames the thresholds were chosen against.
        assert_eq!(resolution_for_span(340.0), Resolution::Land110m);
        assert_eq!(resolution_for_span(55.0), Resolution::Land50m);
        assert_eq!(resolution_for_span(5.0), Resolution::Land10m);
        // The boundaries themselves, so a `>` that should be `>=` fails here
        // rather than in a picture nobody diffs.
        assert_eq!(resolution_for_span(COARSE_SPAN_DEG), Resolution::Land110m);
        assert_eq!(resolution_for_span(MEDIUM_SPAN_DEG), Resolution::Land50m);
    }

    /// The clip is what stops a 5° crop emitting a 90 000-point Eurasia.
    #[test]
    fn clipping_keeps_what_the_box_can_see_and_drops_the_rest() {
        // A square ring around the origin, one point per degree along each
        // side, and a box covering only its western edge.
        let mut ring: Ring = Vec::new();
        for x in -10..=10 {
            ring.push((f64::from(x), -10.0));
        }
        for y in -10..=10 {
            ring.push((10.0, f64::from(y)));
        }
        for x in (-10..=10).rev() {
            ring.push((f64::from(x), 10.0));
        }
        for y in (-10..=10).rev() {
            ring.push((-10.0, f64::from(y)));
        }
        let mut kept: Vec<Vec<(f64, f64)>> = Vec::new();
        clip_ring(&ring, (-11.0, -2.0, -9.0, 2.0), &mut kept);

        let points: usize = kept.iter().map(|line| line.len()).sum();
        assert!(
            points > 0 && points < ring.len() / 4,
            "the western edge is a small fraction of the ring; kept {points} of {}",
            ring.len()
        );
        for line in &kept {
            assert!(
                line.iter()
                    .any(|(lon, lat)| (-11.0..=-9.0).contains(lon) && (-2.0..=2.0).contains(lat)),
                "every kept polyline must touch the box"
            );
        }
    }

    #[test]
    fn a_ring_entirely_outside_the_box_contributes_nothing() {
        let ring: Ring = vec![(20.0, 20.0), (21.0, 20.0), (21.0, 21.0), (20.0, 20.0)];
        let mut kept = Vec::new();
        clip_ring(&ring, (-1.0, -1.0, 1.0, 1.0), &mut kept);
        assert!(kept.is_empty());
    }

    /// A segment whose endpoints straddle the box with neither inside is still
    /// a line the reader would see, and dropping it puts a hole in the coast.
    #[test]
    fn a_segment_crossing_the_box_survives_with_both_ends_outside() {
        let ring: Ring = vec![(-10.0, 0.0), (10.0, 0.0), (10.0, 1.0)];
        let mut kept = Vec::new();
        clip_ring(&ring, (-1.0, -1.0, 1.0, 1.0), &mut kept);
        assert_eq!(kept, vec![vec![(-10.0, 0.0), (10.0, 0.0)]]);
    }
}
