//! The world's coastline, decoded from a vendored TopoJSON (plan E12).
//!
//! **A quiet stroke under the graph, and nothing else.** The map this project
//! draws is a *graph* view whose positions happen to be places; the coast is
//! there so a reader can tell the North Sea from a scatter plot, not so they can
//! navigate. That decides everything here: one land outline at 1:110,000,000, no
//! borders, no labels, no fill, drawn first so every node and every link is on
//! top of it.
//!
//! **An interactive tile basemap is a recorded anti-goal** (plan E12). Network
//! tiles contradict the offline localhost model this whole app is built on — the
//! CLI serves from `127.0.0.1` with the frontend embedded in the binary — and a
//! tile pyramid's camera contradicts a layout whose zoom is derived from the
//! data's own bounding box. The 21 KB of TopoJSON below is the whole of the
//! ambition, on purpose.
//!
//! **Hand-rolled decoder, ~100 lines, no crate.** TopoJSON has one shape that
//! matters here: quantised integer deltas per arc, plus a `transform` that maps
//! them back to degrees. `topojson` (the Rust crate) pulls in `geojson` and
//! `serde_json` types this crate does not otherwise want, to read one file whose
//! structure is known and frozen. The file's provenance, licence and freeze are
//! recorded in `crates/kglite-visual-core/assets/land-110m/README.md`.

use std::sync::OnceLock;

/// The vendored outline. See the README beside it: a frozen data drop from
/// `world-atlas@2.0.2` (ISC package, public-domain Natural Earth data), not a
/// dependency and not something a refresh job updates.
const LAND_110M: &[u8] = include_bytes!("../../assets/land-110m/land-110m.json");

/// One closed ring of the land outline, in `(longitude, latitude)` degrees.
pub type Ring = Vec<(f64, f64)>;

/// Every ring of the world's land, decoded once per process.
///
/// `OnceLock` rather than a decode per render: the bytes are constant, the
/// answer is constant, and a render that re-parsed 55 KB of JSON each time
/// would put the coastline on the critical path of a picture it is only the
/// background of. A decode failure yields an empty outline rather than an error
/// — a map without a coast is still a map, and a render is not the place to
/// discover that a compiled-in constant is malformed. The unit tests below are.
pub fn land() -> &'static [Ring] {
    static RINGS: OnceLock<Vec<Ring>> = OnceLock::new();
    RINGS.get_or_init(|| decode(LAND_110M).unwrap_or_default())
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

    /// The vendored file is the thing that actually ships, so it is asserted —
    /// against numbers a wrong decode cannot produce.
    #[test]
    fn the_vendored_world_decodes_into_a_world() {
        let rings = land();
        assert!(
            rings.len() > 90,
            "land-110m carries ~100 landmasses; got {}",
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
            "the world spans the whole longitude range; got {west}..{east}"
        );
        assert!(
            (-90.0..=-85.0).contains(&south) && (83.0..=84.0).contains(&north),
            "Antarctica to northern Greenland; got {south}..{north}"
        );
        // Norway is the case this exists for: some land in the box the sodir
        // acceptance renders sit in.
        assert!(
            rings.iter().any(|ring| ring
                .iter()
                .any(|(lon, lat)| (4.0..12.0).contains(lon) && (58.0..66.0).contains(lat))),
            "no coastline inside southern Norway's box"
        );
    }
}
