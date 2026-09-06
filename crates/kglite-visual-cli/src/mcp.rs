//! The MCP face: an agent driving the view a human is watching (plan D14).
//!
//! **Served BY the running server, at `/mcp`.** There is no second process, no
//! discovery file and no new place for state to accumulate — "attach" is the
//! URL the launch contract already prints, now with an `mcp` key beside `url`.
//! rmcp's `StreamableHttpService` implements `tower_service::Service` with
//! `Error = Infallible`, so it mounts as one more route on the axum router that
//! is already serving the frontend, the JSON twin and the WebSocket.
//!
//! Bounded record inspection complements the shared-view tools. Bulk querying
//! remains owned by the graph's MCP server.
//!
//! The human and agent share one revisioned view. Each committed change is
//! broadcast in commit order. Explicit expected stamps and preparation bases
//! refuse stale work without changing that view.
//!
//! **What an agent can and cannot know.** It can know the content of the view
//! exactly: [`kglite_visual_core::ViewState`] is the same truth the browser's
//! `window.__kglv` reports. Whether it can know the *geometry* is now a
//! question with two answers, and `set_layout` is what moves between them: with
//! the viewer's GPU simulation running the server never receives the final
//! positions, and under a static kernel the server computed the arrangement and
//! the client is holding it still. Which caveat applies is
//! [`geometry_caveat`](kglite_visual_core::geometry_caveat)'s answer, from
//! `view_state.layout_kernel`, and no surface here writes its own wording.

use std::sync::Arc;

use base64::Engine as _;
use kglite_visual_core::control::{
    AppearanceRequest, FocusRequest, HighlightConcept, HighlightRequest,
};
use kglite_visual_core::error::CoreError;
use kglite_visual_core::records::{
    BrowseTypeRequest, LoadNodesRequest, NodeHandle, RecordsRequest,
};
use kglite_visual_core::render::{RenderFormat, RenderRequest, RenderSource, Theme};
use kglite_visual_core::request::{
    CypherRequest, EdgeDirection, ExpandRequest, LayoutKernel, LayoutRequest, Request,
    SearchRequest, SlotRequest,
};
use kglite_visual_core::shared::{CaptionRequest, RevisionStamp, SharedRequest};
use kglite_visual_core::subset::SubsetRequest;
use kglite_visual_core::{geometry_caveat, ExportFormat, Response};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::{Deserialize, Serialize};

use crate::broadcast::{AppState, DispatchError, Execution};

#[path = "mcp_output.rs"]
mod output;

/// The path the MCP endpoint is mounted at, owned here so the router and the
/// launch contract cannot disagree about it.
pub const MCP_PATH: &str = "/mcp";

/// What an agent is told it has connected to.
///
/// **This string is part of the product.** It is the only thing standing
/// between "an agent that navigates a graph with a human watching" and "an
/// agent that treats a shared screen as a private scratchpad and narrates
/// pixel positions it cannot see". Every sentence in it is here because getting
/// it wrong has a specific failure:
///
/// - *shared view, human watching* — otherwise an agent resets and re-expands
///   freely, and the person in front of the screen watches it flicker.
/// - *revision conflicts* — otherwise an agent overwrites state that changed
///   while it was preparing its next action.
/// - *geometry caveat* — otherwise an agent says "as you can see, top left",
///   which is a claim about a screen it has never seen. Conditional since G3:
///   the claim is false under a static layout the server itself computed, and
///   an unconditional caveat would be a rule agents learn to ignore.
/// - *data querying belongs elsewhere* — otherwise this becomes a worse
///   `kglite-mcp-server`, one tool at a time.
/// - *look at the saved queries first* — otherwise an agent writes its own
///   Cypher over a schema it has just met, while the person beside it has a
///   query for that exact question saved under a name they chose.
/// - *export takes the view, not the graph* — otherwise `export_view` reads as
///   "give me this graph as GraphML", an agent calls it on the entry screen,
///   gets a refusal it does not understand, and concludes the tool is broken
///   rather than that it had loaded nothing to export.
const INSTRUCTIONS: &str = "\
You are attached to a RUNNING kglite-visual window: an interactive graph view \
that a human being is looking at right now, in their browser. These tools move \
that view and inspect its records. Shared view changes are immediately visible to them.

Treat it as a shared workspace, not a scratchpad:
- Narrate what you are doing before you do it, so the change on screen is \
expected rather than startling.
- Prefer small, reversible steps. `expand` then `collapse` beats `reset_view`, \
which discards whatever the human had drilled into.
- Shared changes are ordered and acknowledged. Pass `expected` from `view_state` \
to refuse overwriting a newer revision. A revision conflict changes nothing; \
re-read the view before deciding what to do next. Legacy calls without an \
expected stamp remain supported; stale prepared work always refuses.

What you can and cannot know:
- `view_state` is exact about CONTENT — slots, types, counts, tombstones, and \
what the response bound last truncated. It is the same truth the page's own \
debug hook reports.
- GEOMETRY depends on `view_state.layout_kernel`, and you can change which \
answer applies. While it says `simulation` — the default — the layout runs on \
the viewer's GPU and the server never receives the final positions: never tell \
the user where something is on their screen, and use `focus` and `highlight` to \
point at things instead, because those move THEIR view. Call `set_layout` with \
a static kernel and the arrangement becomes this server's own: their simulation \
stops, dragging is disabled, and relative position is then safe to describe. \
Read the `geometry_caveat` that comes back rather than remembering which mode \
you are in.
- `render` is a separate deterministic server pass. Its legacy live-view target \
includes loaded content and schema context, including hidden nodes. Explicit \
`scope: visible` captures the visible retained instance subset. Both recompute \
geometry with their own folding and labels; neither is a browser screenshot.

Scope: this server steers a picture. It is not the place to mine the graph. \
Bulk querying, schema exploration and result tables belong to the graph's own \
MCP server (kglite-mcp-server); `show_cypher` here exists to put a result \
ON SCREEN, not to read it back. `records` inspects bounded fields for exact \
source handles returned by `browse_type`, `load_nodes`, or `show_cypher`. These \
handles expire with the session generation; they are not source id-field values.

The one thing here that is not about the screen: `list_saved_queries` and \
`run_saved_query` read the queries THIS USER saved for THIS graph. Start there \
before writing Cypher of your own — a saved query is what the person you are \
working with already decided was worth keeping, and running one shows them a \
result they will recognise.

`export_view` defaults to loaded instance nodes and all source relations between \
them, including content hidden by filters. Explicit `scope: visible` previews \
and exports exactly the visible retained instance subset. It refuses empty \
instance scope, so load what you want first. Read the `notes` it returns before \
telling the user what they have — the notes describe format fidelity limits.

Every response is bounded in core and says so. A truncated answer is reported \
as truncated in `view_state.last_slice` and drawn into the banner of any \
render; if you see one, the honest report is the bound, not the subset.";

