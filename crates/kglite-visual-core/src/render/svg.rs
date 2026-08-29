//! The SVG emitter — the native output format (plan D13).
//!
//! **Self-contained by construction.** No external stylesheet, no web font, no
//! `<image href>`: an agent that hands the file to a human, a chat client that
//! inlines it, and `resvg` rasterising it all have to see the same picture, and
//! any of them may be offline. Text stays text, so the file is greppable and
//! scales, which is the reason SVG is the native format and PNG the derived one.
//!
//! **Every number is emitted at fixed precision.** The golden baseline is an
//! exact one (CLAUDE.md → "Gate honesty"), and a baseline that carried
//! `f64::to_string`'s full 17 significant digits would be pinning the last bit
//! of every accumulated sum — including the one place this pipeline touches
//! libm, `ln_1p` in the size ramp, which the IEEE standard does not require to
//! be correctly rounded. Two decimals is finer than a rendered pixel and
//! coarser than any disagreement a conforming libm can produce, so the baseline
//! describes the picture rather than the machine.

use super::encoding::{self, Palette, Theme};
use super::labels::{self, LabelSpec};
use super::layout::{LabelSide, Positions};
use super::Scene;

/// The font stack, mirroring `.kglv-label` in `frontend/src/styles.css`, with
/// concrete families appended.
///
/// The CSS stack ends at `sans-serif`, which a browser resolves and an SVG
/// rasteriser may not: `ui-sans-serif` and `system-ui` are CSS-only keywords,
/// so a renderer that knew nothing else would fall off the end of the list.
/// The named families are the ones actually present on the three platforms this
/// ships to, and `sans-serif` remains the terminal fallback.
const FONT_STACK: &str =
    "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif";

/// Mirrors `.kglv-status` in `frontend/src/styles.css`.
const MONO_STACK: &str = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace";

/// Label chip geometry, mirroring `.kglv-label` in `frontend/src/styles.css`:
/// `padding: 2px 7px`, `border-radius: 5px`, `font: 12px/1.4`, `gap: 6px`.
const CHIP_HEIGHT: f64 = 20.0;
const CHIP_RADIUS: f64 = 5.0;
const CHIP_PAD_X: f64 = 7.0;
const CHIP_GAP: f64 = 6.0;
const LABEL_FONT_PX: f64 = 12.0;
/// Baseline offset inside the chip, from `font: 12px/1.4` plus 2 px of padding.
const LABEL_BASELINE: f64 = 14.0;
/// Per-character advance for the chip's own *drawn* width.
///
/// **Deliberately not [`super::labels::estimate_width`]'s constants, and the two
/// have different jobs.** That estimate is the ported collision footprint, and
/// it must stay bit-for-bit the app's or the two sides resolve collisions
/// differently. These decide how wide the rounded rectangle behind the text is
/// drawn, and they are a little more generous because the app has no equivalent
/// decision to make: a browser lays the chip out with flexbox around whatever
/// the glyphs actually measure, and this emitter has no metrics at all. Tuned
/// against the case that exposed it — sodir's instance titles are bold
/// uppercase ("BALDER FM", "SHETLAND GP"), which at 12px/600 run past 6.9 px a
/// character and pushed the count chip on top of the name's last letters.
const NAME_ADVANCE: f64 = 7.7;
const COUNT_ADVANCE: f64 = 7.0;
const BADGE_WIDTH: f64 = 34.0;

/// Gap between a circle's edge and the top of its label.
///
/// Mirrors `y: y + source.radius(slot) + 4` in `LabelOverlay.update`
/// (`frontend/src/labels.ts`) — below the circle, never on it: a label centred
/// on its point hides the size that encodes the count.
const LABEL_GAP_PX: f64 = 4.0;

/// The status block, mirroring `.kglv-status`: `top: 12px; left: 12px;
/// padding: 8px 12px; font: 12px/1.6`.
const STATUS_X: f64 = 12.0;
const STATUS_Y: f64 = 12.0;
const STATUS_PAD_X: f64 = 12.0;
const STATUS_PAD_Y: f64 = 8.0;
const STATUS_LINE_PX: f64 = 19.2;
const STATUS_FONT_PX: f64 = 12.0;

/// Keep-out from the canvas edge, for a label chip the grid pushed outward.
const EDGE_PAD: f64 = 6.0;

