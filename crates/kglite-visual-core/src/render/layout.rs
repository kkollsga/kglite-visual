//! Seeded, convergent, deterministic Fruchterman–Reingold — the export layout
//! (plan D13).
//!
//! **This does not reopen P4's stop rule.** What P4 retired was a
//! GPU-competitive *interactive* layout: the browser's simulation holds 60 fps
//! at the response bound, so a Rust one would have been a second answer to a
//! solved question. This is the parking lot's explicitly anticipated export
//! case — there is no GPU in the loop, there is no frame budget, and the
//! requirement is the opposite one: the same input must produce the same bytes
//! forever, so a golden SVG can be an exact baseline.
//!
//! **Hand-rolled, after reading three candidate crates** (2026-08-29):
//!
//! - `forceatlas2` 0.8.0 is **AGPL-3.0-only**. This workspace is MIT and ships
//!   a wheel; that is the same class of licence foot-gun the `@cosmograph/*`
//!   ban exists for, and it is decided by the manifest, not by the code.
//! - `fdg-sim` 0.9.1 (MIT) seeds its initial placement from
//!   `quad_rand::RandomRange`, a **process-global** RNG its API exposes no seed
//!   for — two renders in one process would disagree, which is the one property
//!   this module exists to have. It also carries `glam` 0.21, whose `Vec3` is
//!   SIMD-backed on some targets and scalar on others; glam makes no
//!   cross-target bit-identity promise.
//! - `graph-explorer-layout` 0.7.0 declares `rust-version = 1.90`, above this
//!   workspace's 1.88 MSRV (which is kglite's).
//!
//! So: ~120 lines here, with the determinism discipline the P2 lattice
//! established.
//!
//! **No libm in anything a golden test hashes.** The arithmetic below is
//! `+ - * /` and `sqrt`. `sqrt` is the one "transcendental-looking" operation
//! IEEE-754 requires to be correctly rounded, so it is exactly reproducible
//! across platforms; `sin`, `cos`, `exp`, `ln` and `powf` are the platform's
//! libm and are not, which is why the initial placement is an integer lattice
//! with an integer-derived jitter rather than the obvious golden-angle spiral.
//! Accumulation runs in index order, never over a hash map, because float
//! addition is not associative and an iteration order that varies is a sum that
//! varies.
//!
//! **Bounded input only.** [`run`] refuses more than [`MAX_LAYOUT_NODES`]
//! nodes rather than laying them out slowly. The choke point stays
//! `expand::effective_bound`; this is the assertion that nothing reached the
//! renderer around it.

use crate::error::CoreError;
use crate::expand::MAX_EXPANSION_NODES;

use super::structure;

/// Hard ceiling on nodes one layout may place — the D5 node bound, named from
/// it rather than copied, so moving the bound moves this with it.
pub const MAX_LAYOUT_NODES: usize = MAX_EXPANSION_NODES;

/// Iterations at or below [`EXACT_PAIRS_BELOW`] nodes.
///
/// Convergence, not a budget: at these sizes the layout stops moving well
/// before this and the extra passes cost microseconds. Fixed rather than
/// adaptive because "same input, same bytes" has to include "same number of
/// passes" — a convergence test on a float threshold is a place two machines
/// can stop at different iterations.
const ITERATIONS: u32 = 600;

/// Above this node count the repulsion pass uses the cutoff grid rather than
/// every pair, and the iteration count drops to [`ITERATIONS_LARGE`].
///
/// A meta-graph never reaches this (the tier bounds cap it at 200 types, 50 at
/// the top-types tier); a query or expansion slice at the D5 bound does. All
/// pairs at 5 000 nodes is 12.5M distance computations per pass, which is
/// seconds per image and not what an agent asked for.
const EXACT_PAIRS_BELOW: usize = 700;
const ITERATIONS_LARGE: u32 = 260;

/// Repulsion beyond this many ideal-distances is zero.
///
/// **One force law at every size.** This started out as the grid pass's cell
/// size and nothing else, with the small-graph path repelling every pair at any
/// distance — and that asymmetry was a bug with a picture attached. Unbounded
/// far-field repulsion makes a cloud's natural radius grow with the *node
/// count*, so on the 98-type sodir meta-graph the isolated types settled
/// thousands of pixels out, the fit step scaled the whole layout down to bring
/// them back, and the connected core arrived as a knot of overlapping labels.
/// With the cutoff applied everywhere, a node is pushed only by its neighbours
/// and held only by gravity, and the cloud settles at a density instead of at a
/// size.
///
/// Three `k` is where FR's `k²/d` term has fallen to a ninth of its value at
/// `k`; past that the gravity term is what holds the picture together, which is
/// the same division of labour the app's force config makes
/// (`simulationRepulsion` vs `simulationGravity` in `frontend/src/render.ts`).
const CUTOFF_K: f64 = 3.0;

/// Pull toward the origin, per unit of distance from it.
///
/// Without it a cutoff repulsion lets disconnected components — and a real
/// meta-graph has them; sodir's has isolated types with no edges at all — drift
/// apart forever, because past the cutoff nothing acts on them.
///
/// The number is the app's own `simulationGravity` (`FORCE_CONFIG` in
/// `frontend/src/render.ts`), which was tuned against this same graph: at 0.05
/// the isolated types drifted off the visible space, at 0.25 the whole layout
/// compressed back toward uniform.
const GRAVITY: f64 = 0.12;

/// Repulsion and attraction scales. `REPULSION_SCALE` mirrors
/// `simulationRepulsion: 4` in `frontend/src/render.ts` — a large departure
/// from its engine's default, bought by iterating on exactly the graph this
/// module was iterated on.
///
/// **`ATTRACTION_SCALE` no longer mirrors `simulationLinkSpring: 0.15`, and
/// cannot** (P11): the two sides now use different attraction *laws*, so a
/// shared number would be a coincidence rather than a parity. cosmos.gl's link
/// spring is Hookean; [`attract`] here is LinLog's saturating pull, and a
/// spring constant and a saturation scale are not the same quantity. What the
/// two still share is the encoding — radius, colour, link width — which is what
/// `super::encoding` exists to hold, and geometry was never claimed
/// (`session::GEOMETRY_CAVEAT`).
///
/// Textbook Fruchterman–Reingold weights the two equally, and on a meta-graph
/// that is wrong in a specific, visible way: a hub type carries twenty-odd
/// relationships, so it accumulates twenty attraction terms against ninety-odd
/// repulsion terms that are mostly far away and therefore weak. The connected
/// core collapses into a knot while the isolated types keep the whole frame —
/// which is precisely the first sodir render this module produced, and the same
/// failure P7 fixed on the GPU side by moving these two numbers.
///
/// **Restated for the LinLog attraction P11 moved to** (see [`attract`]). With
/// FR's `d²/k` the equilibrium was `d = k * (REPULSION/ATTRACTION)^(1/3)`; with
/// a saturating attraction it is where `REPULSION * k²/d` meets
/// `ATTRACTION * k * d/(d+k)`, and `ATTRACTION` is set so that crossing still
/// lands near three ideal distances — the room a 130 px label chip needs, and
/// the number the constants below were tuned to on this graph. A weaker
/// attraction with the same repulsion would not be "LinLog"; it would be a
/// layout that flew apart.
const REPULSION_SCALE: f64 = 4.0;
const ATTRACTION_SCALE: f64 = 1.8;

/// Extra clearance around a node, in pixels, before the soft-collision term
/// fires.
///
/// **A label's footprint, not a circle's.** The thing that has to not overlap
/// on a meta-graph is the *name*, and a name is a 130 x 30 px chip
/// (`labels::CELL_WIDTH`); two circles 20 px apart are perfectly legible and
/// their labels are unreadable. Two thirds of a cell, because the chip is
/// centred under its node and the label grid's own nudging covers the rest.
const CLEARANCE_PX: f64 = 110.0;

/// Stiffness of the soft-collision term, relative to the FR repulsion at `k`.
///
/// Plain `k²/d` cannot separate two large circles: it is a function of centre
/// distance and knows nothing about radius, so on a meta-graph whose radii span
/// 6–36 px the biggest types settle inside each other. This term is what makes
/// the layout size-aware.
const OVERLAP_STIFFNESS: f64 = 6.0;

/// Starting temperature, as a fraction of the shorter canvas side. The classic
/// FR schedule cools linearly to zero, which is what makes a fixed iteration
/// count a *converged* layout rather than a truncated one.
const INITIAL_TEMPERATURE: f64 = 0.10;

/// Ideal-distance scale. `k = IDEAL_SPACING * sqrt(area / n)` is FR's own
/// formula; the multiplier is this project's, chosen so the settled extent
/// lands near the canvas and the fit step is a nudge rather than a rescale.
const IDEAL_SPACING: f64 = 0.78;

/// One node, as the layout sees it: a position to solve for and a radius that
/// decides how much room it needs.
#[derive(Debug, Clone, Copy)]
pub struct LayoutNode {
    /// Drawn radius in pixels — [`super::encoding::type_radius`]'s output.
    pub radius: f64,
}

/// The rectangle a layout has to land inside.
///
/// One struct rather than three positional `f64`s: the three travel together
/// through every entry point here, and `reserved_top` in particular is a number
/// only `super::svg` can compute — a caller that passed the three separately
/// eventually passes them in the wrong order, and the failure is a picture
/// drawn underneath its own status block.
#[derive(Debug, Clone, Copy)]
pub struct Canvas {
    pub width: f64,
    pub height: f64,
    /// Vertical strip the status block occupies, which nothing may be laid out
    /// under. `super::svg::status_block_height` is the only honest source.
    pub reserved_top: f64,
}

/// Which side of its node a label belongs on.
///
/// **A layout fact, not an emitter one** (P11 round 2). On a hop ring every
/// label under its node points at the ring's centre, so the left half's names
/// are drawn *across* the spokes they belong to and a reader traces the wrong
/// line. Placing them outward — left of a node on the left, right of one on the
/// right — makes the chip radiate with the branch it names. Only the layout
/// knows where the centre is, so only the layout can say.
///
/// **Outward has four directions, not two** (P11 round 3). Round 2 read only the
/// sign of `x`, so a node at the very top of a ring — where its neighbours are
/// beside it and the free space is above it — was given a chip pointing *along*
/// the ring, straight into the two names next to it. The collision grid then
/// dropped whichever lost, which is where six of the wellbore and Troll renders'
/// missing labels went. At the top and bottom of an ellipse the outward
/// direction is vertical, and the rule is the same rule: the chip radiates with
/// the branch it names. See [`outward`] for where the sectors are cut.
///
/// **No frontend counterpart, and that is not a parity gap.** The app's overlay
/// follows a cosmos.gl force simulation, which has no ring and therefore no
/// outward; `LabelOverlay.update` places every chip below its point because
/// below is the only direction that means the same thing everywhere in *that*
/// picture. What the two sides still share is the encoding — the chip, the
/// count, the badges, the collision grid — which is what `super::encoding` and
/// `super::labels` hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelSide {
    /// Under the circle, the app's own placement.
    #[default]
    Below,
    Left,
    Right,
    /// Over the circle — the outward direction at the top of a ring.
    Above,
}

/// Where the layout put everything, in pixels, already fitted to the canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct Positions {
    pub xy: Vec<(f64, f64)>,
    /// Per node, parallel to `xy`. See [`LabelSide`].
    pub label_side: Vec<LabelSide>,
    /// The packed communities, in packing order — empty for every layout that
    /// did not pack any. See [`Island`].
    pub islands: Vec<Island>,
}

impl Positions {
    /// Positions with every label under its node and no islands — what a layout
    /// with no centre to radiate from and no grouping to draw produces.
    fn below(xy: Vec<(f64, f64)>) -> Self {
        let label_side = vec![LabelSide::Below; xy.len()];
        Self {
            xy,
            label_side,
            islands: Vec::new(),
        }
    }
}

