//! One open graph, and every answer a consumer can ask it for.
//!
//! A session owns the `Arc<DirGraph>`, the slot space, and the meta-graph
//! computed once at open. It knows nothing about HTTP or WebSockets: it hands
//! back response structs and, for the binary path, framed byte vectors. The
//! CLI, the wheel and a desktop shell each move those bytes their own way.
//!
//! **The view is behind a lock; the graph is not.** `Arc<DirGraph>` is
//! `Send + Sync` and read-only here, so concurrent queries need no
//! serialisation at all. What P3 added that *does* need it is the slot space:
//! expansion appends and collapse tombstones, and two requests allocating
//! concurrently would hand out the same slot twice. The lock is therefore held
//! across the slot bookkeeping and **never** across a Cypher execution or a
//! graph walk — a write lock held for the length of a 30-second query would
//! block the meta-graph a page reload asks for.

use std::sync::{Arc, RwLock};

use kglite::api::introspection::{compute_schema, schema_overview_to_json};
use kglite::api::{DirGraph, NodeIndex};
use serde::Serialize;
use ts_rs::TS;

use crate::error::CoreError;
use crate::expand::{self, ExpansionPreview, PreviewScope};
use crate::meta_graph::{self, DetailTier, MetaGraphResponse, MetaGraphStats};
use crate::protocol::{MessageType, ResponseEncoder, PROTOCOL_VERSION};
use crate::query::{self, QueryConfig, QueryTable, SearchResponse};
use crate::request::{
    CypherRequest, ExpandRequest, Request, SearchRequest, SlotRequest, TypeRequest,
};
use crate::stats::{self, NodeDetail, PropertyStatsResponse};
use crate::values::value_to_display;
use crate::view::{Compaction, GraphSliceMeta, SliceKind, SliceNode, SlotEntry, View, ViewEdge};
use crate::{bound::BoundInfo, layout};

/// What the client needs to know about the session it is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SessionInfo {
    /// The wire format this server speaks. A client that decodes a different
    /// number refuses rather than guessing (`protocol.rs`).
    pub protocol_version: u32,
    /// `kglite-visual-core`'s version.
    pub core_version: String,
    /// The graph, as the caller named it.
    pub graph: String,
    /// The tier the server chose for this graph's meta-graph.
    pub tier: DetailTier,
    /// Slots handed out so far — meta-nodes plus whatever expansion appended.
    pub slot_count: u32,
    /// Slots currently tombstoned.
    pub tombstone_count: u32,
    /// The nodes-per-expansion ceiling this build enforces (D5), so a client
    /// can say what a bound *would* do before it fires.
    pub max_expansion_nodes: u32,
    /// The rows-per-query ceiling.
    pub max_query_rows: u32,
    /// Query wall-clock ceiling, in seconds.
    pub query_timeout_secs: u32,
    pub stats: MetaGraphStats,
}

/// A server-side failure, delivered in band.
///
/// A client that shows an empty graph on failure is indistinguishable from one
/// showing an empty graph on success, so every error takes a message frame of
/// its own rather than closing the socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ErrorMessage {
    pub message: String,
}

/// The schema document behind `/api/describe` (D12).
///
/// Deliberately **not** a ts-rs type: `schema` is kglite's own JSON shape,
/// rendered by the engine's `schema_overview_to_json` so every binding's
/// schema document is byte-identical. Generating a TypeScript type for it here
/// would be this crate claiming ownership of a shape kglite owns. The frontend
/// does not consume this endpoint; agents and `curl` do.
#[derive(Debug, Clone, Serialize)]
pub struct DescribeResponse {
    pub protocol_version: u32,
    /// The same tier the meta-graph carries, so an agent reading only this
    /// endpoint learns how much of the schema it is being shown.
    pub tier: DetailTier,
    pub core_type_count: u32,
    /// kglite's canonical schema JSON.
    pub schema: serde_json::Value,
}

/// A change to what is on screen: metadata plus the two float arrays.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct GraphSlice {
    pub meta: GraphSliceMeta,
    /// Set when this response left the view sparse enough to be worth
    /// reclaiming. A sibling of `meta` rather than a field inside it, because
    /// on the binary path it is [`MessageType::Compaction`] — its own frame,
    /// so a client cannot skip it while parsing the slice — and carrying it in
    /// both places would put two copies of the remap on the wire.
    pub compaction: Option<Compaction>,
    /// Positions for the slots from `meta.first_slot`, `[x0, y0, x1, y1, …]`.
    pub points: Vec<f32>,
    /// **Every** link in the view as `[src0, tgt0, …]` slot indices (D4).
    pub links: Vec<f32>,
}

