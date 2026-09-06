//! One open graph, and every answer a consumer can ask it for.
//!
//! A session owns the `Arc<DirGraph>`, the slot space, and the meta-graph
//! computed once at open. It knows nothing about HTTP or WebSockets: it hands
//! back response structs and, for the binary path, framed byte vectors. The
//! CLI, the wheel and a desktop shell each move those bytes their own way.
//!
//! Shared state is committed atomically; graph walks run against private candidates.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
};
use std::time::{SystemTime, UNIX_EPOCH};

use kglite::api::introspection::{compute_schema, schema_overview_to_json};
use kglite::api::{DirGraph, NodeIndex};
use serde::Serialize;
use ts_rs::TS;

use crate::error::CoreError;
use crate::expand::{self, ExpansionPreview, PreviewScope};
use crate::meta_graph::{self, DetailTier, MetaGraphResponse, MetaGraphStats};
use crate::protocol::{MessageType, ResponseEncoder, PROTOCOL_VERSION};
use crate::query::{self, QueryConfig, QueryTable, SearchResponse};
use crate::records::{self, BrowseTypeRequest, LoadNodesRequest, NodeHandle, RecordTable};
use crate::render::live_layout::{layout_live_view, LayoutResult};
use crate::request::{
    CypherRequest, ExpandRequest, LayoutKernel, LayoutRequest, Request, SearchRequest, SlotRequest,
    TypeRequest,
};
use crate::shared::{RevisionStamp, SharedRequest, SharedSnapshot, SharedViewState};
use crate::stats::{self, NodeDetail, PropertyStatsResponse};
use crate::subset::SubsetSnapshot;
use crate::validate::{validate_query, ValidateResponse};
use crate::values::{value_to_display, value_to_json};
use crate::view::{Compaction, GraphSliceMeta, SliceKind, SliceNode, SlotEntry, View, ViewEdge};
use crate::{bound::BoundInfo, layout};

/// What the client needs to know about the session it is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SessionInfo {
    pub generation: String,
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
///
/// The editor's schema-aware completions are the obvious counter-example and
/// deliberately are not one: they feed from the meta-graph, which the entry
/// screen already carries, and from `property-stats` per type, fetched lazily
/// (plan E2). Reversing this decision because a browser feature would have
/// found it convenient would leave the note above describing an endpoint the
/// frontend depends on.
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

/// The caveat every agent-facing surface has to repeat **while the viewer's own
/// simulation owns the geometry** — which is where every session starts.
///
/// One constant, because it is asserted by the MCP tool descriptions and
/// returned inside [`ViewState`]. Three copies of a caveat is three places for
/// it to stop being true.
///
/// **It stopped being unconditional in G3** (`R17`). Until the layout wire
/// existed, "the server does not know where the points ended up" was a fact
/// about the architecture: the only positions the server ever sent were a seed
/// the GPU immediately overwrote. A static kernel (plan E5) inverts that — the
/// server computes the arrangement, the client applies it authoritatively, the
/// simulation is destroyed and dragging is off — so the sentence below would be
/// a false claim on exactly the views a peer most wants to describe. Which of
/// the two applies is [`geometry_caveat`]'s answer, and
/// [`ViewState::layout_kernel`] is the field it reads.
pub const GEOMETRY_CAVEAT: &str = "The live layout runs on the viewer's GPU and the server does \
     not know where the points ended up (`layout_kernel` is `simulation`). A render of this view \
     is content-identical and geometry-different: same nodes, same links, same \
     truncation, a different arrangement. Describe what is in the view, never \
     where it is on the user's screen — or ask for a static layout, after which \
     the arrangement is this server's own and can be described.";

/// …and the caveat that replaces it once a static kernel is in force.
///
/// Still a caveat, because two things stay unknowable. The **camera** is the
/// viewer's — they zoom and pan freely — so relative position is describable
/// and screen position is not, which is the same rule as before applied one
/// level down. And `render` is a *separate* pass with its own fold, its own
/// separation and its own choice of kernel, so its picture is not a photograph
/// of the screen either.
pub const GEOMETRY_STATIC_CAVEAT: &str =
    "This view is under a static layout THIS SERVER computed (`layout_kernel` names the \
     kernel): the viewer's simulation is off, dragging is disabled, and nothing moves a point \
     until the next layout request. So the arrangement on their screen is the one that was \
     sent, and relative position is safe to describe — 'the ring around X', 'the island on \
     the left'. Their camera is still their own, so never name a screen coordinate; and \
     `render` lays out independently (it folds fans and separates circles for the page it \
     draws), so its picture may still differ from what they see.";

