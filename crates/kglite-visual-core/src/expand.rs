//! Bounded neighbourhood expansion, and the preview that makes the bound an
//! informed choice (plan D5 + D12).
//!
//! **Preview first.** A user who expands `KNOWS` on a type with 180 of them
//! wanted 180; a user who expands `CITES` on a type with 4 million did not, and
//! finding out by watching a bound fire is a surprise, not an answer. So every
//! selection can be asked what an expansion *would* add, per relationship type
//! and direction, before anything is fetched:
//!
//! - **type level** — from `get_or_compute_type_connectivity()`, the
//!   cardinality cache persisted inside the `.kgl`. O(#triples), no node
//!   access, and already in memory because the meta-graph read it at open.
//!   (`compute_neighbors_schema` answers the same question by walking every
//!   edge of every node of the type, which is O(E) on the type — the same trap
//!   `compute_type_connectivity` is next to. See the deviation note below.)
//! - **node level** — `count_edges_filtered`, per relationship type and
//!   direction, which the disk backend serves from its CSR offsets.
//!
//! **The bound is enforced here, in core.** [`effective_bound`] is the single
//! choke point every expansion goes through, and it clamps: a client asking for
//! `u32::MAX` nodes gets [`MAX_EXPANSION_NODES`] and `truncated: true`, not the
//! nodes. There is deliberately no function in this module that returns an
//! unbounded neighbourhood — not a private one, not one behind a flag. A
//! guarantee the client implements is not a guarantee.

use std::collections::HashSet;
use std::time::Instant;

use kglite::api::{DirGraph, Direction, GraphRead, InternedKey, NodeIndex};
use serde::Serialize;
use ts_rs::TS;

use crate::bound::{Bound, BoundInfo};
use crate::request::EdgeDirection;

/// Hard ceiling on nodes one expansion may add.
///
/// **Provisional; P4 sets the final number** from the measured scale table.
/// The plan's Phase 0 evidence puts cosmos.gl's live simulation at ~20k–50k
/// points on M3-class hardware, so 5 000 leaves an order of magnitude of
/// headroom for several expansions in one session and stays far inside the
/// "render then freeze" regime under SwiftShader, where the e2e runs.
pub const MAX_EXPANSION_NODES: usize = 5_000;

/// Hard ceiling on the serialized size of one expansion's node list.
///
/// The count bound alone is defeated by shape: 5 000 nodes with 8-character
/// titles is 400 KB and 5 000 with a paragraph in the title field is 50 MB.
/// Four protocol chunks' worth.
pub const MAX_EXPANSION_BYTES: usize = 2 * 1024 * 1024;

/// Nodes returned when a client names no limit.
///
/// Below the ceiling on purpose: the default is what a click produces, and a
/// click should not be able to spend the whole budget by accident. A client
/// that wants more asks for more and is still clamped.
pub const DEFAULT_EXPANSION_NODES: usize = 1_000;

/// Serialized-size estimate for one node in a slice.
///
/// Title plus type name plus the JSON scaffolding around slot, node id and the
/// two keys. Deliberately an over-estimate: a byte bound that under-counts is a
/// byte bound that does not hold.
pub(crate) fn slice_node_bytes(title: &str, node_type: &str) -> usize {
    title.len() + node_type.len() + 96
}

/// The bound one expansion actually runs under.
///
/// **The choke point.** Every expansion path in this crate calls this, and it
/// is total: there is no input, including `None` and `Some(u32::MAX)`, for
/// which it returns a bound above the ceilings above. That property has its own
/// test, because "the bound is enforced in core" is only true while nothing
/// routes around it.
pub fn effective_bound(requested: Option<u32>) -> Bound {
    let asked = requested
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_EXPANSION_NODES);
    Bound {
        max_items: asked.min(MAX_EXPANSION_NODES),
        max_bytes: MAX_EXPANSION_BYTES,
    }
}

/// Where a preview's counts came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum PreviewScope {
    /// Every node of a type, from the persisted cardinality cache. Expanding
    /// this walks the whole type.
    Type,
    /// One node's own edges, counted directly.
    Node,
}