/// What the bound did to one slice, both halves.
///
/// Nodes and links are bounded together — one byte budget, charged by whichever
/// list is asking — so they are also produced and reported together. Passing
/// them as two arguments through the assembly path is how one of them gets
/// forgotten at a call site.
struct SliceBounds {
    nodes: BoundInfo,
    links: BoundInfo,
}

/// Every answer a request can produce.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Response {
    Query(QueryTable),
    Preview(ExpansionPreview),
    Slice(GraphSlice),
    NodeDetail(NodeDetail),
    Search(SearchResponse),
    PropertyStats(PropertyStatsResponse),
}

/// An open graph.
pub struct Session {
    graph: Arc<DirGraph>,
    source: String,
    view: RwLock<View>,
    meta_graph: MetaGraphResponse,
    config: QueryConfig,
}

impl Session {
    /// Open a session over an already-loaded graph.
    ///
    /// The meta-graph is computed here, once: it is the entry screen, it is
    /// O(#types), and recomputing it per request would make a page reload
    /// re-walk the type index for no new information.
    pub fn open(graph: Arc<DirGraph>, source: impl Into<String>) -> Self {
        Self::open_with(graph, source, QueryConfig::default())
    }

    pub fn open_with(graph: Arc<DirGraph>, source: impl Into<String>, config: QueryConfig) -> Self {
        let mut view = View::new();
        let meta_graph = meta_graph::compute(&graph, &mut view);
        Self {
            graph,
            source: source.into(),
            view: RwLock::new(view),
            meta_graph,
            config,
        }
    }

    pub fn graph(&self) -> &Arc<DirGraph> {
        &self.graph
    }

    pub fn meta_graph(&self) -> &MetaGraphResponse {
        &self.meta_graph
    }

    pub fn config(&self) -> QueryConfig {
        self.config
    }