/// Horizontal room a side-placed label needs, in final canvas pixels.
///
/// Reserved out of the *fitted* box rather than added to the pre-fit extent,
/// because [`fit`] scales positions and a pre-fit allowance would arrive scaled
/// down by exactly the factor that made room for it. Sized at the median sodir
/// instance title ("Statoil Petroleum AS" is longer, "1/3-4" much shorter); the
/// emitter still clamps a chip that overruns, which is the backstop.
const SIDE_LABEL_ROOM_PX: f64 = 100.0;

/// Keep-out between the fitted picture and the canvas edge.
const MARGIN_PX: f64 = 26.0;

/// Lay `nodes` out under `links`, into a `width` x `height` canvas.
///
/// `links` are index pairs into `nodes`; an out-of-range index is skipped
/// rather than panicking, because the caller assembles them from a slot space
/// and a dropped node there must not take the image down with it.
///
/// `seed` reaches only the initial placement's jitter — the force pass has no
/// randomness at all — so two seeds are two starting points of the same
/// deterministic process, not two samples of a stochastic one.
pub fn run(
    nodes: &[LayoutNode],
    links: &[(usize, usize)],
    canvas: Canvas,
    seed: u64,
) -> Result<Positions, CoreError> {
    let Canvas {
        width,
        height,
        reserved_top,
    } = canvas;
    if nodes.len() > MAX_LAYOUT_NODES {
        // The structural half of D5. `expand::effective_bound` is the choke
        // point every slice goes through; this is the assertion that nothing
        // reached the renderer around it. Refusing beats laying out slowly:
        // an unbounded layout is O(n) passes over an unbounded set, and the
        // image it eventually produced would be a hairball nobody could read.
        return Err(over_bound(nodes.len()));
    }
    let count = nodes.len();
    if count == 0 {
        return Ok(Positions::below(Vec::new()));
    }
    if count == 1 {
        return Ok(Positions::below(vec![(
            width / 2.0,
            (height + reserved_top) / 2.0,
        )]));
    }

    let area = width * height;
    let k = IDEAL_SPACING * (area / count as f64).sqrt();
    let mut xy = seed_positions(count, k, seed);

    let iterations = if count <= EXACT_PAIRS_BELOW {
        ITERATIONS
    } else {
        ITERATIONS_LARGE
    };
    let temperature_0 = INITIAL_TEMPERATURE * width.min(height);
    // Gravity is stronger along the shorter axis, in exactly the canvas's
    // proportion. An isotropic pull settles a circular cloud, and a circular
    // cloud fitted into a 16:10 rectangle wastes the two ends — some 30% of the
    // area, which at meta-graph density is the difference between labels that
    // clear each other and labels that do not. This is the one place the canvas
    // shape reaches the force pass, and it reaches it as a ratio, so the layout
    // stays a function of the request rather than of a viewport.
    let aspect = width / height;
    let (gravity_x, gravity_y) = if aspect >= 1.0 {
        (GRAVITY, GRAVITY * aspect)
    } else {
        (GRAVITY / aspect, GRAVITY)
    };
    let mut disp = vec![(0.0f64, 0.0f64); count];

    for step in 0..iterations {
        for d in disp.iter_mut() {
            *d = (0.0, 0.0);
        }
        if count <= EXACT_PAIRS_BELOW {
            repel_all_pairs(&xy, nodes, k, &mut disp);
        } else {
            repel_within_cutoff(&xy, nodes, k, &mut disp);
        }
        attract(&xy, links, k, count, &mut disp);
        for (i, (x, y)) in xy.iter().enumerate() {
            disp[i].0 -= x * gravity_x;
            disp[i].1 -= y * gravity_y;
        }

        // Linear cooling to zero. The last passes move nothing, which is what
        // "convergent" means here and why a fixed count is honest.
        let temperature = temperature_0 * (1.0 - f64::from(step) / f64::from(iterations));
        for (position, delta) in xy.iter_mut().zip(disp.iter()) {
            let magnitude = (delta.0 * delta.0 + delta.1 * delta.1).sqrt();
            if magnitude <= f64::EPSILON {
                continue;
            }
            let capped = magnitude.min(temperature) / magnitude;
            position.0 += delta.0 * capped;
            position.1 += delta.1 * capped;
        }
    }

    Ok(Positions::below(
        fit(&xy, nodes, width, height, reserved_top, 0.0).0,
    ))
}

/// Slack added on top of "the circles touch", in pixels, when [`separate`]
/// pushes an interpenetrating pair apart.
///
/// Touching circles read as one blob at thumbnail size, and a pass that stopped
/// at exact contact would also leave the invariant sitting on the edge of a
/// float comparison.
const SEPARATION_SLACK_PX: f64 = 1.5;

/// Passes [`separate`] makes before it accepts what it has.
///
/// Gauss–Seidel over a neighbour grid converges quickly on a layout that is
/// nearly right and never converges at all on a canvas with less area than its
/// circles — a fixed count is the only honest stopping rule for both, and the
/// loop exits early the moment a pass moves nothing.
const SEPARATION_PASSES: usize = 48;

/// Push interpenetrating circles apart, in **final canvas pixels**.
///
/// **The invariant [`fit`] cannot hold on its own** (P11 round 3). `fit` scales
/// positions and deliberately not radii — a radius is an encoding, and circles
/// that shrank with the layout would stop meaning what the app's circles mean.
/// The consequence is structural rather than incidental: any layout whose
/// settled extent exceeds the canvas arrives scaled by `s < 1`, every gap the
/// kernel computed arrives multiplied by `s`, and a gap that was exactly the two
/// radii arrives smaller than them. Round 2 mitigated that by making the island
/// packing aim lower, which is a constant standing in for a guarantee: it moved
/// the threshold and did not move the failure.
///
/// So the guarantee is asserted where it is meant: after the fit, on the
/// coordinates that are about to be drawn. Each overlapping pair is pushed apart
/// along its own axis — half the shortfall each, plus
/// [`SEPARATION_SLACK_PX`] — and everything is clamped back inside the canvas at
/// the end of every pass.
///
/// **Deterministic**: pairs are found through the same sorted-`Vec` neighbour
/// grid [`repel_within_cutoff`] uses, visited in index order, and the update is
/// in place, so the result is a pure function of the input. `sqrt` is the only
/// non-arithmetic operation. A coincident pair is separated along an
/// index-derived axis rather than an arbitrary one.
///
/// **Structure-preserving by construction**: a pair that already clears is never
/// touched, so a ring whose slots were computed correctly, or a force layout
/// that converged with room to spare, comes through this untouched. What moves
/// is what was overlapping.
///
/// **What it cannot do**: a canvas with less area than its circles have no
/// arrangement without overlap, and this does not invent one — it spreads the
/// remaining overlap instead of stacking it. The picture's honesty about that
/// case is the label budget and the fold, not this.
pub fn separate(xy: &mut [(f64, f64)], nodes: &[LayoutNode], canvas: Canvas) {
    let count = xy.len().min(nodes.len());
    if count < 2 {
        return;
    }
    let widest = nodes[..count]
        .iter()
        .map(|n| n.radius)
        .fold(0.0f64, f64::max);
    // Two of the widest radii plus the slack is the longest push this pass can
    // ever demand, so a pair that overlaps is always inside the 3x3 neighbourhood.
    let cell = (2.0 * widest + SEPARATION_SLACK_PX).max(1.0);
    let floor_y = canvas.reserved_top;
    let mut cells: Vec<((i64, i64), usize)> = Vec::with_capacity(count);

    for _ in 0..SEPARATION_PASSES {
        cells.clear();
        cells.extend((0..count).map(|i| ((cell_of(xy[i].0, cell), cell_of(xy[i].1, cell)), i)));
        // Stable by construction: the key carries the node index, which is unique.
        cells.sort_unstable();
        let mut moved = false;
        for i in 0..count {
            let key_x = cell_of(xy[i].0, cell);
            let key_y = cell_of(xy[i].1, cell);
            for dx in -1..=1i64 {
                for dy in -1..=1i64 {
                    let key = (key_x + dx, key_y + dy);
                    let start = cells.partition_point(|(c, _)| *c < key);
                    for (c, j) in cells[start..].iter() {
                        if *c != key {
                            break;
                        }
                        let j = *j;
                        // Each pair once, and only forward, so the visit order
                        // is the index order and not the grid's.
                        if j <= i {
                            continue;
                        }
                        let want = nodes[i].radius + nodes[j].radius + SEPARATION_SLACK_PX;
                        let (mut ux, mut uy) = (xy[j].0 - xy[i].0, xy[j].1 - xy[i].1);
                        let distance = (ux * ux + uy * uy).sqrt();
                        if distance >= want {
                            continue;
                        }
                        if distance <= 1e-9 {
                            let axis = ((i + j) % 2) as f64;
                            (ux, uy) = (1.0 - axis, axis);
                        } else {
                            (ux, uy) = (ux / distance, uy / distance);
                        }
                        let push = (want - distance) / 2.0;
                        xy[i].0 -= ux * push;
                        xy[i].1 -= uy * push;
                        xy[j].0 += ux * push;
                        xy[j].1 += uy * push;
                        moved = true;
                    }
                }
            }
        }
        for i in 0..count {
            let radius = nodes[i].radius;
            xy[i].0 = xy[i].0.clamp(radius, (canvas.width - radius).max(radius));
            xy[i].1 = xy[i].1.clamp(
                floor_y + radius,
                (canvas.height - radius).max(floor_y + radius),
            );
        }
        if !moved {
            break;
        }
    }
}

/// Clearance between two neighbours sitting side by side on a hop ring.
///
/// Smaller than [`CLEARANCE_PX`], which is a *label* footprint: on a ring the
/// circles are what must not touch, and demanding a whole label cell per node
/// would push a 70-node ring to a radius no canvas has. The label grid thins
/// what it cannot fit, and on a ring it has an easy job — the candidates are
/// already spread along an arc instead of piled in a blob.
const RING_SLOT_PAD_PX: f64 = 14.0;

/// Gap between one hop ring and the next.
const RING_GAP_PX: f64 = 96.0;

/// Extra arc inserted between two same-type runs on a ring, as a multiple of
/// one slot.
///
/// **This is the whole point of the ring** (P11 direction (a)): without it the
/// ring is a uniform necklace and "the branches" the user could not see stay
/// invisible. With it, each type occupies a contiguous arc with visible space
/// on either side, so a reader counts branches before reading a single label.
const GROUP_GAP_SLOTS: f64 = 1.6;

/// Radius of the innermost ring, before occupancy widens it.
const FIRST_RING_PX: f64 = 96.0;

/// Gap between two concentric sub-rings of the same hop.
///
/// Added to twice the widest circle on the ring, so the sub-rings clear each
/// other whatever the nodes are sized at.
const SUBRING_GAP_PX: f64 = 22.0;

/// Most sub-rings one hop may be split across.
///
/// A hop drawn as eight nested circles has stopped being a ring diagram and
/// become a dartboard; past this the ring simply grows, and the picture is
/// honest about being too full.
const MAX_SUBRINGS: usize = 6;

/// Widest a ring may be stretched into an ellipse, as a multiple of its height.
///
/// **A circular ring in a 16:10 frame wastes the left and right thirds** — the
/// fit step scales the whole picture to the *shorter* side, so the leaves land
/// closer together than the canvas could have held them and roughly 30% of the
/// image is empty margin. Stretching the ring's x-axis to the canvas's own
/// aspect makes the ellipse fill the frame; the fit step's scale is then set by
/// both axes at once instead of by height alone.
///
/// **Stretch, never squash.** The map is `x *= s` with `s >= 1` and `y`
/// untouched, so the arc length between two neighbours only ever *grows*: at
/// the top and bottom of the ellipse it grows by `s`, and at the sides it is
/// unchanged. A ring that cleared its slots as a circle still clears them as an
/// ellipse, which is why no capacity arithmetic changes here. Capped because
/// past about 2:1 the "ring" reads as two horizontal rows.
const MAX_RING_STRETCH: f64 = 1.8;