/// Vertical strip the status block occupies, for a block of `lines` lines.
///
/// Exported to the layout rather than recomputed there: two copies of one
/// rectangle is two things that can disagree, and the way they would disagree
/// is a type name drawn under an opaque panel.
pub(crate) fn status_block_height(lines: usize) -> f64 {
    if lines == 0 {
        return 0.0;
    }
    STATUS_Y + lines as f64 * STATUS_LINE_PX + 2.0 * STATUS_PAD_Y + STATUS_Y
}

/// Emit the whole document.
pub(crate) fn emit(
    scene: &Scene,
    positions: &Positions,
    width: u32,
    height: u32,
    theme: Theme,
) -> String {
    let palette = theme.palette();
    let mut out = String::with_capacity(4_096 + scene.nodes.len() * 256);

    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\" font-family=\"{}\">\n",
        escape(FONT_STACK)
    ));
    out.push_str(&format!(
        "<title>{}</title>\n",
        escape(scene.status.first().map(String::as_str).unwrap_or("graph"))
    ));
    out.push_str(&format!(
        "<rect width=\"{width}\" height=\"{height}\" fill=\"{}\"/>\n",
        palette.background
    ));

    emit_islands(&mut out, scene, positions, &palette);
    emit_links(&mut out, scene, positions, &palette);
    emit_nodes(&mut out, scene, positions);
    emit_labels(&mut out, scene, positions, &palette, width, height);
    emit_status(&mut out, scene, &palette);

    out.push_str("</svg>\n");
    out
}

/// Corner radius of an island boundary, in pixels — soft enough that it reads
/// as a region and not as a table cell.
const ISLAND_CORNER_PX: f64 = 22.0;

/// A quiet boundary around each packed community, drawn behind everything.
///
/// **Position alone was not carrying it.** The island layout puts a community
/// in one place, but a reader scanning a hundred type names cannot tell "this
/// gap is a boundary" from "this gap is where the packing happened to leave
/// room" — which is exactly the coordinator's round-1 verdict on the
/// meta-graph: island-ness is not visible. A faint tint and a hairline say
/// where one group ends, using the two channels the picture is not already
/// spending on the data (nodes carry hue, links carry width).
///
/// **Quiet is a requirement, not a preference.** At any weight a reader would
/// call a box, the boundary becomes the loudest thing in the image and the
/// graph inside it becomes decoration. The tint is 0.045 alpha and the stroke
/// 0.16, which is visible on a scan and invisible on a glance at one node.
///
/// The tray of unattached singletons gets a dashed outline and no tint: it is
/// not a community, and a solid hull round it would claim the opposite of what
/// the partition found.
fn emit_islands(out: &mut String, scene: &Scene, positions: &Positions, palette: &Palette) {
    if positions.islands.is_empty() {
        return;
    }
    for island in &positions.islands {
        let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
        let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for member in &island.members {
            let (Some((x, y)), Some(node)) = (positions.xy.get(*member), scene.nodes.get(*member))
            else {
                continue;
            };
            min_x = min_x.min(x - node.radius);
            max_x = max_x.max(x + node.radius);
            min_y = min_y.min(y - node.radius);
            max_y = max_y.max(y + node.radius);
        }
        if !min_x.is_finite() || !min_y.is_finite() {
            continue;
        }
        let pad = super::layout::ISLAND_HULL_PAD_PX;
        let (x, y) = (min_x - pad, min_y - pad);
        let (w, h) = (max_x - min_x + 2.0 * pad, max_y - min_y + 2.0 * pad);
        if island.orphans {
            out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"none\" \
                 stroke=\"{}\" stroke-opacity=\"0.16\" stroke-width=\"1\" \
                 stroke-dasharray=\"4 6\"/>\n",
                num(x),
                num(y),
                num(w),
                num(h),
                num(ISLAND_CORNER_PX),
                palette.island
            ));
            continue;
        }
        out.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\" \
             fill-opacity=\"0.045\" stroke=\"{}\" stroke-opacity=\"0.16\" stroke-width=\"1\"/>\n",
            num(x),
            num(y),
            num(w),
            num(h),
            num(ISLAND_CORNER_PX),
            palette.island,
            palette.island
        ));
    }
}