    /// Slot of a type node, for a caller that has a name rather than a slot.
    pub fn slot_of_type(&self, name: &str) -> Option<u32> {
        self.read().slot_of_type(name)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, View> {
        // A poisoned lock means a previous request panicked mid-mutation, so
        // the slot space may be inconsistent. Propagating the panic is the
        // honest outcome: continuing would hand out indices from a half-updated
        // map, which surfaces as wrong nodes on screen rather than as a crash.
        self.view
            .read()
            .expect("the view lock was poisoned by a panicking request")
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, View> {
        self.view
            .write()
            .expect("the view lock was poisoned by a panicking request")
    }

    pub fn info(&self) -> SessionInfo {
        let view = self.read();
        SessionInfo {
            protocol_version: PROTOCOL_VERSION,
            core_version: crate::VERSION.to_string(),
            graph: self.source.clone(),
            tier: self.meta_graph.meta.tier,
            slot_count: view.slot_count(),
            tombstone_count: view.tombstone_count(),
            max_expansion_nodes: expand::MAX_EXPANSION_NODES as u32,
            max_query_rows: query::MAX_QUERY_ROWS as u32,
            query_timeout_secs: self.config.timeout.as_secs().min(u64::from(u32::MAX)) as u32,
            stats: self.meta_graph.meta.stats,
        }
    }

    /// kglite's schema document plus the tier this session chose.
    ///
    /// `compute_schema` reads per-type metadata the engine already holds; it
    /// is not a node scan. It is still the heaviest call on this type, so a
    /// server runs it off the async runtime.
    pub fn describe(&self) -> DescribeResponse {
        let schema = compute_schema(&self.graph);
        DescribeResponse {
            protocol_version: PROTOCOL_VERSION,
            tier: self.meta_graph.meta.tier,
            core_type_count: self.meta_graph.meta.stats.core_type_count,
            schema: schema_overview_to_json(&schema),
        }
    }

    /// Dispatch one request.
    ///
    /// **Blocking.** Every arm may run Cypher or walk the graph, so the caller
    /// runs this off its reactor, on a thread with at least
    /// [`crate::query::QUERY_THREAD_STACK_BYTES`] of stack.
    pub fn handle(&self, request: &Request) -> Result<Response, CoreError> {
        match request {
            Request::Cypher(req) => self.cypher(req),
            Request::Preview(req) => self.preview(req.slot).map(Response::Preview),
            Request::Expand(req) => self.expand(req).map(Response::Slice),
            Request::Collapse(req) => self.collapse(req).map(Response::Slice),
            Request::NodeDetail(req) => self.node_detail(req).map(Response::NodeDetail),
            Request::Search(req) => self.search(req).map(Response::Search),
            Request::PropertyStats(req) => self.property_stats(req).map(Response::PropertyStats),
        }
    }

    fn cypher(&self, request: &CypherRequest) -> Result<Response, CoreError> {
        let table = query::run_cypher(&self.graph, request, self.config)?;
        if !request.as_graph {
            return Ok(Response::Query(table));
        }
        // "Show in graph" without a second round trip: the nodes the result
        // already named, mapped into the slot space.
        let slice = self.absorb(
            SliceKind::Query,
            &query::node_indices(&table),
            &table
                .relationships
                .iter()
                .map(|r| {
                    (
                        NodeIndex::new(r.source_id as usize),
                        NodeIndex::new(r.target_id as usize),
                        r.name.clone(),
                    )
                })
                .collect::<Vec<_>>(),
            BoundInfo::new(table.node_ids.len(), table.node_ids.len()),
            // The query path refuses no link of its own: the row bound already
            // decided what this result contains. Links whose endpoints did not
            // make it into the slot space are counted inside `absorb`.
            0,
        );
        Ok(Response::Slice(slice))
    }

    /// Per-relationship counts for what expanding `slot` would add.
    pub fn preview(&self, slot: u32) -> Result<ExpansionPreview, CoreError> {
        let entry = self.entry(slot)?;
        let deadline = None;
        let (scope, node_type, title, relationships) = match entry {
            SlotEntry::Type { name } => {
                let previews = expand::preview_for_type(&self.graph, &name);
                (PreviewScope::Type, name, String::new(), previews)
            }
            SlotEntry::Node { node_id, node_type } => {
                let index = NodeIndex::new(node_id as usize);
                let title = self
                    .graph
                    .node_view(index)
                    .map(|v| value_to_display(&v.title()))
                    .unwrap_or_default();
                let previews = expand::preview_for_node(&self.graph, index, &node_type, deadline);
                (PreviewScope::Node, node_type, title, previews)
            }
            SlotEntry::Tombstone => {
                return Err(CoreError::Request(format!(
                    "slot {slot} was collapsed; there is nothing there to expand"
                )))
            }
        };

        let total_edges = relationships.iter().map(|r| r.count).sum();
        Ok(ExpansionPreview {
            protocol_version: PROTOCOL_VERSION,
            slot,
            scope,
            node_type,
            title,
            relationships,
            total_edges,
            max_nodes: expand::MAX_EXPANSION_NODES as u32,
        })
    }

    fn expand(&self, request: &ExpandRequest) -> Result<GraphSlice, CoreError> {
        let seeds = match self.entry(request.slot)? {
            // The flagship drill-in: a meta-graph type node expands into the
            // instance nodes of that type. Every node of the type is a seed,
            // and the bound is what makes that safe on a 100M-node graph.
            SlotEntry::Type { name } => self
                .graph
                .type_indices
                .get(&name)
                .map(|nodes| nodes.iter().collect::<Vec<NodeIndex>>())
                .unwrap_or_default(),
            SlotEntry::Node { node_id, .. } => vec![NodeIndex::new(node_id as usize)],
            SlotEntry::Tombstone => {
                return Err(CoreError::Request(format!(
                    "slot {} was collapsed; there is nothing there to expand",
                    request.slot
                )))
            }
        };

        let found = expand::expand(
            &self.graph,
            &seeds,
            request.relationship.as_deref(),
            request.direction,
            expand::effective_bound(request.limit),
            self.config.deadline(),
        );
        let edges: Vec<(NodeIndex, NodeIndex, String)> = found
            .edges
            .iter()
            .map(|e| (e.source, e.target, e.name.clone()))
            .collect();
        let links_refused = (found.link_bound.total - found.link_bound.returned) as usize;
        Ok(self.absorb(
            SliceKind::Expand,
            &found.nodes,
            &edges,
            found.bound,
            links_refused,
        ))
    }

    /// Map a set of kglite nodes and edges into the slot space and describe the
    /// result.
    ///
    /// The one place slots are allocated after open, so the write lock is taken
    /// exactly here — after every graph read the request needed, never around
    /// one.
    /// `links_refused` is what the producer found and did not hand over — the
    /// expansion's byte budget firing. Links dropped *here*, for an endpoint
    /// the node bound did not admit, are counted below and land in the same
    /// number: from the client's side they are one fact, "this slice is not
    /// showing you every edge it found".
    fn absorb(
        &self,
        kind: SliceKind,
        nodes: &[NodeIndex],
        edges: &[(NodeIndex, NodeIndex, String)],
        bound: BoundInfo,
        links_refused: usize,
    ) -> GraphSlice {
        let mut view = self.write();
        let first_slot = view.slot_count();
        let mut added: Vec<SliceNode> = Vec::new();

        for index in nodes {
            let (node_type, title) = match self.graph.node_view(*index) {
                Some(node) => (
                    node.node_type_str(&self.graph.interner).to_string(),
                    value_to_display(&node.title()),
                ),
                None => continue,
            };
            let node_id = index.index() as u32;
            let (slot, is_new) = view.intern_node(node_id, &node_type);
            if is_new {
                added.push(SliceNode {
                    slot,
                    node_id,
                    node_type,
                    title,
                });
            }
        }

        let mut links_added = 0usize;
        let mut links_dropped = links_refused;
        for (source, target, name) in edges {
            let (Some(source_slot), Some(target_slot)) = (
                view.slot_of_node(source.index() as u32),
                view.slot_of_node(target.index() as u32),
            ) else {
                // An endpoint the bound did not admit. Sending the link anyway
                // would be an index into a slot the client was never given.
                links_dropped += 1;
                continue;
            };
            view.add_edge(ViewEdge {
                source_slot,
                target_slot,
                name: name.clone(),
                meta: false,
            });
            links_added += 1;
        }
        let link_bound = BoundInfo {
            returned: links_added as u32,
            total: (links_added + links_dropped) as u32,
            truncated: links_dropped > 0,
        };

        self.finish_slice(
            &mut view,
            kind,
            first_slot,
            added,
            Vec::new(),
            SliceBounds {
                nodes: bound,
                links: link_bound,
            },
        )
    }

    fn collapse(&self, request: &SlotRequest) -> Result<GraphSlice, CoreError> {
        let mut view = self.write();
        let entry = view.entry(request.slot).cloned();
        let tombstones = match entry {
            // Collapsing a type node puts the drill-in back where it started:
            // the type node stays, every instance of it goes.
            Some(SlotEntry::Type { name }) => view.collapse_type(&name),
            Some(SlotEntry::Node { .. }) => view.tombstone(&[request.slot]),
            Some(SlotEntry::Tombstone) => Vec::new(),
            None => {
                return Err(CoreError::Request(format!(
                    "slot {} is not in this view",
                    request.slot
                )))
            }
        };
        let count = tombstones.len();
        let first_slot = view.slot_count();
        Ok(self.finish_slice(
            &mut view,
            SliceKind::Collapse,
            first_slot,
            Vec::new(),
            tombstones,
            SliceBounds {
                nodes: BoundInfo::new(count, count),
                // A collapse adds no links; it removes them. Nothing was cut
                // from what it *did* send, which is what this field is about.
                links: BoundInfo::new(0, 0),
            },
        ))
    }

    /// Assemble the response, compacting if the view has gone sparse enough.
    ///
    /// Compaction happens *after* the slice is described and is carried in the
    /// same response, so a client applies "here is what changed" and "here is
    /// where everything moved" in one step. Two messages would leave a window
    /// in which the client's map and the server's disagree.
    fn finish_slice(
        &self,
        view: &mut View,
        kind: SliceKind,
        first_slot: u32,
        nodes: Vec<SliceNode>,
        tombstones: Vec<u32>,
        bounds: SliceBounds,
    ) -> GraphSlice {
        let compaction: Option<Compaction> = view
            .should_compact()
            .then(|| view.compact(PROTOCOL_VERSION));

        // After a compaction every slot moved, so the client cannot splice: it
        // needs the whole position array, from slot zero. `nodes` and
        // `tombstones` stay in the PRE-compaction space — the whole metadata
        // half does — because the client applies them and *then* applies the
        // remap, which is the only order in which both lists mean anything.
        //
        // Dropping `nodes` here instead was a real defect, found by driving the
        // running server rather than by a test: an expansion into a view that
        // was already 30% tombstoned compacts, and the nodes it had just added
        // arrived with no labels, no ids and no way to select them.
        let (first_slot, points) = match &compaction {
            Some(_) => (0, layout::positions_for(view.slot_count())),
            None => (
                first_slot,
                layout::positions_range(first_slot, view.slot_count() - first_slot),
            ),
        };
        let links: Vec<f32> = view
            .edges()
            .iter()
            .flat_map(|e| [e.source_slot as f32, e.target_slot as f32])
            .collect();

        GraphSlice {
            meta: GraphSliceMeta {
                protocol_version: PROTOCOL_VERSION,
                kind,
                first_slot,
                nodes,
                tombstones,
                edges: view.edges().to_vec(),
                slot_count: view.slot_count(),
                tombstone_count: view.tombstone_count(),
                bound: bounds.nodes,
                link_bound: bounds.links,
            },
            compaction,
            points,
            links,
        }
    }

    fn node_detail(&self, request: &SlotRequest) -> Result<NodeDetail, CoreError> {
        let SlotEntry::Node { node_id, .. } = self.entry(request.slot)? else {
            return Err(CoreError::Request(format!(
                "slot {} is not an instance node; it has no stored properties",
                request.slot
            )));
        };
        stats::node_detail(&self.graph, request.slot, node_id)
    }

    fn search(&self, request: &SearchRequest) -> Result<SearchResponse, CoreError> {
        let mut response = query::search(&self.graph, request, self.config)?;
        // A hit already on screen is highlighted; one that is not offers "load
        // into view". The client cannot tell them apart — only the session
        // knows the slot space — so the answer carries the distinction.
        let view = self.read();
        for hit in &mut response.hits {
            hit.slot = view.slot_of_node(hit.node_id);
        }
        Ok(response)
    }

    fn property_stats(&self, request: &TypeRequest) -> Result<PropertyStatsResponse, CoreError> {
        stats::property_stats(&self.graph, &request.node_type)
    }

    fn entry(&self, slot: u32) -> Result<SlotEntry, CoreError> {
        self.read()
            .entry(slot)
            .cloned()
            .ok_or_else(|| CoreError::Request(format!("slot {slot} is not in this view")))
    }

    /// The meta-graph as protocol frames: metadata JSON, then points, then
    /// links, with the terminal flag on the last.
    pub fn meta_graph_frames(&self) -> Vec<Vec<u8>> {
        let mut enc = ResponseEncoder::new();
        enc.push_json(
            MessageType::MetaGraphMeta,
            &serde_json::to_string(&self.meta_graph.meta)
                .expect("MetaGraphMeta is plain data and always serializes"),
        );
        enc.push_f32(MessageType::Points, &self.meta_graph.points);
        enc.push_f32(MessageType::Links, &self.meta_graph.links);
        enc.finish()
    }

    /// The session info as a single terminal frame.
    pub fn session_info_frames(&self) -> Vec<Vec<u8>> {
        let mut enc = ResponseEncoder::new();
        enc.push_json(
            MessageType::SessionInfo,
            &serde_json::to_string(&self.info())
                .expect("SessionInfo is plain data and always serializes"),
        );
        enc.finish()
    }
}

/// Frame a response for the binary transport.
///
/// Free-standing rather than a `Session` method so the framing and the
/// answering are separable: the JSON twin serializes the very same [`Response`]
/// and never comes through here (test-plan §2 — one encoder, two serializers).
pub fn response_frames(response: &Response) -> Vec<Vec<u8>> {
    let mut enc = ResponseEncoder::new();
    match response {
        Response::Query(table) => enc.push_json(MessageType::QueryTable, &json_of(table)),
        Response::Preview(preview) => {
            enc.push_json(MessageType::ExpansionPreview, &json_of(preview))
        }
        Response::NodeDetail(detail) => enc.push_json(MessageType::NodeDetail, &json_of(detail)),
        Response::Search(search) => enc.push_json(MessageType::SearchResult, &json_of(search)),
        Response::PropertyStats(stats) => {
            enc.push_json(MessageType::PropertyStats, &json_of(stats))
        }
        Response::Slice(slice) => {
            // Metadata first, then the arrays — the same order the meta-graph
            // uses, so one assembler handles both.
            enc.push_json(MessageType::GraphSlice, &json_of(&slice.meta));
            if let Some(compaction) = &slice.compaction {
                enc.push_json(MessageType::Compaction, &json_of(compaction));
            }
            enc.push_f32(MessageType::Points, &slice.points);
            enc.push_f32(MessageType::Links, &slice.links);
        }
    }
    enc.finish()
}

fn json_of<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("every response type is plain data")
}

/// Frame an error for the binary transport.
///
/// Free-standing rather than a `Session` method: the failures worth reporting
/// most are the ones that happen before a session exists.
pub fn error_frames(message: impl Into<String>) -> Vec<Vec<u8>> {
    let payload = ErrorMessage {
        message: message.into(),
    };
    let mut enc = ResponseEncoder::new();
    enc.push_json(
        MessageType::Error,
        &serde_json::to_string(&payload).expect("ErrorMessage is plain data"),
    );
    enc.finish()
}