/// Which caveat is true right now.
///
/// One function rather than a branch at each surface: the whole reason the
/// caveat is a constant is that a second copy is a second place for it to rot,
/// and a second `if` is exactly that in another shape.
pub const fn geometry_caveat(kernel: LayoutKernel) -> &'static str {
    if kernel.is_static() {
        GEOMETRY_STATIC_CAVEAT
    } else {
        GEOMETRY_CAVEAT
    }
}

/// What the last view-mutating response did, kept so an agent can ask.
///
/// The bound metadata rides out with the slice and is then gone; the browser
/// keeps it in its status bar, and an MCP client has no status bar. Without
/// this, `view_state` could say how many nodes are on screen but not whether
/// that number is the whole answer — which is the D5 failure exactly.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LastSlice {
    pub kind: SliceKind,
    /// What the bound did to the nodes.
    pub bound: BoundInfo,
    /// What the bound did to the links.
    pub link_bound: BoundInfo,
    /// The banner the app is showing for it, verbatim — the same words, from
    /// the same function the headless render draws into an image.
    pub banner: Option<String>,
}

/// One type node currently on the meta-graph, and what has been drilled into.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewTypeNode {
    pub slot: u32,
    pub name: String,
    /// Members in the graph, not on screen.
    pub count: u32,
    pub capabilities: Vec<String>,
    pub supporting: bool,
    /// Instances of this type currently in the view.
    pub instances_on_screen: u32,
}

/// The response bounds this build enforces, so an agent can predict a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ViewBounds {
    pub max_expansion_nodes: u32,
    pub max_query_rows: u32,
    pub query_timeout_secs: u32,
}

/// What is on the shared screen, as structured truth.
///
/// The server-side equivalent of the browser's `window.__kglv`, and the answer
/// to "what is the user looking at" for a peer that cannot look. It is
/// deliberately *not* a `ts-rs` type: the frontend has its own, richer view of
/// this — it is the thing being described — and generating a TypeScript mirror
/// would be this file claiming ownership of a shape the client already owns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewState {
    pub stamp: RevisionStamp,
    pub subset: SubsetSnapshot,
    pub subset_revision: String,
    pub topology_revision: String,
    pub protocol_version: u32,
    pub graph: String,
    pub tier: DetailTier,
    /// Slots handed out, tombstones included.
    pub slot_count: u32,
    /// Slots that currently draw something.
    pub live_count: u32,
    pub tombstone_count: u32,
    pub link_count: u32,
    pub types: Vec<ViewTypeNode>,
    /// Instance nodes on screen, by type, descending — the drill-in state in
    /// one field.
    pub instances_by_type: Vec<(String, u32)>,
    pub last_slice: Option<LastSlice>,
    pub bounds: ViewBounds,
    /// Who owns the arrangement on screen (plan E5).
    ///
    /// `simulation` — the session default — means the viewer's GPU does, and
    /// the server does not know where anything is. Any other value is a kernel
    /// this server computed and broadcast, under which the simulation is off
    /// and dragging is disabled: the arrangement is knowable, which is what
    /// [`geometry_caveat`] switches on.
    pub layout_kernel: LayoutKernel,
    /// [`GEOMETRY_CAVEAT`] or [`GEOMETRY_STATIC_CAVEAT`], whichever
    /// `layout_kernel` makes true — carried in the payload so a client that
    /// only ever reads tool *results* still meets it.
    pub geometry_caveat: &'static str,
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
    Shared(Box<SharedSnapshot>),
    Query(QueryTable),
    Records(RecordTable),
    Preview(ExpansionPreview),
    Slice(GraphSlice),
    NodeDetail(NodeDetail),
    Search(SearchResponse),
    PropertyStats(PropertyStatsResponse),
    Layout(LayoutResult),
}