/// Lay `nodes` out as hop rings around `seed` — the ego / expansion layout
/// (P11 direction (a)).
///
/// Same-type siblings are grouped into contiguous arcs on their ring, with a
/// gap between runs, so a fan of forty wellbores reads as one branch rather
/// than as forty dots in a cloud. Ring radius grows with *occupancy*: a ring
/// that has to hold sixty nodes is pushed out until sixty circles fit on it,
/// which is why this does not degenerate into overlapping arcs the way a fixed
/// radius does.
///
/// `group` is the arc key — the node's type, in practice. Nodes with the same
/// key land adjacent; the key's numeric value never reaches the picture, only
/// its equality does, so the caller may assign it however it likes as long as
/// it does so deterministically.
///
/// **No force pass runs after this.** That is what licenses the trigonometry
/// here against this module's no-libm rule: a ring position is `centre + r *
/// unit(angle)` and goes straight to the emitter, which prints two decimals, so
/// a platform libm's last bit is rounded away rather than amplified through six
/// hundred iterations. [`unit`] quantises the sine and cosine besides, so even
/// that rounding has nothing to do.
pub fn radial(
    nodes: &[LayoutNode],
    links: &[(usize, usize)],
    seed: usize,
    group: &[u32],
    canvas: Canvas,
) -> Result<Positions, CoreError> {
    let Canvas {
        width,
        height,
        reserved_top,
    } = canvas;
    let count = nodes.len();
    if count > MAX_LAYOUT_NODES {
        return Err(over_bound(count));
    }
    if count == 0 {
        return Ok(Positions::below(Vec::new()));
    }
    if seed >= count {
        return Err(CoreError::Request(format!(
            "the layout was handed seed {seed} for a scene of {count} nodes"
        )));
    }
    // The ellipse the rings are drawn on, and the side room the outward labels
    // will need — both read off the box this layout was given, so an island's
    // square sub-canvas gets a circle and no side allowance while the whole
    // image gets the frame's own shape.
    let usable_x = (width - 2.0 * MARGIN_PX - 2.0 * SIDE_LABEL_ROOM_PX).max(1.0);
    let usable_y = (height - reserved_top - 2.0 * MARGIN_PX).max(1.0);
    let stretch = (usable_x / usable_y).clamp(1.0, MAX_RING_STRETCH);
    let side_room = if stretch > 1.0 {
        SIDE_LABEL_ROOM_PX
    } else {
        0.0
    };

    let (hop, parent) = structure::hops(count, links, seed);
    let deepest = hop.iter().copied().max().unwrap_or(0);
    let mut xy = vec![(0.0f64, 0.0f64); count];
    // Angle each node was placed at, so the next ring can sit its children near
    // their parents instead of scattering them around the circle.
    let mut angle_of = vec![0.0f64; count];
    let mut radius = 0.0f64;

    for ring in 1..=deepest {
        let mut members: Vec<usize> = (0..count).filter(|i| hop[*i] == ring).collect();
        if members.is_empty() {
            continue;
        }
        if ring == 1 {
            // The first ring groups by type: these are the seed's direct
            // branches, and the branch is what the reader is counting.
            // Bigger runs first, so the dominant branch starts at the top.
            let mut run_size: std::collections::HashMap<u32, usize> =
                std::collections::HashMap::new();
            for i in &members {
                *run_size.entry(group[*i]).or_insert(0) += 1;
            }
            members.sort_by(|a, b| {
                let (sa, sb) = (run_size[&group[*a]], run_size[&group[*b]]);
                sb.cmp(&sa)
                    .then_with(|| group[*a].cmp(&group[*b]))
                    .then_with(|| a.cmp(b))
            });
        } else {
            // Deeper rings follow their parents around: a child drawn on the
            // far side of the picture from its parent is a line across the
            // whole image and a branch nobody can trace.
            members.sort_by(|a, b| {
                let pa = parent[*a].map(|p| angle_of[p]).unwrap_or(0.0);
                let pb = parent[*b].map(|p| angle_of[p]).unwrap_or(0.0);
                pa.total_cmp(&pb)
                    .then_with(|| group[*a].cmp(&group[*b]))
                    .then_with(|| a.cmp(b))
            });
        }

        // Arc each member needs, plus the gaps between same-key runs.
        let slots: Vec<f64> = members
            .iter()
            .map(|i| 2.0 * nodes[*i].radius + RING_SLOT_PAD_PX)
            .collect();
        let mean_slot = slots.iter().sum::<f64>() / slots.len() as f64;
        let mut gaps = vec![0.0f64; members.len()];
        for position in 1..members.len() {
            if group[members[position]] != group[members[position - 1]] {
                gaps[position] = GROUP_GAP_SLOTS * mean_slot;
            }
        }
        let needed: f64 = slots.iter().sum::<f64>() + gaps.iter().sum::<f64>();
        let widest = members
            .iter()
            .map(|i| nodes[*i].radius)
            .fold(0.0f64, f64::max);
        let inner =
            (radius + RING_GAP_PX + widest).max(if ring == 1 { FIRST_RING_PX } else { 0.0 });
        let step = 2.0 * widest + SUBRING_GAP_PX;

        // **Occupancy splits the hop into concentric sub-rings rather than
        // inflating one.** A single circle holding `n` nodes has radius
        // `n * slot / 2π`, so its *area* grows with `n²` — and all of that area
        // is the empty middle. Forty-five leaves that way is a 370 px disc with
        // nothing in it, several of those overflow the canvas, and `fit` scales
        // the whole picture down around circles it deliberately does not
        // scale — which is how a ring arrives as a solid arc of overlapping
        // dots. Splitting the same forty-five across three concentric arcs
        // holds the radius near the first ring's and keeps the gaps the size
        // they were computed to be.
        let mut rings = 1usize;
        while rings < MAX_SUBRINGS {
            let capacity: f64 = (0..rings).map(|j| TAU * (inner + j as f64 * step)).sum();
            if capacity >= needed {
                break;
            }
            rings += 1;
        }
        let capacity: f64 = (0..rings).map(|j| TAU * (inner + j as f64 * step)).sum();
        // Members fill the sub-rings in order, so a type's contiguous arc stays
        // contiguous — it may wrap onto the next circle out, which reads as one
        // branch two rows deep rather than as two branches.
        let mut assigned: Vec<(usize, f64)> = Vec::with_capacity(rings);
        let mut cut = 0usize;
        for sub in 0..rings {
            // Each sub-ring takes the share of the arc its own circumference
            // is, so the inner circles are not packed tighter than the outer.
            let share = needed * (TAU * (inner + sub as f64 * step)) / capacity;
            let start = cut;
            let mut arc = 0.0f64;
            while cut < members.len() && (arc < share || sub + 1 == rings) {
                arc += slots[cut] + gaps[cut];
                cut += 1;
            }
            assigned.push((start, arc.max(1.0)));
        }

        for (sub, (start, arc)) in assigned.iter().enumerate() {
            let end = assigned
                .get(sub + 1)
                .map(|(next, _)| *next)
                .unwrap_or(members.len());
            if *start >= end {
                continue;
            }
            let sub_radius = inner + sub as f64 * step;
            // Spread whatever the sub-ring did not need, so a sparse one is a
            // circle and not an arc with a hole in it.
            let scale = TAU * sub_radius / arc;
            let mut cursor = 0.0f64;
            for position in *start..end {
                let node = members[position];
                cursor += gaps[position] * scale;
                let angle =
                    QUARTER_TURN_BACK + (cursor + slots[position] * scale / 2.0) / sub_radius;
                cursor += slots[position] * scale;
                let (ux, uy) = unit(angle);
                // The ellipse: x is stretched, y is not. See MAX_RING_STRETCH.
                xy[node] = (sub_radius * ux * stretch, sub_radius * uy);
                angle_of[node] = angle;
            }
        }
        radius = inner + (rings - 1) as f64 * step;
    }

    // Outward from the centre, which sits at the origin in this space: a node
    // on the left half wears its name on its left. The seed itself keeps the
    // app's placement — it has no outward, and its label belongs under the
    // halo rather than beside it.
    let label_side: Vec<LabelSide> = (0..count)
        .map(|i| {
            if i == seed {
                LabelSide::Below
            } else {
                outward(xy[i])
            }
        })
        .collect();

    Ok(Positions {
        xy: fit(&xy, nodes, width, height, reserved_top, side_room).0,
        label_side,
        islands: Vec::new(),
    })
}

const TAU: f64 = std::f64::consts::TAU;
/// Rings start at twelve o'clock rather than three, so the first — largest —
/// branch is where a reader's eye already is.
const QUARTER_TURN_BACK: f64 = -std::f64::consts::FRAC_PI_2;

/// The unit vector at `angle`, quantised to 1/4096.
///
/// The quantisation is the determinism guard: `sin` and `cos` are the
/// platform's libm and IEEE-754 does not require them to be correctly rounded,
/// so two machines may disagree in the last bit. Rounding to a 1/4096 grid
/// throws that bit away — the two agree exactly — at a cost of under 0.25 mrad
/// of angular error, which at the largest radius any canvas here allows is a
/// fifth of a pixel.
fn unit(angle: f64) -> (f64, f64) {
    let quantise = |v: f64| (v * 4_096.0).round() / 4_096.0;
    (quantise(angle.cos()), quantise(angle.sin()))
}

/// Clear space between the inner shell's circles and the outer shell's, in
/// pixels.
///
/// Wide enough that the two shells read as two rings rather than as one thick
/// band — this gap is the entire visual claim "these are two kinds of thing".
const SHELL_GAP_PX: f64 = 66.0;

/// Where a two-shell arrangement's circles land, before any of them is placed.
///
/// Solved once and read twice: [`shells`] lays out against it, and [`islands`]
/// sizes the box it hands [`shells`] from it. A ring needs the room a ring
/// needs, and `fit` cannot give it any — the same reason the radial branch
/// there sizes its box from a circumference rather than from a node count.
struct ShellGeometry {
    inner_radius: f64,
    /// Radius of the innermost sub-ring of the outer shell.
    base: f64,
    /// Distance between two of the outer shell's sub-rings.
    step: f64,
    rings: usize,
    /// Half-width of the whole arrangement, outermost circle included.
    extent: f64,
}

fn shell_geometry(nodes: &[LayoutNode], inner: &[usize], outer: &[usize]) -> ShellGeometry {
    let widest = |members: &[usize]| {
        members
            .iter()
            .map(|i| nodes[*i].radius)
            .fold(0.0f64, f64::max)
    };
    let slots = |members: &[usize]| -> f64 {
        members
            .iter()
            .map(|i| 2.0 * nodes[*i].radius + RING_SLOT_PAD_PX)
            .sum()
    };
    let inner_widest = widest(inner);
    let outer_widest = widest(outer);
    let inner_radius = (slots(inner) / TAU).max(FIRST_RING_PX);
    let needed = slots(outer);
    let mut base = inner_radius + inner_widest + SHELL_GAP_PX + outer_widest;
    let step = 2.0 * outer_widest + SUBRING_GAP_PX;
    // The outer shell splits into concentric sub-rings for the same reason a hop
    // does (see MAX_SUBRINGS): one circle holding `n` nodes has radius `n·slot/2π`
    // and an area that grows with `n²`, all of it the empty middle.
    let capacity = |rings: usize, base: f64| -> f64 {
        (0..rings).map(|j| TAU * (base + j as f64 * step)).sum()
    };
    let mut rings = 1usize;
    while rings < MAX_SUBRINGS && capacity(rings, base) < needed {
        rings += 1;
    }
    if capacity(rings, base) < needed {
        // Past the sub-ring cap the shell simply grows, and the picture is
        // honest about being full. Solved rather than searched: capacity is
        // `TAU * (rings * base + step * (0 + 1 + … + rings-1))`.
        let triangle = (rings * (rings - 1)) as f64 / 2.0;
        base = (needed / TAU - step * triangle) / rings as f64;
    }
    let extent = base + (rings - 1) as f64 * step + outer_widest;
    ShellGeometry {
        inner_radius,
        base,
        step,
        rings,
        extent,
    }
}

