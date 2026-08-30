//! The render's exact baselines, regenerated here and diffed by the gate.
//!
//! **This is an exact baseline, and it fails the moment "make it pass" is
//! cheaper than "explain the diff"** (CLAUDE.md → "Gate honesty"). Every
//! deliberate change to the visual encoding, the layout or the emitter moves
//! these files; regenerate in the same commit and say why. Regenerating to
//! silence a diff nobody can explain is the failure the whole mechanism exists
//! to prevent, and there is nothing else in the tree that would notice a
//! layout that quietly stopped being deterministic.
//!
//! Three documents, because each pins something the others cannot:
//!
//! 1. `fixture-meta-dark.svg` — the meta-graph path: the size ramp, the link
//!    widths, the capability badges, the label grid with `place_all` on, the
//!    dark palette.
//! 2. `fixture-meta-light.svg` — the same scene through the other palette, so a
//!    light-theme constant cannot drift unnoticed. Same geometry by
//!    construction: the theme reaches the chrome and never the layout.
//! 3. `fixture-cypher-dark.svg` — the instance path: uniform radii, floor link
//!    widths, the title-or-`type id` display fallback, and the label grid with
//!    `place_all` **off**, which is a different branch of the same function.
//! 4. `fixture-geo-dark.svg` — the geographic kernel (G4): the equirectangular
//!    projection, the `cos φ₁` aspect correction, the vendored coastline and
//!    the graticule. The fixture's ten `City` nodes carry real
//!    latitude/longitude and are spread over the whole globe, so this pins the
//!    world-scale case; sodir's 56–82°N case is exercised by the acceptance
//!    renders, which are not committed.
//!
//! `make check-render-baseline` is the gate step that diffs them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kglite_visual_core::render::{self, RenderFormat, RenderRequest, RenderSource, Theme};
use kglite_visual_core::request::CypherRequest;
use kglite_visual_core::{load_graph, GraphSource, QueryConfig};

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("meta.kgl")
}

fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
}

fn fixture_graph() -> Arc<kglite::api::DirGraph> {
    load_graph(GraphSource::Path(&fixture_path())).expect("the committed fixture loads")
}

/// Write only when the content differs, so an unchanged run leaves the file's
/// mtime alone — the embedded-bundle freshness check compares mtimes, and a
/// baseline rewritten on every `cargo test` would keep marking sources newer
/// than the bundle built from them. (Same rule, same reason, as
/// `protocol_baseline.rs`; P7 paid for learning it.)
fn write_if_changed(path: &Path, content: &str) {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("golden directory");
    }
    std::fs::write(path, content).expect("write golden");
}

fn render_to_string(request: &RenderRequest) -> String {
    let graph = fixture_graph();
    let rendered = render::render(&graph, "meta.kgl", QueryConfig::default(), request)
        .expect("the fixture renders");
    assert_eq!(rendered.format, RenderFormat::Svg);
    String::from_utf8(rendered.bytes).expect("the emitter writes UTF-8")
}

fn meta_request(theme: Theme) -> RenderRequest {
    RenderRequest {
        source: RenderSource::Meta,
        format: RenderFormat::Svg,
        // Smaller than the CLI default: the golden is read by humans in a
        // diff, and 2000x1250 of coordinates is a wall. The encoding is the
        // same at every size.
        width: 900,
        height: 600,
        seed: 1234,
        theme,
        kernel: None,
    }
}

fn cypher_request() -> RenderRequest {
    RenderRequest {
        source: RenderSource::Cypher(CypherRequest {
            // A relationship the fixture actually has. The first draft of this
            // golden named `WORKS_ON`, which the fixture does not have: it
            // rendered a canvas with zero nodes on it, and a golden of nothing
            // is a check that cannot fail. kglite says so on stderr; nothing
            // else was going to.
            query: "MATCH (p:Person)-[r:WORKS_AT]->(c:Company) RETURN p, r, c".to_string(),
            params: Default::default(),
            limit: Some(24),
            as_graph: true,
        }),
        format: RenderFormat::Svg,
        width: 900,
        height: 600,
        seed: 1234,
        theme: Theme::Dark,
        kernel: None,
    }
}

