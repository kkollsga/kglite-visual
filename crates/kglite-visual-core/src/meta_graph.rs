//! The type-level meta-graph — the entry screen (plan D12).
//!
//! Progressive disclosure is the product: kglite's disk mode reaches 100M+
//! nodes and no browser renders that, so the first thing a user sees is the
//! *schema* — one node per label, one link per relationship type, both
//! carrying counts.
//!
//! **Cheap by construction.** Node counts come from `graph.type_indices`
//! (O(#types); the engine maintains it and restores it on load) and the links
//! from `DirGraph::get_or_compute_type_connectivity()`, which reads the
//! cardinality cache persisted inside the `.kgl`. The similarly-named
//! `kglite::api::introspection::compute_type_connectivity` is the O(E) scan
//! that *fills* that cache — calling it here would walk every edge of a 100M
//! node graph to draw a dozen circles.

use std::collections::HashMap;

use kglite::api::{DirGraph, GraphRead};
use serde::Serialize;
use ts_rs::TS;

use crate::bound::{self, Bound, BoundInfo};
use crate::layout;
use crate::protocol::PROTOCOL_VERSION;
use crate::view::{View, ViewEdge};

/// How much of the schema the server decided to send.
///
/// **Mirrored from kglite, which does not export it.** `GraphScale` and
/// `graph_scale()` live in `crates/kglite/src/graph/introspection/mod.rs`
/// (kglite 0.16.13), inside the `pub(crate) mod graph` that the curated
/// `kglite::api` facade seals — `api::introspection` re-exports the detail
/// enums and `SchemaOverview` but not the scale classifier. The thresholds
/// below are that function's, verbatim, and the duplication is deliberate:
/// this crate cannot name the upstream type. Read kglite's CHANGELOG on every
/// floor bump and re-check these four ranges.
///
/// The classification counts **core** types only — a type with a parent in
/// `parent_types` is a supporting type and does not push a graph into a
/// coarser tier, exactly as upstream does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum DetailTier {
    /// 0–15 core types: every type, with its links.
    Full,
    /// 16–200 core types: every type, without inline property detail.
    Compact,
    /// 201–5000 core types: the [`TOP_TYPES_LIMIT`] largest, truncation
    /// reported.
    TopTypes,
    /// 5001+ core types: statistics only. A meta-graph of five thousand
    /// labelled circles is not a picture of anything, so the client renders
    /// the stats panel and navigates by search instead.
    Summary,
}

/// Types kept at the [`DetailTier::TopTypes`] tier.
pub const TOP_TYPES_LIMIT: usize = 50;

/// Byte ceiling for the meta-graph's type list (D5's byte half). One protocol
/// chunk's worth: a metadata frame larger than a chunk would be the only
/// message in the protocol that cannot be chunked *and* cannot be bounded.
pub const MAX_META_BYTES: usize = 512 * 1024;

/// Ceiling on meta-graph links. Link count is quadratic in type count in the
/// worst case, so the node bound alone does not bound this list.
pub const MAX_META_LINKS: usize = 4_000;

/// Classify a graph by core type count. See [`DetailTier`] for the citation.
pub fn tier_for_core_types(core_types: usize) -> DetailTier {
    match core_types {
        0..=15 => DetailTier::Full,
        16..=200 => DetailTier::Compact,
        201..=5000 => DetailTier::TopTypes,
        _ => DetailTier::Summary,
    }
}

impl DetailTier {
    /// The response bound this tier implies.
    fn node_bound(self) -> Bound {
        let max_items = match self {
            DetailTier::Full | DetailTier::Compact => usize::MAX,
            DetailTier::TopTypes => TOP_TYPES_LIMIT,
            DetailTier::Summary => 0,
        };
        Bound {
            max_items,
            max_bytes: MAX_META_BYTES,
        }
    }

    /// True when the client should draw points and links rather than the
    /// statistics panel.
    pub fn renders_graph(self) -> bool {
        !matches!(self, DetailTier::Summary)
    }
}

/// One type node of the meta-graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct MetaTypeNode {
    /// Index in the session's slot space. Meta-nodes and instance nodes share
    /// one allocator (D12 identity contract), so this index is directly
    /// comparable with an expanded node's.
    pub slot: u32,
    pub name: String,
    /// Members of this type.
    pub count: u32,
    /// `ts` / `geo` / `loc` / `vec`, in kglite's own order and with its own
    /// rule that `loc` is suppressed when `geo` is present.
    pub capabilities: Vec<String>,
    /// True when this type has a parent in kglite's `is_a` forest. Supporting
    /// types do not count toward the tier classification.
    pub supporting: bool,
}

/// One relationship type between two type nodes, with its edge count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct MetaEdge {
    pub source_slot: u32,
    pub target_slot: u32,
    pub name: String,
    pub count: u32,
}