/// Lay a bipartite scene out as two concentric shells (P11 round 3).
///
/// **What a blob loses and this keeps.** A force layout over a dense community
/// settles into a disc whose only readable property is "many"; the two classes
/// are interleaved, so the one fact the data is *made of* — that these are two
/// kinds of thing joined only to each other — is the fact the picture destroys.
/// Two rings put the classes in different places, and the spokes between them
/// are then radial rather than a mesh.
///
/// The inner shell is the smaller class, evenly spaced. Each outer member is
/// placed next to its **principal hub** — the inner neighbour with the highest
/// degree, ties to the lowest index — so an outer run under one hub is
/// contiguous and its spokes converge instead of crossing the picture. That
/// ordering is the whole difference between a readable double ring and a
/// spirograph.
///
/// No force pass follows, which is what licenses the trigonometry here under
/// this module's no-libm rule; [`unit`] quantises besides. Same argument as
/// [`radial`].
fn shells(
    nodes: &[LayoutNode],
    links: &[(usize, usize)],
    part: &structure::Bipartition,
    canvas: Canvas,
) -> Result<Positions, CoreError> {
    let Canvas {
        width,
        height,
        reserved_top,
    } = canvas;
    let count = nodes.len();
    if count > MAX_LAYOUT_NODES {
        return Err(over_bound(count));
    }
    let geometry = shell_geometry(nodes, &part.inner, &part.outer);
    let degree = structure::degrees(count, links);
    let neighbours = structure::adjacency(count, links);

    let mut xy = vec![(0.0f64, 0.0f64); count];
    let mut angle_of = vec![0.0f64; count];
    for (position, member) in part.inner.iter().enumerate() {
        let angle = QUARTER_TURN_BACK + TAU * position as f64 / part.inner.len() as f64;
        let (ux, uy) = unit(angle);
        xy[*member] = (geometry.inner_radius * ux, geometry.inner_radius * uy);
        angle_of[*member] = angle;
    }

    let mut is_inner = vec![false; count];
    for member in &part.inner {
        is_inner[*member] = true;
    }
    // Each outer member's principal hub: the inner neighbour it is most likely
    // to be read as belonging to.
    let hub_of: Vec<usize> = part
        .outer
        .iter()
        .map(|member| {
            let mut best = usize::MAX;
            for peer in &neighbours[*member] {
                if !is_inner[*peer] {
                    continue;
                }
                // Strictly greater, so an exact tie keeps the lower index.
                if best == usize::MAX || degree[*peer] > degree[best] {
                    best = *peer;
                }
            }
            best
        })
        .collect();
    let mut order: Vec<usize> = (0..part.outer.len()).collect();
    order.sort_by(|a, b| {
        let key = |i: usize| {
            let hub = hub_of[i];
            let angle = if hub == usize::MAX {
                f64::MAX
            } else {
                angle_of[hub]
            };
            (angle, part.outer[i])
        };
        let (ka, kb) = (key(*a), key(*b));
        ka.0.total_cmp(&kb.0).then_with(|| ka.1.cmp(&kb.1))
    });

    // The sub-rings take the share of the members their own circumference is,
    // so the inner circles are not packed tighter than the outer.
    let slot = |member: usize| 2.0 * nodes[member].radius + RING_SLOT_PAD_PX;
    let needed: f64 = part.outer.iter().map(|m| slot(*m)).sum();
    let capacity: f64 = (0..geometry.rings)
        .map(|j| TAU * (geometry.base + j as f64 * geometry.step))
        .sum();
    let mut cut = 0usize;
    for sub in 0..geometry.rings {
        let radius = geometry.base + sub as f64 * geometry.step;
        let share = needed * (TAU * radius) / capacity;
        let start = cut;
        let mut arc = 0.0f64;
        while cut < order.len() && (arc < share || sub + 1 == geometry.rings) {
            arc += slot(part.outer[order[cut]]);
            cut += 1;
        }
        if start >= cut {
            continue;
        }
        // Spread whatever this sub-ring did not need, so a sparse one is a
        // circle and not an arc with a hole in it.
        let scale = TAU * radius / arc.max(1.0);
        let mut cursor = 0.0f64;
        for slotted in &order[start..cut] {
            let member = part.outer[*slotted];
            let angle = QUARTER_TURN_BACK + (cursor + slot(member) * scale / 2.0) / radius;
            cursor += slot(member) * scale;
            let (ux, uy) = unit(angle);
            xy[member] = (radius * ux, radius * uy);
            angle_of[member] = angle;
        }
    }

    let label_side = (0..count).map(|i| outward(xy[i])).collect();
    Ok(Positions {
        xy: fit(&xy, nodes, width, height, reserved_top, 0.0).0,
        label_side,
        islands: Vec::new(),
    })
}

/// Which side of a node its name belongs on, given the node's offset from the
/// centre it radiates from.
///
/// **The sector is cut on the offset itself, in the ring's own — already
/// stretched — coordinates**, so the boundary follows the shape a reader is
/// looking at rather than an angle in an ellipse nobody drew. A node whose
/// vertical offset beats its horizontal one is at the top or the bottom of the
/// ring, where its neighbours are beside it and the room is above or below; at
/// the sides it is the other way round. On a ring stretched to a 16:10 frame
/// that puts roughly the top and bottom sixths in the vertical sectors, which is
/// exactly the arc where round 2's side-only rule was drawing chips through
/// their neighbours.
fn outward((x, y): (f64, f64)) -> LabelSide {
    if y.abs() > x.abs() {
        if y < 0.0 {
            LabelSide::Above
        } else {
            LabelSide::Below
        }
    } else if x < 0.0 {
        LabelSide::Left
    } else {
        LabelSide::Right
    }
}

/// Padding between two packed islands, as a multiple of one node's spacing —
/// and a floor, in pixels, for the case where the spacing itself is tiny.
///
/// **The gap between two islands has to beat the gap inside one, or there are
/// no islands.** Round 1 packed at a flat 54 px while a node inside an island
/// got up to 120 px of its own, so the *lane* between two communities was
/// narrower than the space between two members of one, and the coordinator's
/// verdict on the meta-graph was that island-ness is not visible. It is a
/// multiple of the spacing now, so the two move together and the relationship
/// between them is fixed rather than coincidental.
const ISLAND_PAD_SPACINGS: f64 = 1.5;
const ISLAND_PAD_MIN_PX: f64 = 80.0;

/// Quiet keep-out between an island's outermost circle and the boundary drawn
/// around it, in pixels. Emitted geometry, not layout — see [`Island`].
pub const ISLAND_HULL_PAD_PX: f64 = 16.0;

/// Bounds on the side length one node is given inside its island.
///
/// The spacing itself is **derived from the canvas and the node count**, and
/// that is a bug fix rather than a refinement: [`fit`] scales positions and
/// deliberately does not scale radii — a radius is an encoding, and a picture
/// whose circles shrank with the layout would stop meaning what the app's
/// circles mean. So a packing that comes out larger than the canvas is scaled
/// down *around* full-size circles, and a ring laid out with a 14 px gap
/// between neighbours arrives with the neighbours touching. Sizing the islands
/// so the packing lands near the canvas keeps that scale near 1, which is the
/// only way the gaps in this module mean what they say.
///
/// Constant *across islands within one image*, on purpose: it is what makes a
/// big island look big. Sizing every island to the same box would say a
/// two-node community and a two-hundred-node one are the same thing.
const ISLAND_SPACING_MIN_PX: f64 = 30.0;
const ISLAND_SPACING_MAX_PX: f64 = 120.0;

/// Share of the canvas the packed islands aim to fill.
///
/// Under 1 because shelf packing leaves gaps and [`fit`] adds a margin; aiming
/// at the whole canvas overshoots and reintroduces the shrink this is avoiding.
///
/// **Lowered in round 2 to buy the lanes.** The padding above is now a multiple
/// of the spacing, so islands that fill 62% of the frame leave lanes that get
/// scaled away by `fit` the moment the padded packing overflows. At 0.44 the
/// island interiors are denser, the lanes survive at the width they were
/// computed at, and the picture answers "how many groups are there" before it
/// answers anything else.
const ISLAND_FILL: f64 = 0.9;

/// One packed community, for the emitter.
///
/// **A boundary is a claim, so it names what it encloses.** A tinted hull round
/// a Louvain community says "these belong together" and that is exactly the
/// finding; the same hull round the tray of unattached singletons would say the
/// opposite of the truth, which is why `orphans` is carried rather than
/// inferred from a size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Island {
    /// Node indices into the scene, ascending.
    pub members: Vec<usize>,
    /// True for the one island that is a *tray* of communities of one rather
    /// than a community.
    pub orphans: bool,
}

/// Lay each community out on its own and pack the results (P11 direction (c)).
///
/// **Islands, not smears.** A single force pass over a graph with community
/// structure produces one cloud with slightly denser patches; a reader cannot
/// name the patches and the picture carries no more information than a node
/// count. Laying each community out in its own box and packing the boxes makes
/// the grouping a *spatial* fact, which is the one visual channel that survives
/// being scaled down to a chat thumbnail.
///
/// Every community is laid out by whichever kernel fits it — a community that
/// is itself a star gets [`radial`], so a hub-and-spokes island reads as
/// hub-and-spokes rather than as a disc.
pub fn islands(
    nodes: &[LayoutNode],
    links: &[(usize, usize)],
    community: &[usize],
    groups: usize,
    group: &[u32],
    canvas: Canvas,
    seed: u64,
) -> Result<Positions, CoreError> {
    let Canvas {
        width,
        height,
        reserved_top,
    } = canvas;
    let count = nodes.len();
    if count > MAX_LAYOUT_NODES {
        return Err(over_bound(count));
    }
    if count == 0 {
        return Ok(Positions::below(Vec::new()));
    }

    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); groups];
    for (index, group_id) in community.iter().enumerate() {
        if *group_id < groups {
            buckets[*group_id].push(index);
        }
    }
    // Communities of one are the meta-graph's isolated types and a query's
    // stragglers. Packed as their own boxes they would be a row of single
    // circles with a gap between each; gathered into one island they read as
    // what they are — "these are unattached" — in a corner of the picture.
    let mut orphans: Vec<usize> = Vec::new();
    let mut islands: Vec<Vec<usize>> = Vec::new();
    for bucket in buckets {
        match bucket.len() {
            0 => {}
            1 => orphans.push(bucket[0]),
            _ => islands.push(bucket),
        }
    }
    // Largest first, then chained by how heavily each island is tied to the one
    // before it — see `order_by_affinity`. The packer lays boxes out in this
    // order along shelves, so it decides which islands end up adjacent, and
    // adjacency is what a cross-island line's length costs.
    islands.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
    order_by_affinity(&mut islands, links, count);
    let orphan_island = !orphans.is_empty();
    if orphan_island {
        orphans.sort_unstable();
        islands.push(orphans);
    }

    // Island box area is `1.0625 * spacing^2 * members`, so the whole packing
    // is `1.0625 * spacing^2 * count`; solve that against the share of the
    // canvas the packing should fill.
    let mut spacing = (ISLAND_FILL * width * (height - reserved_top) / (1.0625 * count as f64))
        .sqrt()
        .clamp(ISLAND_SPACING_MIN_PX, ISLAND_SPACING_MAX_PX);

    // **Solve, measure what the fit would cost, and re-solve at a spacing the
    // canvas can wear** (P11 round 3). The area estimate above ignores what
    // shelf packing wastes — the lane between two boxes, the ragged end of a
    // shelf, the square box a ring insists on — so the packing it sizes routinely
    // comes out larger than the frame, and `fit` then multiplies every lane and
    // every gap inside every island by the same `s < 1` while the circles stay
    // full size. Shrinking `spacing` by exactly that `s` is what makes the next
    // packing land near the frame, and the *floors* in here are why this
    // converges to something useful rather than to nothing: a radial island's box
    // is sized from its own circumference, the orphan tray's cell from its widest
    // circle, and `pad` from `ISLAND_PAD_MIN_PX`. None of those follow `spacing`
    // down, so the fixed point is "as much spacing as the frame can hold, with
    // the room the circles actually need kept whole".
    //
    // Three passes because the second is usually within a few per cent of the
    // fixed point and the third is the guard; `separate` holds the invariant
    // whatever this converges to.
    const PACK_PASSES: usize = 3;
    let mut solved: Option<Packed> = None;
    for pass in 0..PACK_PASSES {
        let packed = pack_at_spacing(
            nodes,
            links,
            &islands,
            orphan_island,
            group,
            spacing,
            canvas,
            seed,
            count,
        )?;
        let (fitted, scale) = fit(&packed.xy, nodes, width, height, reserved_top, 0.0);
        solved = Some(Packed {
            xy: fitted,
            label_side: packed.label_side,
        });
        if scale >= 1.0 || pass + 1 == PACK_PASSES {
            break;
        }
        let next = (spacing * scale).max(ISLAND_SPACING_MIN_PX);
        // A pass that would barely move the spacing has found the floors, and
        // re-solving to land on the same picture is a wasted force pass.
        if next >= spacing - 0.5 {
            break;
        }
        spacing = next;
    }
    // `PACK_PASSES` is not zero, so the loop always ran; the fallback is the
    // compiler's price for saying so rather than a case that occurs.
    let Packed { xy, label_side } = solved.unwrap_or_else(|| Packed {
        xy: vec![(0.0f64, 0.0f64); count],
        label_side: vec![LabelSide::Below; count],
    });

    let last = islands.len().saturating_sub(1);
    let reported: Vec<Island> = islands
        .into_iter()
        .enumerate()
        .map(|(index, mut members)| {
            members.sort_unstable();
            Island {
                members,
                orphans: orphan_island && index == last,
            }
        })
        .collect();

    Ok(Positions {
        xy,
        label_side,
        islands: reported,
    })
}