#[test]
fn generate_meta_graph_goldens() {
    for (theme, name) in [
        (Theme::Dark, "fixture-meta-dark.svg"),
        (Theme::Light, "fixture-meta-light.svg"),
    ] {
        write_if_changed(
            &goldens_dir().join(name),
            &render_to_string(&meta_request(theme)),
        );
    }
}

/// The geographic kernel, on the one committed fixture that has coordinates.
///
/// **`kernel: Some(Geo)`, not `auto`** — deliberately. Ten cities out of a
/// 60-person graph is nowhere near `geo::AUTO_PLACEABLE_SHARE`, so `auto` would
/// (correctly) draw something else and this golden would silently stop pinning
/// the projection. Naming the kernel is what makes the file a check of the
/// thing it is named after.
fn geo_request() -> RenderRequest {
    RenderRequest {
        source: RenderSource::Cypher(CypherRequest {
            query: "MATCH (c:City) RETURN c".to_string(),
            params: Default::default(),
            limit: Some(24),
            as_graph: true,
        }),
        format: RenderFormat::Svg,
        width: 900,
        height: 600,
        seed: 1234,
        theme: Theme::Dark,
        kernel: Some(kglite_visual_core::request::LayoutKernel::Geo),
    }
}

#[test]
fn generate_geo_golden() {
    let svg = render_to_string(&geo_request());
    // The same vacuity guard the cypher golden carries, plus the two things
    // that make this file a *geo* golden rather than another scatter of dots:
    // the coastline path and the graticule are drawn from the vendored
    // TopoJSON, and a golden with neither would pin nothing this test claims.
    assert!(
        svg.matches("<circle").count() >= 10,
        "the geo golden must actually contain the ten cities"
    );
    assert!(
        svg.contains("kglv-coast"),
        "the geo golden must carry the coastline it is a golden of"
    );
    assert!(
        svg.contains("kglv-graticule"),
        "the geo golden must carry the graticule"
    );
    write_if_changed(&goldens_dir().join("fixture-geo-dark.svg"), &svg);
}

/// The map is a claim about places, so the same request must land the same node
/// on the same pixel — and a *different* node somewhere else.
#[test]
fn the_geo_render_is_a_pure_function_of_its_request() {
    let first = render_to_string(&geo_request());
    let second = render_to_string(&geo_request());
    assert_eq!(first, second);

    // The seed reaches the force pass's initial placement and nothing else, so
    // a map must be identical under a different one: geography is not seeded.
    let mut reseeded = geo_request();
    reseeded.seed = 99;
    assert_eq!(
        render_to_string(&reseeded),
        first,
        "a map has no seed to vary — the positions come from the data"
    );
}

#[test]
fn generate_cypher_golden() {
    let svg = render_to_string(&cypher_request());
    // The vacuity guard. A golden generated from a query that matches nothing
    // is a stable file that pins no encoding at all, which is exactly how this
    // baseline was born the first time.
    assert!(
        svg.matches("<circle").count() > 10,
        "the cypher golden must actually contain a graph"
    );
    write_if_changed(&goldens_dir().join("fixture-cypher-dark.svg"), &svg);
}

/// The property the golden rests on, asserted directly rather than inferred
/// from a file that has not changed.
///
/// A golden that is only ever compared against its own previous self cannot
/// tell "the layout is deterministic" from "the layout is deterministic on this
/// machine, this run". Two renders in one process, byte-compared, is the
/// smallest statement of the first.
#[test]
fn two_renders_of_one_request_are_byte_identical() {
    let first = render_to_string(&meta_request(Theme::Dark));
    let second = render_to_string(&meta_request(Theme::Dark));
    assert_eq!(
        first, second,
        "the render is a pure function of its request"
    );

    // And a different seed really is a different picture, or `--seed` is a flag
    // that does nothing.
    let mut moved = meta_request(Theme::Dark);
    moved.seed = 99;
    assert_ne!(render_to_string(&moved), first);
}