/// Whole-graph totals, which stay meaningful at every tier — they are what the
/// [`DetailTier::Summary`] panel is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct MetaGraphStats {
    pub node_count: u32,
    pub edge_count: u32,
    pub node_type_count: u32,
    pub relationship_type_count: u32,
    /// Node types that are not supporting types — the number the tier is
    /// classified from.
    pub core_type_count: u32,
}

/// The meta-graph's metadata half: everything except the two float arrays.
///
/// Split from the arrays so one encoder feeds both serializers (test-plan §2):
/// the binary path sends this as a JSON frame and the arrays as typed-array
/// frames, the JSON twin serializes [`MetaGraphResponse`] whole. The arrays
/// are the same bytes either way, so twin-vs-binary divergence is not a bug
/// class here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct MetaGraphMeta {
    /// Echoed so an HTTP-only client (curl, an agent) sees the same version
    /// the binary framing carries in its first word.
    pub protocol_version: u32,
    pub tier: DetailTier,
    pub stats: MetaGraphStats,
    pub nodes: Vec<MetaTypeNode>,
    pub edges: Vec<MetaEdge>,
    pub node_bound: BoundInfo,
    pub edge_bound: BoundInfo,
}

/// The complete meta-graph answer.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct MetaGraphResponse {
    pub meta: MetaGraphMeta,
    /// `[x0, y0, x1, y1, …]`, one pair per node in `meta.nodes` order, which
    /// is slot order.
    pub points: Vec<f32>,
    /// `[src0, tgt0, …]` slot indices as `f32` — exact to 2^24 and ready for
    /// cosmos.gl's `setLinks` without a conversion pass.
    pub links: Vec<f32>,
}

/// kglite's four per-type capability flags, recomputed here because
/// `TypeCapabilities` is `pub(super)` inside kglite's introspection module.
///
/// Every source below is a `DirGraph` field the engine already maintains —
/// three config maps and the embedding store's keys — so the whole scan is
/// O(#types) and needs no node access. Mirrors
/// `crates/kglite/src/graph/introspection/capabilities.rs::compute_type_capabilities`
/// (kglite 0.16.13), including the `loc`-suppressed-by-`geo` rule in
/// `flags_csv`.
fn capabilities_for(graph: &DirGraph, node_type: &str) -> Vec<String> {
    let has_timeseries = graph.timeseries_configs.contains_key(node_type);

    let (mut has_location, has_geometry) = match graph.spatial_configs.get(node_type) {
        Some(sc) => (
            sc.location.is_some() || !sc.points.is_empty(),
            sc.geometry.is_some() || !sc.shapes.is_empty(),
        ),
        None => (false, false),
    };
    if !has_location {
        if let Some(meta) = graph.node_type_metadata.get(node_type) {
            has_location = meta.values().any(|t| t.eq_ignore_ascii_case("point"));
        }
    }
    let has_embeddings = graph.embeddings.keys().any(|(nt, _)| nt == node_type);

    let mut flags = Vec::new();
    if has_timeseries {
        flags.push("ts".to_string());
    }
    if has_geometry {
        flags.push("geo".to_string());
    }
    if has_location && !has_geometry {
        flags.push("loc".to_string());
    }
    if has_embeddings {
        flags.push("vec".to_string());
    }
    flags
}