/// One pre-fit packing: the two things [`islands`] needs back from a pass.
struct Packed {
    xy: Vec<(f64, f64)>,
    label_side: Vec<LabelSide>,
}

/// Lay every island out at `spacing` and shelf-pack the results, in the
/// packing's own coordinates — [`islands`] fits the answer to the canvas.
///
/// Split out of [`islands`] because that function now calls it more than once:
/// see the fixed point there.
#[allow(clippy::too_many_arguments)]
fn pack_at_spacing(
    nodes: &[LayoutNode],
    links: &[(usize, usize)],
    islands: &[Vec<usize>],
    orphan_island: bool,
    group: &[u32],
    spacing: f64,
    canvas: Canvas,
    seed: u64,
    count: usize,
) -> Result<Packed, CoreError> {
    let Canvas { width, height, .. } = canvas;
    let pad = (spacing * ISLAND_PAD_SPACINGS).max(ISLAND_PAD_MIN_PX);

    let mut xy = vec![(0.0f64, 0.0f64); count];
    let mut label_side = vec![LabelSide::Below; count];
    // (island index, width, height) — solved before anything is placed, so the
    // packer works on final sizes.
    let mut boxes: Vec<(f64, f64)> = Vec::with_capacity(islands.len());
    let mut placements: Vec<Vec<(f64, f64)>> = Vec::with_capacity(islands.len());

    for (island_index, members) in islands.iter().enumerate() {
        let side = spacing * (members.len() as f64).sqrt();
        // Slightly wide, matching the canvas: label chips are horizontal, and
        // an island shaped like the page it lands on packs without waste.
        let (box_width, box_height) = (side * 1.25, side * 0.85);
        let local_nodes: Vec<LayoutNode> = members.iter().map(|i| nodes[*i]).collect();
        let position_of: std::collections::HashMap<usize, usize> = members
            .iter()
            .enumerate()
            .map(|(local, global)| (*global, local))
            .collect();
        let local_links: Vec<(usize, usize)> = links
            .iter()
            .filter_map(|(a, b)| Some((*position_of.get(a)?, *position_of.get(b)?)))
            .collect();
        let local_group: Vec<u32> = members.iter().map(|i| group[*i]).collect();

        // The tray of unattached singletons is a **grid**, never a force pass.
        // A force layout over nodes with no edges at all is nothing but the
        // repulsion term against gravity: it settles into a blob whose shape is
        // an artefact of the seed lattice, and a reader looking for meaning in
        // it finds a pattern that is not in the data. A grid says exactly what
        // is true — "these are unattached, here they are, there are this many"
        // — and says it at a glance.
        if orphan_island && island_index + 1 == islands.len() {
            let columns = ((members.len() as f64).sqrt() * 1.4).ceil().max(1.0);
            let rows = (members.len() as f64 / columns).ceil().max(1.0);
            let cell = spacing.max(2.0 * local_nodes.iter().map(|n| n.radius).fold(0.0, f64::max));
            let placed: Vec<(f64, f64)> = (0..members.len())
                .map(|i| {
                    let column = (i % columns as usize) as f64;
                    let row = (i / columns as usize) as f64;
                    (column * cell, row * cell)
                })
                .collect();
            boxes.push(((columns - 1.0) * cell + 1.0, (rows - 1.0) * cell + 1.0));
            placements.push(placed);
            continue;
        }

        // A different starting point per island, so two islands of the same
        // size are not two copies of one picture.
        let island_seed = seed ^ ((island_index as u64 + 1).wrapping_mul(0x9E37_79B9));
        let plan = structure::plan(local_nodes.len(), &local_links, &[]);
        // **Two kinds of thing joined only to each other, drawn as two shells**
        // (P11 round 3). Tested only where the island is not already a star,
        // because a star is bipartite too and `radial` is the better picture of
        // one. `structure::bipartition` refuses everything else — a single odd
        // cycle, a class of two, a forest — and the force layout stays the
        // honest answer for a genuinely shapeless interior.
        let shell_part = match plan {
            structure::Plan::Radial { .. } => None,
            _ => structure::bipartition(local_nodes.len(), &local_links),
        };
        // A ring needs the room a ring needs, and `fit` cannot give it any:
        // that step scales *positions* and deliberately leaves radii alone, so
        // a ring squeezed into a box that is too short arrives as a solid arc
        // of overlapping circles rather than as a ring of readable ones. The
        // box for a radial or shelled island is therefore sized from the ring's
        // own circumference, and is square, because a ring is.
        let (box_width, box_height) = match (&plan, &shell_part) {
            (_, Some(part)) => {
                let extent = shell_geometry(&local_nodes, &part.inner, &part.outer).extent;
                let square = (2.0 * extent).max(side);
                (square, square)
            }
            (structure::Plan::Radial { .. }, _) => {
                let widest = local_nodes.iter().map(|n| n.radius).fold(0.0f64, f64::max);
                let circumference = local_nodes.len() as f64 * (2.0 * widest + RING_SLOT_PAD_PX);
                let ring = (circumference / std::f64::consts::PI + 4.0 * widest).max(side);
                (ring, ring)
            }
            _ => (box_width, box_height),
        };
        let island_canvas = Canvas {
            width: box_width,
            height: box_height,
            reserved_top: 0.0,
        };
        if let Some(part) = shell_part {
            let solved = shells(&local_nodes, &local_links, &part, island_canvas)?;
            for (member, side) in members.iter().zip(solved.label_side.iter()) {
                label_side[*member] = *side;
            }
            boxes.push((box_width, box_height));
            placements.push(solved.xy);
            continue;
        }
        let solved = match plan {
            structure::Plan::Radial { seed: centre } => radial(
                &local_nodes,
                &local_links,
                centre,
                &local_group,
                island_canvas,
            )?,
            _ => run(&local_nodes, &local_links, island_canvas, island_seed)?,
        };
        for (member, side) in members.iter().zip(solved.label_side.iter()) {
            label_side[*member] = *side;
        }
        boxes.push((box_width, box_height));
        placements.push(solved.xy);
    }

    // Shelf packing, at the strip width whose result is closest in shape to the
    // canvas.
    //
    // **Searched rather than computed** because a closed form over box areas is
    // wrong exactly when it matters: six islands of similar width pack two to a
    // shelf whatever the arithmetic says they should, and the tall column that
    // comes out is scaled down to the canvas *height*, wasting a third of the
    // width. Trying every shelf count from one to the island count is at most a
    // few hundred trivial passes and picks the packing that actually happened.
    let widest = boxes.iter().fold(1.0f64, |m, (w, _)| m.max(*w));
    let total_width: f64 = boxes.iter().map(|(w, _)| w + pad).sum();
    let want = (width / height).max(0.2);
    let mut best: Option<(f64, Vec<(f64, f64)>)> = None;
    for shelves in 1..=boxes.len() {
        let strip = (total_width / shelves as f64).max(widest);
        let (offsets, extent) = shelve(&boxes, strip, pad);
        let aspect = extent.0 / extent.1.max(1.0);
        // Log-ratio, so "twice as wide as wanted" and "half as wide" cost the
        // same; a linear difference would systematically prefer wide packings.
        let cost = (aspect / want).max(want / aspect);
        if best.as_ref().is_none_or(|(seen, _)| cost < *seen) {
            best = Some((cost, offsets));
        }
    }
    let offsets = best.map(|(_, offsets)| offsets).unwrap_or_default();
    for (island_index, (offset_x, offset_y)) in offsets.iter().enumerate() {
        for (member, (x, y)) in islands[island_index]
            .iter()
            .zip(placements[island_index].iter())
        {
            xy[*member] = (offset_x + x, offset_y + y);
        }
    }

    Ok(Packed { xy, label_side })
}

/// Reorder islands so heavily linked ones end up adjacent in the packing.
///
/// **A cross-island line is the ink the island layout is trying not to spend.**
/// Round 1 packed strictly by size, so two communities joined by forty edges
/// could land at opposite corners and draw forty lines across the whole frame —
/// which is most of what made the meta-graph read as a web rather than as
/// islands. Greedy chaining: the largest island opens, and each next slot goes
/// to whichever unplaced island is most heavily tied to the one just placed,
/// falling back on its weight to everything placed so far, then on size, then
/// on its lowest member — a total order, so the result is a function of the
/// input and not of a scan order.
///
/// Greedy rather than an optimal seriation because the objective is a
/// travelling-salesman shape and the input is at most a few dozen islands whose
/// shelf positions are decided afterwards anyway: a better ordering buys
/// nothing a shelf wrap does not immediately spend.
fn order_by_affinity(islands: &mut Vec<Vec<usize>>, links: &[(usize, usize)], count: usize) {
    if islands.len() < 3 {
        return;
    }
    let mut island_of = vec![usize::MAX; count];
    for (index, members) in islands.iter().enumerate() {
        for member in members {
            island_of[*member] = index;
        }
    }
    let n = islands.len();
    let mut weight = vec![0u32; n * n];
    for (a, b) in links {
        let (Some(ia), Some(ib)) = (island_of.get(*a), island_of.get(*b)) else {
            continue;
        };
        if *ia == usize::MAX || *ib == usize::MAX || ia == ib {
            continue;
        }
        weight[ia * n + ib] += 1;
        weight[ib * n + ia] += 1;
    }

    let mut placed = vec![false; n];
    let mut order: Vec<usize> = Vec::with_capacity(n);
    // Index 0 is the largest island — `islands` arrived sorted by size.
    let mut current = 0usize;
    placed[0] = true;
    order.push(0);
    let mut to_placed = vec![0u32; n];
    for i in 0..n {
        to_placed[i] = weight[current * n + i];
    }
    while order.len() < n {
        let mut best = usize::MAX;
        let mut best_key = (0u32, 0u32, 0usize);
        for candidate in 0..n {
            if placed[candidate] {
                continue;
            }
            let key = (
                weight[current * n + candidate],
                to_placed[candidate],
                islands[candidate].len(),
            );
            // Strictly greater, so an exact tie keeps the lower island index —
            // which is the larger island, and after that the lower first member.
            if best == usize::MAX || key > best_key {
                best = candidate;
                best_key = key;
            }
        }
        placed[best] = true;
        order.push(best);
        current = best;
        for i in 0..n {
            to_placed[i] += weight[best * n + i];
        }
    }

    let mut reordered: Vec<Vec<usize>> = Vec::with_capacity(n);
    let mut source: Vec<Option<Vec<usize>>> = islands.drain(..).map(Some).collect();
    for index in order {
        if let Some(members) = source[index].take() {
            reordered.push(members);
        }
    }
    *islands = reordered;
}