/// What expanding one relationship type would add.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RelationshipPreview {
    /// Relationship type.
    pub name: String,
    /// `out` or `in`. Never `both`: a preview that summed the two would hide
    /// which direction the edges actually run, and direction is half of what
    /// the user is choosing.
    pub direction: EdgeDirection,
    /// Node type on the far end.
    pub other_type: String,
    /// Edges of this (type, direction, far type) triple. **Edges, not nodes** —
    /// the node count after deduplication is at most this and usually less, so
    /// this is an upper bound on what an expansion adds, and the UI says so.
    pub count: u32,
}

/// The answer to "what would expanding this add?".
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ExpansionPreview {
    pub protocol_version: u32,
    pub slot: u32,
    pub scope: PreviewScope,
    /// The type node's name, or the instance node's type.
    pub node_type: String,
    /// The instance node's title. Empty for a type-level preview.
    pub title: String,
    /// Largest first — the order the buttons are drawn in, and the order the
    /// bound would keep.
    pub relationships: Vec<RelationshipPreview>,
    /// Sum of `relationships[].count`: the whole neighbourhood, as an edge
    /// count.
    pub total_edges: u32,
    /// The ceiling any expansion from here will run under, so the UI can say
    /// "this would be truncated" before the click rather than after.
    pub max_nodes: u32,
}

/// Per-relationship counts for every node of `node_type`.
///
/// **Deviation from D12, recorded here rather than in a plan file** (a plan
/// file is gitignored; this comment ships): D12 named
/// `compute_neighbors_schema` for the type level. That function walks every
/// edge of every node of the type — O(E) on the type — to produce exactly the
/// `(relationship, other type, count)` triples that
/// `get_or_compute_type_connectivity()` already holds, persisted in the `.kgl`
/// and read at session open. Using the cache gives the same numbers in
/// O(#triples) with no node access, which is the property the whole
/// progressive-disclosure entry screen rests on. `compute_neighbors_schema`
/// stays the right answer for a graph whose cache is absent; kglite fills the
/// cache on load (see the boundary doc's finding 1), so that case does not
/// arise here.
pub fn preview_for_type(graph: &DirGraph, node_type: &str) -> Vec<RelationshipPreview> {
    let mut out = Vec::new();
    for triple in graph.get_or_compute_type_connectivity() {
        if triple.count == 0 {
            // kglite derives all-zero triples for a `.kgl` saved without a
            // cardinality cache (boundary doc, finding 1). A preview button
            // promising zero edges is worse than no button.
            continue;
        }
        if triple.src == node_type {
            out.push(RelationshipPreview {
                name: triple.conn.clone(),
                direction: EdgeDirection::Out,
                other_type: triple.tgt.clone(),
                count: triple.count as u32,
            });
        }
        if triple.tgt == node_type {
            out.push(RelationshipPreview {
                name: triple.conn.clone(),
                direction: EdgeDirection::In,
                other_type: triple.src.clone(),
                count: triple.count as u32,
            });
        }
    }
    sort_previews(&mut out);
    out
}

/// Per-relationship counts for one node.
///
/// The candidate `(relationship, far type)` pairs come from the same
/// connectivity cache — that is what keeps this O(#triples) calls to
/// `count_edges_filtered` rather than a walk over the node's whole adjacency
/// for every relationship type in the graph.
pub fn preview_for_node(
    graph: &DirGraph,
    node: NodeIndex,
    node_type: &str,
    deadline: Option<Instant>,
) -> Vec<RelationshipPreview> {
    let mut out = Vec::new();
    for candidate in preview_for_type(graph, node_type) {
        let dir = match candidate.direction {
            EdgeDirection::Out => Direction::Outgoing,
            EdgeDirection::In => Direction::Incoming,
            // `preview_for_type` never emits `both`; see the field's doc.
            EdgeDirection::Both => continue,
        };
        let count = graph
            .graph
            .count_edges_filtered(
                node,
                dir,
                Some(InternedKey::from_str(&candidate.name)),
                Some(InternedKey::from_str(&candidate.other_type)),
                deadline,
            )
            .unwrap_or(0);
        if count == 0 {
            continue;
        }
        out.push(RelationshipPreview {
            count: count as u32,
            ..candidate
        });
    }
    sort_previews(&mut out);
    out
}