/// Links, in two groups: the ones inside a community and the ones between two.
///
/// **Ink is a budget and the count is known before the first line is drawn.**
/// At the meta-graph's 124 links the full 0.45 stroke opacity is right; at the
/// discovery expansion's 3,374 the same value composites to a white wash with
/// the nodes floating on it, which is the whole of what was left wrong with
/// that image. `encoding::link_ink` spends the budget as `1/sqrt(n)`, so the
/// *total* perceived ink stays near constant instead of the per-line ink.
///
/// The split into two groups is the second half: a bridge between communities
/// is what made round 1's meta-graph read as a web, and it is drawn as
/// background so the lines that make a group read as a group are the ones a
/// reader sees first. Both are drawn — an edge the picture hides is an edge the
/// reader concludes is not there — and cross-island lines go down first so a
/// within-island line is never buried under one.
fn emit_links(out: &mut String, scene: &Scene, positions: &Positions, palette: &Palette) {
    if scene.links.is_empty() {
        return;
    }
    let ink = encoding::link_ink(scene.links.len());
    let width_ink = encoding::link_width_ink(scene.links.len());

    let mut island_of = vec![usize::MAX; scene.nodes.len()];
    for (index, island) in positions.islands.iter().enumerate() {
        for member in &island.members {
            if let Some(slot) = island_of.get_mut(*member) {
                *slot = index;
            }
        }
    }
    let crosses = |link: &super::SceneLink| -> bool {
        let (Some(a), Some(b)) = (island_of.get(link.source), island_of.get(link.target)) else {
            return false;
        };
        *a != usize::MAX && *b != usize::MAX && a != b
    };

    for (cross, opacity) in [
        (
            true,
            palette.link_opacity * ink * encoding::CROSS_ISLAND_INK,
        ),
        (false, palette.link_opacity * ink),
    ] {
        let mut group = String::new();
        for link in scene.links.iter().filter(|link| crosses(link) == cross) {
            let (Some(a), Some(b)) = (positions.xy.get(link.source), positions.xy.get(link.target))
            else {
                continue;
            };
            group.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-width=\"{}\"/>\n",
                num(a.0),
                num(a.1),
                num(b.0),
                num(b.1),
                num(link.width * width_ink)
            ));
        }
        if group.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "<g stroke=\"{}\" stroke-opacity=\"{}\" stroke-linecap=\"round\">\n",
            palette.link,
            num(opacity)
        ));
        out.push_str(&group);
        out.push_str("</g>\n");
    }
}

/// Circles, largest first.
///
/// Draw order is not part of the encoding — it is the one thing a 2D canvas has
/// and cosmos.gl's point pass does not expose — so it is chosen for legibility:
/// painting the big types first leaves the small ones visible on top of them
/// instead of swallowed. Ties break on slot, so the order stays a function of
/// the input.
fn emit_nodes(out: &mut String, scene: &Scene, positions: &Positions) {
    let mut order: Vec<usize> = (0..scene.nodes.len()).collect();
    order.sort_by(|a, b| {
        scene.nodes[*b]
            .radius
            .total_cmp(&scene.nodes[*a].radius)
            .then_with(|| scene.nodes[*a].slot.cmp(&scene.nodes[*b].slot))
    });

    out.push_str("<g>\n");
    for i in order {
        let node = &scene.nodes[i];
        let Some((x, y)) = positions.xy.get(i) else {
            continue;
        };
        if node.aggregate.is_some() {
            emit_wedge(out, scene, positions, i, *x, *y);
            continue;
        }
        let [r, g, b, a] = node.color;
        if node.emphasis {
            emit_halo(out, *x, *y, node.radius, rgb(r, g, b).as_str());
        }
        // A hairline of the node's OWN colour at a stronger opacity than its
        // fill. The supporting-type dim is an alpha (`SUPPORTING_ALPHA`), and an
        // alpha reads very differently against the two grounds: at 0.41 over
        // `#0d1117` a circle is still a circle, and over white it is a smudge
        // that a reader scanning for a type simply misses. The outline keeps
        // "this one recedes" while keeping it findable, and it is drawn in both
        // themes so the two images stay the same picture.
        out.push_str(&format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\" fill-opacity=\"{}\" \
             stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"1\"><title>{}</title></circle>\n",
            num(*x),
            num(*y),
            num(node.radius),
            rgb(r, g, b),
            num(a),
            rgb(r, g, b),
            num((a * 1.9).min(1.0)),
            escape(&node.text)
        ));
    }
    out.push_str("</g>\n");
}

