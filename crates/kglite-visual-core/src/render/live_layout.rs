//! A static arrangement for the **live** view, computed here and pushed to
//! every client (plan E5).
//!
//! **This is the one thing the browser's force layout cannot do: hold still.**
//! cosmos.gl's simulation is the right answer for "what shape is this graph",
//! and the wrong one for "put the same graph in the same place twice" — a hop
//! ring stays a hop ring only while nothing re-heats it. The structure-chosen
//! kernels already existed for the headless render — P11's three, plus G4's
//! geographic map; this hands the same kernels' output to the *live* view, so
//! the picture a user is looking at can be an arrangement rather than a settle.
//!
//! **The scene is [`super::live_scene`]'s, and deliberately so.** Reusing the
//! render's scene builder is what keeps the two arrangements answering the same
//! question: the live render draws these nodes, these links, these radii, and a
//! second scene builder here would drift from it one encoding change at a time.
//!
//! ## Two things this does NOT do, and why
//!
//! - **No [`super::layout::separate`] pass.** That pass enforces "two circles
//!   do not interpenetrate" in *final SVG pixels*, against radii
//!   `super::encoding` computed for a 2000x1250 document. The browser's point
//!   radius is a different encoding entirely — cosmos.gl sizes points in its
//!   own units, scaled by zoom — so running the separation here would spread
//!   the layout to clear overlaps that do not exist on the receiving screen and
//!   leave the ones that do. The kernels' own spacing (which is what gives a
//!   ring its radius and an island its box) is the part that transfers; the
//!   pixel-exact non-overlap guarantee is not.
//! - **No fold, no label placement, no emphasis.** Those shape a *picture*.
//!   What crosses the wire here is one position per slot.
//!
//! **Positions are origin-centred**, like [`crate::layout`]'s lattice, because
//! that is what the client's `toRendererSpace` shift assumes. The kernels work
//! in canvas coordinates, so the canvas centre is subtracted on the way out.

use serde::Serialize;
use ts_rs::TS;

use crate::error::CoreError;
use crate::protocol::PROTOCOL_VERSION;
use crate::request::{LayoutKernel, LayoutRequest};
use crate::session::Session;

use super::{geo, layout, structure, DEFAULT_HEIGHT, DEFAULT_WIDTH};

/// What a client needs to know about an arrangement it did not compute.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LayoutMeta {
    pub protocol_version: u32,
    /// What the caller asked for — `auto` included, so a reader can tell a
    /// chosen kernel from a requested one.
    pub kernel_requested: LayoutKernel,
    /// What actually ran. Never `auto` (that is a question, not an answer). It
    /// can differ from `kernel_requested`
    /// for a reason the caller cannot predict: `islands` over a scene with no
    /// community structure has no islands to pack, and falling back to `force`
    /// while *saying* `force` is the honest outcome — a caller that reads this
    /// field learns the arrangement it is looking at is not the one it named.
    pub kernel_chosen: LayoutKernel,
    /// The slot a radial layout was centred on. `None` for every other kernel.
    pub seed_slot: Option<u32>,
    /// Slots the position array covers, tombstones included.
    pub slot_count: u32,
    /// Slots that got a real position. `slot_count - live_count` are the NaNs.
    pub live_count: u32,
    /// Wall-clock milliseconds the kernel took, for the same reason
    /// [`super::Rendered::layout_ms`] carries it: the kernels have very
    /// different costs and a caller that suddenly waits needs the number.
    pub layout_ms: f64,
}

/// An arrangement: the metadata, and one `(x, y)` per slot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayoutResult {
    pub meta: LayoutMeta,
    /// `[x0, y0, x1, y1, …]` over the **whole** slot space in slot order, so a
    /// client applies it with one `setPointPositions` and no splice
    /// arithmetic. A tombstoned slot is `NaN`, which is cosmos.gl's absence.
    pub points: Vec<f32>,
}