/// Lay boxes out on shelves no wider than `strip`; returns each box's offset
/// and the extent of the whole packing.
fn shelve(boxes: &[(f64, f64)], strip: f64, pad: f64) -> (Vec<(f64, f64)>, (f64, f64)) {
    let (mut cursor_x, mut cursor_y, mut shelf_height) = (0.0f64, 0.0f64, 0.0f64);
    let (mut extent_x, mut extent_y) = (0.0f64, 0.0f64);
    let mut offsets = Vec::with_capacity(boxes.len());
    for (box_width, box_height) in boxes {
        if cursor_x > 0.0 && cursor_x + box_width > strip {
            cursor_x = 0.0;
            cursor_y += shelf_height + pad;
            shelf_height = 0.0;
        }
        offsets.push((cursor_x, cursor_y));
        cursor_x += box_width + pad;
        shelf_height = shelf_height.max(*box_height);
        extent_x = extent_x.max(cursor_x);
        extent_y = extent_y.max(cursor_y + shelf_height);
    }
    (offsets, (extent_x.max(1.0), extent_y.max(1.0)))
}

/// The refusal every layout entry point shares — the structural half of D5.
fn over_bound(count: usize) -> CoreError {
    CoreError::Request(format!(
        "a render lays out at most {MAX_LAYOUT_NODES} nodes (the D5 response bound); \
         this request carries {count}. Narrow the query or lower --limit."
    ))
}

/// Every pair inside the cutoff repels, `k²/d`, plus the soft-collision term.
///
/// Symmetric application (`i` gets `+`, `j` gets `-`) rather than a full double
/// loop: half the distance computations, and the pair is visited in a fixed
/// order so the sum is the same one every time. Same law as
/// [`repel_within_cutoff`] — this path only differs in how it finds the pairs.
fn repel_all_pairs(xy: &[(f64, f64)], nodes: &[LayoutNode], k: f64, disp: &mut [(f64, f64)]) {
    let cutoff = CUTOFF_K * k;
    for i in 0..xy.len() {
        for j in (i + 1)..xy.len() {
            if separation(xy, i, j) > cutoff {
                continue;
            }
            let (ux, uy, magnitude) = repulsion(xy, nodes, i, j, k);
            disp[i].0 += ux * magnitude;
            disp[i].1 += uy * magnitude;
            disp[j].0 -= ux * magnitude;
            disp[j].1 -= uy * magnitude;
        }
    }
}

/// The same repulsion, but only against nodes within [`CUTOFF_K`] ideal
/// distances, found through a uniform grid.
///
/// The grid is a sorted `Vec` of `(cell, index)` rather than a hash map: a hash
/// map's iteration order is not a promise, and a float sum whose order is not a
/// promise is not deterministic. Every cell's members stay in node-index order,
/// and the nine neighbour cells are read in a fixed order.
fn repel_within_cutoff(xy: &[(f64, f64)], nodes: &[LayoutNode], k: f64, disp: &mut [(f64, f64)]) {
    let cell = CUTOFF_K * k;
    let cutoff = CUTOFF_K * k;
    let mut cells: Vec<((i64, i64), usize)> = xy
        .iter()
        .enumerate()
        .map(|(i, (x, y))| ((cell_of(*x, cell), cell_of(*y, cell)), i))
        .collect();
    // Stable by construction: the key includes the node index, which is unique.
    cells.sort_unstable();

    for i in 0..xy.len() {
        let cx = cell_of(xy[i].0, cell);
        let cy = cell_of(xy[i].1, cell);
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                let key = (cx + dx, cy + dy);
                let start = cells.partition_point(|(c, _)| *c < key);
                for (c, j) in cells[start..].iter() {
                    if *c != key {
                        break;
                    }
                    if *j == i || separation(xy, i, *j) > cutoff {
                        continue;
                    }
                    let (ux, uy, magnitude) = repulsion(xy, nodes, i, *j, k);
                    disp[i].0 += ux * magnitude;
                    disp[i].1 += uy * magnitude;
                }
            }
        }
    }
}

fn cell_of(coordinate: f64, cell: f64) -> i64 {
    (coordinate / cell).floor() as i64
}

fn separation(xy: &[(f64, f64)], i: usize, j: usize) -> f64 {
    let dx = xy[i].0 - xy[j].0;
    let dy = xy[i].1 - xy[j].1;
    (dx * dx + dy * dy).sqrt()
}

/// The unit vector from `j` to `i`, and the repulsion magnitude along it.
///
/// A coincident pair — two nodes the seed lattice put at the same point, which
/// cannot happen, or a collapse that drove them together, which can — is pushed
/// apart along a fixed axis derived from the indices rather than by an
/// arbitrary choice, so it stays a pure function of the input.
fn repulsion(
    xy: &[(f64, f64)],
    nodes: &[LayoutNode],
    i: usize,
    j: usize,
    k: f64,
) -> (f64, f64, f64) {
    let dx = xy[i].0 - xy[j].0;
    let dy = xy[i].1 - xy[j].1;
    let distance = (dx * dx + dy * dy).sqrt();
    if distance <= 1e-9 {
        let axis = ((i + j) % 2) as f64;
        return (1.0 - axis, axis, k);
    }
    let mut magnitude = REPULSION_SCALE * k * k / distance;
    let personal = nodes[i].radius + nodes[j].radius + CLEARANCE_PX;
    if distance < personal {
        // Size-aware separation. Linear in the overlap, so it is strong where
        // circles interpenetrate and gone the moment they do not.
        magnitude += (personal - distance) * OVERLAP_STIFFNESS;
    }
    (dx / distance, dy / distance, magnitude)
}

/// Linked nodes attract — LinLog's saturating pull, not Fruchterman–Reingold's
/// `d²/k` (P11).
///
/// **What changes in the picture.** FR's attraction grows with the *square* of
/// the distance, so one long edge outweighs a dozen short ones and the layout
/// spends its whole budget dragging outliers inward; the result is the uniform
/// blob the user described as "like a 3D graph on a 2D image". LinLog's energy
/// is `Σ d − Σ ln d`, whose attraction force is **constant** with distance, so a
/// cluster's internal edges are what decide where it sits and a single bridge
/// between two clusters no longer collapses them together. That is the property
/// that makes groups read as groups.
///
/// **`d / (d + k)` rather than a literal constant, and no `ln` anywhere.** A
/// force that does not fall off as `d → 0` is a discontinuity at a coincident
/// pair, and 600 iterations of that is a jitter generator. This is linear below
/// one ideal distance and flat above it — LinLog's far field with FR's
/// well-behaved near field. The `ln` LinLog's derivation names never appears:
/// this pass runs inside an iterative loop, where a platform libm's last bit is
/// amplified rather than rounded away, which is the rule this module's header
/// states.
fn attract(
    xy: &[(f64, f64)],
    links: &[(usize, usize)],
    k: f64,
    count: usize,
    disp: &mut [(f64, f64)],
) {
    for (source, target) in links {
        if *source >= count || *target >= count || source == target {
            continue;
        }
        let dx = xy[*source].0 - xy[*target].0;
        let dy = xy[*source].1 - xy[*target].1;
        let distance = (dx * dx + dy * dy).sqrt();
        if distance <= 1e-9 {
            continue;
        }
        let magnitude = ATTRACTION_SCALE * k * distance / (distance + k);
        let (ux, uy) = (dx / distance, dy / distance);
        disp[*source].0 -= ux * magnitude;
        disp[*source].1 -= uy * magnitude;
        disp[*target].0 += ux * magnitude;
        disp[*target].1 += uy * magnitude;
    }
}

/// The starting arrangement: the P2 square-spiral lattice, scaled to `k`, with
/// an integer-derived jitter to break its symmetry.
///
/// A perfect lattice is a fixed point of a symmetric force law in more places
/// than one would like — four nodes on a square stay on it — so the jitter is
/// not cosmetic. It is derived from a splitmix64 finalizer over
/// `(seed, index, axis)`: a fixed bit mix, no RNG state, no platform float
/// behaviour, exactly the discipline `crate::layout` already runs under.
fn seed_positions(count: usize, k: f64, seed: u64) -> Vec<(f64, f64)> {
    let mut walker = SpiralWalker::default();
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let (cx, cy) = walker.next_cell();
        let jx = jitter(seed, index as u64, 0) * k;
        let jy = jitter(seed, index as u64, 1) * k;
        out.push((f64::from(cx) * k + jx, f64::from(cy) * k + jy));
    }
    out
}

/// A ±0.35 offset, quantized to 1/4096 so it is exact in `f64` and in `f32`.
fn jitter(seed: u64, index: u64, axis: u64) -> f64 {
    let mut z = seed ^ (index << 1) ^ axis;
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    const STEPS: u64 = 2_867;
    let step = (z % STEPS) as i64 - (STEPS as i64 / 2);
    step as f64 / 4_096.0
}

/// A square spiral outward from the origin — the same walk `crate::layout`
/// uses, for the same reason: an integer walk has no closed form to get subtly
/// wrong and no trigonometry to disagree about.
#[derive(Debug)]
struct SpiralWalker {
    x: i32,
    y: i32,
    dir: usize,
    run_len: i32,
    run_left: i32,
    runs_at_len: u8,
    first: bool,
}

impl Default for SpiralWalker {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            dir: 0,
            run_len: 1,
            run_left: 1,
            runs_at_len: 0,
            first: true,
        }
    }
}

impl SpiralWalker {
    fn next_cell(&mut self) -> (i32, i32) {
        if self.first {
            self.first = false;
            return (0, 0);
        }
        const STEPS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
        let (dx, dy) = STEPS[self.dir];
        self.x += dx;
        self.y += dy;
        self.run_left -= 1;
        if self.run_left == 0 {
            self.dir = (self.dir + 1) % 4;
            self.runs_at_len += 1;
            if self.runs_at_len == 2 {
                self.runs_at_len = 0;
                self.run_len += 1;
            }
            self.run_left = self.run_len;
        }
        (self.x, self.y)
    }
}