/// The halo's outer radius and its ring, as multiples of the node's own.
const HALO_DISC: f64 = 2.6;
const HALO_RING: f64 = 1.7;

/// A soft disc and a ring behind the node a radial layout is centred on.
///
/// **The seed has to be findable in one glance** (P11 round 2). Before this it
/// was a six-pixel dot identical to every leaf on the ring around it, and the
/// only thing saying "this is the node you asked about" was that it happened to
/// be in the middle — which stops being a signal the moment the picture has two
/// clusters in it. Size alone is not enough either: an instance node's radius
/// is an encoding that says *nothing* about the graph, so inflating it without
/// a second mark would read as "this node is big".
///
/// Drawn in the node's own colour, so the emphasis says "here" rather than
/// introducing a colour the palette does not otherwise use.
fn emit_halo(out: &mut String, x: f64, y: f64, radius: f64, color: &str) {
    out.push_str(&format!(
        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{color}\" fill-opacity=\"0.12\"/>\n",
        num(x),
        num(y),
        num(radius * HALO_DISC)
    ));
    out.push_str(&format!(
        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{color}\" \
         stroke-opacity=\"0.55\" stroke-width=\"1.5\"/>\n",
        num(x),
        num(y),
        num(radius * HALO_RING)
    ));
}

/// Half-angle of the aggregate wedge, as a rotation matrix.
///
/// `cos(0.85)` and `sin(0.85)` written out as literals rather than computed:
/// this is the one place the emitter would otherwise call a trigonometric
/// function, and a decimal literal is bit-identical on every platform where a
/// libm is not. 0.85 rad opens the fan to ~97°, which reads as a fan and not as
/// a pie chart with a slice missing.
const WEDGE_COS: f64 = 0.659_983_145_884_982_2;
const WEDGE_SIN: f64 = 0.751_280_405_140_292_7;

/// Where the wedge's apex sits, as a fraction of its radius behind the node's
/// own point — so the glyph's *mass* lands where the layout put it while its
/// point still touches the link coming in.
const WEDGE_APEX_BACK: f64 = 0.55;

/// One folded fan, drawn as a wedge opening away from its parent.
///
/// **Shape is the honesty channel** (P11 direction (e)). This glyph stands for
/// nodes that are not in the picture, so it must not be mistakable for one that
/// is: it is a sector rather than a circle, it is dashed rather than solid, and
/// it is drawn in `AGGREGATE_COLOR` rather than in any type's hue. Its label
/// carries the exact count and the words "showing none"; none of the three
/// signals is load-bearing alone, and at thumbnail size the colour is the one
/// that survives.
///
/// The orientation comes from the single link the glyph has — a fan opens away
/// from the thing it hangs off, which is where a reader expects its members to
/// be. With no link (a fan whose parent was itself dropped, which the fold does
/// not produce today) it opens right, and the arbitrary choice is fixed rather
/// than derived so the document stays a function of the input.
fn emit_wedge(
    out: &mut String,
    scene: &Scene,
    positions: &Positions,
    index: usize,
    x: f64,
    y: f64,
) {
    let node = &scene.nodes[index];
    let anchor = scene
        .links
        .iter()
        .find_map(|link| match (link.source, link.target) {
            (source, target) if source == index => positions.xy.get(target),
            (source, target) if target == index => positions.xy.get(source),
            _ => None,
        });
    let (ux, uy) = match anchor {
        Some((ax, ay)) => {
            let (dx, dy) = (x - ax, y - ay);
            let length = (dx * dx + dy * dy).sqrt();
            if length <= 1e-9 {
                (1.0, 0.0)
            } else {
                (dx / length, dy / length)
            }
        }
        None => (1.0, 0.0),
    };
    let radius = node.radius;
    let (apex_x, apex_y) = (
        x - ux * radius * WEDGE_APEX_BACK,
        y - uy * radius * WEDGE_APEX_BACK,
    );
    let arc = radius * 1.6;
    // Rotate the axis by ±the half-angle. A 2x2 rotation, so no trigonometric
    // call is made at render time at all.
    let left = (
        ux * WEDGE_COS + uy * WEDGE_SIN,
        -ux * WEDGE_SIN + uy * WEDGE_COS,
    );
    let right = (
        ux * WEDGE_COS - uy * WEDGE_SIN,
        ux * WEDGE_SIN + uy * WEDGE_COS,
    );
    let [r, g, b, a] = node.color;
    out.push_str(&format!(
        "<path d=\"M {} {} L {} {} A {} {} 0 0 1 {} {} Z\" fill=\"{}\" fill-opacity=\"{}\" \
         stroke=\"{}\" stroke-opacity=\"{}\" stroke-width=\"1.5\" stroke-dasharray=\"5 3\">\
         <title>{}</title></path>\n",
        num(apex_x),
        num(apex_y),
        num(apex_x + arc * left.0),
        num(apex_y + arc * left.1),
        num(arc),
        num(arc),
        num(apex_x + arc * right.0),
        num(apex_y + arc * right.1),
        rgb(r, g, b),
        num(a * 0.45),
        rgb(r, g, b),
        num((a * 1.6).min(1.0)),
        escape(&node.text)
    ));
}