/// The theme reaches the chrome and never the geometry.
///
/// Stated as a test because it is the reason the light golden is cheap: if a
/// palette could move a node, every theme would need its own layout review.
#[test]
fn the_two_themes_differ_only_in_colour() {
    let dark = render_to_string(&meta_request(Theme::Dark));
    let light = render_to_string(&meta_request(Theme::Light));
    let geometry = |svg: &str| -> Vec<String> {
        svg.lines()
            .filter(|line| line.starts_with("<circle") || line.starts_with("<line"))
            .map(|line| {
                line.split_whitespace()
                    .filter(|token| {
                        token.starts_with("cx=")
                            || token.starts_with("cy=")
                            || token.starts_with("r=")
                            || token.starts_with("x1=")
                            || token.starts_with("y1=")
                            || token.starts_with("x2=")
                            || token.starts_with("y2=")
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    };
    assert_eq!(geometry(&dark), geometry(&light));
    assert_ne!(dark, light, "the palettes must actually differ");
}

/// D5, drawn into the image.
///
/// The fixture is far inside every bound, so this drives the *truncated* case
/// by asking for a limit the graph exceeds — and asserts the words, not a
/// boolean beside them, because the words are what a reader of the image gets.
#[test]
fn a_truncated_render_says_so_in_the_image_and_in_the_metadata() {
    let graph = fixture_graph();
    let request = RenderRequest {
        source: RenderSource::Expand(render::ExpandSource {
            node_type: "Person".to_string(),
            relationship: None,
            direction: Default::default(),
            limit: Some(12),
        }),
        format: RenderFormat::Svg,
        width: 900,
        height: 600,
        seed: 1,
        theme: Theme::Dark,
        kernel: None,
    };
    let rendered = render::render(&graph, "meta.kgl", QueryConfig::default(), &request)
        .expect("the expansion renders");
    assert!(rendered.truncated, "12 of a 60-member type is a clip");
    assert_eq!(rendered.nodes, 12);
    let banner = rendered
        .banners
        .first()
        .expect("a truncated render carries a banner")
        .clone();
    assert!(
        banner.starts_with("showing 12 of "),
        "the app's own wording: {banner}"
    );
    let svg = String::from_utf8(rendered.bytes).expect("UTF-8");
    assert!(
        svg.contains(&banner),
        "the banner has to be IN the image; an image travels without its response"
    );
}

/// The meta-graph names every type, and an instance slice does not have to.
///
/// The two branches of `labels::choose`, observed through the emitter rather
/// than the function, so a caller that stopped passing `place_all` is caught.
#[test]
fn every_type_gets_a_label_on_the_meta_graph() {
    let svg = render_to_string(&meta_request(Theme::Dark));
    for name in ["Person", "Project", "Skill", "City", "Company"] {
        assert!(
            // Name and count share one `<text>`; the count is a `<tspan>` so
            // the renderer's own glyph advance separates them.
            svg.contains(&format!(">{name}<tspan")),
            "the meta-graph IS its labels; {name} has none"
        );
    }
}

/// The node a radial layout centres is findable in one glance.
///
/// Before P11 round 2 the centre was a six-pixel dot identical to every leaf on
/// the ring around it, and "it is in the middle" was the only signal — which
/// stops being one the moment the picture holds two clusters, as this fixture's
/// does. Asserted through the emitted document rather than the flag, because
/// the flag being set and the glyph being drawn are two different facts.
#[test]
fn the_node_a_ring_is_centred_on_is_drawn_larger_and_haloed() {
    let svg = render_to_string(&cypher_request());
    let circles: Vec<&str> = svg
        .lines()
        .filter(|line| line.starts_with("<circle"))
        .collect();
    let radius_of = |line: &str| -> f64 {
        line.split(" r=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.0)
    };
    let haloes = circles
        .iter()
        .filter(|line| line.contains("fill-opacity=\"0.12\""))
        .count();
    assert!(
        haloes > 0,
        "a ring's centre carries a halo; none was emitted"
    );
    let widest_leaf = circles
        .iter()
        .filter(|line| line.contains("<title>Person_"))
        .map(|line| radius_of(line))
        .fold(0.0f64, f64::max);
    let centre = circles
        .iter()
        .filter(|line| line.contains("<title>Company_"))
        .map(|line| radius_of(line))
        .fold(0.0f64, f64::max);
    assert!(
        centre > widest_leaf,
        "the centre ({centre}) must outsize a leaf ({widest_leaf})"
    );
}

/// A packed community is enclosed, and the enclosure stays quiet.
///
/// Both halves matter. Without a boundary a reader cannot tell "this gap is
/// where one group ends" from "this gap is where the packing left room", which
/// was the coordinator's round-1 verdict on the meta-graph. With a loud one the
/// boxes become the picture and the graph inside them becomes decoration, so
/// the opacity is asserted too — a boundary nobody can ignore is the same
/// defect wearing the opposite sign.
#[test]
fn a_packed_community_is_enclosed_quietly() {
    let svg = render_to_string(&cypher_request());
    let hulls: Vec<&str> = svg
        .lines()
        .filter(|line| line.starts_with("<rect") && line.contains("rx=\"22\""))
        .collect();
    assert!(
        !hulls.is_empty(),
        "the island layout drew no boundary at all"
    );
    for hull in &hulls {
        assert!(
            hull.contains("fill-opacity=\"0.045\"") || hull.contains("fill=\"none\""),
            "an island boundary must stay a hint, not a box: {hull}"
        );
        assert!(
            hull.contains("stroke-opacity=\"0.16\""),
            "an island boundary must stay a hint, not a box: {hull}"
        );
    }
}

/// A dense picture spends less ink per line than a sparse one, and a
/// cross-community line is quieter than one inside a community.
///
/// The failure: 3,374 links drawn at the 0.45 stroke opacity that suits 124
/// composite into a white wash with the nodes floating on it. Read out of the
/// emitted groups rather than off `encoding::link_ink`, because the ramp
/// existing and the emitter using it are two facts — and the cross-island split
/// is only observable here.
#[test]
fn a_dense_render_draws_quieter_lines_than_a_sparse_one() {
    let opacities = |svg: &str| -> Vec<f64> {
        let mut out: Vec<f64> = svg
            .lines()
            .filter(|line| line.starts_with("<g stroke=\"#"))
            .filter_map(|line| {
                line.split("stroke-opacity=\"")
                    .nth(1)?
                    .split('"')
                    .next()?
                    .parse()
                    .ok()
            })
            .collect();
        out.sort_by(f64::total_cmp);
        out
    };

    let sparse = opacities(&render_to_string(&cypher_request()));
    let dense = opacities(&render_to_string(&RenderRequest {
        source: RenderSource::Cypher(CypherRequest {
            // Every edge a Person has: 511 links over 107 nodes, past the point
            // where the full budget is legible.
            query: "MATCH (a:Person)-[r]-(b) RETURN a, r, b".to_string(),
            params: Default::default(),
            limit: Some(900),
            as_graph: true,
        }),
        ..meta_request(Theme::Dark)
    }));

    assert!(!sparse.is_empty() && !dense.is_empty(), "no link groups");
    assert!(
        dense.last() < sparse.last(),
        "the dense picture must draw quieter: {dense:?} vs {sparse:?}"
    );
    assert!(
        dense.len() == 2 && dense[0] < dense[1],
        "cross-community lines are drawn as background: {dense:?}"
    );
}

/// A canvas too small for the schema draws the largest types and says so.
///
/// The meta-graph arrives at one of the engine's tiers — a question about the
/// *graph* — and a canvas is a second bound the engine knows nothing about. At
/// 800x500 sodir's `compact` 98 types are 98 circles and 98 names in a canvas
/// with room for 24: round 3's verdict on that image was "honest and useless".
/// Keeping the largest is right; keeping them silently would make the picture
/// claim the graph has fewer types than it does, which is the failure D5 names
/// for a clipped result.
#[test]
fn a_canvas_too_small_for_the_schema_draws_the_largest_and_says_so() {
    let request = RenderRequest {
        width: 300,
        height: 250,
        ..meta_request(Theme::Dark)
    };
    let rendered = render_meta(&request);
    let svg = String::from_utf8(rendered.bytes.clone()).expect("the emitter writes UTF-8");

    assert_eq!(rendered.types_total, Some(5));
    assert_eq!(rendered.types_shown, Some(2), "the canvas holds two chips");
    assert_eq!(
        svg.matches("<circle").count(),
        2,
        "the types it cannot name are not drawn either — an unlabelled circle \
         in a meta-graph is a dot standing for nothing"
    );
    assert!(
        svg.contains("top 2 of 5 types shown — render larger for all"),
        "the drop has to be in the picture, with the way out beside it"
    );
    // The two it kept are the largest, and the fixture's smallest is gone. A
    // "top N" that took an arbitrary N would pass every assertion above.
    assert!(svg.contains(">Person<"), "the largest type: {svg}");
    assert!(
        !svg.contains(">City<"),
        "a small type survived the clip: {svg}"
    );

    // …and every name on the picture is drawn, so the picture owes no second
    // count. The two lines answer different questions and only one applies.
    assert_eq!(rendered.names_shown, None);
    assert_eq!(chip_count(&svg), 2);
    assert!(!svg.contains("names shown"));
}

/// The same graph on a canvas that fits it says nothing at all.
///
/// The pairing is the point: a tier line that appeared unconditionally would be
/// noise, and one that never appeared would be the silent clip.
#[test]
fn a_canvas_that_fits_the_schema_reports_no_tier() {
    let rendered = render_meta(&meta_request(Theme::Dark));
    let svg = String::from_utf8(rendered.bytes.clone()).expect("the emitter writes UTF-8");
    assert_eq!(rendered.types_total, None);
    assert_eq!(rendered.types_shown, None);
    assert_eq!(rendered.names_shown, None);
    assert_eq!(svg.matches("<circle").count(), 5, "every type is drawn");
    assert!(!svg.contains("types shown"));
    assert!(!svg.contains("names shown"));
}

/// No label lands inside the status block.
///
/// The layout keeps every *circle* below the block's strip, and that was not
/// enough: a node right under the block whose label the ring layout puts
/// *above* it drew its chip inside the panel, which is where round 3's 800x500
/// sodir render lost `WellboreCore 8,510`. The strip owes a label its reach.
///
/// **What this check does and does not cover.** It goes red on a reservation
/// that ignores the status block entirely — proven by removing it — but the
/// committed 5-type fixture never settles a node close enough under the panel
/// to exercise the `LABEL_REACH_ABOVE` term on its own; the evidence for that
/// term is the sodir meta-graph at 800x500, where round 4's first tiered render
/// still had a chip inside the panel and this one does not.
#[test]
fn no_label_is_drawn_inside_the_status_block() {
    for (width, height) in [(300, 250), (400, 250), (900, 600), (800, 500)] {
        let svg = render_to_string(&RenderRequest {
            width,
            height,
            ..meta_request(Theme::Dark)
        });
        let block = rects(&svg, 6.0);
        assert_eq!(block.len(), 1, "{width}x{height}: one status panel");
        let chips = rects(&svg, 5.0);
        assert!(!chips.is_empty(), "{width}x{height}: no chips to check");
        for chip in &chips {
            assert!(
                !overlaps(chip, &block[0]),
                "{width}x{height}: a label chip {chip:?} is under the status block {:?}",
                block[0]
            );
        }
    }
}

/// Every `<rect>` in the document with the given corner radius, as
/// `(x, y, x + width, y + height)`. The radius is what tells the three kinds
/// apart: 5 is a label chip, 6 the status panel, 22 an island boundary.
fn rects(svg: &str, radius: f64) -> Vec<(f64, f64, f64, f64)> {
    let number = |line: &str, key: &str| -> Option<f64> {
        line.split(&format!("{key}=\""))
            .nth(1)?
            .split('"')
            .next()?
            .parse()
            .ok()
    };
    svg.lines()
        .filter(|line| line.starts_with("<rect"))
        .filter(|line| number(line, "rx") == Some(radius))
        .filter_map(|line| {
            let (x, y) = (number(line, "x")?, number(line, "y")?);
            Some((
                x,
                y,
                x + number(line, "width")?,
                y + number(line, "height")?,
            ))
        })
        .collect()
}

fn overlaps(a: &(f64, f64, f64, f64), b: &(f64, f64, f64, f64)) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

fn render_meta(request: &RenderRequest) -> render::Rendered {
    let graph = fixture_graph();
    render::render(&graph, "meta.kgl", QueryConfig::default(), request)
        .expect("the fixture renders")
}

/// Label chips in an emitted document — the `rx="5"` rounded rect the label
/// group draws, which no other element in the picture uses.
fn chip_count(svg: &str) -> usize {
    svg.lines()
        .filter(|line| line.starts_with("<rect") && line.contains("rx=\"5\""))
        .count()
}

/// A render request that cannot produce a picture fails with a message naming
/// what to do about it, rather than emitting an empty canvas.
#[test]
fn a_type_this_graph_does_not_have_is_refused_not_drawn_empty() {
    let graph = fixture_graph();
    let request = RenderRequest {
        source: RenderSource::Expand(render::ExpandSource {
            node_type: "Nonexistent".to_string(),
            relationship: None,
            direction: Default::default(),
            limit: None,
        }),
        ..meta_request(Theme::Dark)
    };
    let err = render::render(&graph, "meta.kgl", QueryConfig::default(), &request)
        .expect_err("an unknown type must be refused");
    assert!(err.to_string().contains("Nonexistent"), "{err}");
}

/// A request that selects nothing is refused, not drawn empty.
///
/// The case that bought this: a golden generated from a query naming a
/// relationship the fixture does not have came back as a valid SVG of an empty
/// canvas — indistinguishable from a renderer that had broken. An agent handed
/// that image has no way to tell the two apart.
#[test]
fn a_request_that_selects_nothing_is_refused_rather_than_drawn_empty() {
    let graph = fixture_graph();
    let request = RenderRequest {
        source: RenderSource::Cypher(CypherRequest {
            query: "MATCH (p:Person) WHERE p.title = 'nobody-by-this-name' RETURN p".to_string(),
            params: Default::default(),
            limit: None,
            as_graph: true,
        }),
        ..meta_request(Theme::Dark)
    };
    let err = render::render(&graph, "meta.kgl", QueryConfig::default(), &request)
        .expect_err("an empty selection must be refused");
    assert!(err.to_string().contains("nothing to draw"), "{err}");
}

#[test]
fn a_canvas_outside_the_supported_range_is_refused() {
    let graph = fixture_graph();
    for (width, height) in [(10u32, 600u32), (900, 10), (99_999, 600)] {
        let request = RenderRequest {
            width,
            height,
            ..meta_request(Theme::Dark)
        };
        assert!(
            render::render(&graph, "meta.kgl", QueryConfig::default(), &request).is_err(),
            "{width}x{height} must be refused"
        );
    }
}