/// Compute an arrangement for what is currently on screen.
///
/// Reads the session's slot space and moves nothing in it: no slot is
/// allocated, none is tombstoned, no link changes. The only thing that changes
/// is where the client draws what it already has.
pub fn layout_live_view(
    session: &Session,
    request: &LayoutRequest,
) -> Result<LayoutResult, CoreError> {
    let slot_count = session.view_read().slot_count();
    if request.kernel == LayoutKernel::Simulation {
        // Handing the view back is a *message*, not an arrangement: there are
        // no positions to send, because the whole point is that the viewer's
        // own simulation is about to compute them. An empty points array is
        // the honest payload — a lattice here would be one last server
        // arrangement flashing on screen before the GPU pulled it apart.
        return Ok(LayoutResult {
            meta: LayoutMeta {
                protocol_version: PROTOCOL_VERSION,
                kernel_requested: LayoutKernel::Simulation,
                kernel_chosen: LayoutKernel::Simulation,
                seed_slot: None,
                slot_count,
                live_count: 0,
                layout_ms: 0.0,
            },
            points: Vec::new(),
        });
    }

    let scene = super::live_scene(session);
    let links: Vec<(usize, usize)> = scene.links.iter().map(|l| (l.source, l.target)).collect();
    let count = scene.nodes.len();

    // The slot a `seed_slot` hint names, as an index into the scene. A slot
    // that is not on screen is not an error: the caller pointed at something
    // tombstoned or never loaded, and the structure pass still has an opinion
    // about where the centre is.
    let seed_hint = request.seed_slot.and_then(|slot| {
        scene
            .nodes
            .iter()
            .position(|node| node.slot == slot)
            .filter(|index| *index < count)
    });

    // **The geographic question is asked of the scene, not of the topology.**
    // `structure::plan` reads links; whether these nodes are anywhere is a fact
    // about their properties, which is why the decision sits here rather than
    // inside the kernel chooser.
    let geo_points: Vec<Option<geo::LonLat>> = scene.nodes.iter().map(|n| n.geo).collect();
    let use_geo = match request.kernel {
        LayoutKernel::Geo => true,
        LayoutKernel::Auto => geo::auto_eligible(&geo_points),
        _ => false,
    };
    let plan = choose_plan(request.kernel, count, &links, seed_hint.as_slice());
    let nodes: Vec<layout::LayoutNode> = scene
        .nodes
        .iter()
        .map(|n| layout::LayoutNode { radius: n.radius })
        .collect();
    let canvas = layout::Canvas {
        width: f64::from(DEFAULT_WIDTH),
        height: f64::from(DEFAULT_HEIGHT),
        // No status block on this canvas: nothing is being drawn, so there is
        // no reserved strip for a layout to stay out from under.
        reserved_top: 0.0,
    };
    let groups = super::arc_groups(&scene);

    let started = std::time::Instant::now();
    let positions = if use_geo {
        // The coastline stays behind. cosmos.gl draws points and links and has
        // no background layer, so what crosses the wire here is what crosses it
        // for every other kernel: one position per slot. The map picture — coast,
        // graticule, tray boundary — is the static render's, and the picker says
        // so in as many words.
        geo::layout(&geo_points, &nodes, canvas)?.0
    } else {
        match &plan {
            structure::Plan::Radial { seed } => {
                layout::radial(&nodes, &links, *seed, &groups, canvas)?
            }
            structure::Plan::Islands { community, count } => layout::islands(
                &nodes,
                &links,
                community,
                *count,
                &groups,
                canvas,
                LAYOUT_SEED,
            )?,
            structure::Plan::Force => layout::run(&nodes, &links, canvas, LAYOUT_SEED)?,
        }
    };
    let layout_ms = started.elapsed().as_secs_f64() * 1_000.0;

    // NaN everywhere first: every slot the scene did not carry is a tombstone,
    // and absence is the server's call (D4). Filling only the live slots into a
    // zeroed array would stack every collapsed node on the origin.
    let mut points = vec![f32::NAN; slot_count as usize * 2];
    let (half_width, half_height) = (canvas.width / 2.0, canvas.height / 2.0);
    let mut live_count = 0u32;
    for (index, node) in scene.nodes.iter().enumerate() {
        let Some((x, y)) = positions.xy.get(index) else {
            continue;
        };
        let at = node.slot as usize * 2;
        // A scene node whose slot is past the array is not reachable — both
        // come from the same read of the same view — but the write would be a
        // panic rather than a wrong picture, so it is bounded rather than
        // asserted.
        if at + 1 >= points.len() {
            continue;
        }
        points[at] = (x - half_width) as f32;
        points[at + 1] = (y - half_height) as f32;
        live_count += 1;
    }

    Ok(LayoutResult {
        meta: LayoutMeta {
            protocol_version: PROTOCOL_VERSION,
            kernel_requested: request.kernel,
            kernel_chosen: if use_geo {
                LayoutKernel::Geo
            } else {
                kernel_of(&plan)
            },
            seed_slot: match &plan {
                structure::Plan::Radial { seed } if !use_geo => {
                    scene.nodes.get(*seed).map(|node| node.slot)
                }
                _ => None,
            },
            slot_count,
            live_count,
            layout_ms,
        },
        points,
    })
}

/// Seed handed to the two kernels that take one.
///
/// Fixed rather than caller-supplied: `seed` reaches only the initial
/// placement's jitter (`super::layout::run`), so varying it varies the
/// arrangement of the *same* graph, and a picture that reshuffles every time a
/// user re-picks the same kernel is a picture they cannot compare with the one
/// they just saw. The headless render exposes `--seed` for exactly the opposite
/// reason: there, a second arrangement of the same data is the feature.
const LAYOUT_SEED: u64 = 0;