fn emit_labels(
    out: &mut String,
    scene: &Scene,
    positions: &Positions,
    palette: &Palette,
    width: u32,
    height: u32,
) {
    let mut degree = vec![0u32; scene.nodes.len()];
    for link in &scene.links {
        for end in [link.source, link.target] {
            if let Some(slot) = degree.get_mut(end) {
                *slot += 1;
            }
        }
    }
    let specs: Vec<LabelSpec> = scene
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, node)| {
            let (x, y) = positions.xy.get(i)?;
            let mut spec = LabelSpec {
                slot: node.slot,
                text: node.text.clone(),
                badges: node.badges.clone(),
                weight: node.weight,
                degree: degree.get(i).copied().unwrap_or(0),
                show_count: node.show_count,
                dimmed: node.dimmed,
                // An aggregate's label is the only thing on the picture saying
                // how many nodes are behind the glyph, so neither it nor the
                // node it hangs off is ever thinned.
                pinned: node.pinned || node.aggregate.is_some(),
                x: *x,
                y: y + node.radius + LABEL_GAP_PX,
            };
            // Outward from a ring's centre where the layout asked for it. A
            // side chip is centred on the node's own row rather than dropped
            // below it, so the name sits at the end of the spoke it belongs to
            // and a reader traces one line instead of guessing between two; at
            // the top of a ring outward is *up*, which is the direction with the
            // room in it. See `layout::LabelSide`.
            let side = positions
                .label_side
                .get(i)
                .copied()
                .unwrap_or(LabelSide::Below);
            match side {
                LabelSide::Below => {}
                LabelSide::Above => spec.y = y - node.radius - LABEL_GAP_PX - CHIP_HEIGHT,
                LabelSide::Left | LabelSide::Right => {
                    let reach = node.radius + LABEL_GAP_PX + draw_width(&spec) / 2.0;
                    spec.x = if side == LabelSide::Left {
                        x - reach
                    } else {
                        x + reach
                    };
                    spec.y = y - CHIP_HEIGHT / 2.0;
                }
            }
            Some(spec)
        })
        .collect();
    // The app offers only what cosmos.gl's point sampler returned (except on
    // the meta-graph, where `everyPoint` bypasses it). Here there is no
    // sampler — nothing is off screen and nothing is thinned by a GPU pass — so
    // the collision grid is the only thinning, which is the behaviour the app
    // has on the view that matters most and a denser one on an instance slice.
    let placed = labels::choose(
        &specs,
        scene.place_all_labels,
        labels::budget(width, height),
    );
    if placed.is_empty() {
        return;
    }

    let by_slot: std::collections::HashMap<u32, &LabelSpec> =
        specs.iter().map(|s| (s.slot, s)).collect();

    out.push_str(&format!("<g font-size=\"{}\">\n", num(LABEL_FONT_PX)));
    for label in &placed {
        let Some(spec) = by_slot.get(&label.slot) else {
            continue;
        };
        let chip_width = draw_width(spec);
        // Clamped into the canvas. The layout already keeps every *circle*
        // inside, but the collision grid nudges a chip up to two rows and a
        // long name is wider than the node it names, so the last few pixels are
        // this emitter's to defend. The app's overlay solves the same problem
        // with `overflow: hidden`, which is the one answer an image cannot use:
        // a clipped name is a name nobody can read, and there is no scrolling
        // out from under it.
        let left = (label.x - chip_width / 2.0).clamp(
            EDGE_PAD,
            (f64::from(width) - chip_width - EDGE_PAD).max(EDGE_PAD),
        );
        let top = label.y.clamp(
            EDGE_PAD,
            (f64::from(height) - CHIP_HEIGHT - EDGE_PAD).max(EDGE_PAD),
        );
        let (text_color, name_weight) = if spec.dimmed {
            (palette.label_dim_text, "500")
        } else {
            (palette.label_text, "600")
        };
        let (fill_opacity, stroke_opacity) = if spec.dimmed {
            // Mirrors `.kglv-label-dim`: quieter chip, quieter border.
            (0.55, 0.18)
        } else {
            (palette.chip_fill_opacity, palette.chip_stroke_opacity)
        };

        out.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" \
             fill=\"{}\" fill-opacity=\"{}\" stroke=\"{}\" stroke-opacity=\"{}\"/>\n",
            num(left),
            num(top),
            num(chip_width),
            num(CHIP_HEIGHT),
            num(CHIP_RADIUS),
            palette.chip_fill,
            num(fill_opacity),
            palette.chip_stroke,
            num(stroke_opacity)
        ));

        let baseline = top + LABEL_BASELINE;
        // Name and count in ONE `<text>`, the count as a `<tspan>` with a `dx`
        // and no `x` of its own — so the renderer advances the pen by what the
        // glyphs actually measured and the two can never land on top of each
        // other. Placing the count at an estimated offset is what put "1" over
        // the "FM" of "BALDER FM" on the first sodir neighbourhood render.
        let count = if spec.show_count {
            format!(
                "<tspan fill=\"{}\" font-weight=\"400\" dx=\"{}\">{}</tspan>",
                palette.label_count,
                num(CHIP_GAP),
                escape(&encoding::group_thousands(spec.weight))
            )
        } else {
            // No count where there is no count. Every instance node in the
            // graph carries the number 1, and the P9 renders printed it beside
            // three hundred names.
            String::new()
        };
        out.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-weight=\"{}\">{}{}</text>\n",
            num(left + CHIP_PAD_X),
            num(baseline),
            text_color,
            name_weight,
            escape(&spec.text),
            count
        ));

        // Badges are rectangles, so they cannot ride the text flow; they are
        // right-aligned inside the chip instead, which is where the app's
        // flexbox puts them and needs no guess about where the count ended.
        let mut cursor =
            left + chip_width - CHIP_PAD_X - spec.badges.len() as f64 * BADGE_WIDTH + CHIP_GAP;
        for badge in &spec.badges {
            // Mirrors `.kglv-badge`: a pill, uppercase, 10px/700.
            out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"14\" rx=\"7\" fill=\"{}\">\
                 <title>{}</title></rect>\n",
                num(cursor),
                num(top + 3.0),
                num(BADGE_WIDTH - CHIP_GAP),
                encoding::badge_color(badge),
                escape(encoding::badge_title(badge))
            ));
            out.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" fill=\"#ffffff\" font-size=\"10\" font-weight=\"700\" \
                 letter-spacing=\"0.4\">{}</text>\n",
                num(cursor + 5.0),
                num(top + 13.0),
                escape(&badge.to_uppercase())
            ));
            cursor += BADGE_WIDTH;
        }
    }
    out.push_str("</g>\n");
}