// ---------------------------------------------------------------------------
// Tool arguments
//
// Every one of these derives `Default`, because rmcp dispatches a call with no
// `arguments` through `T::default()`. A struct with a required field and no
// sensible default would turn "the agent omitted the body" into a
// deserialization error instead of a usable message.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum FieldArg {
    Property {
        name: String,
    },
    Derived {
        calculation_id: String,
        column: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
enum ScalarArg {
    UniqueId(String),
    Int64(String),
    Float64(f64),
    String(String),
    Boolean(bool),
    Date(String),
    Timestamp(String),
    Point {
        lat: f64,
        lon: f64,
    },
    Duration {
        months: i32,
        days: i32,
        seconds: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum PredicateArg {
    Type {
        node_types: Vec<String>,
    },
    Category {
        field: FieldArg,
        values: Vec<ScalarArg>,
        #[serde(default)]
        include_null: bool,
        #[serde(default)]
        include_missing: bool,
    },
    NumericRange {
        field: FieldArg,
        min: Option<ScalarArg>,
        max: Option<ScalarArg>,
        #[serde(default)]
        include_null: bool,
        #[serde(default)]
        include_missing: bool,
    },
    Missing {
        field: FieldArg,
        #[serde(default)]
        include_null: bool,
        #[serde(default)]
        include_missing: bool,
    },
    Relation {
        names: Vec<String>,
    },
    HideIsolated,
}

fn enabled_default() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
struct FilterArg {
    id: String,
    #[serde(default = "enabled_default")]
    enabled: bool,
    predicate: PredicateArg,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum CalculationKindArg {
    Degree,
    WeakComponents,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
struct CalculateArgs {
    kind: CalculationKindArg,
    /// Omit to create a new frozen result; name a matching existing result to recompute it explicitly.
    #[serde(default)]
    calculation_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct SubsetArgs {
    #[serde(default)]
    predicates: Vec<FilterArg>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct CaptionArgs {
    #[serde(default)]
    caption_by: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct ExpectedArg {
    generation: String,
    revision: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct SharedOptions {
    #[serde(default)]
    expected: Option<ExpectedArg>,
    #[serde(default)]
    request_id: Option<String>,
}

impl SharedOptions {
    fn request(self, request: Request) -> SharedRequest {
        SharedRequest {
            request,
            expected: self.expected.map(|stamp| RevisionStamp {
                generation: stamp.generation,
                revision: stamp.revision,
            }),
            request_id: self.request_id,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct SharedArgs<T> {
    #[serde(flatten)]
    args: T,
    #[serde(flatten)]
    options: SharedOptions,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct HandleArg {
    generation: String,
    node_id: u32,
}

impl From<HandleArg> for NodeHandle {
    fn from(value: HandleArg) -> Self {
        Self {
            generation: value.generation,
            node_id: value.node_id,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ViewReferenceArg {
    Node { handle: HandleArg },
    Type { name: String },
}
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum BookmarkFocusArg {
    Fit,
    References { references: Vec<ViewReferenceArg> },
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct SaveViewArgs {
    name: String,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    selected: Option<Vec<ViewReferenceArg>>,
    #[serde(default)]
    focus: Option<BookmarkFocusArg>,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum StorageArg {
    #[default]
    Durable,
    Session,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct NamedViewArgs {
    storage: StorageArg,
    name: String,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct HistoryRestoreArgs {
    id: String,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct RecordsArgs {
    #[serde(default)]
    handles: Vec<HandleArg>,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    field_refs: Option<Vec<FieldArg>>,
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum FieldPathArg {
    Index { index: u32 },
    Key { key: String },
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct FieldDetailArgs {
    handle: HandleArg,
    field: String,
    #[serde(default)]
    path: Vec<FieldPathArg>,
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct BrowseTypeArgs {
    #[serde(default)]
    node_type: String,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct LoadNodesArgs {
    #[serde(default)]
    handles: Vec<HandleArg>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RelationArg {
    generation: String,
    edge_id: u32,
    source: HandleArg,
    target: HandleArg,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct LoadEntitiesArgs {
    #[serde(default)]
    nodes: Vec<HandleArg>,
    #[serde(default)]
    relationships: Vec<RelationArg>,
}

/// Which way an expansion walks. A local mirror of core's
/// [`EdgeDirection`] so the JSON schema an agent reads is a plain enum rather
/// than whatever `ts-rs` and `serde` happen to agree on.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum DirectionArg {
    /// Follow edges pointing away from the seed.
    Out,
    /// Follow edges pointing at the seed.
    In,
    /// Both, deduplicated by node. The default: a caller who has not said which
    /// way an edge runs wants the neighbourhood, not half of it.
    #[default]
    Both,
}

impl From<DirectionArg> for EdgeDirection {
    fn from(value: DirectionArg) -> Self {
        match value {
            DirectionArg::Out => EdgeDirection::Out,
            DirectionArg::In => EdgeDirection::In,
            DirectionArg::Both => EdgeDirection::Both,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum ConceptArg {
    /// Search and query hits. Several may stand out at once.
    #[default]
    Highlighted,
    /// What the selection panel describes — one thing, with a ring around it.
    Selected,
}

impl From<ConceptArg> for HighlightConcept {
    fn from(value: ConceptArg) -> Self {
        match value {
            ConceptArg::Highlighted => HighlightConcept::Highlighted,
            ConceptArg::Selected => HighlightConcept::Selected,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum FormatArg {
    /// A PNG, embedded in the reply so you can look at it.
    #[default]
    Png,
    /// SVG source, returned as text. Bigger in tokens; scales without blurring.
    Svg,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum ThemeArg {
    /// The app's own dark palette. The default, because it is what the human
    /// is looking at.
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum RenderTargetArg {
    /// Legacy loaded content and schema context, including hidden nodes,
    /// re-laid out server-side. Explicit scope selects captured instance output.
    #[default]
    LiveView,
    /// The type-level meta-graph, drawn from scratch. Does not touch the live
    /// view.
    Meta,
    /// A Cypher result, drawn from scratch. Does not touch the live view; use
    /// `show_cypher` for that.
    Cypher,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct CypherArgs {
    /// Read-only Cypher. Nodes and relationships in the result are mapped into
    /// the shared view.
    pub query: String,
    /// Values for the query's `$name` placeholders, as a JSON object. Never
    /// string-interpolated into the query text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct ExpandArgs {
    /// The slot to expand out of. A type slot loads instances of that type; an
    /// instance slot loads its neighbours. `view_state` lists the slots.
    pub slot: u32,
    /// Relationship type to walk. Omit to walk every type — the expensive case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    #[serde(default)]
    pub direction: DirectionArg,
    /// Nodes wanted. Clamped to the server's ceiling; asking for more returns
    /// the ceiling and says it truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct SlotArgs {
    /// The slot to act on.
    pub slot: u32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct FocusArgs {
    /// Slots to frame. An empty list frames the whole view.
    #[serde(default)]
    pub slots: Vec<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct HighlightArgs {
    /// Slots to mark. An empty list with no `search` clears the concept.
    #[serde(default)]
    pub slots: Vec<u32>,
    /// Instead of naming slots: run this server-side search and mark whichever
    /// hits are already on screen. The count of hits that were NOT on screen is
    /// reported back, because those are the ones you would have to load first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    /// Restrict `search` to one node type. Searching every type is the slow
    /// path on a large graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    #[serde(default)]
    pub concept: ConceptArg,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct AppearanceArgs {
    /// Omit/null clears this channel in legacy mode; presentation-only preserves it.
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub color_by: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub size_by: Option<Option<String>>,
    #[serde(default)]
    pub presentation: Option<PresentationArgs>,
    /// Canonical source or calculated field. Explicit null clears the channel.
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub color_field: Option<Option<FieldArg>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub size_field: Option<Option<FieldArg>>,
}
fn present_nullable<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct PresentationArgs {
    label_density: f32,
    prioritize_selected_labels: bool,
    prioritize_hovered_labels: bool,
    edge_opacity: f32,
    node_size_min: f32,
    node_size_max: f32,
    legend_visible: bool,
}
impl Default for PresentationArgs {
    fn default() -> Self {
        let settings = kglite_visual_core::presentation::PresentationSettings::default();
        Self {
            label_density: settings.label_density,
            prioritize_selected_labels: settings.prioritize_selected_labels,
            prioritize_hovered_labels: settings.prioritize_hovered_labels,
            edge_opacity: settings.edge_opacity,
            node_size_min: settings.node_size_min,
            node_size_max: settings.node_size_max,
            legend_visible: settings.legend_visible,
        }
    }
}

/// Which arrangement `set_layout` asks for. A local mirror of core's
/// [`LayoutKernel`], for the reason [`DirectionArg`] is one: the schema an
/// agent reads should be a plain enum with a sentence per variant.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum KernelArg {
    /// Read the graph's structure and choose. The default: a neighbourhood
    /// becomes hop rings, a community-structured graph becomes packed islands,
    /// anything shapeless falls back to a force pass.
    #[default]
    Auto,
    /// Hop rings around one node. Pass `seed_slot` to say which.
    Radial,
    /// Communities laid out separately and packed, so a group reads as a group.
    /// Falls back to `force` on a graph with no community structure, and says
    /// so in `kernel_chosen`.
    Islands,
    /// A seeded force layout, computed here and held still.
    Force,
    /// A geographic projection: every node with a lat/lon location or a WKT
    /// geometry is drawn where it actually is, and anything without one goes
    /// into a labelled tray rather than being hidden. Refused with a sentence
    /// when nothing in the view has a coordinate.
    Geo,
    /// Hand the arrangement back to the viewer's own GPU simulation, which is
    /// where every session starts. After this you can no longer know where
    /// anything is.
    Simulation,
}

impl From<KernelArg> for LayoutKernel {
    fn from(value: KernelArg) -> Self {
        match value {
            KernelArg::Auto => LayoutKernel::Auto,
            KernelArg::Radial => LayoutKernel::Radial,
            KernelArg::Islands => LayoutKernel::Islands,
            KernelArg::Force => LayoutKernel::Force,
            KernelArg::Geo => LayoutKernel::Geo,
            KernelArg::Simulation => LayoutKernel::Simulation,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct LayoutArgs {
    #[serde(default)]
    pub kernel: KernelArg,
    /// The slot a `radial` layout should be centred on. `view_state` lists the
    /// slots. Ignored by every other kernel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_slot: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct SavedQueryArgs {
    /// The saved query's name, exactly as `list_saved_queries` reports it.
    pub name: String,
}

/// Which file `export_view` writes. A local mirror of core's [`ExportFormat`],
/// for the reason [`DirectionArg`] is one.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum ExportFormatArg {
    /// XML that Gephi, yEd and Cytoscape all open. The default — and the one
    /// whose node names import as `n0`, `n1`, …; the answer says why.
    #[default]
    Graphml,
    /// Gephi's own XML. Its node labels import as the titles.
    Gexf,
    /// `id,type,title`, one row per node.
    Csv,
    /// `source,target,type`, one row per edge — the other half of `csv`, and a
    /// separate call because a zip would be a new dependency for two text
    /// files.
    CsvEdges,
    /// D3's `{"nodes": [...], "links": [...]}`.
    Json,
}

impl From<ExportFormatArg> for ExportFormat {
    fn from(value: ExportFormatArg) -> Self {
        match value {
            ExportFormatArg::Graphml => ExportFormat::Graphml,
            ExportFormatArg::Gexf => ExportFormat::Gexf,
            ExportFormatArg::Csv => ExportFormat::Csv,
            ExportFormatArg::CsvEdges => ExportFormat::CsvEdges,
            ExportFormatArg::Json => ExportFormat::Json,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum OutputScopeArg {
    Visible,
    LoadedInduced,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct CapturedArgs {
    /// Omit for legacy output. Explicit scope requires expected and subset_revision.
    #[serde(default)]
    scope: Option<OutputScopeArg>,
    #[serde(default)]
    expected: Option<ExpectedArg>,
    #[serde(default)]
    subset_revision: Option<String>,
    /// Omit for a preview; supply its exact digest to validate final output.
    #[serde(default)]
    preview_digest: Option<String>,
    /// Export-local file ID to session source-handle mapping, capped at 2 MiB.
    #[serde(default)]
    include_identity: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct ExportArgs {
    #[serde(default)]
    pub format: ExportFormatArg,
    #[serde(flatten)]
    pub captured: CapturedArgs,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct RenderArgs {
    #[serde(flatten)]
    pub captured: CapturedArgs,
    #[serde(default)]
    pub seed: u32,
    #[serde(default)]
    pub target: RenderTargetArg,
    /// Cypher to draw. Required when `target` is `cypher`, ignored otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Values for the query's `$name` placeholders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub format: FormatArg,
    #[serde(default)]
    pub theme: ThemeArg,
    /// Canvas width in pixels. Defaults to the app's own 2000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Canvas height in pixels. Defaults to 1250.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Force one arrangement instead of letting the structure choose. Unset
    /// reads the scene. `simulation` is not a rendering — there is no viewer's
    /// GPU behind an image — and is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<KernelArg>,
}

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

/// The MCP handler. One per session, cheap: it holds the shared state by
/// `Arc` and adds a router.
#[derive(Clone)]
pub struct ViewControl {
    state: AppState,
    tool_router: ToolRouter<ViewControl>,
}

#[tool_router]
impl ViewControl {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "What is on the shared screen right now: the slot space, the type nodes \
                       and their drill-in state, instance counts by type, tombstones, and what \
                       the response bound did to the last change. Read this before acting and \
                       after anything surprising — another party may have moved the view. \
                       `layout_kernel` says who owns the arrangement: `simulation` means the \
                       viewer's GPU does and this server cannot know where anything is; any \
                       other value is a static layout it computed, under which relative \
                       position is describable. `geometry_caveat` spells out whichever \
                       applies — read it rather than assuming."
    )]
    async fn view_state(&self) -> Result<CallToolResult, McpError> {
        let mut value = serde_json::to_value(self.state.session.view_state())
            .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        // The one fact `ViewState` structurally cannot carry: core knows the
        // slot space, only the transport knows whether anyone is watching it.
        // An agent steering a view with no viewers is talking to itself, and
        // that is worth saying out loud.
        value["connected_viewers"] = self.state.bus.client_count().into();
        ok_json(&value)
    }

    #[tool(
        description = "Run read-only Cypher and put the resulting nodes and relationships INTO \
                       the shared view, where the human can see them. Bounded in core: a result \
                       past the row ceiling comes back truncated and says so. This is a display \
                       verb — if you want to read a table, ask the graph's own MCP server."
    )]
    async fn show_cypher(
        &self,
        Parameters(shared): Parameters<SharedArgs<CypherArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        self.mutate(
            Request::Cypher(CypherRequest {
                query: args.query,
                params: args.params.map(into_params).unwrap_or_default(),
                limit: None,
                as_graph: true,
            }),
            options,
        )
        .await
    }

    #[tool(
        description = "Inspect typed fields for generation-scoped source node handles without \
                       changing the shared view. At most 500 rows, 32 fields, 5000 handles \
                       and 2 MiB per answer; default page size is 100. Missing, null, unavailable \
                       and truncated cells are explicit. Use returned next_offset to page. \
                       Handles are source identities, never renderer slots or Cypher id values. fields names source properties only; additive field_refs accepts canonical property/derived references discovered from view_state.calculations. Source fields precede field_refs and share the 32-field bound. Values outside a frozen calculation input are unavailable."
    )]
    async fn records(
        &self,
        Parameters(args): Parameters<RecordsArgs>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .run(Request::Records(RecordsRequest {
                handles: args.handles.into_iter().map(Into::into).collect(),
                fields: args.fields,
                field_refs: args.field_refs.map(catalog_args).transpose()?,
                offset: args.offset,
                limit: args.limit.unwrap_or(100),
            }))
            .await?
        {
            Ok(Response::Records(table)) => ok_json(&serde_json::json!(table)),
            Ok(_) => Err(McpError::internal_error(
                "records returned an unexpected response",
                None,
            )),
            Err(err) => Ok(refused(&err)),
        }
    }

    #[tool(
        description = "Read a bounded source field or nested value by node handle. Text pages use UTF-8 byte offsets; list/map pages use item offsets. The response preserves typed cells and explicit null, missing and unavailable states. At most 256 KiB per answer, 32 KiB text per page, 128 collection items and path depth 8. This is a private read and changes no shared state."
    )]
    async fn field_detail(
        &self,
        Parameters(args): Parameters<FieldDetailArgs>,
    ) -> Result<CallToolResult, McpError> {
        use kglite_visual_core::field_detail::{FieldDetailRequest, FieldPathSegment};
        let path = args
            .path
            .into_iter()
            .map(|segment| match segment {
                FieldPathArg::Index { index } => FieldPathSegment::Index { index },
                FieldPathArg::Key { key } => FieldPathSegment::Key { key },
            })
            .collect();
        match self
            .run(Request::FieldDetail(FieldDetailRequest {
                handle: args.handle.into(),
                field: args.field,
                path,
                offset: args.offset,
                limit: args.limit,
            }))
            .await?
        {
            Ok(Response::FieldDetail(detail)) => ok_json(&serde_json::json!(detail)),
            Ok(_) => Err(McpError::internal_error(
                "field detail returned an unexpected response",
                None,
            )),
            Err(error) => Ok(refused(&error)),
        }
    }

    #[tool(
        description = "Load a bounded set of source nodes of one type into the shared view, \
                       including disconnected nodes. Does not require choosing a relationship. \
                       Returns exact session handles for record inspection and loading; reports \
                       the core bound and broadcasts the same slice to all attached browsers."
    )]
    async fn browse_type(
        &self,
        Parameters(shared): Parameters<SharedArgs<BrowseTypeArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        self.mutate(
            Request::BrowseType(BrowseTypeRequest {
                node_type: args.node_type,
                limit: args.limit,
            }),
            options,
        )
        .await
    }

    #[tool(
        description = "Load exact generation-scoped source node handles into the shared view. \
                       A handle never uses a Cypher id-field value or a renderer slot, so duplicate \
                       keys and slot compaction cannot redirect a selected row. Stale generation \
                       handles are refused. Core bounds apply and all browsers receive the change."
    )]
    async fn load_nodes(
        &self,
        Parameters(shared): Parameters<SharedArgs<LoadNodesArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        self.mutate(
            Request::LoadNodes(LoadNodesRequest {
                handles: args.handles.into_iter().map(Into::into).collect(),
            }),
            options,
        )
        .await
    }

    #[tool(
        description = "Load exact source node and relationship references returned by a query row. Relationships retain their source edge IDs, including parallel relations and self-loops; scalar values are never interpreted as identities. Stale handles or mismatched edge endpoints refuse without changing the view. This shared mutation accepts expected revision stamps and broadcasts one bounded update."
    )]
    async fn load_entities(
        &self,
        Parameters(shared): Parameters<SharedArgs<LoadEntitiesArgs>>,
    ) -> Result<CallToolResult, McpError> {
        use kglite_visual_core::query_provenance::{LoadEntitiesRequest, RelationHandle};
        let SharedArgs { args, options } = shared;
        self.mutate(
            Request::LoadEntities(LoadEntitiesRequest {
                nodes: args.nodes.into_iter().map(Into::into).collect(),
                relationships: args
                    .relationships
                    .into_iter()
                    .map(|relation| RelationHandle {
                        generation: relation.generation,
                        edge_id: relation.edge_id,
                        source: relation.source.into(),
                        target: relation.target.into(),
                    })
                    .collect(),
            }),
            options,
        )
        .await
    }

    #[tool(
        description = "Load a slot's neighbours into the shared view. A type slot loads \
                       instances of that type; an instance slot loads what it is connected to. \
                       Bounded in core — `limit` is a request, not a guarantee, and the answer \
                       reports what was cut. Naming a `relationship` is the cheap case; walking \
                       every type is the expensive one."
    )]
    async fn expand(
        &self,
        Parameters(shared): Parameters<SharedArgs<ExpandArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        self.mutate(
            Request::Expand(ExpandRequest {
                slot: args.slot,
                relationship: args.relationship,
                direction: args.direction.into(),
                limit: args.limit,
            }),
            options,
        )
        .await
    }

    #[tool(
        description = "Remove a slot's expansion from the shared view. A type slot removes every \
                       instance of that type; an instance slot removes itself. The slot numbers \
                       that were in use are not reissued, so anything you were holding stays \
                       valid — unless the answer carries a compaction, which renumbers \
                       everything and is reported."
    )]
    async fn collapse(
        &self,
        Parameters(shared): Parameters<SharedArgs<SlotArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        self.mutate(Request::Collapse(SlotRequest { slot: args.slot }), options)
            .await
    }

    #[tool(
        description = "Make things stand out on the human's screen. Either name `slots`, or give \
                       a `search` string and let the server find them — search hits that are \
                       already loaded are marked, and the ones that are not are counted back to \
                       you so you know what to `show_cypher` first. `concept` picks the channel: \
                       `highlighted` for a set of results, `selected` for the one thing you are \
                       talking about."
    )]
    async fn highlight(
        &self,
        Parameters(shared): Parameters<SharedArgs<HighlightArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, mut options } = shared;
        if args.search.is_some() && options.expected.is_none() {
            let stamp = self.state.session.shared_stamp();
            options.expected = Some(ExpectedArg {
                generation: stamp.generation,
                revision: stamp.revision,
            });
        }
        let concept: HighlightConcept = args.concept.into();
        let (slots, note) = match &args.search {
            None => (args.slots.clone(), None),
            Some(needle) => {
                let request = Request::Search(SearchRequest {
                    query: needle.clone(),
                    node_type: args.node_type.clone(),
                    property: None,
                    mode: Default::default(),
                    limit: None,
                });
                let response = match self.run(request).await? {
                    Ok(response) => response,
                    Err(err) => return Ok(refused(&err)),
                };
                let Response::Search(search) = response else {
                    return Err(McpError::internal_error(
                        "a search request answered with something other than search hits",
                        None,
                    ));
                };
                let on_screen: Vec<u32> = search.hits.iter().filter_map(|hit| hit.slot).collect();
                let cold = search.hits.len() - on_screen.len();
                (
                    on_screen,
                    Some(serde_json::json!({
                        "hits": search.hits.len(),
                        "hits_not_loaded": cold,
                        "bound": search.bound,
                    })),
                )
            }
        };

        let execution = match self
            .execute(
                Request::Highlight(HighlightRequest {
                    slots: slots.clone(),
                    concept,
                }),
                options,
            )
            .await?
        {
            Ok(execution) => execution,
            Err(error) => return Ok(refused(&error)),
        };
        ok_json(&serde_json::json!({
            "marked": slots.len(),
            "slots": slots,
            "concept": concept,
            "connected_viewers": self.state.bus.client_count(),
            "stamp": execution.stamp,
            "request_id": execution.request_id,
            "search": note,
        }))
    }

    #[tool(
        description = "Zoom the human's camera to frame these slots — the honest way to say \
                       'look at this', because you cannot see their screen and they can. An \
                       empty list frames the whole view, which is what you want after a \
                       collapse. Changes nothing about what is loaded."
    )]
    async fn focus(
        &self,
        Parameters(shared): Parameters<SharedArgs<FocusArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        let execution = match self
            .execute(
                Request::Focus(FocusRequest {
                    slots: args.slots.clone(),
                }),
                options,
            )
            .await?
        {
            Ok(execution) => execution,
            Err(error) => return Ok(refused(&error)),
        };
        ok_json(&serde_json::json!({
            "framed": args.slots.len(), "slots": args.slots,
            "connected_viewers": self.state.bus.client_count(), "stamp": execution.stamp,
            "request_id": execution.request_id,
        }))
    }

    #[tool(
        description = "Set shared colour/size source or calculated field channels and optional presentation settings. Use canonical color_field/size_field property or derived references; legacy color_by/size_by always name source properties. Contradictory canonical and legacy inputs refuse atomically. Without presentation, omitted/null channels clear to structural defaults. Presentation-only preserves both channels. When any channel is supplied, omitted channels clear. Explicit channel fields plus presentation change atomically; explicit null clears a channel. Presentation defaults are applied to omitted presentation fields; density/opacity0–1 and finite numeric size range are bounded in core."
    )]
    async fn set_appearance(
        &self,
        Parameters(shared): Parameters<SharedArgs<AppearanceArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        let has_channels = args.color_by.is_some()
            || args.size_by.is_some()
            || args.color_field.is_some()
            || args.size_field.is_some();
        let mut fields = serde_json::to_value(&args)
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        fields
            .as_object_mut()
            .expect("appearance args object")
            .remove("presentation");
        let appearance: AppearanceRequest = catalog_args(fields)?;
        if let Some(presentation) = args.presentation {
            let presentation = catalog_args(presentation)?;
            let request = if has_channels {
                Request::Style(kglite_visual_core::presentation::StyleRequest {
                    appearance: Some(appearance),
                    presentation: Some(presentation),
                })
            } else {
                Request::Presentation(presentation)
            };
            return self.settings(request, options).await;
        }
        let execution = match self
            .execute(Request::Appearance(appearance), options)
            .await?
        {
            Ok(execution) => execution,
            Err(error) => return Ok(refused(&error)),
        };
        let Response::Shared(snapshot) = execution.response else {
            return Err(McpError::internal_error(
                "appearance returned an unexpected response",
                None,
            ));
        };
        let mut value = serde_json::to_value(snapshot.meta.appearance)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        value["connected_viewers"] = self.state.bus.client_count().into();
        value["stamp"] = serde_json::json!(execution.stamp);
        value["request_id"] = serde_json::json!(execution.request_id);
        ok_json(&value)
    }

    #[tool(
        description = "Calculate degree or weak components over the exact visible retained relation multiset. Degree exposes in/out/total; parallel relations count separately and a self-loop adds one in, one out, two total. Weak components ignore direction and include isolates. Results are frozen namespaced derived fields for records, appearance and filters; applying a filter never recursively recalculates them. Omit calculation_id for a new result, or name a matching existing result to recompute explicitly on the current view. At most eight results are retained; source properties are unchanged. Pass expected from view_state to refuse a changed input; one acknowledged update reaches every viewer."
    )]
    async fn calculate(
        &self,
        Parameters(shared): Parameters<SharedArgs<CalculateArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        let request = catalog_args::<_, kglite_visual_core::calculations::CalculateRequest>(args)?;
        self.settings(Request::Calculate(request), options).await
    }

    #[tool(
        description = "Replace the enabled predicate list over already loaded instances and relationships. Predicates combine by conjunction; empty predicates clear the filters. Type, category, exact numeric range, missing/null, relation type and explicit hide-isolated predicates are supported. Source search is unchanged. Pass expected generation/revision to refuse overwriting a peer's later state; the returned subset counts are authoritative."
    )]
    async fn set_subset(
        &self,
        Parameters(shared): Parameters<SharedArgs<SubsetArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        let request: SubsetRequest = serde_json::from_value(serde_json::json!(args))
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        self.settings(Request::Subset(request), options).await
    }

    #[tool(
        description = "Set the shared caption property for loaded instance nodes, or clear it with null to use their source titles. The choice is acknowledged with a revision and restored when another browser connects. Source data is unchanged; expected generation/revision prevents replacing a later peer choice."
    )]
    async fn set_caption(
        &self,
        Parameters(shared): Parameters<SharedArgs<CaptionArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        self.settings(
            Request::Caption(CaptionRequest {
                caption_by: args.caption_by,
            }),
            options,
        )
        .await
    }

    #[tool(
        description = "Re-arrange the shared view with a layout computed HERE, and hold it \
                       still. This is the one tool that changes what you can know: under a \
                       static kernel the viewer's simulation is off and dragging is disabled, \
                       so the arrangement on their screen is the one this server sent, and \
                       relative position ('the ring around X', 'the island on the left') \
                       becomes safe to describe. `auto` reads the structure and picks; \
                       `radial` needs a `seed_slot` to centre on; `islands` groups \
                       communities; `force` is a held-still force pass; `geo` puts every \
                       node with coordinates where it actually is (unplaceable nodes go to \
                       a labelled tray); `simulation` hands the layout back to their GPU and \
                       takes that knowledge away again. Check `kernel_chosen` in the answer — a \
                       kernel with nothing to work with falls back and says so. Changes \
                       nothing about what is loaded."
    )]
    async fn set_layout(
        &self,
        Parameters(shared): Parameters<SharedArgs<LayoutArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        let request = Request::Layout(LayoutRequest {
            kernel: args.kernel.into(),
            seed_slot: args.seed_slot,
        });
        let execution = match self.execute(request, options).await? {
            Ok(execution) => execution,
            Err(err) => return Ok(refused(&err)),
        };
        let response = execution.response;
        let Response::Layout(result) = &response else {
            return Err(McpError::internal_error(
                "a layout request answered with something other than a layout",
                None,
            ));
        };
        // The caveat is re-stated per call rather than left to `view_state`,
        // because this is the call that changed which one is true — and an
        // agent that acts on the old one describes a screen it cannot see.
        ok_json(&serde_json::json!({
            "stamp": execution.stamp,
            "request_id": execution.request_id,
            "kernel_requested": result.meta.kernel_requested,
            "kernel_chosen": result.meta.kernel_chosen,
            "seed_slot": result.meta.seed_slot,
            "slots_placed": result.meta.live_count,
            "layout_ms": result.meta.layout_ms,
            "connected_viewers": self.state.bus.client_count(),
            "geometry_caveat": geometry_caveat(result.meta.kernel_chosen),
        }))
    }

    #[tool(
        description = "Collapse everything back to the entry screen — the type-level meta-graph \
                       the session opened with. Destructive to the human's place in the graph, \
                       so prefer `collapse` on what you added. The type nodes stay; only \
                       instances are removed."
    )]
    async fn reset_view(
        &self,
        Parameters(options): Parameters<SharedOptions>,
    ) -> Result<CallToolResult, McpError> {
        self.mutate(Request::Reset, options).await
    }

    #[tool(
        description = "Draw a deterministic server image, never a browser screenshot. Omitted scope preserves legacy target behavior: live-view includes loaded content and schema context, including hidden nodes; meta/cypher draw a private scene. Explicit visible or loaded-induced scope requires expected and subset_revision from view_state. No preview_digest returns an image preview with digest; supplying the digest validates the exact settings before final output. Read scope/counts/folding/label/geometry notes; positions differ from the browser camera and simulation."
    )]
    async fn render(
        &self,
        Parameters(args): Parameters<RenderArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.captured.scope.is_some() {
            return output::render_captured(self, args).await;
        }
        if args.captured.has_capture_fields() {
            return Ok(refused_text(
                "captured output fields require an explicit scope",
            ));
        }

        let source = match args.target {
            RenderTargetArg::LiveView => RenderSource::LiveView,
            RenderTargetArg::Meta => RenderSource::Meta,
            RenderTargetArg::Cypher => {
                let Some(query) = args.query.clone() else {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(
                        "`target: cypher` needs a `query`. Pass one, or use \
                         `target: live-view` to draw what is already on screen.",
                    )]));
                };
                RenderSource::Cypher(CypherRequest {
                    query,
                    params: args.params.clone().map(into_params).unwrap_or_default(),
                    limit: None,
                    as_graph: true,
                })
            }
        };
        let format = match args.format {
            FormatArg::Png => RenderFormat::Png,
            FormatArg::Svg => RenderFormat::Svg,
        };
        let request = RenderRequest {
            source,
            format,
            width: args
                .width
                .unwrap_or(kglite_visual_core::render::DEFAULT_WIDTH),
            height: args
                .height
                .unwrap_or(kglite_visual_core::render::DEFAULT_HEIGHT),
            seed: u64::from(args.seed),
            theme: match args.theme {
                ThemeArg::Dark => Theme::Dark,
                ThemeArg::Light => Theme::Light,
            },
            kernel: match args.layout {
                Some(KernelArg::Simulation) => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(
                        "`layout: simulation` hands the arrangement to the viewer's GPU, \
                         and an image has no viewer. Use the `layout` tool for that, or \
                         name a kernel this render can compute: auto, radial, islands, \
                         force, geo.",
                    )]))
                }
                other => other.map(Into::into),
            },
        };

        let session = Arc::clone(&self.state.session);
        let rendered = match tokio::task::spawn_blocking(move || {
            kglite_visual_core::render_for(&session, &request)
        })
        .await
        .map_err(|err| McpError::internal_error(format!("render task failed: {err}"), None))?
        {
            Ok(rendered) => rendered,
            Err(err) => return Ok(refused(&err)),
        };

        // The counts and the truncation state travel beside the picture as well
        // as inside it: an image is not introspectable, and a caller that only
        // read the text half must still learn the answer was clipped.
        let mut summary = serde_json::json!({
            "target": args.target,
            "nodes": rendered.nodes,
            "links": rendered.links,
            "truncated": rendered.truncated,
            "banners": rendered.banners,
            "width": rendered.width,
            "height": rendered.height,
            // The caveat about the *view*, not about this picture: what an
            // agent does with a render is talk to the user about their screen,
            // and which claims that permits is the live view's question. The
            // description above is where this pass's own independence is said.
            "geometry_caveat": geometry_caveat(self.state.session.layout_kernel()),
        });
        // Added rather than always present, so a key that *is* there always
        // carries a number: the canvas clipped the schema, or the grid thinned
        // a name off the picture. An agent reading the text half instead of
        // opening the image learns the same two things the status block draws.
        for (key, value) in [
            ("types_shown", rendered.types_shown),
            ("types_total", rendered.types_total),
            ("names_shown", rendered.names_shown),
        ] {
            if let (Some(value), Some(object)) = (value, summary.as_object_mut()) {
                object.insert(key.to_string(), value.into());
            }
        }
        let picture = match format {
            RenderFormat::Png => ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(&rendered.bytes),
                "image/png",
            ),
            // SVG is not an image content block: the MCP image type carries a
            // raster, and a client handed base64 XML under `image/svg+xml`
            // renders a broken thumbnail. As text it is at least readable.
            RenderFormat::Svg => ContentBlock::text(String::from_utf8_lossy(&rendered.bytes)),
        };
        Ok(CallToolResult::success(vec![
            ContentBlock::text(summary.to_string()),
            picture,
        ]))
    }

    #[tool(
        description = "Export graph text as GraphML, GEXF, CSV or D3 JSON. Omitted scope preserves legacy loaded nodes and induced source relations, including hidden or previously unretained relations. Explicit visible scope keeps the exact visible retained relation multiset; loaded-induced explicitly requests the wider relation scope. Explicit scope requires expected and subset_revision. No preview_digest returns preview metadata only; supplying it validates unchanged scope/settings before returning final text. include_identity optionally maps export-local file IDs to session-scoped source handles under2MiB. Read format fidelity notes; file IDs are not durable source keys."
    )]
    async fn export_view(
        &self,
        Parameters(args): Parameters<ExportArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.captured.scope.is_some() {
            return output::export_captured(self, args).await;
        }
        if args.captured.has_capture_fields() {
            return Ok(refused_text(
                "captured output fields require an explicit scope",
            ));
        }

        let session = Arc::clone(&self.state.session);
        let format: ExportFormat = args.format.into();
        let exported = match tokio::task::spawn_blocking(move || session.export_view(format))
            .await
            .map_err(|err| McpError::internal_error(format!("export task failed: {err}"), None))?
        {
            Ok(exported) => exported,
            Err(err) => return Ok(refused(&err)),
        };

        // The bytes as text, not as an attachment: every one of these formats
        // is UTF-8 text, an MCP reply has no file channel, and base64 would be
        // an agent's decode step for something it can already read. The counts
        // come first so a caller that stops reading at the summary still learns
        // the size of what follows.
        let summary = serde_json::json!({
            "format": exported.format.as_str(),
            "filename": exported.filename,
            "nodes": exported.nodes,
            "bytes": exported.bytes.len(),
            "notes": exported.notes(),
        });
        let text = String::from_utf8_lossy(&exported.bytes).into_owned();
        Ok(CallToolResult::success(vec![
            ContentBlock::text(summary.to_string()),
            ContentBlock::text(text),
        ]))
    }

    #[tool(
        description = "The Cypher this user has SAVED for this graph, plus the queries recently \
                       run from the panel. Read it before writing a query of your own: a saved \
                       query is what the person you are working with already decided was worth \
                       keeping, and it will use their names for things. Returns names and query \
                       text; run one with `run_saved_query`."
    )]
    async fn list_saved_queries(&self) -> Result<CallToolResult, McpError> {
        let store = Arc::clone(&self.state.queries);
        let file = match tokio::task::spawn_blocking(move || store.list())
            .await
            .map_err(|err| McpError::internal_error(format!("store task failed: {err}"), None))?
        {
            Ok(file) => file,
            Err(err) => return Ok(refused_text(&err.to_string())),
        };
        ok_json(&serde_json::json!({
            "graph": file.graph_label,
            "saved": file.saved,
            // Recent, not exhaustive, and capped — say so rather than let a
            // short list read as "this is everything that ran".
            "recent": file.history,
            "recent_cap": crate::queries::MAX_HISTORY,
        }))
    }

    #[tool(
        description = "Run one of this user's saved queries by name and put its nodes and \
                       relationships INTO the shared view — `show_cypher` with the text taken \
                       from the store instead of from you, so it executes by exactly the same \
                       path and is bounded in core exactly the same way. `list_saved_queries` \
                       has the names. The run is added to the user's recent list, because they \
                       are watching it happen."
    )]
    async fn run_saved_query(
        &self,
        Parameters(shared): Parameters<SharedArgs<SavedQueryArgs>>,
    ) -> Result<CallToolResult, McpError> {
        let SharedArgs { args, options } = shared;
        let store = Arc::clone(&self.state.queries);
        let name = args.name.clone();
        let found = match tokio::task::spawn_blocking(move || store.get(&name))
            .await
            .map_err(|err| McpError::internal_error(format!("store task failed: {err}"), None))?
        {
            Ok(found) => found,
            Err(err) => return Ok(refused_text(&err.to_string())),
        };
        let Some(query) = found else {
            return Ok(refused_text(&format!(
                "this graph has no saved query named {:?}. `list_saved_queries` names them.",
                args.name
            )));
        };

        // Recorded before the run, not after: history is "what was asked for",
        // and a query that failed is exactly the one a user wants back in the
        // editor to fix. A store failure here must not stop the run.
        let store = Arc::clone(&self.state.queries);
        let recorded = query.clone();
        if let Ok(Err(err)) = tokio::task::spawn_blocking(move || store.record(&recorded)).await {
            eprintln!("kglite-visual: could not record query history: {err}");
        }

        self.mutate(
            Request::Cypher(CypherRequest {
                query,
                params: Default::default(),
                limit: None,
                as_graph: true,
            }),
            options,
        )
        .await
    }

    #[tool(
        description = "List bounded saved-view names in the global durable catalog and this session's temporary catalog, their separate quotas, and current save eligibility. Durable eligibility is verified on save."
    )]
    async fn list_views(&self) -> Result<CallToolResult, McpError> {
        catalog_result(crate::views_api::list_value(self.state.clone()).await)
    }
    #[tool(
        description = "Save this exact exploration without replaying Cypher. Source identity decides durable or session-only storage. Names have explicit replace intent; no quota evicts another view. A peer change during IO can save the captured revision while leaving the live view dirty."
    )]
    async fn save_view(
        &self,
        Parameters(args): Parameters<SharedArgs<SaveViewArgs>>,
    ) -> Result<CallToolResult, McpError> {
        catalog_result(crate::views_api::save_value(self.state.clone(), catalog_args(args)?).await)
    }
    #[tool(
        description = "Restore an exact named exploration from durable or session storage after source identity and bounds validation. Pass expected to refuse newer shared revisions. Unsupported or changed sources leave the current view intact."
    )]
    async fn restore_view(
        &self,
        Parameters(args): Parameters<SharedArgs<NamedViewArgs>>,
    ) -> Result<CallToolResult, McpError> {
        catalog_result(
            crate::views_api::restore_value(self.state.clone(), catalog_args(args)?).await,
        )
    }
    #[tool(
        description = "Delete one explicitly named saved view from durable or session storage. Other names and the current graph contents are preserved; the matching saved marker is cleared."
    )]
    async fn delete_view(
        &self,
        Parameters(args): Parameters<SharedArgs<NamedViewArgs>>,
    ) -> Result<CallToolResult, McpError> {
        catalog_result(
            crate::views_api::delete_value(self.state.clone(), catalog_args(args)?).await,
        )
    }
    #[tool(
        description = "Read bounded recovery checkpoints for this shared session, including the oldest available checkpoint and eviction count. Recovery history is separate from named saved views."
    )]
    async fn view_history(&self) -> Result<CallToolResult, McpError> {
        catalog_result(crate::views_api::history_value(self.state.clone()).await)
    }
    #[tool(
        description = "Restore a recovery checkpoint as a new shared change, without rerunning its query. Pass expected from view_state; stale or evicted checkpoints refuse without changing the current view."
    )]
    async fn restore_history(
        &self,
        Parameters(args): Parameters<SharedArgs<HistoryRestoreArgs>>,
    ) -> Result<CallToolResult, McpError> {
        catalog_result(
            crate::views_api::history_restore_value(self.state.clone(), catalog_args(args)?).await,
        )
    }

    async fn settings(
        &self,
        request: Request,
        options: SharedOptions,
    ) -> Result<CallToolResult, McpError> {
        match self.execute(request, options).await? {
            Ok(execution) => {
                let Response::Shared(snapshot) = execution.response else {
                    return Err(McpError::internal_error(
                        "settings returned an unexpected response",
                        None,
                    ));
                };
                ok_json(
                    &serde_json::json!({"state": snapshot.meta, "request_id": execution.request_id}),
                )
            }
            Err(error) => Ok(refused(&error)),
        }
    }

    /// Run a view-mutating request, broadcast it, and report what it did.
    async fn mutate(
        &self,
        request: Request,
        options: SharedOptions,
    ) -> Result<CallToolResult, McpError> {
        let execution = match self.execute(request, options).await? {
            Ok(execution) => execution,
            Err(error) => return Ok(refused(&error)),
        };
        self.slice_report(&execution)
    }

    async fn execute(
        &self,
        request: Request,
        options: SharedOptions,
    ) -> Result<Result<Execution, CoreError>, McpError> {
        match self.state.execute(options.request(request)).await {
            Ok(execution) => Ok(Ok(execution)),
            Err(DispatchError::Core(error)) => Ok(Err(error)),
            Err(DispatchError::Task(message)) => Err(McpError::internal_error(message, None)),
        }
    }

    /// Dispatch off the reactor.
    ///
    /// The inner `Result` is the *session's* — a refusal an agent can act on
    /// (a bad slot, kglite's own parse error) — while the outer is a bug here.
    /// Collapsing them would send a Cypher syntax error to the client as a
    /// protocol error, which MCP clients render as "tool result missing due to
    /// internal error": the one message that helps nobody.
    async fn run(&self, request: Request) -> Result<Result<Response, CoreError>, McpError> {
        debug_assert!(
            !request.is_shared(),
            "shared requests require ordered execution"
        );
        let session = Arc::clone(&self.state.session);
        tokio::task::spawn_blocking(move || session.handle(&request))
            .await
            .map_err(|err| McpError::internal_error(format!("request task failed: {err}"), None))
    }

    /// What a caller needs to know about a slice, without the float arrays.
    ///
    /// The positions and the whole link list are on the wire for the renderer;
    /// an agent has no use for ten thousand coordinates and every use for the
    /// slots it just gained. Sending the arrays would be tokens spent on
    /// numbers nobody reads.
    fn slice_report(&self, execution: &Execution) -> Result<CallToolResult, McpError> {
        let Response::Slice(slice) = &execution.response else {
            return Err(McpError::internal_error(
                "a view-mutating request answered with something other than a slice",
                None,
            ));
        };
        let added: Vec<serde_json::Value> = slice
            .meta
            .nodes
            .iter()
            .map(|node| {
                serde_json::json!({
                    "slot": node.slot,
                    "node_id": node.node_id,
                    "handle": node.handle,
                    "typed_key": node.typed_key,
                    "type": node.node_type,
                    "title": node.title,
                })
            })
            .collect();
        ok_json(&serde_json::json!({
            "stamp": execution.stamp,
            "request_id": execution.request_id,
            "kind": slice.meta.kind,
            "added": added,
            "collapsed_slots": slice.meta.tombstones,
            "slot_count": slice.meta.slot_count,
            "tombstone_count": slice.meta.tombstone_count,
            "link_count": slice.meta.edges.len(),
            "bound": slice.meta.bound,
            "link_bound": slice.meta.link_bound,
            // A compaction renumbered every slot. Anything the caller was
            // holding is stale, and saying so is cheaper than sending the remap
            // to a peer that keeps no slot map of its own.
            "compacted": slice.compaction.is_some(),
            "connected_viewers": self.state.bus.client_count(),
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ViewControl {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "kglite-visual",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }
}

/// The mountable service.
///
/// `LocalSessionManager` because the sessions are in this process and die with
/// it, which is correct: an MCP session here is a conversation about a view
/// that also dies with the process.
pub fn service(state: AppState) -> StreamableHttpService<ViewControl, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(ViewControl::new(state.clone())),
        Arc::new(LocalSessionManager::default()),
        // rmcp's default `allowed_hosts` is loopback-only DNS-rebinding
        // protection, which is exactly this project's own bind rule
        // (`server::bind` — 127.0.0.1 only). Left alone deliberately: the two
        // agree, and widening one without the other would be a security
        // posture nobody decided.
        StreamableHttpServerConfig::default(),
    )
}

/// A session refusal, in the client's face rather than in the protocol.
///
/// `CallToolResult::error`, not `Err(McpError)`. MCP clients render a protocol
/// error opaquely — "tool result missing due to internal error" — and throw the
/// message away, which for a Cypher syntax error means discarding the position,
/// the expected token and the schema name kglite spent effort producing.
fn refused(err: &CoreError) -> CallToolResult {
    if let CoreError::Conflict(conflict) = err {
        return CallToolResult::error(vec![ContentBlock::text(
            serde_json::json!(conflict).to_string(),
        )]);
    }
    CallToolResult::error(vec![ContentBlock::text(err.to_string())])
}

/// The same shape as [`refused`], for a failure that is not a [`CoreError`].
///
/// The saved-query store's refusals are ceilings and missing names, which an
/// agent can act on the same way it acts on a bad slot — so they take the same
/// route, into the client's face rather than into the protocol.
fn refused_text(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_string())])
}

