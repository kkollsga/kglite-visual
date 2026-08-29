//! What the layout reads about a scene before it places anything (P11).
//!
//! **The complaint this module answers, verbatim:** "looks chaotic, no visible
//! structures or branches — like a 3D graph on a 2D image". One force layout
//! over every input is what produces that. A force layout is the right tool for
//! a graph with no discoverable shape and the wrong one for a star, a bipartite
//! result, or a schema with disconnected islands — and all three of those are
//! what this project's real inputs mostly are. So the layout is chosen from the
//! scene's structure rather than fixed, and this module is where the structure
//! is measured.
//!
//! **Everything here is deterministic and libm-free.** Accumulation runs in
//! index order, never over a hash map; ties break on the lower index. The same
//! discipline `super::layout` runs under, for the same reason: a golden SVG is
//! an exact baseline.
//!
//! **Local Louvain, not `kglite::api::algorithms::louvain_communities`, and the
//! reason is the meta-graph.** kglite 0.16.14 does expose modularity community
//! detection (`louvain_communities` / `leiden_communities`, with a
//! `scope: Option<&HashSet<NodeIndex>>` that could be handed a slice's node
//! set). It operates on kglite's *node* space — and the meta-graph, which is
//! half of what this project draws, has no nodes in that space at all: its
//! vertices are type names and its edges are aggregated counts. One of the two
//! sources this module serves is therefore unreachable from the engine call,
//! and running two different partitioners would make "which grouping am I
//! looking at" depend on the request. The partition also has to be of the
//! *drawn* subgraph — the ≤5 000 nodes the bound admitted — not of the graph
//! they came from, and at that size a local pass costs microseconds.

/// Below this, a scene is too small for grouping to say anything: three islands
/// of two nodes is not a structure, it is a spaced-out list.
const MIN_NODES_FOR_ISLANDS: usize = 12;

/// A community holding this fraction or more of the scene is not a grouping —
/// it is the scene, and packing it as "one island" would be the force layout
/// with extra steps.
const DOMINANT_COMMUNITY_SHARE: f64 = 0.92;

/// A node needs this many neighbours before it can be read as an ego centre.
const MIN_EGO_DEGREE: usize = 6;

/// …and this share of the scene's other nodes must hang directly off it.
///
/// 0.6 rather than something near 1.0 so a star whose leaves are themselves
/// linked — sodir's wellbore neighbourhood, where the far ends share licences —
/// still reads as the star it is.
const EGO_COVERAGE: f64 = 0.6;

/// How a scene is laid out.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// One node at the centre, everything else on hop rings around it.
    Radial { seed: usize },
    /// Communities laid out separately and packed, so a group reads as a group.
    Islands { community: Vec<usize>, count: usize },
    /// The generic force layout — the fallback for input with no shape to find.
    Force,
}

/// Choose a layout for `count` nodes under `links`.
///
/// `seed_hint` is what the *request* said its origin was (an expansion's seed
/// type, a node the caller centred a query on). A hint of exactly one node is
/// taken at its word; anything else falls through to structure, because a
/// request that names 900 origins has not named a centre.
pub fn plan(count: usize, links: &[(usize, usize)], seed_hint: &[usize]) -> Plan {
    if count == 0 {
        return Plan::Force;
    }
    if let [seed] = seed_hint {
        if *seed < count {
            return Plan::Radial { seed: *seed };
        }
    }
    if let Some(seed) = ego_centre(count, links) {
        return Plan::Radial { seed };
    }
    if count < MIN_NODES_FOR_ISLANDS {
        return Plan::Force;
    }
    let community = louvain(count, links);
    let groups = community.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    if groups < 2 {
        return Plan::Force;
    }
    let mut sizes = vec![0usize; groups];
    for c in &community {
        sizes[*c] += 1;
    }
    let largest = sizes.iter().copied().max().unwrap_or(0);
    if (largest as f64) >= DOMINANT_COMMUNITY_SHARE * count as f64 {
        return Plan::Force;
    }
    Plan::Islands {
        community,
        count: groups,
    }
}

