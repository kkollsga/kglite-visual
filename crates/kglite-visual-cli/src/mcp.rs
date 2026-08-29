//! The MCP face: an agent driving the view a human is watching (plan D14).
//!
//! **Served BY the running server, at `/mcp`.** There is no second process, no
//! discovery file and no new place for state to accumulate — "attach" is the
//! URL the launch contract already prints, now with an `mcp` key beside `url`.
//! rmcp's `StreamableHttpService` implements `tower_service::Service` with
//! `Error = Infallible`, so it mounts as one more route on the axum router that
//! is already serving the frontend, the JSON twin and the WebSocket.
//!
//! **The tool surface is small on purpose.** Data-heavy querying belongs to
//! `kglite-mcp-server`, which owns the schema, the Cypher reference and the
//! result formatting. These nine tools do the one thing that server cannot:
//! move a *shared* view. Every one of them is a verb about the screen.
//!
//! **Two callers, one view, last writer wins** (D14, v1). The human and the
//! agent are collaborators on one slot space, not two tenants of two. An
//! expansion either of them asks for is broadcast to both, and neither is
//! notified that the other did something — they see it.
//!
//! **What an agent can and cannot know.** It can know the content of the view
//! exactly: [`kglite_visual_core::ViewState`] is the same truth the browser's
//! `window.__kglv` reports. It cannot know the geometry — the layout runs on
//! the viewer's GPU and the server never receives the final positions — so
//! every surface here repeats `core`'s one
//! [`GEOMETRY_CAVEAT`](kglite_visual_core::GEOMETRY_CAVEAT).

use std::sync::Arc;

use base64::Engine as _;
use kglite_visual_core::control::{Appearance, Command, Focus, Highlight, HighlightConcept};
use kglite_visual_core::error::CoreError;
use kglite_visual_core::render::{RenderFormat, RenderRequest, RenderSource, Theme};
use kglite_visual_core::request::{
    CypherRequest, EdgeDirection, ExpandRequest, Request, SearchRequest, SlotRequest,
};
use kglite_visual_core::{Response, GEOMETRY_CAVEAT};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::{Deserialize, Serialize};

use crate::broadcast::AppState;

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
/// - *last writer wins* — otherwise an agent assumes the view it left is the
///   view it returns to.
/// - *geometry caveat* — otherwise an agent says "as you can see, top left",
///   which is a claim about a screen it has never seen.
/// - *data querying belongs elsewhere* — otherwise this becomes a worse
///   `kglite-mcp-server`, one tool at a time.
const INSTRUCTIONS: &str = "\
You are attached to a RUNNING kglite-visual window: an interactive graph view \
that a human being is looking at right now, in their browser. These tools move \
that view. Everything you do here is immediately visible to them.

Treat it as a shared workspace, not a scratchpad:
- Narrate what you are doing before you do it, so the change on screen is \
expected rather than startling.
- Prefer small, reversible steps. `expand` then `collapse` beats `reset_view`, \
which discards whatever the human had drilled into.
- Last writer wins. There is no locking and no conflict report: if the human \
clicks while you work, the view is whatever happened last. Call `view_state` \
to re-read it rather than assuming your last call still describes the screen.

What you can and cannot know:
- `view_state` is exact about CONTENT — slots, types, counts, tombstones, and \
what the response bound last truncated. It is the same truth the page's own \
debug hook reports.
- You cannot know GEOMETRY. The layout runs on the viewer's GPU and the server \
never receives the final positions. `render` draws the same content with its \
own deterministic layout: content-identical, geometry-different. Never tell the \
user where something is on their screen. Use `focus` and `highlight` to point \
at things instead — those move THEIR view, which is the honest way to say \
'look at this'.