/// Compute the meta-graph, allocating one slot per type node from `view`.
///
/// The view is threaded in rather than created here because expansion draws
/// instance slots from the same allocator and re-sends the same link list; a
/// meta-graph that owned a private counter would hand out slot 3 twice, and one
/// that kept its links to itself would have them dropped by the first expansion
/// (`setLinks` replaces the buffer whole).
pub fn compute(graph: &DirGraph, view: &mut View) -> MetaGraphResponse {
    let mut types: Vec<(String, u32, bool)> = graph
        .type_indices
        .iter()
        .map(|(name, nodes)| {
            let name = name.to_string();
            let supporting = graph.parent_types.contains_key(&name);
            (name, nodes.len() as u32, supporting)
        })
        .collect();

    // Totals come from the graph itself, not from summing what we are about to
    // send. Both are O(1), and only the graph's own counters stay right when a
    // bound clips the type list or when the connectivity cache under-reports:
    // a stats panel that adds up the rows it displayed cannot tell the user
    // what it is not showing them.
    let node_count = graph.graph.node_count() as u64;
    let edge_count = graph.graph.edge_count() as u64;
    let core_type_count = types.iter().filter(|(_, _, s)| !s).count();
    let tier = tier_for_core_types(core_type_count);

    // Largest first; name breaks ties. The tie-break is not cosmetic: the
    // prefix the bound keeps, the slot each type gets and therefore its
    // position all follow this order, so an arbitrary one would make the
    // committed positions baseline depend on hash-map iteration order.
    types.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let (kept, node_bound) = bound::apply(types, tier.node_bound(), |(name, _, _)| {
        // Serialized size of one MetaTypeNode: the JSON scaffolding around a
        // name, plus room for four capability flags.
        name.len() + 96
    });

    let mut slot_of: HashMap<&str, u32> = HashMap::with_capacity(kept.len());
    let mut nodes = Vec::with_capacity(kept.len());
    for (name, count, supporting) in &kept {
        let slot = view.intern_type(name);
        slot_of.insert(name.as_str(), slot);
        nodes.push(MetaTypeNode {
            slot,
            name: name.clone(),
            count: *count,
            capabilities: capabilities_for(graph, name),
            supporting: *supporting,
        });
    }

    // The persisted cardinality cache, not the O(E) scan. See the module doc.
    let triples = graph.get_or_compute_type_connectivity();
    let mut relationship_types: Vec<&str> = triples.iter().map(|t| t.conn.as_str()).collect();
    relationship_types.sort_unstable();
    relationship_types.dedup();

    let mut edges: Vec<MetaEdge> = triples
        .iter()
        .filter_map(|t| {
            // A link whose endpoint type was dropped by the bound has no slot
            // to point at; sending it would be an index into nothing.
            Some(MetaEdge {
                source_slot: *slot_of.get(t.src.as_str())?,
                target_slot: *slot_of.get(t.tgt.as_str())?,
                name: t.conn.clone(),
                count: t.count as u32,
            })
        })
        .collect();
    edges.sort_unstable_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.source_slot.cmp(&b.source_slot))
            .then_with(|| a.target_slot.cmp(&b.target_slot))
    });
    let (edges, edge_bound) = bound::apply(
        edges,
        Bound {
            max_items: MAX_META_LINKS,
            max_bytes: MAX_META_BYTES,
        },
        |e| e.name.len() + 64,
    );

    // The meta links join the view, so every later slice re-sends them whole
    // (D4). Without this the first expansion's `setLinks` would silently erase
    // the meta-graph's own edges.
    for edge in &edges {
        view.add_edge(ViewEdge {
            source_slot: edge.source_slot,
            target_slot: edge.target_slot,
            name: edge.name.clone(),
            meta: true,
        });
    }

    let points = layout::positions_for(nodes.len() as u32);
    let links: Vec<f32> = edges
        .iter()
        .flat_map(|e| [e.source_slot as f32, e.target_slot as f32])
        .collect();

    MetaGraphResponse {
        meta: MetaGraphMeta {
            protocol_version: PROTOCOL_VERSION,
            tier,
            stats: MetaGraphStats {
                node_count: node_count as u32,
                edge_count: edge_count as u32,
                node_type_count: node_bound.total,
                relationship_type_count: relationship_types.len() as u32,
                core_type_count: core_type_count as u32,
            },
            nodes,
            edges,
            node_bound,
            edge_bound,
        },
        points,
        links,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_thresholds_match_kglites_graph_scale() {
        // The exact boundaries of kglite's `graph_scale`. If a floor bump
        // moves them upstream, this test is what notices.
        assert_eq!(tier_for_core_types(0), DetailTier::Full);
        assert_eq!(tier_for_core_types(15), DetailTier::Full);
        assert_eq!(tier_for_core_types(16), DetailTier::Compact);
        assert_eq!(tier_for_core_types(200), DetailTier::Compact);
        assert_eq!(tier_for_core_types(201), DetailTier::TopTypes);
        assert_eq!(tier_for_core_types(5000), DetailTier::TopTypes);
        assert_eq!(tier_for_core_types(5001), DetailTier::Summary);
    }

    #[test]
    fn only_the_summary_tier_stops_rendering_a_graph() {
        assert!(DetailTier::Full.renders_graph());
        assert!(DetailTier::Compact.renders_graph());
        assert!(DetailTier::TopTypes.renders_graph());
        assert!(!DetailTier::Summary.renders_graph());
    }

    #[test]
    fn the_top_types_tier_clips_to_fifty_and_the_summary_tier_to_nothing() {
        assert_eq!(DetailTier::TopTypes.node_bound().max_items, 50);
        assert_eq!(DetailTier::Summary.node_bound().max_items, 0);
        assert_eq!(DetailTier::Full.node_bound().max_items, usize::MAX);
    }

    #[test]
    fn tier_serializes_as_the_kebab_case_the_client_switches_on() {
        assert_eq!(
            serde_json::to_string(&DetailTier::TopTypes).unwrap(),
            "\"top-types\""
        );
        assert_eq!(
            serde_json::to_string(&DetailTier::Summary).unwrap(),
            "\"summary\""
        );
    }
}
