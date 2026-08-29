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

/// Where the layout put everything, in pixels, already fitted to the canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct Positions {
    pub xy: Vec<(f64, f64)>,
}

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
        return Ok(Positions { xy: Vec::new() });
    }
    if count == 1 {
        return Ok(Positions {
            xy: vec![(width / 2.0, (height + reserved_top) / 2.0)],
        });
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

    Ok(Positions {
        xy: fit(&xy, nodes, width, height, reserved_top),
    })
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
        return Ok(Positions { xy: Vec::new() });
    }
    if seed >= count {
        return Err(CoreError::Request(format!(
            "the layout was handed seed {seed} for a scene of {count} nodes"
        )));
    }

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
                xy[node] = (sub_radius * ux, sub_radius * uy);
                angle_of[node] = angle;
            }
        }
        radius = inner + (rings - 1) as f64 * step;
    }

    Ok(Positions {
        xy: fit(&xy, nodes, width, height, reserved_top),
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

/// Padding between two packed islands, in pixels.
const ISLAND_PAD_PX: f64 = 54.0;

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
const ISLAND_FILL: f64 = 0.62;

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
        return Ok(Positions { xy: Vec::new() });
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
    // Largest first: the eye lands on the biggest structure, and the packing
    // below wastes least when the tall shelves come first.
    islands.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
    if !orphans.is_empty() {
        orphans.sort_unstable();
        islands.push(orphans);
    }

    // Island box area is `1.0625 * spacing^2 * members`, so the whole packing
    // is `1.0625 * spacing^2 * count`; solve that against the share of the
    // canvas the packing should fill.
    let spacing = (ISLAND_FILL * width * (height - reserved_top) / (1.0625 * count as f64))
        .sqrt()
        .clamp(ISLAND_SPACING_MIN_PX, ISLAND_SPACING_MAX_PX);

    let mut xy = vec![(0.0f64, 0.0f64); count];
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

        // A different starting point per island, so two islands of the same
        // size are not two copies of one picture.
        let island_seed = seed ^ ((island_index as u64 + 1).wrapping_mul(0x9E37_79B9));
        let plan = structure::plan(local_nodes.len(), &local_links, &[]);
        // A ring needs the room a ring needs, and `fit` cannot give it any:
        // that step scales *positions* and deliberately leaves radii alone, so
        // a ring squeezed into a box that is too short arrives as a solid arc
        // of overlapping circles rather than as a ring of readable ones. The
        // box for a radial island is therefore sized from the ring's own
        // circumference, and is square, because a ring is.
        let (box_width, box_height) = match plan {
            structure::Plan::Radial { .. } => {
                let widest = local_nodes.iter().map(|n| n.radius).fold(0.0f64, f64::max);
                let circumference = local_nodes.len() as f64 * (2.0 * widest + RING_SLOT_PAD_PX);
                let ring = (circumference / std::f64::consts::PI + 4.0 * widest).max(side);
                (ring, ring)
            }
            _ => (box_width, box_height),
        };
        let solved = match plan {
            structure::Plan::Radial { seed: centre } => radial(
                &local_nodes,
                &local_links,
                centre,
                &local_group,
                Canvas {
                    width: box_width,
                    height: box_height,
                    reserved_top: 0.0,
                },
            )?,
            _ => run(
                &local_nodes,
                &local_links,
                Canvas {
                    width: box_width,
                    height: box_height,
                    reserved_top: 0.0,
                },
                island_seed,
            )?,
        };
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
    let total_width: f64 = boxes.iter().map(|(w, _)| w + ISLAND_PAD_PX).sum();
    let want = (width / height).max(0.2);
    let mut best: Option<(f64, Vec<(f64, f64)>)> = None;
    for shelves in 1..=boxes.len() {
        let strip = (total_width / shelves as f64).max(widest);
        let (offsets, extent) = shelve(&boxes, strip);
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

    Ok(Positions {
        xy: fit(&xy, nodes, width, height, reserved_top),
    })
}

/// Lay boxes out on shelves no wider than `strip`; returns each box's offset
/// and the extent of the whole packing.
fn shelve(boxes: &[(f64, f64)], strip: f64) -> (Vec<(f64, f64)>, (f64, f64)) {
    let (mut cursor_x, mut cursor_y, mut shelf_height) = (0.0f64, 0.0f64, 0.0f64);
    let (mut extent_x, mut extent_y) = (0.0f64, 0.0f64);
    let mut offsets = Vec::with_capacity(boxes.len());
    for (box_width, box_height) in boxes {
        if cursor_x > 0.0 && cursor_x + box_width > strip {
            cursor_x = 0.0;
            cursor_y += shelf_height + ISLAND_PAD_PX;
            shelf_height = 0.0;
        }
        offsets.push((cursor_x, cursor_y));
        cursor_x += box_width + ISLAND_PAD_PX;
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
/// the canvas, with room under the widest one for its label.
///
/// Uniform, never per-axis: a non-uniform fit would turn a circle into an
/// ellipse's worth of visual weight and break the size encoding, which is the
/// one thing parity is about.
fn fit(
    xy: &[(f64, f64)],
    nodes: &[LayoutNode],
    width: f64,
    height: f64,
    reserved_top: f64,
) -> Vec<(f64, f64)> {
    // Room for the label chip that hangs below the widest circle, plus the
    // status block the emitter draws in the top-left corner.
    const MARGIN_PX: f64 = 26.0;
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
    let usable_x = (width - 2.0 * MARGIN_PX).max(1.0);
    // `reserved_top` is the status block's strip. The app draws that block
    // *over* the canvas and lets it cover whatever is under it — it is
    // `pointer-events: none` chrome a user can scroll out from behind. An image
    // has no behind: a type name under the status block is a type name nobody
    // will ever read, so the layout is told about the strip instead.
    let usable_y = (height - reserved_top - 2.0 * MARGIN_PX).max(1.0);
    let scale = (usable_x / span_x).min(usable_y / span_y);
    // Centre what is left over, so a wide graph is not pinned to one edge.
    let offset_x = MARGIN_PX + (usable_x - span_x * scale) / 2.0;
    let offset_y = reserved_top + MARGIN_PX + (usable_y - span_y * scale) / 2.0;

    xy.iter()
        .map(|(x, y)| {
            (
                (x - min_x) * scale + offset_x,
                (y - min_y) * scale + offset_y,
            )
        })
        .collect()
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