/// Degree of every node, counting each link once per endpoint.
///
/// Self-loops and out-of-range endpoints are skipped rather than counted: a
/// link the layout will not draw must not decide the layout either.
pub fn degrees(count: usize, links: &[(usize, usize)]) -> Vec<usize> {
    let mut degree = vec![0usize; count];
    for (a, b) in links {
        if *a >= count || *b >= count || a == b {
            continue;
        }
        degree[*a] += 1;
        degree[*b] += 1;
    }
    degree
}

/// Undirected adjacency, each list in ascending index order.
///
/// Sorted rather than in insertion order because every consumer here walks it
/// and sums or compares as it goes; insertion order is the caller's link order,
/// and a layout that depended on that would move when a query's row order did.
pub fn adjacency(count: usize, links: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); count];
    for (a, b) in links {
        if *a >= count || *b >= count || a == b {
            continue;
        }
        out[*a].push(*b);
        out[*b].push(*a);
    }
    for list in out.iter_mut() {
        list.sort_unstable();
        list.dedup();
    }
    out
}

/// The one node everything hangs off, if there is one.
///
/// The test is deliberately structural rather than "highest degree wins": a
/// dense expansion has a highest-degree node too, and centring it would draw a
/// hairball with one node in the middle of it. What makes a star a star is that
/// removing the centre disconnects nearly everything.
fn ego_centre(count: usize, links: &[(usize, usize)]) -> Option<usize> {
    if count < MIN_EGO_DEGREE + 1 {
        return None;
    }
    let degree = degrees(count, links);
    let mut best = 0usize;
    for i in 1..count {
        // Strictly greater, so an exact tie keeps the lower index and the
        // choice stays a function of the input.
        if degree[i] > degree[best] {
            best = i;
        }
    }
    if degree[best] < MIN_EGO_DEGREE {
        return None;
    }
    let neighbours = adjacency(count, links);
    let direct = neighbours[best].len();
    if (direct as f64) < EGO_COVERAGE * (count - 1) as f64 {
        return None;
    }
    // A second node of the same degree means two centres, and a picture with
    // two centres drawn as one is a lie about the shape.
    if (0..count).filter(|i| degree[*i] == degree[best]).count() > 1 {
        return None;
    }
    Some(best)
}

/// Smallest island a two-shell arrangement says anything about.
///
/// Under this a shell is a circle of eight dots with a circle of two inside it,
/// which is a worse picture than the force layout it replaced.
const MIN_NODES_FOR_SHELLS: usize = 10;

/// Smallest either side of a two-shell arrangement may be.
///
/// Two nodes on the inner shell is a pair with a halo, not a shell — and the
/// star it usually means is [`Plan::Radial`]'s job, which is tested first.
const MIN_SHELL_CLASS: usize = 3;

/// The two sides of a bipartite island, smaller side first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bipartition {
    /// The inner shell — the smaller class, which is the hub side in every
    /// real case here (companies to discoveries, fields to wellbores).
    pub inner: Vec<usize>,
    pub outer: Vec<usize>,
}