/// The status block and the truncation banners, top-left, mirroring
/// `.kglv-status` and `.kglv-warn`.
///
/// **D5's banner is drawn INTO the image and not beside it.** An image travels
/// without its response; a truncated render that only reported the fact in a
/// JSON field would be a complete-looking picture the moment anyone pasted it
/// anywhere.
fn emit_status(out: &mut String, scene: &Scene, palette: &Palette) {
    let lines: Vec<(&str, &str)> = scene
        .status
        .iter()
        .map(|line| (line.as_str(), palette.status_text))
        .chain(
            scene
                .banners
                .iter()
                .map(|line| (line.as_str(), palette.warn)),
        )
        .collect();
    if lines.is_empty() {
        return;
    }
    let longest = lines
        .iter()
        .map(|(line, _)| line.chars().count())
        .max()
        .unwrap_or(0);
    // A monospace advance at 12px; the block only has to be wide enough not to
    // clip, so an estimate is what it needs (there is no text metric here, by
    // the same argument the label widths make).
    let box_width = longest as f64 * 7.25 + 2.0 * STATUS_PAD_X;
    let box_height = lines.len() as f64 * STATUS_LINE_PX + 2.0 * STATUS_PAD_Y;

    out.push_str(&format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"6\" fill=\"{}\" \
         fill-opacity=\"0.85\" stroke=\"{}\" stroke-opacity=\"0.30\"/>\n",
        num(STATUS_X),
        num(STATUS_Y),
        num(box_width),
        num(box_height),
        palette.status_fill,
        palette.status_stroke
    ));
    out.push_str(&format!(
        "<g font-family=\"{}\" font-size=\"{}\">\n",
        escape(MONO_STACK),
        num(STATUS_FONT_PX)
    ));
    for (i, (line, color)) in lines.iter().enumerate() {
        out.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>\n",
            num(STATUS_X + STATUS_PAD_X),
            num(STATUS_Y + STATUS_PAD_Y + (i as f64 + 0.75) * STATUS_LINE_PX),
            color,
            escape(line)
        ));
    }
    out.push_str("</g>\n");
}