/// Largest first; name then far type break ties.
///
/// The tie-break is not cosmetic: the preview list is what the expand buttons
/// are drawn from and what an e2e assertion reads, so an order derived from
/// hash-map iteration would make both flap.
fn sort_previews(previews: &mut [RelationshipPreview]) {
    previews.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.other_type.cmp(&b.other_type))
    });
}

/// One edge an expansion found, in kglite's node space.
///
/// The caller maps these into slots; this module deliberately does not know
/// about the slot space, so the bound is decided by what was *fetched*, not by
/// what happened to be on screen already.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FoundEdge {
    pub source: NodeIndex,
    pub target: NodeIndex,
    pub name: String,
}

/// What one expansion found, already bounded.
pub struct Expansion {
    /// Nodes to add, in discovery order. Deduplicated.
    pub nodes: Vec<NodeIndex>,
    /// Edges among them. Never references a node outside `nodes`.
    pub edges: Vec<FoundEdge>,
    /// What the bound did (D5).
    pub bound: BoundInfo,
}

/// Walk the neighbourhood of `seeds`, bounded.
///
/// `seeds` is one node for an instance expansion and every node of a type for a
/// type-node expansion — the flagship drill-in. Either way the walk stops at
/// the bound, and every edge kept has both endpoints in `nodes`, so the caller
/// can never be handed an index into something it was not sent.
pub fn expand(
    graph: &DirGraph,
    seeds: &[NodeIndex],
    relationship: Option<&str>,
    direction: EdgeDirection,
    bound: Bound,
    deadline: Option<Instant>,
) -> Expansion {
    let conn = relationship.map(|name| InternedKey::from_str(name).as_u64());
    let directions: &[Direction] = match direction {
        EdgeDirection::Out => &[Direction::Outgoing],
        EdgeDirection::In => &[Direction::Incoming],
        EdgeDirection::Both => &[Direction::Outgoing, Direction::Incoming],
    };

    let mut nodes: Vec<NodeIndex> = Vec::new();
    let mut seen: HashSet<NodeIndex> = HashSet::new();
    let mut edges: Vec<FoundEdge> = Vec::new();
    let mut bytes: usize = 0;
    // Every edge the walk saw, whether or not both its endpoints fit. This is
    // the `total` half of the truncation metadata: without it the UI could only
    // say "5 000", which reads as complete.
    let mut total_reachable: HashSet<NodeIndex> = HashSet::new();
    let mut truncated = false;

    // The arena guard the disk backend needs for materialised node reads. A
    // no-op on memory and mapped graphs; leaving it out would grow the query
    // arena for the process lifetime on a disk-backed one.
    let _guard = graph.begin_read_pass();

    let admit = |idx: NodeIndex,
                 nodes: &mut Vec<NodeIndex>,
                 seen: &mut HashSet<NodeIndex>,
                 bytes: &mut usize|
     -> bool {
        if seen.contains(&idx) {
            return true;
        }
        let size = match graph.node_view(idx) {
            Some(view) => slice_node_bytes(
                &crate::values::value_to_display(&view.title()),
                view.node_type_str(&graph.interner),
            ),
            None => slice_node_bytes("", ""),
        };
        if nodes.len() >= bound.max_items || *bytes + size > bound.max_bytes {
            return false;
        }
        seen.insert(idx);
        nodes.push(idx);
        *bytes += size;
        true
    };

    'walk: for seed in seeds {
        if deadline.is_some_and(|dl| Instant::now() > dl) {
            truncated = true;
            break 'walk;
        }
        total_reachable.insert(*seed);
        for dir in directions {
            // Two walks, because the relationship name has two sources. With a
            // named relationship it is known a priori, so `iter_peers_filtered`
            // — which the disk backend serves without materialising EdgeData at
            // all — is the right call. Without one the name has to be read off
            // each edge, and reading it is the whole reason the slower walk
            // exists: a link the client cannot name is a link the user cannot
            // interpret.
            let found: Vec<(NodeIndex, String)> = match relationship {
                Some(name) => graph
                    .graph
                    .iter_peers_filtered(*seed, *dir, conn)
                    .map(|(peer, _)| (peer, name.to_string()))
                    .collect(),
                None => graph
                    .graph
                    .edges_directed_filtered(*seed, *dir, None)
                    .map(|er| {
                        let peer = match dir {
                            Direction::Outgoing => er.target(),
                            Direction::Incoming => er.source(),
                        };
                        // `connection_type()`, not `weight().connection_type`:
                        // the disk backend reads the former from its CSR
                        // endpoint table and materialises `EdgeData` only for
                        // the latter, which this walk never needs.
                        (
                            peer,
                            graph.interner.resolve(er.connection_type()).to_string(),
                        )
                    })
                    .collect(),
            };

            for (peer, name) in found {
                total_reachable.insert(peer);
                // The seed is admitted lazily, alongside its first peer: a seed
                // with no matching edges is not part of this expansion's
                // answer, and admitting it eagerly would spend the bound on
                // isolated nodes before reaching the connected ones.
                if !admit(*seed, &mut nodes, &mut seen, &mut bytes)
                    || !admit(peer, &mut nodes, &mut seen, &mut bytes)
                {
                    truncated = true;
                    // Keep walking: `total_reachable` is what makes "showing X
                    // of Y" true, and stopping here would make Y a lie.
                    continue;
                }
                let (source, target) = match dir {
                    Direction::Outgoing => (*seed, peer),
                    Direction::Incoming => (peer, *seed),
                };
                edges.push(FoundEdge {
                    source,
                    target,
                    name,
                });
            }
        }
    }

    // Dedup after the walk rather than during it: `Both` reaches a
    // reciprocated edge twice, and a per-insert containment check over a Vec is
    // quadratic in the bound.
    edges.sort_unstable();
    edges.dedup();

    let info = BoundInfo {
        returned: nodes.len() as u32,
        total: total_reachable.len() as u32,
        truncated: truncated || nodes.len() < total_reachable.len(),
    };
    Expansion {
        nodes,
        edges,
        bound: info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_request_can_ask_for_an_unbounded_expansion() {
        // The R1 property this module is built around: there is no input for
        // which the choke point returns a bound above the ceiling. If a future
        // change adds a "no limit" escape hatch, this is what refuses it.
        for requested in [
            None,
            Some(0),
            Some(1),
            Some(DEFAULT_EXPANSION_NODES as u32),
            Some(MAX_EXPANSION_NODES as u32),
            Some(MAX_EXPANSION_NODES as u32 + 1),
            Some(u32::MAX),
        ] {
            let bound = effective_bound(requested);
            assert!(
                bound.max_items <= MAX_EXPANSION_NODES,
                "{requested:?} produced max_items {}",
                bound.max_items
            );
            assert!(bound.max_bytes <= MAX_EXPANSION_BYTES);
        }
    }

    #[test]
    fn an_absent_limit_is_the_default_not_the_ceiling() {
        assert_eq!(effective_bound(None).max_items, DEFAULT_EXPANSION_NODES);
        const {
            assert!(
                DEFAULT_EXPANSION_NODES < MAX_EXPANSION_NODES,
                "a default at the ceiling makes the ceiling untestable from a request"
            )
        };
    }

    #[test]
    fn a_request_under_the_ceiling_is_honoured_exactly() {
        assert_eq!(effective_bound(Some(7)).max_items, 7);
    }

    #[test]
    fn the_ceiling_clamps_rather_than_rejecting() {
        // Clamp, not error: a client asking for too much wants as much as it
        // can have, and `truncated: true` is how it learns it did not get it.
        assert_eq!(
            effective_bound(Some(u32::MAX)).max_items,
            MAX_EXPANSION_NODES
        );
    }
}