/// Two classes with every edge between them, if the scene has exactly that
/// shape (P11 round 3).
///
/// **The blob this answers has a name.** A Louvain community that is not a star
/// falls through to the force layout, and a force layout over a *dense* community
/// is a disc of dots: the coordinator's round-2 verdict on the discovery /
/// licensee render's left island, and on the meta-graph's two densest families.
/// A reader can count the dots and learn nothing else. But most of those blobs
/// are not shapeless — they are two kinds of thing joined only to each other,
/// which is the commonest shape in this data set (a discovery has licensees, a
/// wellbore has a field, a licence has areas), and two kinds of thing drawn as
/// two concentric shells reads as two kinds of thing at a glance.
///
/// **Exact, not approximate.** A single odd cycle refuses the whole partition
/// rather than being tolerated as noise: "these are the two sides" is a claim
/// about the data, and a near-bipartite graph drawn as bipartite puts some
/// members on the shell they do not belong to with nothing in the picture saying
/// which. The force fallback is the honest answer for anything that fails here.
///
/// Three further refusals, each because the picture would be worse than the one
/// it replaces: a scene under [`MIN_NODES_FOR_SHELLS`]; a class under
/// [`MIN_SHELL_CLASS`]; and a scene with fewer edges than nodes, which is a
/// forest — every forest is bipartite, and a forest's shape is its branching,
/// not its two sides.
pub fn bipartition(count: usize, links: &[(usize, usize)]) -> Option<Bipartition> {
    if count < MIN_NODES_FOR_SHELLS {
        return None;
    }
    let neighbours = adjacency(count, links);
    let edges: usize = neighbours.iter().map(|list| list.len()).sum::<usize>() / 2;
    if edges < count {
        return None;
    }
    // An unattached node has no side, and colouring it 0 would file it under a
    // class it has no relationship to.
    if neighbours.iter().any(|list| list.is_empty()) {
        return None;
    }

    const UNCOLOURED: u8 = u8::MAX;
    let mut colour = vec![UNCOLOURED; count];
    for start in 0..count {
        if colour[start] != UNCOLOURED {
            continue;
        }
        colour[start] = 0;
        let mut frontier = vec![start];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for node in frontier {
                for peer in &neighbours[node] {
                    if colour[*peer] == UNCOLOURED {
                        colour[*peer] = 1 - colour[node];
                        next.push(*peer);
                    } else if colour[*peer] == colour[node] {
                        return None;
                    }
                }
            }
            // Ascending, so the walk visits in index order and the colouring is
            // a function of the input rather than of a frontier's push order.
            next.sort_unstable();
            frontier = next;
        }
    }

    let first: Vec<usize> = (0..count).filter(|i| colour[*i] == 0).collect();
    let second: Vec<usize> = (0..count).filter(|i| colour[*i] == 1).collect();
    if first.len() < MIN_SHELL_CLASS || second.len() < MIN_SHELL_CLASS {
        return None;
    }
    // The smaller class goes inside: it is the one whose members are shared, so
    // the spokes fan outward from it, and an exact tie keeps the class holding
    // node 0 so the choice stays a function of the input.
    let (inner, outer) = if second.len() < first.len() {
        (second, first)
    } else {
        (first, second)
    };
    Some(Bipartition { inner, outer })
}

/// Hop distance from `seed`, over undirected links.
///
/// A node the walk cannot reach — a query result carrying an unconnected
/// component — is given `unreachable`, one past the deepest real ring, so it is
/// drawn outside the structure rather than folded into it.
pub fn hops(count: usize, links: &[(usize, usize)], seed: usize) -> (Vec<u32>, Vec<Option<usize>>) {
    let neighbours = adjacency(count, links);
    let mut hop = vec![u32::MAX; count];
    let mut parent: Vec<Option<usize>> = vec![None; count];
    if seed >= count {
        return (hop, parent);
    }
    hop[seed] = 0;
    let mut frontier = vec![seed];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for node in frontier {
            for peer in &neighbours[node] {
                if hop[*peer] != u32::MAX {
                    continue;
                }
                hop[*peer] = hop[node] + 1;
                parent[*peer] = Some(node);
                next.push(*peer);
            }
        }
        // Ascending, so the next level is visited in index order and the
        // parent a node ends up with is the lowest-indexed candidate, always.
        next.sort_unstable();
        frontier = next;
    }
    let deepest = hop
        .iter()
        .filter(|h| **h != u32::MAX)
        .max()
        .copied()
        .unwrap_or(0);
    for h in hop.iter_mut() {
        if *h == u32::MAX {
            *h = deepest + 1;
        }
    }
    (hop, parent)
}

/// Multilevel Louvain modularity optimisation over an unweighted graph.
///
/// Standard algorithm, with every source of nondeterminism removed: nodes are
/// visited in index order, community weights are accumulated into a dense
/// scratch vector rather than a map, and an exact-tie gain keeps the lower
/// community id. Resolution is fixed at 1.0 — a knob nobody can see the effect
/// of on an image is a knob that gets set wrong.
pub fn louvain(count: usize, links: &[(usize, usize)]) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    let mut edges: Vec<(usize, usize, f64)> = Vec::with_capacity(links.len());
    for (a, b) in links {
        if *a >= count || *b >= count || a == b {
            continue;
        }
        edges.push((*a, *b, 1.0));
    }
    let mut mapping: Vec<usize> = (0..count).collect();
    let mut level_nodes = count;
    // Four levels is past where a graph of this size stops coarsening; the loop
    // exits on "no node moved" long before, and the cap is only here so a
    // pathological weight pattern cannot spin.
    for _ in 0..4 {
        let mut part = one_level(level_nodes, &edges);
        let groups = renumber(&mut part);
        if groups == level_nodes {
            break;
        }
        for m in mapping.iter_mut() {
            *m = part[*m];
        }
        edges = aggregate(&part, groups, &edges);
        level_nodes = groups;
        if groups == 1 {
            break;
        }
    }
    let mut out = mapping;
    renumber(&mut out);
    out
}