Scope: this server steers a picture. It is not the place to mine the graph. \
Bulk querying, schema exploration and result tables belong to the graph's own \
MCP server (kglite-mcp-server); `show_cypher` here exists to put a result \
ON SCREEN, not to read it back.

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
    /// The live view — exactly the nodes and links on the human's screen,
    /// re-laid out server-side. The default.
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
    /// Property driving the colour channel, or omit/null to clear it back to
    /// the structural encoding. Must be a property the viewer's own
    /// property-statistics menu offers for the type on screen; a name nothing
    /// carries colours the view uniformly rather than failing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_by: Option<String>,
    /// Property driving the size channel. Same rules as `color_by`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_by: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
struct RenderArgs {
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
                       CONTENT only: it says nothing about where anything is on the user's \
                       screen, and it cannot."
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
        Parameters(args): Parameters<CypherArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.mutate(Request::Cypher(CypherRequest {
            query: args.query,
            params: args.params.map(into_params).unwrap_or_default(),
            limit: None,
            as_graph: true,
        }))
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
        Parameters(args): Parameters<ExpandArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.mutate(Request::Expand(ExpandRequest {
            slot: args.slot,
            relationship: args.relationship,
            direction: args.direction.into(),
            limit: args.limit,
        }))
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
        Parameters(args): Parameters<SlotArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.mutate(Request::Collapse(SlotRequest { slot: args.slot }))
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
        Parameters(args): Parameters<HighlightArgs>,
    ) -> Result<CallToolResult, McpError> {
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

        if let Err(err) = self.state.session.check_live_slots(&slots) {
            return Ok(refused(&err));
        }
        let clients = self
            .state
            .bus
            .publish_command(&Command::Highlight(Highlight::new(slots.clone(), concept)));
        ok_json(&serde_json::json!({
            "marked": slots.len(),
            "slots": slots,
            "concept": concept,
            "connected_viewers": clients,
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
        Parameters(args): Parameters<FocusArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(err) = self.state.session.check_live_slots(&args.slots) {
            return Ok(refused(&err));
        }
        let clients = self
            .state
            .bus
            .publish_command(&Command::Focus(Focus::new(args.slots.clone())));
        ok_json(&serde_json::json!({
            "framed": args.slots.len(),
            "slots": args.slots,
            "connected_viewers": clients,
        }))
    }

    #[tool(
        description = "Drive the colour-by and size-by channels of the shared view from node \
                       properties. Omitting a field clears that channel back to the app's \
                       structural encoding. The property name is not validated here — the \
                       viewer's own property statistics decide what is meaningful, and a name \
                       nothing carries renders uniformly rather than failing."
    )]
    async fn set_appearance(
        &self,
        Parameters(args): Parameters<AppearanceArgs>,
    ) -> Result<CallToolResult, McpError> {
        let clients = self
            .state
            .bus
            .publish_command(&Command::Appearance(Appearance::new(
                args.color_by.clone(),
                args.size_by.clone(),
            )));
        ok_json(&serde_json::json!({
            "color_by": args.color_by,
            "size_by": args.size_by,
            "connected_viewers": clients,
        }))
    }

    #[tool(
        description = "Collapse everything back to the entry screen — the type-level meta-graph \
                       the session opened with. Destructive to the human's place in the graph, \
                       so prefer `collapse` on what you added. The type nodes stay; only \
                       instances are removed."
    )]
    async fn reset_view(&self) -> Result<CallToolResult, McpError> {
        let session = Arc::clone(&self.state.session);
        let slice = tokio::task::spawn_blocking(move || session.reset())
            .await
            .map_err(|err| McpError::internal_error(format!("reset task failed: {err}"), None))?;
        let response = Response::Slice(slice);
        self.state.bus.publish_if_view_mutating(&response);
        self.slice_report(&response)
    }

    #[tool(
        description = "Draw an image you can actually look at. `target: live-view` (the default) \
                       draws exactly the nodes and links on the human's screen; `meta` and \
                       `cypher` draw something new WITHOUT touching their view. \
                       GEOMETRY DIFFERS FROM THEIR SCREEN: the arrangement here is this \
                       server's own deterministic layout, not the GPU simulation they are \
                       watching. Same nodes, same links, same truncation banner, different \
                       positions — so describe what is in the picture, never where it sits."
    )]
    async fn render(
        &self,
        Parameters(args): Parameters<RenderArgs>,
    ) -> Result<CallToolResult, McpError> {
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
            seed: 0,
            theme: match args.theme {
                ThemeArg::Dark => Theme::Dark,
                ThemeArg::Light => Theme::Light,
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
        let summary = serde_json::json!({
            "target": args.target,
            "nodes": rendered.nodes,
            "links": rendered.links,
            // Absent unless the canvas clipped the schema or the grid thinned a
            // name off the picture — an agent reading this instead of opening
            // the image gets the same two facts the status block draws.
            "types_shown": rendered.types_shown,
            "types_total": rendered.types_total,
            "names_shown": rendered.names_shown,
            "truncated": rendered.truncated,
            "banners": rendered.banners,
            "width": rendered.width,
            "height": rendered.height,
            "geometry_caveat": GEOMETRY_CAVEAT,
        });
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

    /// Run a view-mutating request, broadcast it, and report what it did.
    async fn mutate(&self, request: Request) -> Result<CallToolResult, McpError> {
        let response = match self.run(request).await? {
            Ok(response) => response,
            Err(err) => return Ok(refused(&err)),
        };
        self.state.bus.publish_if_view_mutating(&response);
        self.slice_report(&response)
    }

    /// Dispatch off the reactor.
    ///
    /// The inner `Result` is the *session's* — a refusal an agent can act on
    /// (a bad slot, kglite's own parse error) — while the outer is a bug here.
    /// Collapsing them would send a Cypher syntax error to the client as a
    /// protocol error, which MCP clients render as "tool result missing due to
    /// internal error": the one message that helps nobody.
    async fn run(&self, request: Request) -> Result<Result<Response, CoreError>, McpError> {
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
    fn slice_report(&self, response: &Response) -> Result<CallToolResult, McpError> {
        let Response::Slice(slice) = response else {
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
                    "type": node.node_type,
                    "title": node.title,
                })
            })
            .collect();
        ok_json(&serde_json::json!({
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
    CallToolResult::error(vec![ContentBlock::text(err.to_string())])
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

    /// The nine tools D14 fixed, by name.
    ///
    /// A list, not a count: "nine tools" would still pass if `focus` were
    /// renamed to `zoom`, and the name is the API — an agent's prompt refers to
    /// it and a rename breaks every conversation that mentions one.
    const EXPECTED: [&str; 9] = [
        "collapse",
        "expand",
        "focus",
        "highlight",
        "render",
        "reset_view",
        "set_appearance",
        "show_cypher",
        "view_state",
    ];

    fn router() -> ToolRouter<ViewControl> {
        ViewControl::tool_router()
    }

    #[test]
    fn the_surface_is_exactly_the_nine_tools_the_design_fixed() {
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
    fn the_instructions_say_the_four_things_an_agent_gets_wrong_without_them() {
        // Each substring is a specific failure this string exists to prevent —
        // see the doc comment on INSTRUCTIONS. Asserted here so a later edit
        // that "tightens the wording" cannot quietly drop one.
        for phrase in [
            "human being is looking at",
            "Last writer wins",
            "geometry-different",
            "kglite-mcp-server",
        ] {
            assert!(
                INSTRUCTIONS.contains(phrase),
                "the instructions no longer say {phrase:?}"
            );
        }
    }

    #[test]
    fn the_geometry_caveat_is_cores_one_copy() {
        // Two wordings of this caveat is one wording that stops being true.
        assert!(GEOMETRY_CAVEAT.contains("geometry-different"));
        let render = router()
            .get("render")
            .expect("render is in the surface")
            .description
            .clone()
            .expect("render has a description");
        assert!(
            render.contains("GEOMETRY DIFFERS FROM THEIR SCREEN"),
            "the render tool must warn about geometry in its own description too"
        );
    }
}