/// An open graph.
pub struct Session {
    graph: Arc<DirGraph>,
    source: String,
    generation: String,
    state: RwLock<SharedViewState>,
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
            generation: new_generation(),
            state: RwLock::new(SharedViewState::new(view)),
            meta_graph,
            config,
        }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn node_handle(&self, node_id: u32) -> NodeHandle {
        NodeHandle {
            generation: self.generation.clone(),
            node_id,
        }
    }

    pub(crate) fn check_handles(&self, handles: &[NodeHandle]) -> Result<(), CoreError> {
        if handles.len() > records::MAX_RECORD_HANDLES {
            return Err(CoreError::Request(
                "at most 5000 node handles are allowed".into(),
            ));
        }
        if handles
            .iter()
            .any(|handle| handle.generation != self.generation)
        {
            return Err(CoreError::Request(
                "node handle belongs to a different session generation".into(),
            ));
        }
        Ok(())
    }

    pub fn browse_type(&self, request: &BrowseTypeRequest) -> Result<GraphSlice, CoreError> {
        match self.handle(&Request::BrowseType(request.clone()))? {
            Response::Slice(slice) => Ok(slice),
            _ => unreachable!(),
        }
    }

    fn browse_type_uncommitted(
        &self,
        request: &BrowseTypeRequest,
    ) -> Result<GraphSlice, CoreError> {
        let members = self
            .graph
            .type_indices
            .get(&request.node_type)
            .ok_or_else(|| {
                CoreError::Request(format!("unknown node type {:?}", request.node_type))
            })?;
        let bound = expand::effective_bound(request.limit);
        let nodes: Vec<_> = members.iter().take(bound.max_items).collect();
        self.absorb(
            SliceKind::Query,
            &nodes,
            &[],
            BoundInfo::new(nodes.len(), members.len()),
            0,
        )
    }

    pub fn load_nodes(&self, request: &LoadNodesRequest) -> Result<GraphSlice, CoreError> {
        match self.handle(&Request::LoadNodes(request.clone()))? {
            Response::Slice(slice) => Ok(slice),
            _ => unreachable!(),
        }
    }

    fn load_nodes_uncommitted(&self, request: &LoadNodesRequest) -> Result<GraphSlice, CoreError> {
        self.check_handles(&request.handles)?;
        let _guard = self.graph.begin_read_pass();
        let mut nodes = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for handle in &request.handles {
            let index = NodeIndex::new(handle.node_id as usize);
            if self.graph.node_view(index).is_none() {
                return Err(CoreError::Request(format!(
                    "node {} is absent from this source snapshot",
                    handle.node_id
                )));
            }
            if seen.insert(index) {
                nodes.push(index);
            }
        }
        self.absorb(
            SliceKind::Search,
            &nodes,
            &[],
            BoundInfo::new(nodes.len(), nodes.len()),
            0,
        )
    }

    fn typed_node_key(&self, node_id: u32) -> records::RecordCell {
        self.graph
            .node_view(NodeIndex::new(node_id as usize))
            .map(|node| records::cell(&node.id()))
            .unwrap_or(records::RecordCell::Missing)
    }

    fn loaded_nodes(&self, view: &View) -> Vec<SliceNode> {
        view.live_entries()
            .filter_map(|(slot, entry)| {
                let SlotEntry::Node {
                    node_id,
                    node_type,
                    title,
                } = entry
                else {
                    return None;
                };
                let node = self.graph.node_view(NodeIndex::new(*node_id as usize))?;
                Some(SliceNode {
                    handle: self.node_handle(*node_id),
                    typed_key: records::cell(&node.id()),
                    slot,
                    node_id: *node_id,
                    node_type: node_type.clone(),
                    title: title.clone(),
                    key: node_key(&node.id()),
                })
            })
            .collect()
    }

    fn validate_loaded(&self, view: &View) -> Result<(), CoreError> {
        let count = view
            .live_entries()
            .filter(|(_, entry)| matches!(entry, SlotEntry::Node { .. }))
            .count();
        let edges = view.edges().iter().filter(|edge| !edge.meta).count();
        if count > records::MAX_LOADED_NODES || edges > records::MAX_LOADED_EDGES {
            return Err(CoreError::Request(format!("loaded view would contain {count} nodes and {edges} relations; limits are 5000 nodes and 20000 relations. Collapse content before adding more")));
        }
        // Charge actual JSON escaping and keys, plus the binary topology/positions.
        let binary_bytes = view.slot_count() as usize * 8 + view.edges().len() * 8;
        // Reserve fixed slice fields and framing as well as the two variable lists.
        let bytes = records::serialized_bytes(
            &(self.loaded_nodes(view), view.edges()),
            records::MAX_LOADED_BYTES.saturating_sub(binary_bytes + 1024),
        )? + binary_bytes
            + 1024;
        if bytes > records::MAX_LOADED_BYTES {
            return Err(CoreError::Request(format!("loaded view would use {bytes} bytes; limit is {} bytes. Collapse content before adding more", records::MAX_LOADED_BYTES)));
        }
        Ok(())
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

    pub(crate) fn view_read(&self) -> ViewReadGuard<'_> {
        ViewReadGuard(self.state_read())
    }

    pub(crate) fn state_read(&self) -> std::sync::RwLockReadGuard<'_, SharedViewState> {
        self.state.read().expect("shared state lock poisoned")
    }
    pub(crate) fn state_write(&self) -> std::sync::RwLockWriteGuard<'_, SharedViewState> {
        self.state.write().expect("shared state lock poisoned")
    }
    fn read(&self) -> std::sync::RwLockReadGuard<'_, SharedViewState> {
        self.state_read()
    }
    fn write(&self) -> std::sync::RwLockWriteGuard<'_, SharedViewState> {
        self.state_write()
    }
    pub(crate) fn fork_state(&self, state: SharedViewState) -> Self {
        Self {
            graph: self.graph.clone(),
            source: self.source.clone(),
            generation: self.generation.clone(),
            state: RwLock::new(state),
            meta_graph: self.meta_graph.clone(),
            config: self.config,
        }
    }

    pub fn info(&self) -> SessionInfo {
        let view = self.read();
        SessionInfo {
            generation: self.generation.clone(),
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

    /// Parse-only validation of one query. Nothing is executed (`validate`).
    ///
    /// Not a [`Request`] variant, and not on the binary wire: the editor asks
    /// this while the user is typing, the answer is a handful of strings, and
    /// putting it on the socket that carries typed arrays would mean a message
    /// type and a protocol bump for it. `queries.rs` documents the same
    /// boundary from the browser's side.
    ///
    /// **Blocking**, like everything else here — parsing runs kglite's parser,
    /// which is what [`crate::query::QUERY_THREAD_STACK_BYTES`] exists for.
    pub fn validate(&self, query: &str) -> ValidateResponse {
        validate_query(&self.graph, query)
    }

    /// Dispatch one request.
    ///
    /// **Blocking.** Every arm may run Cypher or walk the graph, so the caller
    /// runs this off its reactor, on a thread with at least
    /// [`crate::query::QUERY_THREAD_STACK_BYTES`] of stack.
    pub fn handle(&self, request: &Request) -> Result<Response, CoreError> {
        if request.is_shared() {
            self.apply_shared(&SharedRequest::new(request.clone()))
                .map(|event| event.response)
        } else {
            self.handle_uncommitted(request)
        }
    }
    pub(crate) fn handle_uncommitted(&self, request: &Request) -> Result<Response, CoreError> {
        match request {
            Request::Reset => Ok(Response::Slice(self.reset_uncommitted())),
            Request::Subset(_)
            | Request::Appearance(_)
            | Request::Caption(_)
            | Request::Focus(_)
            | Request::Highlight(_) => self.settings_uncommitted(request),
            Request::Cypher(req) => self.cypher(req),
            Request::Records(req) => self.records(req).map(Response::Records),
            Request::BrowseType(req) => self.browse_type_uncommitted(req).map(Response::Slice),
            Request::LoadNodes(req) => self.load_nodes_uncommitted(req).map(Response::Slice),
            Request::Preview(req) => self.preview(req.slot).map(Response::Preview),
            Request::Expand(req) => self.expand(req).map(Response::Slice),
            Request::Collapse(req) => self.collapse(req).map(Response::Slice),
            Request::NodeDetail(req) => self.node_detail(req).map(Response::NodeDetail),
            Request::Search(req) => self.search(req).map(Response::Search),
            Request::PropertyStats(req) => self.property_stats(req).map(Response::PropertyStats),
            Request::Layout(req) => self.layout(req).map(Response::Layout),
        }
    }

    /// Compute a static arrangement for the live view (plan E5).
    ///
    /// The kernel is recorded **only after the layout succeeded**: a refused
    /// `geo` must not leave the session claiming it knows a geometry it never
    /// computed, and that is the same order every other state write here takes.
    fn layout(&self, request: &LayoutRequest) -> Result<LayoutResult, CoreError> {
        let result = layout_live_view(self, request)?;
        let mut state = self.state_write();
        state.layout_kernel = result.meta.kernel_chosen;
        state.last_layout = result
            .meta
            .kernel_chosen
            .is_static()
            .then(|| result.clone());
        Ok(result)
    }

    /// Who owns the arrangement on screen. See [`ViewState::layout_kernel`].
    pub fn layout_kernel(&self) -> LayoutKernel {
        self.state_read().layout_kernel
    }

    /// The arrangement every attached client is currently holding, if the
    /// server owns it. `None` under the simulation, where nobody does.
    ///
    /// The second half of the resync: [`Session::sync_slice`] tells a newcomer
    /// what is in the view, and this tells it where — see the field's own
    /// documentation for why it is the remembered answer rather than a fresh
    /// one.
    pub fn last_layout(&self) -> Option<LayoutResult> {
        self.state_read().last_layout.clone()
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
                        r.edge_id,
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
        )?;
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
            SlotEntry::Node {
                node_id,
                node_type,
                title,
            } => {
                let index = NodeIndex::new(node_id as usize);
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
        let run = |seeds: Box<dyn Iterator<Item = NodeIndex> + '_>| {
            expand::expand_iter(
                &self.graph,
                seeds,
                request.relationship.as_deref(),
                request.direction,
                expand::effective_bound(request.limit),
                self.config.deadline(),
            )
        };
        let found = match self.entry(request.slot)? {
            SlotEntry::Type { name } => {
                let nodes = self
                    .graph
                    .type_indices
                    .get(&name)
                    .ok_or_else(|| CoreError::Request(format!("unknown node type {name:?}")))?;
                run(Box::new(nodes.iter()))
            }
            SlotEntry::Node { node_id, .. } => {
                run(Box::new(std::iter::once(NodeIndex::new(node_id as usize))))
            }
            SlotEntry::Tombstone => {
                return Err(CoreError::Request(format!(
                    "slot {} was collapsed; there is nothing there to expand",
                    request.slot
                )))
            }
        };
        let edges: Vec<(u32, NodeIndex, NodeIndex, String)> = found
            .edges
            .iter()
            .map(|e| (e.edge_id, e.source, e.target, e.name.clone()))
            .collect();
        let links_refused = (found.link_bound.total - found.link_bound.returned) as usize;
        self.absorb(
            SliceKind::Expand,
            &found.nodes,
            &edges,
            found.bound,
            links_refused,
        )
    }

    /// Map a set of kglite nodes and edges into the slot space and describe the
    /// result.
    ///
    /// Commit admission atomically after walking the source. The candidate is
    /// checked against cumulative count and serialized byte ceilings before
    /// replacing the live view.
    /// `links_refused` is what the producer found and did not hand over — the
    /// expansion's byte budget firing. Links dropped *here*, for an endpoint
    /// the node bound did not admit, are counted below and land in the same
    /// number: from the client's side they are one fact, "this slice is not
    /// showing you every edge it found".
    fn absorb(
        &self,
        kind: SliceKind,
        nodes: &[NodeIndex],
        edges: &[(u32, NodeIndex, NodeIndex, String)],
        bound: BoundInfo,
        links_refused: usize,
    ) -> Result<GraphSlice, CoreError> {
        let _guard = self.graph.begin_read_pass();
        let mut live = self.write();
        self.check_admission(&live, nodes, edges)?;
        let mut view = live.view.clone();
        let first_slot = view.slot_count();
        let added = self.admit_nodes(&mut view, nodes)?;

        let mut links_added = 0usize;
        let mut edge_bytes = 0usize;
        let mut links_dropped = links_refused;
        for (edge_id, source, target, name) in edges {
            let (Some(source_slot), Some(target_slot)) = (
                view.slot_of_node(source.index() as u32),
                view.slot_of_node(target.index() as u32),
            ) else {
                // An endpoint the bound did not admit. Sending the link anyway
                // would be an index into a slot the client was never given.
                links_dropped += 1;
                continue;
            };
            let edge = ViewEdge {
                edge_id: Some(*edge_id),
                source_slot,
                target_slot,
                name: name.clone(),
                meta: false,
            };
            edge_bytes += records::serialized_bytes(
                &edge,
                records::MAX_LOADED_BYTES.saturating_sub(edge_bytes),
            )?;
            view.add_edge(edge);
            links_added += 1;
        }
        let link_bound = BoundInfo {
            returned: links_added as u32,
            total: (links_added + links_dropped) as u32,
            truncated: links_dropped > 0,
        };

        self.validate_loaded(&view)?;
        live.view = view;
        Ok(self.finish_slice(
            &mut live,
            kind,
            first_slot,
            added,
            Vec::new(),
            SliceBounds {
                nodes: bound,
                links: link_bound,
            },
        ))
    }

    fn admit_nodes(
        &self,
        view: &mut View,
        nodes: &[NodeIndex],
    ) -> Result<Vec<SliceNode>, CoreError> {
        let mut added: Vec<SliceNode> = Vec::new();
        let mut bytes = 0usize;

        for index in nodes {
            let node_id = index.index() as u32;
            if view.slot_of_node(node_id).is_some() {
                continue;
            }
            let (node_type, title, key) = match self.graph.node_view(*index) {
                Some(node) => {
                    let title = node.title();
                    if matches!(&*title, kglite::api::Value::String(text) if text.len() > records::MAX_LOADED_BYTES)
                    {
                        return Err(CoreError::Request(
                            "node title exceeds the loaded-view byte ceiling".into(),
                        ));
                    }
                    if !matches!(&*title, kglite::api::Value::String(_))
                        && matches!(records::cell(&title), records::RecordCell::Truncated { .. })
                    {
                        return Err(CoreError::Request(
                            "node title exceeds the bounded value limit".into(),
                        ));
                    }
                    (
                        node.node_type_str(&self.graph.interner).to_string(),
                        value_to_display(&title),
                        node_key(&node.id()),
                    )
                }
                None => continue,
            };
            let node = SliceNode {
                handle: self.node_handle(node_id),
                typed_key: self.typed_node_key(node_id),
                slot: view.slot_count(),
                node_id,
                node_type,
                title,
                key,
            };
            bytes +=
                records::serialized_bytes(&node, records::MAX_LOADED_BYTES.saturating_sub(bytes))?;
            view.intern_node(node_id, &node.node_type, &node.title);
            added.push(node);
        }

        Ok(added)
    }

    fn check_admission(
        &self,
        view: &View,
        nodes: &[NodeIndex],
        edges: &[(u32, NodeIndex, NodeIndex, String)],
    ) -> Result<(), CoreError> {
        let mut admitted: std::collections::HashSet<u32> = view
            .live_entries()
            .filter_map(|(_, entry)| match entry {
                SlotEntry::Node { node_id, .. } => Some(*node_id),
                _ => None,
            })
            .collect();
        for node in nodes {
            admitted.insert(node.index() as u32);
            if admitted.len() > records::MAX_LOADED_NODES {
                return Err(CoreError::Request(
                    "loaded view exceeds the 5000 node limit; collapse content before adding more"
                        .into(),
                ));
            }
        }
        let mut relations: std::collections::HashSet<u32> = view
            .edges()
            .iter()
            .filter_map(|edge| edge.edge_id)
            .collect();
        for (edge_id, source, target, _) in edges {
            if admitted.contains(&(source.index() as u32))
                && admitted.contains(&(target.index() as u32))
            {
                relations.insert(*edge_id);
                if relations.len() > records::MAX_LOADED_EDGES {
                    return Err(CoreError::Request("loaded view exceeds the 20000 relation limit; collapse content before adding more".into()));
                }
            }
        }
        Ok(())
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
        state: &mut SharedViewState,
        kind: SliceKind,
        first_slot: u32,
        nodes: Vec<SliceNode>,
        tombstones: Vec<u32>,
        bounds: SliceBounds,
    ) -> GraphSlice {
        let view = &mut state.view;
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

        // The bound metadata rides out with the slice and is then gone. An MCP
        // client has no status bar to keep it in, so the session keeps it: see
        // `LastSlice`.
        state.last_slice = Some(LastSlice {
            kind,
            bound: bounds.nodes,
            link_bound: bounds.links,
            banner: crate::render::encoding::truncation_banner(
                bounds.nodes.truncated,
                bounds.nodes.returned,
                bounds.nodes.total,
                if kind == SliceKind::Collapse {
                    "collapsed"
                } else {
                    "nodes"
                },
                Some((
                    bounds.links.truncated,
                    bounds.links.returned,
                    bounds.links.total,
                )),
            ),
        });

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

    /// Collapse everything back to the entry screen.
    ///
    /// Not `collapse` in a loop: one slice, one compaction decision, one
    /// message to every client. A reset that arrived as forty collapses would
    /// make forty round trips and forty renders of intermediate states nobody
    /// asked to see.
    ///
    /// The type nodes stay. "Reset" restores the screen a session opens with,
    /// which is the meta-graph — a blank canvas would be "close".
    pub fn reset(&self) -> Result<GraphSlice, CoreError> {
        match self.handle(&Request::Reset)? {
            Response::Slice(slice) => Ok(slice),
            _ => unreachable!(),
        }
    }

    fn reset_uncommitted(&self) -> GraphSlice {
        let mut view = self.write();
        let tombstones = view.tombstone_all_instances();
        let count = tombstones.len();
        let first_slot = view.slot_count();
        self.finish_slice(
            &mut view,
            SliceKind::Collapse,
            first_slot,
            Vec::new(),
            tombstones,
            SliceBounds {
                nodes: BoundInfo::new(count, count),
                links: BoundInfo::new(0, 0),
            },
        )
    }

    /// Write the live view out in somebody else's format (plan E8).
    ///
    /// **The scope is the view, and it cannot be anything else.** The node list
    /// is assembled here from the live slot space and handed to
    /// [`crate::export::export_nodes`], which takes a slice rather than
    /// kglite's `Option<&CurrentSelection>` — so the engine's whole-graph mode
    /// is not something a server handler could reach by passing the wrong
    /// argument. That matters more here than the type signature suggests: this
    /// is a viewer built around a response bound, and an export endpoint that
    /// answered `None` would stream half a gigabyte in reply to a click.
    ///
    /// Type nodes are not in it. They are this app's own summary of the graph,
    /// not nodes kglite holds, and there is nothing to export them as.
    pub fn export_view(
        &self,
        format: crate::export::ExportFormat,
    ) -> Result<crate::export::ExportedView, CoreError> {
        let nodes: Vec<NodeIndex> = {
            let view = self.read();
            view.live_entries()
                .filter_map(|(_, entry)| match entry {
                    SlotEntry::Node { node_id, .. } => Some(NodeIndex::new(*node_id as usize)),
                    SlotEntry::Type { .. } | SlotEntry::Tombstone => None,
                })
                .collect()
        };
        crate::export::export_nodes(&self.graph, &nodes, format, &self.source)
    }

    /// The whole view, from slot zero, for a client that has just connected.
    ///
    /// **The session's truth, not an assumption the newcomer has to make.** A
    /// client used to be greeted with the session info and the meta-graph and
    /// nothing else — slots `0..n`, the entry screen — which is the *opening*
    /// state of a session, not necessarily its current one. Attach to a session
    /// somebody has already drilled into and the two disagree: the next
    /// broadcast arrives with a `first_slot` past the end of the positions
    /// array this client holds, [`crate::view::SliceKind`]'s splice grows the
    /// array over the gap, and every slot in between draws as a point with no
    /// label, no id and nothing to click. Measured in G4 on sodir: 144 of them.
    ///
    /// **An ordinary [`GraphSlice`], deliberately, rather than a frame of its
    /// own.** `first_slot = 0` with positions for the whole space is a shape
    /// the client already decodes — it is what a compaction sends, minus the
    /// remap — so the fix needs no message type, no protocol bump and no second
    /// assembly path that could drift from the first. What it does need is its
    /// own [`SliceKind`], because it is not a change to anything.
    ///
    /// It therefore also does **not** touch `last_slice`: that field answers
    /// "what did the bound do to the last thing that happened", and a client
    /// connecting is not a thing that happened to the view. Nor is it published
    /// to the bus — it is addressed to one socket, and broadcasting a full view
    /// to everyone on every connect would re-upload the whole space to clients
    /// that already hold it.
    pub fn sync_slice(&self) -> GraphSlice {
        let _guard = self.graph.begin_read_pass();
        let view = self.read();
        let mut nodes: Vec<SliceNode> = Vec::new();
        let mut tombstones: Vec<u32> = Vec::new();
        for (slot, entry) in view.entries_with_tombstones() {
            match entry {
                SlotEntry::Node {
                    node_id,
                    node_type,
                    title,
                } => nodes.push(SliceNode {
                    handle: self.node_handle(*node_id),
                    typed_key: self.typed_node_key(*node_id),
                    slot,
                    node_id: *node_id,
                    node_type: node_type.clone(),
                    title: title.clone(),
                    // Re-read from the graph rather than stored in the slot
                    // entry. A resync is bounded by the slot space and happens
                    // when a browser attaches, so one `node_view` per slot is
                    // cheaper than carrying a second copy of the key through
                    // every intern and every compaction — where it would be one
                    // more thing that can fall out of step with the node.
                    key: self
                        .graph
                        .node_view(NodeIndex::new(*node_id as usize))
                        .and_then(|node| node_key(&node.id())),
                }),
                SlotEntry::Tombstone => tombstones.push(slot),
                // The type nodes are already this client's: they arrived in the
                // meta-graph frames immediately before this one, with their
                // names, counts and capability badges. Re-sending them as
                // instance nodes would overwrite that richer label with a
                // poorer one.
                SlotEntry::Type { .. } => {}
            }
        }

        let links: Vec<f32> = view
            .edges()
            .iter()
            .flat_map(|e| [e.source_slot as f32, e.target_slot as f32])
            .collect();
        let node_count = nodes.len();
        let link_count = view.edges().len();
        GraphSlice {
            meta: GraphSliceMeta {
                protocol_version: PROTOCOL_VERSION,
                kind: SliceKind::Sync,
                first_slot: 0,
                nodes,
                tombstones,
                edges: view.edges().to_vec(),
                slot_count: view.slot_count(),
                tombstone_count: view.tombstone_count(),
                // Nothing was cut from this: it is the view, whole. The bounds
                // that shaped it fired on the slices that built it, and those
                // are reported by `last_slice`, which this deliberately leaves
                // alone.
                bound: BoundInfo::new(node_count, node_count),
                link_bound: BoundInfo::new(link_count, link_count),
            },
            compaction: None,
            points: layout::positions_for(view.slot_count()),
            links,
        }
    }

    /// What is on the shared screen, as structured truth (D14).
    ///
    /// The answer to "what is the user looking at" for a peer that cannot look.
    /// Everything here but one field is a fact about *content*; the exception
    /// is [`ViewState::layout_kernel`], which says who owns the geometry, and
    /// [`geometry_caveat`] rides along saying what that permits.
    pub fn view_state(&self) -> ViewState {
        let view = self.read();
        let layout_kernel = view.layout_kernel;

        let mut instances: std::collections::BTreeMap<&str, u32> =
            std::collections::BTreeMap::new();
        for (_, entry) in view.live_entries() {
            if let SlotEntry::Node { node_type, .. } = entry {
                *instances.entry(node_type.as_str()).or_insert(0) += 1;
            }
        }

        let types: Vec<ViewTypeNode> = self
            .meta_graph
            .meta
            .nodes
            .iter()
            .filter(|node| view.entry(node.slot).is_some())
            .map(|node| ViewTypeNode {
                slot: node.slot,
                name: node.name.clone(),
                count: node.count,
                capabilities: node.capabilities.clone(),
                supporting: node.supporting,
                instances_on_screen: instances.get(node.name.as_str()).copied().unwrap_or(0),
            })
            .collect();

        // Descending, so the first rows are the drill-in a reader cares about;
        // the name breaks ties so the answer is stable between two identical
        // views.
        let mut instances_by_type: Vec<(String, u32)> = instances
            .into_iter()
            .map(|(name, count)| (name.to_string(), count))
            .collect();
        instances_by_type.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        ViewState {
            stamp: view.stamp(self.generation()),
            subset: view.subset.clone(),
            subset_revision: view.subset_revision.to_string(),
            topology_revision: view.topology_revision.to_string(),
            protocol_version: PROTOCOL_VERSION,
            graph: self.source.clone(),
            tier: self.meta_graph.meta.tier,
            slot_count: view.slot_count(),
            live_count: view.slot_count() - view.tombstone_count(),
            tombstone_count: view.tombstone_count(),
            link_count: view.edges().len() as u32,
            types,
            instances_by_type,
            last_slice: view.last_slice.clone(),
            bounds: ViewBounds {
                max_expansion_nodes: expand::MAX_EXPANSION_NODES as u32,
                max_query_rows: query::MAX_QUERY_ROWS as u32,
                query_timeout_secs: self.config.timeout.as_secs().min(u64::from(u32::MAX)) as u32,
            },
            layout_kernel,
            geometry_caveat: geometry_caveat(layout_kernel),
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

    /// Refuse a slot list that names something this view cannot point at.
    ///
    /// The steering commands (D14) do not touch the slot space, so nothing here
    /// *has* to fail — a client could drop the slots it does not recognise and
    /// carry on. It fails anyway, and by name: an agent that focused slot 4 000
    /// on a five-slot view has a wrong model of what the user is looking at,
    /// and a silently narrowed camera would leave it holding that model.
    pub fn check_live_slots(&self, slots: &[u32]) -> Result<(), CoreError> {
        if slots.len() > records::MAX_LOADED_NODES + self.meta_graph.meta.nodes.len() {
            return Err(CoreError::Request(
                "too many slots in steering request".into(),
            ));
        }
        let view = self.read();
        for slot in slots {
            match view.entry(*slot) {
                Some(SlotEntry::Tombstone) => {
                    return Err(CoreError::Request(format!(
                        "slot {slot} was collapsed; there is nothing there to point at"
                    )))
                }
                None => {
                    return Err(CoreError::Request(format!(
                        "slot {slot} is not in this view, which holds slots 0..{}",
                        view.slot_count()
                    )))
                }
                Some(SlotEntry::Type { .. } | SlotEntry::Node { .. }) => {}
            }
        }
        Ok(())
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
    if let Response::Shared(snapshot) = response {
        return crate::shared::shared_frames(
            &crate::shared::SharedWireMeta {
                snapshot: snapshot.meta.clone(),
                request_id: None,
                focus: None,
                mutation_kind: None,
            },
            &snapshot.points,
            &snapshot.links,
        );
    }
    let mut enc = ResponseEncoder::new();
    match response {
        Response::Shared(_) => unreachable!(),
        Response::Query(table) => enc.push_json(MessageType::QueryTable, &json_of(table)),
        Response::Records(table) => enc.push_json(MessageType::Records, &json_of(table)),
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
        Response::Layout(result) => {
            // Metadata then positions, the same order every other array-bearing
            // response uses. No links frame: a layout moves points and touches
            // no edge, and sending the link list again would be re-stating what
            // the client already holds.
            enc.push_json(MessageType::Layout, &json_of(&result.meta));
            enc.push_f32(MessageType::Points, &result.points);
        }
    }
    enc.finish()
}

/// Legacy query key, only when JSON can carry it safely. Lossless keys and
/// previews live in `SliceNode::typed_key`; direct records use the node handle.
fn node_key(id: &kglite::api::Value) -> Option<serde_json::Value> {
    match id {
        kglite::api::Value::Null => None,
        kglite::api::Value::Int64(value) if value.unsigned_abs() > 9_007_199_254_740_991 => None,
        kglite::api::Value::String(value) if value.len() > records::MAX_CELL_BYTES => None,
        kglite::api::Value::List(_)
        | kglite::api::Value::Map(_)
        | kglite::api::Value::Node(_)
        | kglite::api::Value::Relationship(_)
        | kglite::api::Value::Path(_)
        | kglite::api::Value::NodeRef(_) => None,
        value => {
            let json = value_to_json(value);
            (serde_json::to_vec(&json).ok()?.len() <= records::MAX_CELL_BYTES).then_some(json)
        }
    }
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

fn new_generation() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) struct ViewReadGuard<'a>(std::sync::RwLockReadGuard<'a, SharedViewState>);
impl std::ops::Deref for ViewReadGuard<'_> {
    type Target = View;
    fn deref(&self) -> &View {
        &self.0.view
    }
}