/// Uniformly scale and translate the settled layout so every circle is inside
/// the canvas, with room under the widest one for its label. Returns the
/// positions and **the scale it applied**.
///
/// Uniform, never per-axis: a non-uniform fit would turn a circle into an
/// ellipse's worth of visual weight and break the size encoding, which is the
/// one thing parity is about.
///
/// **Positions scale; radii do not — and the returned scale is how a caller
/// learns what that cost.** A radius is an encoding, so shrinking it with the
/// layout would make a circle mean something different in a crowded picture
/// than in an empty one. The price is that a scale below 1 multiplies every gap
/// a kernel computed while leaving the circles those gaps were sized for at full
/// width: at `s = 0.5`, two circles laid out exactly touching arrive
/// interpenetrating by half their radii. That is not a tuning problem, and round
/// 2's attempt to keep `s` near 1 by aiming the island packing lower was a
/// constant standing in for a guarantee.
///
/// So the scale is returned rather than swallowed, and there are two callers of
/// it: [`islands`] re-solves at a spacing the canvas can wear when it comes back
/// below 1, and [`separate`] holds the actual no-interpenetration invariant
/// afterwards, in final pixels, for every kernel.
fn fit(
    xy: &[(f64, f64)],
    nodes: &[LayoutNode],
    width: f64,
    height: f64,
    reserved_top: f64,
    side_room: f64,
) -> (Vec<(f64, f64)>, f64) {
    // Room for the label chip that hangs below the widest circle.
    const LABEL_ROOM_PX: f64 = 26.0;

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for ((x, y), node) in xy.iter().zip(nodes.iter()) {
        min_x = min_x.min(x - node.radius);
        max_x = max_x.max(x + node.radius);
        min_y = min_y.min(y - node.radius);
        max_y = max_y.max(y + node.radius + LABEL_ROOM_PX);
    }
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    // `side_room` is subtracted in FINAL pixels, not added to the pre-fit
    // extent: a label drawn beside its node is the same width whatever the
    // layout was scaled by, and an allowance folded into `span_x` would arrive
    // shrunk by exactly the factor it was there to survive.
    let usable_x = (width - 2.0 * MARGIN_PX - 2.0 * side_room).max(1.0);
    // `reserved_top` is the status block's strip. The app draws that block
    // *over* the canvas and lets it cover whatever is under it — it is
    // `pointer-events: none` chrome a user can scroll out from behind. An image
    // has no behind: a type name under the status block is a type name nobody
    // will ever read, so the layout is told about the strip instead.
    let usable_y = (height - reserved_top - 2.0 * MARGIN_PX).max(1.0);
    let scale = (usable_x / span_x).min(usable_y / span_y);
    // Centre what is left over, so a wide graph is not pinned to one edge.
    let offset_x = MARGIN_PX + side_room + (usable_x - span_x * scale) / 2.0;
    let offset_y = reserved_top + MARGIN_PX + (usable_y - span_y * scale) / 2.0;

    let placed = xy
        .iter()
        .map(|(x, y)| {
            (
                (x - min_x) * scale + offset_x,
                (y - min_y) * scale + offset_y,
            )
        })
        .collect();
    (placed, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(count: usize) -> (Vec<LayoutNode>, Vec<(usize, usize)>) {
        let nodes = vec![LayoutNode { radius: 8.0 }; count];
        let links = (0..count).map(|i| (i, (i + 1) % count)).collect();
        (nodes, links)
    }

    #[test]
    fn the_same_input_lays_out_to_the_same_bits() {
        let (nodes, links) = ring(40);
        let a = run(
            &nodes,
            &links,
            Canvas {
                width: 1200.0,
                height: 800.0,
                reserved_top: 0.0,
            },
            7,
        )
        .expect("inside the bound");
        let b = run(
            &nodes,
            &links,
            Canvas {
                width: 1200.0,
                height: 800.0,
                reserved_top: 0.0,
            },
            7,
        )
        .expect("inside the bound");
        assert_eq!(a, b, "the layout is a pure function of its arguments");
        let bits_a: Vec<(u64, u64)> =
            a.xy.iter()
                .map(|(x, y)| (x.to_bits(), y.to_bits()))
                .collect();
        let bits_b: Vec<(u64, u64)> =
            b.xy.iter()
                .map(|(x, y)| (x.to_bits(), y.to_bits()))
                .collect();
        assert_eq!(
            bits_a, bits_b,
            "equal to the bit, not merely to within epsilon"
        );
    }

    #[test]
    fn a_different_seed_is_a_different_picture() {
        // Otherwise `--seed` is a flag that does nothing, which is worse than
        // no flag: a user who dislikes a layout would keep pressing it.
        let (nodes, links) = ring(40);
        let a = run(
            &nodes,
            &links,
            Canvas {
                width: 1200.0,
                height: 800.0,
                reserved_top: 0.0,
            },
            1,
        )
        .expect("inside the bound");
        let b = run(
            &nodes,
            &links,
            Canvas {
                width: 1200.0,
                height: 800.0,
                reserved_top: 0.0,
            },
            2,
        )
        .expect("inside the bound");
        assert_ne!(a, b);
    }

    #[test]
    fn every_node_lands_inside_the_canvas() {
        let (nodes, links) = ring(120);
        let placed = run(
            &nodes,
            &links,
            Canvas {
                width: 1000.0,
                height: 700.0,
                reserved_top: 0.0,
            },
            3,
        )
        .expect("inside the bound");
        for (x, y) in &placed.xy {
            assert!(*x >= 0.0 && *x <= 1000.0, "x escaped the canvas: {x}");
            assert!(*y >= 0.0 && *y <= 700.0, "y escaped the canvas: {y}");
        }
    }

    #[test]
    fn connected_nodes_end_up_nearer_than_unconnected_ones() {
        // The layout's whole claim. Two triangles, no edge between them: the
        // within-triangle distances must beat the between-triangle ones, or the
        // picture says nothing about structure.
        let nodes = vec![LayoutNode { radius: 6.0 }; 6];
        let links = vec![(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3)];
        let placed = run(
            &nodes,
            &links,
            Canvas {
                width: 900.0,
                height: 900.0,
                reserved_top: 0.0,
            },
            11,
        )
        .expect("inside the bound");
        let within = separation(&placed.xy, 0, 1);
        let between = separation(&placed.xy, 0, 3).min(separation(&placed.xy, 2, 5));
        assert!(
            within < between,
            "within {within:.1} should beat between {between:.1}"
        );
    }

    #[test]
    fn large_circles_do_not_settle_inside_each_other() {
        // The soft-collision term's reason to exist: plain k²/d knows nothing
        // about radius, and a meta-graph's radii span 6..36 px.
        let nodes = vec![LayoutNode { radius: 36.0 }; 12];
        let links: Vec<(usize, usize)> = (1..12).map(|i| (0, i)).collect();
        let placed = run(
            &nodes,
            &links,
            Canvas {
                width: 900.0,
                height: 900.0,
                reserved_top: 0.0,
            },
            5,
        )
        .expect("inside the bound");
        for i in 0..12 {
            for j in (i + 1)..12 {
                assert!(
                    separation(&placed.xy, i, j) > 60.0,
                    "nodes {i} and {j} are {:.1} apart with 36 px radii",
                    separation(&placed.xy, i, j)
                );
            }
        }
    }

    /// A star, for the ring tests below: node 0 at the centre, `count - 1`
    /// leaves.
    fn star(count: usize) -> (Vec<LayoutNode>, Vec<(usize, usize)>, Vec<u32>) {
        let nodes = vec![LayoutNode { radius: 6.0 }; count];
        let links: Vec<(usize, usize)> = (1..count).map(|i| (0, i)).collect();
        let group = vec![0u32; count];
        (nodes, links, group)
    }

    #[test]
    fn a_ring_stretches_to_the_canvas_it_was_given() {
        // The waste this fixes: a circular ring in a 16:10 frame is fitted to
        // the shorter side, and the left and right thirds of the image are
        // empty. Measured as the drawn extent, because that is what a reader
        // sees; a stretch factor nobody could observe would be a no-op.
        let (nodes, links, group) = star(40);
        let spread = |canvas: Canvas| {
            let placed = radial(&nodes, &links, 0, &group, canvas).expect("a star lays out");
            let xs: Vec<f64> = placed.xy.iter().map(|(x, _)| *x).collect();
            let ys: Vec<f64> = placed.xy.iter().map(|(_, y)| *y).collect();
            let span = |v: &[f64]| {
                v.iter().copied().fold(f64::MIN, f64::max)
                    - v.iter().copied().fold(f64::MAX, f64::min)
            };
            span(&xs) / span(&ys)
        };
        let wide = spread(Canvas {
            width: 1600.0,
            height: 1000.0,
            reserved_top: 0.0,
        });
        let square = spread(Canvas {
            width: 1000.0,
            height: 1000.0,
            reserved_top: 0.0,
        });
        assert!(
            (square - 1.0).abs() < 0.12,
            "a square canvas still gets a circle: {square:.2}"
        );
        assert!(
            wide > 1.3,
            "a 16:10 canvas must get a ring that uses its width: {wide:.2}"
        );
    }

    #[test]
    fn ring_labels_point_away_from_the_centre() {
        // Without this the left half's chips are drawn across the spokes they
        // belong to, and the top and bottom of the ring — where the neighbours
        // are beside the node and the room is above it — get a chip pointing
        // straight into the two names next to it. Asserted per node against its
        // own offset, so a rule that happened to be right for one sector is not
        // enough.
        let (nodes, links, group) = star(24);
        let placed = radial(
            &nodes,
            &links,
            0,
            &group,
            Canvas {
                width: 1600.0,
                height: 1000.0,
                reserved_top: 0.0,
            },
        )
        .expect("a star lays out");
        let centre = placed.xy[0];
        assert_eq!(
            placed.label_side[0],
            LabelSide::Below,
            "the centre has no outward"
        );
        for i in 1..24 {
            let offset = (placed.xy[i].0 - centre.0, placed.xy[i].1 - centre.1);
            assert_eq!(
                placed.label_side[i],
                outward(offset),
                "node {i} at {offset:?}"
            );
        }
        // And the vertical sectors are actually reached: a rule the geometry
        // never enters is a rule nobody can see the effect of.
        let vertical = (1..24)
            .filter(|i| matches!(placed.label_side[*i], LabelSide::Above | LabelSide::Below))
            .count();
        assert!(
            vertical >= 4,
            "a 24-leaf ring must put some names above and below: {vertical}"
        );
    }

    /// `k` cliques of `size`, plus whatever extra links the caller adds.
    fn cliques(k: usize, size: usize) -> Vec<(usize, usize)> {
        let mut links = Vec::new();
        for clique in 0..k {
            let base = clique * size;
            for i in 0..size {
                for j in (i + 1)..size {
                    links.push((base + i, base + j));
                }
            }
        }
        links
    }

    #[test]
    fn a_bipartite_island_comes_out_as_two_separated_shells() {
        // The blob this replaces: a force layout over a dense two-sided
        // community interleaves the classes, which destroys the one fact the
        // data is made of. Measured as radius from the arrangement's centre,
        // because that is what a reader sees — every hub nearer the middle than
        // every leaf, with a gap between the two bands.
        const HUBS: usize = 6;
        const LEAVES: usize = 42;
        let count = HUBS + LEAVES;
        let nodes = vec![LayoutNode { radius: 6.0 }; count];
        let links: Vec<(usize, usize)> = (0..HUBS)
            .flat_map(|h| (0..LEAVES).map(move |l| (h, HUBS + l)))
            .collect();
        let part = structure::bipartition(count, &links).expect("the fixture is bipartite");
        let placed = shells(
            &nodes,
            &links,
            &part,
            Canvas {
                width: 1000.0,
                height: 1000.0,
                reserved_top: 0.0,
            },
        )
        .expect("two shells lay out");

        let centre = (500.0f64, 500.0f64);
        let radius_of = |i: usize| {
            let (dx, dy) = (placed.xy[i].0 - centre.0, placed.xy[i].1 - centre.1);
            (dx * dx + dy * dy).sqrt()
        };
        let inner_max = (0..HUBS).map(radius_of).fold(0.0f64, f64::max);
        let outer_min = (HUBS..count).map(radius_of).fold(f64::MAX, f64::min);
        assert!(
            outer_min > inner_max,
            "the shells overlap: hubs reach {inner_max:.1}, leaves start at {outer_min:.1}"
        );
        assert!(
            outer_min - inner_max > 2.0 * 6.0,
            "the two shells must read as two rings, not one band: \
             {:.1} px of clear space",
            outer_min - inner_max
        );
    }

    #[test]
    fn islands_pack_their_most_connected_neighbour_next() {
        // Round 1 packed strictly by size, so two communities joined by a
        // bundle of edges could land at opposite corners and draw that bundle
        // across the whole frame. Four equal cliques, with the last one tied to
        // the first: it must come out second in packing order, ahead of the two
        // it has no edge to.
        let size = 6;
        let mut links = cliques(4, size);
        for i in 0..4 {
            links.push((i, 3 * size + i));
        }
        let count = 4 * size;
        let nodes = vec![LayoutNode { radius: 6.0 }; count];
        let community: Vec<usize> = (0..count).map(|i| i / size).collect();
        let group = vec![0u32; count];
        let placed = islands(
            &nodes,
            &links,
            &community,
            4,
            &group,
            Canvas {
                width: 1600.0,
                height: 1000.0,
                reserved_top: 0.0,
            },
            5,
        )
        .expect("four cliques pack");
        assert_eq!(placed.islands.len(), 4);
        assert!(
            placed.islands[1].members.contains(&(3 * size)),
            "the island tied to the first must pack beside it, not opposite it: {:?}",
            placed
                .islands
                .iter()
                .map(|i| i.members[0])
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unattached_nodes_land_on_a_grid_and_are_marked_as_a_tray() {
        // A force pass over nodes with no edges is repulsion against gravity,
        // and it settles into a blob whose shape is an artefact of the seed
        // lattice — a pattern a reader will look for meaning in and not find.
        let size = 5;
        let links = cliques(2, size);
        let loners = 9;
        let count = 2 * size + loners;
        let nodes = vec![LayoutNode { radius: 6.0 }; count];
        let mut community: Vec<usize> = (0..count).map(|i| (i / size).min(2)).collect();
        for (offset, slot) in community.iter_mut().skip(2 * size).enumerate() {
            *slot = 2 + offset;
        }
        let groups = 2 + loners;
        let group = vec![0u32; count];
        let placed = islands(
            &nodes,
            &links,
            &community,
            groups,
            &group,
            Canvas {
                width: 1600.0,
                height: 1000.0,
                reserved_top: 0.0,
            },
            5,
        )
        .expect("two cliques and nine loners pack");
        let tray = placed
            .islands
            .iter()
            .find(|island| island.orphans)
            .expect("the loners are gathered into one tray");
        assert_eq!(tray.members.len(), loners);
        // A grid, observed as one rather than read back off the constant that
        // builds it: `rows x columns` cells hold the nine nodes with at most
        // one ragged row over. A force blob puts every node on its own row and
        // its own column, so its product is `n^2` and it misses this by a wide
        // margin.
        let distinct = |values: Vec<f64>| {
            let mut rounded: Vec<i64> = values.iter().map(|v| (v / 4.0).round() as i64).collect();
            rounded.sort_unstable();
            rounded.dedup();
            rounded.len()
        };
        let rows = distinct(tray.members.iter().map(|i| placed.xy[*i].1).collect());
        let columns = distinct(tray.members.iter().map(|i| placed.xy[*i].0).collect());
        assert!(
            rows * columns >= loners && rows * columns <= loners + columns,
            "nine unattached nodes must fill a lattice, not scatter: {rows} x {columns}"
        );
    }

    /// The smallest pairwise clearance in a placed layout: distance minus the
    /// two radii, so a negative value is interpenetration in pixels.
    fn tightest(xy: &[(f64, f64)], nodes: &[LayoutNode]) -> (f64, usize, usize) {
        let mut worst = (f64::INFINITY, 0usize, 0usize);
        for i in 0..xy.len() {
            for j in (i + 1)..xy.len() {
                let clearance = separation(xy, i, j) - nodes[i].radius - nodes[j].radius;
                if clearance < worst.0 {
                    worst = (clearance, i, j);
                }
            }
        }
        worst
    }

    struct Scene {
        nodes: Vec<LayoutNode>,
        links: Vec<(usize, usize)>,
        community: Vec<usize>,
        groups: usize,
    }

    /// Twelve communities of sixteen, every member wearing a meta-graph-sized
    /// radius — the shape that produced round 2's interpenetrating islands.
    fn dense_islands() -> Scene {
        const GROUPS: usize = 12;
        const SIZE: usize = 16;
        let count = GROUPS * SIZE;
        // A ramp, because a meta-graph's radii span 6..36 px and a collision
        // rule that only ever sees equal circles is not the rule under test.
        let nodes: Vec<LayoutNode> = (0..count)
            .map(|i| LayoutNode {
                radius: 6.0 + ((i % SIZE) as f64) * 2.0,
            })
            .collect();
        let mut links = cliques(GROUPS, SIZE);
        // One bridge per neighbouring pair, so the partition is islands rather
        // than components and `order_by_affinity` has something to chain on.
        for g in 1..GROUPS {
            links.push(((g - 1) * SIZE, g * SIZE + 1));
        }
        let community: Vec<usize> = (0..count).map(|i| i / SIZE).collect();
        Scene {
            nodes,
            links,
            community,
            groups: GROUPS,
        }
    }

    #[test]
    fn a_packing_too_big_for_its_canvas_does_not_arrive_interpenetrating() {
        // **The defect, and why a constant did not fix it.** `fit` scales
        // positions and not radii, so a packing larger than the frame arrives
        // multiplied by `s < 1` around circles that stayed full size, and every
        // gap the packer computed shrinks by `s` while the thing the gap was for
        // does not. Round 2 lowered `ISLAND_FILL` to keep `s` near 1 — a
        // constant standing in for a guarantee, which moves the threshold and
        // leaves the failure reachable. This asserts the guarantee instead, on
        // a scene small enough to state and dense enough to have failed.
        let scene = dense_islands();
        let (nodes, links) = (scene.nodes, scene.links);
        let group = vec![0u32; nodes.len()];
        let canvas = Canvas {
            width: 1600.0,
            height: 1000.0,
            reserved_top: 98.0,
        };
        let mut placed = islands(
            &nodes,
            &links,
            &scene.community,
            scene.groups,
            &group,
            canvas,
            5,
        )
        .expect("twelve dense communities pack");

        let before = tightest(&placed.xy, &nodes);
        separate(&mut placed.xy, &nodes, canvas);
        let after = tightest(&placed.xy, &nodes);

        // R1: the check can go red. Deleting the `separate` call above restores
        // `before`, which is the layout this test was written against, and this
        // assertion is what refuses it.
        assert!(
            before.0 < 0.0,
            "the scene must actually interpenetrate before the pass, or this \
             test proves nothing: tightest clearance was {:.1} px between {} and {}",
            before.0,
            before.1,
            before.2
        );
        assert!(
            after.0 >= 0.0,
            "nodes {} and {} still interpenetrate by {:.1} px after separation",
            after.1,
            after.2,
            -after.0
        );
        // And the pass stayed inside the frame it was handed.
        for (index, (x, y)) in placed.xy.iter().enumerate() {
            let radius = nodes[index].radius;
            assert!(
                *x >= radius - 0.01 && *x <= 1600.0 - radius + 0.01,
                "node {index} left the canvas at x={x}"
            );
            assert!(
                *y >= 98.0 + radius - 0.01 && *y <= 1000.0 - radius + 0.01,
                "node {index} left the canvas at y={y}"
            );
        }
    }

    #[test]
    fn separation_leaves_a_layout_that_already_clears_alone() {
        // The other half of the claim: this is a correction, not a relaxation.
        // A ring whose slots were computed correctly must come through
        // untouched, or every deliberate geometry in this module is negotiable.
        let (nodes, links, group) = star(24);
        let canvas = Canvas {
            width: 1600.0,
            height: 1000.0,
            reserved_top: 0.0,
        };
        let placed = radial(&nodes, &links, 0, &group, canvas).expect("a star lays out");
        let mut moved = placed.xy.clone();
        separate(&mut moved, &nodes, canvas);
        assert_eq!(
            moved, placed.xy,
            "a layout with room to spare must be a fixed point of the pass"
        );
    }

    /// The radial kernel at the D5 response bound, measured rather than
    /// reasoned about.
    ///
    /// **Why this exists.** The radial path is the one an ego or expansion
    /// render takes, `expand::effective_bound` lets 5 000 nodes through, and
    /// round 1 argued from the algorithm's shape that this was cheap. An
    /// inference is not a measurement (CLAUDE.md → "Posted technical claims"),
    /// and the shape argument is not obviously safe: the sub-ring search is
    /// `O(MAX_SUBRINGS)` per hop, but each ring sorts its members and hop 2's
    /// comparator reads a parent's angle, so the constant is not the one a
    /// glance at the loop suggests.
    ///
    /// **A measurement, not a gate.** `#[ignore]`d, so `make gate`'s debug
    /// `cargo test` never runs it: a debug-profile timing is not evidence
    /// (`R11`), and a wall-clock assertion inside a gate is a check that goes
    /// red on a busy machine. Run it deliberately:
    ///
    /// ```text
    /// cargo test --release -p kglite-visual-core --lib \
    ///     the_radial_kernel_at_the_response_bound -- --ignored --nocapture
    /// ```
    ///
    /// The scene is a synthetic star of [`MAX_LAYOUT_NODES`] nodes over eight
    /// type groups — the worst case for this kernel, because every node lands
    /// on one hop and that hop's sort, slot arithmetic and sub-ring search all
    /// run over the whole set at once. A two-hop shape spreads the same work
    /// across two smaller sorts and is strictly cheaper.
    #[test]
    #[ignore = "a measurement, not a gate: cargo test --release … -- --ignored --nocapture"]
    fn the_radial_kernel_at_the_response_bound() {
        let count = MAX_LAYOUT_NODES;
        let nodes = vec![LayoutNode { radius: 6.0 }; count];
        let links: Vec<(usize, usize)> = (1..count).map(|i| (0, i)).collect();
        let group: Vec<u32> = (0..count).map(|i| (i % 8) as u32).collect();
        let canvas = Canvas {
            width: 1600.0,
            height: 1000.0,
            reserved_top: 98.0,
        };
        for run in 1..=3 {
            let started = std::time::Instant::now();
            let placed = radial(&nodes, &links, 0, &group, canvas).expect("at the bound");
            let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(placed.xy.len(), count);
            println!("radial {count} nodes, run {run}: {elapsed:.2} ms");
        }
    }

    #[test]
    fn the_layout_refuses_more_than_the_response_bound_permits() {
        // R1: the structural half of D5. A layout that quietly accepted an
        // unbounded set would make "the bound is enforced in core" false the
        // moment a caller assembled one by hand.
        let nodes = vec![LayoutNode { radius: 4.0 }; MAX_LAYOUT_NODES + 1];
        let err = run(
            &nodes,
            &[],
            Canvas {
                width: 800.0,
                height: 600.0,
                reserved_top: 0.0,
            },
            0,
        )
        .expect_err("over the bound must refuse");
        assert!(
            err.to_string().contains("5000"),
            "the refusal must name the ceiling: {err}"
        );
        // And the boundary itself is allowed, so the ceiling is a ceiling and
        // not an off-by-one.
        let at_bound = vec![LayoutNode { radius: 4.0 }; MAX_LAYOUT_NODES];
        assert!(run(
            &at_bound,
            &[],
            Canvas {
                width: 800.0,
                height: 600.0,
                reserved_top: 0.0
            },
            0
        )
        .is_ok());
    }

    #[test]
    fn an_empty_or_single_node_layout_is_not_a_special_case_that_panics() {
        assert!(run(
            &[],
            &[],
            Canvas {
                width: 800.0,
                height: 600.0,
                reserved_top: 0.0
            },
            0
        )
        .expect("empty")
        .xy
        .is_empty());
        let one = run(
            &[LayoutNode { radius: 9.0 }],
            &[],
            Canvas {
                width: 800.0,
                height: 600.0,
                reserved_top: 0.0,
            },
            0,
        )
        .expect("single");
        assert_eq!(one.xy, vec![(400.0, 300.0)]);
    }

    #[test]
    fn a_link_naming_a_node_that_is_not_there_is_skipped_not_fatal() {
        let nodes = vec![LayoutNode { radius: 6.0 }; 3];
        let placed = run(
            &nodes,
            &[(0, 99), (1, 2)],
            Canvas {
                width: 600.0,
                height: 600.0,
                reserved_top: 0.0,
            },
            0,
        )
        .expect("out-of-range link");
        assert_eq!(placed.xy.len(), 3);
    }
}