/// Turn the requested kernel into a plan the layout module can run.
///
/// A forced kernel that has nothing to work with falls back rather than
/// failing — `islands` over a scene with one community, `radial` over an empty
/// scene — and the fallback is reported through
/// [`LayoutMeta::kernel_chosen`], never hidden.
pub(super) fn choose_plan(
    kernel: LayoutKernel,
    count: usize,
    links: &[(usize, usize)],
    seed_hint: &[usize],
) -> structure::Plan {
    // One hint is a centre the caller named; more than one is an expansion from
    // a whole type, which names no centre. Same rule `structure::plan` states.
    let one_hint = match seed_hint {
        [seed] if *seed < count => Some(*seed),
        _ => None,
    };
    match kernel {
        // `Geo` is decided by the geographic reader, which has data this
        // function does not; `Simulation` returns before the live path reaches
        // here. Both fall through to structure so a caller that got neither
        // still gets an arrangement rather than a panic.
        LayoutKernel::Auto | LayoutKernel::Geo | LayoutKernel::Simulation => {
            structure::plan(count, links, seed_hint)
        }
        LayoutKernel::Force => structure::Plan::Force,
        LayoutKernel::Radial => match radial_seed(count, links, one_hint) {
            Some(seed) => structure::Plan::Radial { seed },
            None => structure::Plan::Force,
        },
        LayoutKernel::Islands => {
            if count == 0 {
                return structure::Plan::Force;
            }
            let community = structure::louvain(count, links);
            let groups = community.iter().copied().max().map(|m| m + 1).unwrap_or(0);
            if groups < 2 {
                // Nothing to pack. `islands` over one community is the force
                // layout with a box drawn around it, so run the force layout
                // and say that is what happened.
                return structure::Plan::Force;
            }
            structure::Plan::Islands {
                community,
                count: groups,
            }
        }
    }
}

/// The node a forced radial layout should be centred on.
///
/// The caller's hint wins. Without one, the structure pass's own ego centre is
/// next — it is the node this scene would have been centred on anyway — and
/// the busiest node is the last resort, because a radial layout with no centre
/// is not a radial layout at all.
fn radial_seed(count: usize, links: &[(usize, usize)], seed_hint: Option<usize>) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if let Some(seed) = seed_hint {
        return Some(seed);
    }
    if let structure::Plan::Radial { seed } = structure::plan(count, links, &[]) {
        return Some(seed);
    }
    let degree = structure::degrees(count, links);
    // Ties break on the lower index, like everything else in the layout path:
    // two runs over the same scene must centre on the same node.
    degree
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(index, _)| index)
}

/// The kernel name a plan corresponds to.
pub(super) fn kernel_of(plan: &structure::Plan) -> LayoutKernel {
    match plan {
        structure::Plan::Radial { .. } => LayoutKernel::Radial,
        structure::Plan::Islands { .. } => LayoutKernel::Islands,
        structure::Plan::Force => LayoutKernel::Force,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring: node 0 in the middle, ten leaves hanging off it.
    fn star(leaves: usize) -> (usize, Vec<(usize, usize)>) {
        (leaves + 1, (1..=leaves).map(|i| (0, i)).collect())
    }

    #[test]
    fn a_forced_kernel_is_the_kernel_that_runs() {
        let (count, links) = star(10);
        assert_eq!(
            kernel_of(&choose_plan(LayoutKernel::Force, count, &links, &[])),
            LayoutKernel::Force,
            "a star would have been chosen as radial by `auto`, and `force` overrides that"
        );
        assert_eq!(
            kernel_of(&choose_plan(LayoutKernel::Auto, count, &links, &[])),
            LayoutKernel::Radial
        );
    }

    #[test]
    fn a_seed_hint_decides_the_centre() {
        let (count, links) = star(10);
        let structure::Plan::Radial { seed } =
            choose_plan(LayoutKernel::Radial, count, &links, &[7])
        else {
            panic!("a forced radial must produce a radial plan");
        };
        assert_eq!(seed, 7, "the caller named the centre and it was taken");
    }

    #[test]
    fn islands_over_one_community_falls_back_and_says_force() {
        // The honesty case: a caller can ask for a grouping a scene does not
        // have, and the answer must not be "islands" over a picture with one.
        let (count, links) = star(20);
        assert_eq!(
            kernel_of(&choose_plan(LayoutKernel::Islands, count, &links, &[])),
            LayoutKernel::Force
        );
    }

    #[test]
    fn an_empty_scene_never_panics_on_any_kernel() {
        for kernel in [
            LayoutKernel::Auto,
            LayoutKernel::Radial,
            LayoutKernel::Islands,
            LayoutKernel::Force,
        ] {
            assert_eq!(
                kernel_of(&choose_plan(kernel, 0, &[], &[])),
                LayoutKernel::Force,
                "{kernel:?} over an empty scene"
            );
        }
    }
}