fn catalog_args<T: Serialize, U: serde::de::DeserializeOwned>(args: T) -> Result<U, McpError> {
    serde_json::to_value(args)
        .and_then(serde_json::from_value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))
}
fn catalog_result(
    result: Result<serde_json::Value, DispatchError>,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => ok_json(&value),
        Err(DispatchError::Core(error)) => Ok(refused(&error)),
        Err(DispatchError::Task(message)) => Err(McpError::internal_error(message, None)),
    }
}

fn ok_json(value: &serde_json::Value) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        value.to_string(),
    )]))
}

fn into_params(
    map: serde_json::Map<String, serde_json::Value>,
) -> std::collections::BTreeMap<String, serde_json::Value> {
    map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite_visual_core::{GEOMETRY_CAVEAT, GEOMETRY_STATIC_CAVEAT};

    /// Names are the API: a count alone cannot detect a rename.
    const EXPECTED: [&str; 27] = [
        "browse_type",
        "calculate",
        "collapse",
        "delete_view",
        "expand",
        "export_view",
        "field_detail",
        "focus",
        "highlight",
        "list_saved_queries",
        "list_views",
        "load_entities",
        "load_nodes",
        "records",
        "render",
        "reset_view",
        "restore_history",
        "restore_view",
        "run_saved_query",
        "save_view",
        "set_appearance",
        "set_caption",
        "set_layout",
        "set_subset",
        "show_cypher",
        "view_history",
        "view_state",
    ];

    fn router() -> ToolRouter<ViewControl> {
        ViewControl::tool_router()
    }

    #[test]
    fn the_surface_is_exactly_the_tools_the_design_fixed() {
        let mut names: Vec<String> = router()
            .list_all()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(names, EXPECTED);
    }

    #[test]
    fn every_tool_carries_a_description_and_an_input_schema() {
        // A tool with no description is a tool an agent picks by name alone,
        // which is how `collapse` gets called on a graph the caller meant to
        // expand.
        for tool in router().list_all() {
            let description = tool
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("{} has no description", tool.name));
            assert!(
                description.len() > 80,
                "{}'s description is too short to choose it by: {description:?}",
                tool.name
            );
            assert_eq!(
                tool.input_schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "{}'s input schema is not an object",
                tool.name
            );
        }
    }

    #[test]
    fn the_instructions_say_the_things_an_agent_gets_wrong_without_them() {
        // Each substring is a specific failure this string exists to prevent —
        // see the doc comment on INSTRUCTIONS. Asserted here so a later edit
        // that "tightens the wording" cannot quietly drop one.
        for phrase in [
            "human being is looking at",
            "stale prepared work always refuses",
            // Export defaults and exact visible scope are distinct contracts.
            "including content hidden by filters",
            "exports exactly the visible retained instance subset",
            "It refuses empty instance scope",
            // Conditional since G3, so the phrase asserted is the condition
            // rather than the old absolute claim: an agent that reads only
            // "you cannot know geometry" would never reach for `set_layout`.
            "depends on `view_state.layout_kernel`",
            "kglite-mcp-server",
        ] {
            assert!(
                INSTRUCTIONS.contains(phrase),
                "the instructions no longer say {phrase:?}"
            );
        }
    }

    #[test]
    fn the_geometry_caveat_is_cores_one_copy_and_switches_on_the_kernel() {
        // Two wordings of this caveat is one wording that stops being true —
        // and since G3 there are two *caveats*, so the thing that must stay
        // single is the function choosing between them.
        assert!(GEOMETRY_CAVEAT.contains("geometry-different"));
        assert_eq!(geometry_caveat(LayoutKernel::Simulation), GEOMETRY_CAVEAT);
        assert_eq!(geometry_caveat(LayoutKernel::Auto), GEOMETRY_CAVEAT);
        for kernel in [
            LayoutKernel::Radial,
            LayoutKernel::Islands,
            LayoutKernel::Force,
        ] {
            assert_eq!(
                geometry_caveat(kernel),
                GEOMETRY_STATIC_CAVEAT,
                "{kernel:?} is a layout this server computed and can describe"
            );
        }

        let render = router()
            .get("render")
            .expect("render is in the surface")
            .description
            .clone()
            .expect("render has a description");
        assert!(
            render.contains("never a browser screenshot")
                && render.contains("positions differ from the browser camera and simulation"),
            "the render tool must warn about geometry in its own description too"
        );
    }
}

#[cfg(test)]
#[path = "mcp_calculations.rs"]
mod calculation_tests;