/// The chip's drawn width — see [`NAME_ADVANCE`] for why this is not
/// [`super::labels::estimate_width`].
fn draw_width(spec: &LabelSpec) -> f64 {
    let count = if spec.show_count {
        CHIP_GAP + encoding::group_thousands(spec.weight).chars().count() as f64 * COUNT_ADVANCE
    } else {
        0.0
    };
    CHIP_PAD_X * 2.0
        + spec.text.chars().count() as f64 * NAME_ADVANCE
        + count
        + spec.badges.len() as f64 * BADGE_WIDTH
}

/// One number, at the fixed precision the module doc argues for.
fn num(value: f64) -> String {
    if !value.is_finite() {
        // Not reachable from a bounded layout, but an emitter that wrote `NaN`
        // into an attribute would produce a document no renderer will open, and
        // the failure would surface as "the image is blank".
        return "0".to_string();
    }
    let text = format!("{value:.2}");
    // `-0.00` and `0.00` are the same point; emitting both would make the
    // baseline depend on which side of zero a sum happened to land.
    let text = if text == "-0.00" {
        "0.00".to_string()
    } else {
        text
    };
    // Trailing zeros carry no information and make the document a third larger.
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// An 0..1 RGBA triple as the `#rrggbb` an SVG attribute takes. Alpha rides
/// separately in `fill-opacity`, which is how SVG 1.1 spells it.
fn rgb(r: f64, g: f64, b: f64) -> String {
    let channel = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32).min(255);
    format!("#{:02x}{:02x}{:02x}", channel(r), channel(g), channel(b))
}

/// XML text and attribute escaping.
///
/// All five predefined entities, including the two that only matter inside
/// attributes: a type name carrying a quote is not hypothetical on a graph
/// built from someone else's data, and an emitter that produced malformed XML
/// would fail as a blank image rather than as an error.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // A control character is not valid XML 1.0 at all, escaped or not.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_emitted_at_fixed_precision() {
        assert_eq!(num(1.0 / 3.0), "0.33");
        assert_eq!(num(-0.001), "0");
        assert_eq!(num(12.0), "12");
        assert_eq!(num(12.5), "12.5");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(f64::INFINITY), "0");
    }

    #[test]
    fn every_xml_metacharacter_is_escaped() {
        assert_eq!(
            escape("a & b < c > d \" e ' f"),
            "a &amp; b &lt; c &gt; d &quot; e &apos; f"
        );
        assert_eq!(escape("tab\there\u{0}"), "tab\there ");
    }

    #[test]
    fn colours_round_to_the_hex_an_attribute_takes() {
        assert_eq!(rgb(0.35, 0.65, 0.98), "#59a6fa");
        assert_eq!(rgb(0.0, 0.0, 0.0), "#000000");
        assert_eq!(rgb(1.0, 1.0, 1.0), "#ffffff");
        assert_eq!(rgb(2.0, -1.0, 0.5), "#ff0080", "out-of-range clamps");
    }
}