/// One Louvain level: local moving until nothing improves.
fn one_level(count: usize, edges: &[(usize, usize, f64)]) -> Vec<usize> {
    let mut adjacency: Vec<Vec<(usize, f64)>> = vec![Vec::new(); count];
    let mut degree = vec![0.0f64; count];
    let mut self_loops = vec![0.0f64; count];
    let mut total = 0.0f64;
    for (a, b, w) in edges {
        total += 2.0 * w;
        if a == b {
            self_loops[*a] += *w;
            degree[*a] += 2.0 * w;
            continue;
        }
        adjacency[*a].push((*b, *w));
        adjacency[*b].push((*a, *w));
        degree[*a] += *w;
        degree[*b] += *w;
    }
    if total <= 0.0 {
        return (0..count).collect();
    }

    let mut community: Vec<usize> = (0..count).collect();
    let mut community_total = degree.clone();
    // Dense scratch + a touched list: a map would iterate in an order the hash
    // seed decides, and the best-gain comparison below reads that order.
    let mut weights = vec![0.0f64; count];
    let mut touched: Vec<usize> = Vec::new();

    for _pass in 0..24 {
        let mut moved = false;
        for node in 0..count {
            let own = community[node];
            for c in touched.drain(..) {
                weights[c] = 0.0;
            }
            for (peer, w) in &adjacency[node] {
                let c = community[*peer];
                if weights[c] == 0.0 {
                    touched.push(c);
                }
                weights[c] += *w;
            }
            community_total[own] -= degree[node];
            let stay = weights[own] - community_total[own] * degree[node] / total;
            let mut best = own;
            let mut best_gain = stay;
            // Ascending, so an exact tie keeps the lowest community id.
            touched.sort_unstable();
            for c in &touched {
                if *c == own {
                    continue;
                }
                let gain = weights[*c] - community_total[*c] * degree[node] / total;
                if gain > best_gain {
                    best_gain = gain;
                    best = *c;
                }
            }
            community_total[best] += degree[node];
            if best != own {
                community[node] = best;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    community
}

/// Renumber a partition into `0..k` by first appearance; returns `k`.
fn renumber(part: &mut [usize]) -> usize {
    let mut seen: Vec<Option<usize>> = vec![None; part.len()];
    let mut next = 0usize;
    for slot in part.iter_mut() {
        // A partition is always over its own index space, so `*slot` indexes
        // `seen`; an aggregate level's ids are renumbered before they are used.
        let id = seen[*slot].unwrap_or_else(|| {
            let id = next;
            next += 1;
            id
        });
        seen[*slot] = Some(id);
        *slot = id;
    }
    next
}

/// Collapse each community into one node and sum the edges between them.
fn aggregate(
    part: &[usize],
    groups: usize,
    edges: &[(usize, usize, f64)],
) -> Vec<(usize, usize, f64)> {
    let mut dense = vec![0.0f64; groups * groups];
    for (a, b, w) in edges {
        let (ca, cb) = (part[*a], part[*b]);
        let (lo, hi) = if ca <= cb { (ca, cb) } else { (cb, ca) };
        dense[lo * groups + hi] += *w;
    }
    let mut out = Vec::new();
    for lo in 0..groups {
        for hi in lo..groups {
            let w = dense[lo * groups + hi];
            if w > 0.0 {
                out.push((lo, hi, w));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_triangles() -> (usize, Vec<(usize, usize)>) {
        (
            6,
            vec![(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (2, 3)],
        )
    }

    #[test]
    fn louvain_separates_two_triangles_joined_by_one_edge() {
        // The textbook fixture. If this comes back as one community the
        // island layout has nothing to pack and the whole path is inert.
        let (count, links) = two_triangles();
        let part = louvain(count, &links);
        assert_eq!(part[0], part[1]);
        assert_eq!(part[0], part[2]);
        assert_eq!(part[3], part[4]);
        assert_eq!(part[3], part[5]);
        assert_ne!(part[0], part[3]);
    }

    #[test]
    fn the_partition_is_a_pure_function_of_the_input() {
        let (count, links) = two_triangles();
        assert_eq!(louvain(count, &links), louvain(count, &links));
        // And of the input's *content*, not of the order it arrived in.
        let mut reversed = links.clone();
        reversed.reverse();
        assert_eq!(louvain(count, &links), louvain(count, &reversed));
    }

    #[test]
    fn a_star_is_planned_radially_and_a_blob_is_not() {
        let star: Vec<(usize, usize)> = (1..20).map(|i| (0, i)).collect();
        assert_eq!(plan(20, &star, &[]), Plan::Radial { seed: 0 });

        // A ring has no centre: every node has degree 2. Centring one would be
        // a claim about the data that the data does not make.
        let ring: Vec<(usize, usize)> = (0..20).map(|i| (i, (i + 1) % 20)).collect();
        assert!(!matches!(plan(20, &ring, &[]), Plan::Radial { .. }));
    }

    #[test]
    fn a_single_seed_hint_wins_over_structure() {
        // The request said where the centre is. A dense expansion from one node
        // may not look like a star to `ego_centre`, and the caller still asked
        // for that node's neighbourhood.
        let dense: Vec<(usize, usize)> = (0..20)
            .flat_map(|i| ((i + 1)..20).map(move |j| (i, j)))
            .collect();
        assert_eq!(plan(20, &dense, &[7]), Plan::Radial { seed: 7 });
        assert!(!matches!(plan(20, &dense, &[]), Plan::Radial { .. }));
    }

    #[test]
    fn disconnected_groups_plan_as_islands() {
        let mut links = Vec::new();
        for group in 0..4 {
            let base = group * 5;
            for i in 0..5 {
                for j in (i + 1)..5 {
                    links.push((base + i, base + j));
                }
            }
        }
        let Plan::Islands { count, community } = plan(20, &links, &[]) else {
            panic!("four cliques are four islands");
        };
        assert_eq!(count, 4);
        assert_eq!(community[0], community[4]);
        assert_ne!(community[0], community[5]);
    }

    /// `hubs` inner nodes, `leaves` outer ones, every outer joined to every
    /// inner — the discovery / licensee shape.
    fn complete_bipartite(hubs: usize, leaves: usize) -> Vec<(usize, usize)> {
        (0..hubs)
            .flat_map(|h| (0..leaves).map(move |l| (h, hubs + l)))
            .collect()
    }

    #[test]
    fn a_two_sided_scene_is_found_and_the_smaller_side_goes_inside() {
        let part = bipartition(4 + 20, &complete_bipartite(4, 20))
            .expect("a complete bipartite graph is bipartite");
        assert_eq!(part.inner, vec![0, 1, 2, 3], "the four hubs go inside");
        assert_eq!(part.outer.len(), 20);
    }

    #[test]
    fn one_odd_cycle_refuses_the_whole_partition() {
        // The refusal is the point: "these are the two sides" is a claim about
        // the data, and a near-bipartite graph drawn as bipartite puts members
        // on the wrong shell with nothing in the picture saying which.
        let mut links = complete_bipartite(4, 20);
        links.push((0, 1));
        assert_eq!(bipartition(24, &links), None);
    }

    #[test]
    fn a_forest_and_a_lopsided_split_are_both_refused() {
        // Every forest is bipartite, and a forest's shape is its branching.
        let path: Vec<(usize, usize)> = (0..19).map(|i| (i, i + 1)).collect();
        assert_eq!(bipartition(20, &path), None, "a path is a forest");
        // Two hubs is a pair with a halo, not a shell.
        assert_eq!(
            bipartition(2 + 20, &complete_bipartite(2, 20)),
            None,
            "a class of two is not a shell"
        );
    }

    #[test]
    fn hops_ring_outward_and_park_the_unreachable_past_the_last_ring() {
        // 0 - 1 - 2, and 3 attached to nothing.
        let (hop, parent) = hops(4, &[(0, 1), (1, 2)], 0);
        assert_eq!(hop[0], 0);
        assert_eq!(hop[1], 1);
        assert_eq!(hop[2], 2);
        assert_eq!(hop[3], 3, "unreachable sits one ring past the deepest");
        assert_eq!(parent[2], Some(1));
        assert_eq!(parent[0], None);
    }
}
