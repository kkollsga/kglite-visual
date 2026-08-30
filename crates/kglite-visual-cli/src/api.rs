//! The JSON twin of the binary protocol (test-plan §2).
//!
//! Every answer the WebSocket can give has a plain-HTTP JSON rendering, from
//! the *same* response structs — one encoder, two serializers, so
//! twin-vs-binary divergence is a compile-time non-problem rather than a drift
//! bug class. The binary path is the performance path; the twin is how an
//! agent, or a human with `curl`, verifies server behaviour with no GPU
//! browser in the loop.
//!
//! **Every handler goes through `spawn_blocking`.** A Cypher execution or a
//! graph walk is synchronous and can run for the whole query deadline; on the
//! async runtime it would stall the WebSocket that is feeding the renderer.
//! The runtime is built with kglite's `QUERY_THREAD_STACK_SIZE`
//! (`main.rs`), and tokio applies that size to its blocking pool as well as its
//! workers — verified against tokio 1.53.1
//! (`runtime/blocking/pool.rs` takes `stack_size` from
//! `builder.thread_stack_size` and passes it to every spawned blocking thread),
//! which is what keeps the Cypher parser off tokio's 2 MiB default.

use std::sync::Arc;

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kglite_visual_core::control::{
    Appearance, AppearanceRequest, Command, Focus, FocusRequest, Highlight, HighlightRequest,
};
use kglite_visual_core::error::CoreError;
use kglite_visual_core::render::RenderRequest;
use kglite_visual_core::request::{
    CypherRequest, ExpandRequest, Request, SearchRequest, SlotRequest, TypeRequest,
};

use crate::broadcast::AppState;
use crate::queries;

/// `GET /api/meta-graph` — the entry screen, positions and links included.
pub async fn meta_graph(State(state): State<AppState>) -> Response {
    // Serialization only; the meta-graph itself was computed once at open.
    Json(state.session.meta_graph()).into_response()
}

/// `GET /api/session` — protocol version, tier, slot space, bounds, stats.
pub async fn session_info(State(state): State<AppState>) -> Response {
    Json(state.session.info()).into_response()
}

/// `GET /api/describe` — kglite's own schema document plus this session's tier
/// (plan D12): the same progressive-disclosure schema an agent gets from
/// kglite's MCP server, served by the running viz session.
pub async fn describe(State(state): State<AppState>) -> Response {
    // `compute_schema` walks per-type metadata rather than nodes, but it is
    // still the heaviest synchronous call in this file and it scales with the
    // schema, not with the request. Off the async runtime it goes: one slow
    // /api/describe must not stall the WebSocket feeding the renderer.
    let session = state.session;
    match tokio::task::spawn_blocking(move || session.describe()).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => task_failed("describe", &err),
    }
}

/// `POST /api/cypher` — `{"query": "...", "params": {...}, "limit": n,
/// "as_graph": bool}`.
pub async fn cypher(state: State<AppState>, Json(body): Json<CypherRequest>) -> Response {
    dispatch(state, Request::Cypher(body)).await
}

/// `POST /api/search` — `{"query": "...", "node_type": "...",
/// "property": "...", "mode": "contains"|"starts-with", "limit": n}`.
pub async fn search(state: State<AppState>, Json(body): Json<SearchRequest>) -> Response {
    dispatch(state, Request::Search(body)).await
}

/// `POST /api/preview` — `{"slot": n}`. Per-relationship counts, no fetch.
pub async fn preview(state: State<AppState>, Json(body): Json<SlotRequest>) -> Response {
    dispatch(state, Request::Preview(body)).await
}

/// `POST /api/expand` — `{"slot": n, "relationship": "...",
/// "direction": "out"|"in"|"both", "limit": n}`.
pub async fn expand(state: State<AppState>, Json(body): Json<ExpandRequest>) -> Response {
    dispatch(state, Request::Expand(body)).await
}

/// `POST /api/collapse` — `{"slot": n}`.
pub async fn collapse(state: State<AppState>, Json(body): Json<SlotRequest>) -> Response {
    dispatch(state, Request::Collapse(body)).await
}

/// `POST /api/node` — `{"slot": n}`. One node's stored properties.
pub async fn node_detail(state: State<AppState>, Json(body): Json<SlotRequest>) -> Response {
    dispatch(state, Request::NodeDetail(body)).await
}

/// `POST /api/render` — `{"source": {...}, "format": "svg"|"png", "width": n,
/// "height": n, "seed": n, "theme": "dark"|"light"}` (plan D13).
///
/// **The one endpoint that answers with bytes, not JSON.** Everything else here
/// is the JSON twin of a WebSocket message; this is an image, and an image
/// wrapped in JSON would be base64 an agent has to undo before it can look at
/// anything. The counts and the truncation state travel *inside the picture*
/// (D5's banner) and in the response headers, so a caller that only reads
/// headers still learns the answer was clipped.
///
/// **It does not touch this session's view.** For every source but one,
/// `core::render_for` opens a private session over the same read-only graph, so
/// a `POST /api/render` cannot move the slot space of whatever browser tab is
/// attached. `{"source": {"type": "live-view"}}` is the exception and the P10
/// addition: it *reads* this session and draws what is on the shared screen —
/// still without moving it. The geometry differs from the user's screen; core
/// owns that caveat's wording (`session::GEOMETRY_CAVEAT`).
pub async fn render(State(state): State<AppState>, Json(body): Json<RenderRequest>) -> Response {
    let session = Arc::clone(&state.session);
    // A render lays out a bounded slice and, for PNG, rasterises it: tens of
    // milliseconds, and on the async runtime that is the WebSocket feeding the
    // renderer stalling for all of them.
    match tokio::task::spawn_blocking(move || kglite_visual_core::render_for(&session, &body)).await
    {
        Ok(Ok(rendered)) => {
            let banners = rendered.banners.join("; ");
            let mut response = (
                [(CONTENT_TYPE, rendered.format.content_type())],
                rendered.bytes,
            )
                .into_response();
            let headers = response.headers_mut();
            for (name, value) in [
                ("x-kglv-nodes", rendered.nodes.to_string()),
                ("x-kglv-links", rendered.links.to_string()),
                ("x-kglv-truncated", rendered.truncated.to_string()),
                ("x-kglv-banner", banners),
            ] {
                // A header value has to be ASCII-safe, and a banner is built
                // from a graph's own type names. A name that cannot ride in a
                // header is dropped from the header and stays in the image,
                // which is the copy that matters.
                if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
                    headers.insert(name, value);
                }
            }
            response
        }
        Ok(Err(err)) => error_response(&err),
        Err(err) => task_failed("render", &err),
    }
}

/// `POST /api/reset` — collapse everything back to the entry screen.
///
/// One slice, not a collapse per type: forty round trips would put thirty-nine
/// intermediate views on the user's screen that nobody asked to see.
pub async fn reset(State(state): State<AppState>) -> Response {
    let session = Arc::clone(&state.session);
    match tokio::task::spawn_blocking(move || session.reset()).await {
        Ok(slice) => {
            let response = kglite_visual_core::Response::Slice(slice);
            state.bus.publish_if_view_mutating(&response);
            Json(response).into_response()
        }
        Err(err) => task_failed("reset", &err),
    }
}

/// `GET /api/view-state` — what is on the shared screen, as structured truth.
///
/// The server-side equivalent of the browser's `window.__kglv` (D14). Cheap:
/// it walks the slot space, never the graph.
pub async fn view_state(State(state): State<AppState>) -> Response {
    Json(state.session.view_state()).into_response()
}

/// `POST /api/focus` — `{"slots": [n, ...]}`. Zoom every attached client's
/// camera to those slots; an empty list frames the whole view (plan D14).
pub async fn focus(State(state): State<AppState>, Json(body): Json<FocusRequest>) -> Response {
    steer(&state, &body.slots, |slots| {
        Command::Focus(Focus::new(slots))
    })
}

/// `POST /api/highlight` — `{"slots": [n, ...], "concept":
/// "highlighted"|"selected"}`. Set one of the index-addressed interaction
/// concepts (D7) on every attached client.
pub async fn highlight(
    State(state): State<AppState>,
    Json(body): Json<HighlightRequest>,
) -> Response {
    let concept = body.concept;
    steer(&state, &body.slots, move |slots| {
        Command::Highlight(Highlight::new(slots, concept))
    })
}

/// `POST /api/appearance` — `{"color_by": "..."|null, "size_by": "..."|null}`.
///
/// **The property name is not validated here, and cannot be.** Which properties
/// a channel can be driven by is a per-type answer that
/// `POST /api/property-stats` computes; this endpoint moves a channel on every
/// client, and the clients are the only parties that know which type's
/// statistics they last fetched. A name nothing carries colours the view
/// uniformly, which is visible rather than silent.
pub async fn appearance(
    State(state): State<AppState>,
    Json(body): Json<AppearanceRequest>,
) -> Response {
    let clients = state
        .bus
        .publish_command(&Command::Appearance(Appearance::new(
            body.color_by,
            body.size_by,
        )));
    steered(clients)
}

/// Validate the slots, publish the command, and report the audience.
///
/// The audience is the answer, not a courtesy: a steering command that reached
/// nobody looked exactly like one that reached the user, and an agent that
/// cannot tell those apart will narrate a screen that never moved.
fn steer(state: &AppState, slots: &[u32], build: impl FnOnce(Vec<u32>) -> Command) -> Response {
    if let Err(err) = state.session.check_live_slots(slots) {
        return error_response(&err);
    }
    let clients = state.bus.publish_command(&build(slots.to_vec()));
    steered(clients)
}

fn steered(clients: usize) -> Response {
    Json(serde_json::json!({ "clients": clients })).into_response()
}

/// `POST /api/property-stats` — `{"node_type": "..."}`.
pub async fn property_stats(state: State<AppState>, Json(body): Json<TypeRequest>) -> Response {
    dispatch(state, Request::PropertyStats(body)).await
}

/// What `POST /api/validate` takes. One field, because one is all it needs.
#[derive(serde::Deserialize)]
pub struct ValidateRequest {
    #[serde(default)]
    pub query: String,
}

/// `POST /api/validate` — `{"query": "..."}`. What is wrong with this Cypher,
/// **without running it**.
///
/// **Parse-only, and that is a property of the call rather than a promise.**
/// `core::validate` reaches `kglite::api::cypher::parse_cypher`, which takes a
/// `&str` and returns an AST — it has no graph argument and therefore cannot
/// touch data. `EXPLAIN` was the fallback this endpoint was scoped against and
/// is not used: it plans, and planning is more work than answering "does this
/// parse". A core test validates a `CREATE` and asserts the node count is
/// unchanged, so the guarantee has an observable half rather than only a
/// signature to point at.
///
/// **HTTP only, never the binary protocol** (plan E3). The editor asks this
/// while a user types, the answer is a handful of strings, and putting it on
/// the wire that carries typed arrays would mean a message type and a protocol
/// bump for three fields. Same boundary the saved-query store draws.
///
/// **It does not move the view**, so unlike the request-shaped POSTs above it
/// publishes nothing to attached browsers.
pub async fn validate(
    State(state): State<AppState>,
    Json(body): Json<ValidateRequest>,
) -> Response {
    let session = Arc::clone(&state.session);
    // Parsing runs kglite's parser, which is what the runtime's enlarged stack
    // size exists for, and it is synchronous — off the reactor it goes, like
    // every other engine call in this file.
    match tokio::task::spawn_blocking(move || session.validate(&body.query)).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => task_failed("validate", &err),
    }
}

/// `GET /api/queries` — this graph's saved queries and recent history.
///
/// Includes the store's own path, because a store that silently went nowhere —
/// a machine with no config directory — must be distinguishable from one that
/// is simply empty.
pub async fn saved_queries(State(state): State<AppState>) -> Response {
    let store = Arc::clone(&state.queries);
    // A read, a write and a delete are all small file operations, but they are
    // still blocking file operations on the runtime that feeds the renderer.
    match tokio::task::spawn_blocking(move || store.list()).await {
        Ok(Ok(file)) => Json(serde_json::json!({
            "store": state.queries.path().map(|p| p.display().to_string()),
            "graph_path": file.graph_path,
            "graph_label": file.graph_label,
            "saved": file.saved,
            "history": file.history,
            "max_saved": queries::MAX_SAVED_PER_GRAPH,
            "max_history": queries::MAX_HISTORY,
        }))
        .into_response(),
        Ok(Err(err)) => store_error(&err),
        Err(err) => task_failed("saved-queries", &err),
    }
}

/// What the three saved-query mutations take. One body shape, because `name`
/// and `query` are the only two things any of them needs.
#[derive(serde::Deserialize)]
pub struct SavedQueryRequest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub query: String,
}

/// `POST /api/queries/save` — `{"name": "...", "query": "..."}`.
///
/// Bounds are refusals here, not truncations: a save that silently dropped the
/// oldest entry to make room would be a store that loses work without saying
/// so. Every ceiling comes back as a `400` naming the number it hit.
pub async fn save_query(
    State(state): State<AppState>,
    Json(body): Json<SavedQueryRequest>,
) -> Response {
    let store = Arc::clone(&state.queries);
    match tokio::task::spawn_blocking(move || store.save(&body.name, &body.query)).await {
        Ok(Ok(saved)) => Json(saved).into_response(),
        Ok(Err(err)) => store_error(&err),
        Err(err) => task_failed("save-query", &err),
    }
}

/// `POST /api/queries/delete` — `{"name": "..."}`.
///
/// POST rather than DELETE, matching the rest of this vocabulary: every
/// mutation here carries a JSON body and reads as a verb in a `curl` line.
pub async fn delete_query(
    State(state): State<AppState>,
    Json(body): Json<SavedQueryRequest>,
) -> Response {
    let store = Arc::clone(&state.queries);
    match tokio::task::spawn_blocking(move || store.delete(&body.name)).await {
        Ok(Ok(removed)) => Json(serde_json::json!({ "removed": removed })).into_response(),
        Ok(Err(err)) => store_error(&err),
        Err(err) => task_failed("delete-query", &err),
    }
}

/// `POST /api/queries/history` — `{"query": "..."}`.
///
/// **Called explicitly, by the two places a query is somebody's question**: the
/// panel's Run button and the `run_saved_query` MCP tool. Recording history
/// from the dispatch path instead would fill it with the queries the app runs
/// on its own behalf — the per-node values behind a colour-by choice, the id
/// list behind "load into view" — which is machine noise in a list a human
/// reads. See `queries::QueryStore::record`.
pub async fn record_query(
    State(state): State<AppState>,
    Json(body): Json<SavedQueryRequest>,
) -> Response {
    let store = Arc::clone(&state.queries);
    match tokio::task::spawn_blocking(move || store.record(&body.query)).await {
        Ok(Ok(())) => Json(serde_json::json!({ "recorded": true })).into_response(),
        Ok(Err(err)) => store_error(&err),
        Err(err) => task_failed("record-query", &err),
    }
}

/// A store refusal is the caller's request being wrong; an I/O failure is not.
fn store_error(err: &queries::StoreError) -> Response {
    let status = match err {
        queries::StoreError::Refused(_) => StatusCode::BAD_REQUEST,
        queries::StoreError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({ "error": err.to_string() })),
    )
        .into_response()
}

/// The one place a request becomes a response on the HTTP side.
///
/// Named handlers above rather than a single `/api/request` endpoint because a
/// `curl` line that says what it is asking for is the whole value of the twin;
/// they all funnel here so there is still exactly one dispatch.
async fn dispatch(State(state): State<AppState>, request: Request) -> Response {
    let session = Arc::clone(&state.session);
    match tokio::task::spawn_blocking(move || session.handle(&request)).await {
        Ok(Ok(response)) => {
            // The fix for the divergence this API shipped with: a POST that
            // moved the slot space now reaches every attached browser as the
            // same slice a WebSocket request would have produced. The caller
            // still gets its body — an HTTP client is not subscribed, so this
            // is an addition to the wire, not a change to it.
            state.bus.publish_if_view_mutating(&response);
            Json(response).into_response()
        }
        Ok(Err(err)) => error_response(&err),
        Err(err) => task_failed("request", &err),
    }
}

/// Map a core failure to a status a caller can branch on.
///
/// The message is kglite's own wherever kglite produced one. `KgError` carries
/// the position, the token it expected and the schema name it could not
/// resolve; replacing that with "query failed" throws away the only part a user
/// can act on.
fn error_response(err: &CoreError) -> Response {
    let status = match err {
        // The caller's request was wrong — a slot that names nothing, a type
        // this graph does not have. A 500 here would send them looking at the
        // server.
        CoreError::Request(_) => StatusCode::BAD_REQUEST,
        // The request was well formed and the engine refused it: a syntax
        // error, a timeout, a type mismatch.
        CoreError::Query(_) => StatusCode::UNPROCESSABLE_ENTITY,
        CoreError::Load(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({ "error": err.to_string() })),
    )
        .into_response()
}

fn task_failed(what: &str, err: &tokio::task::JoinError) -> Response {
    // A JoinError means the blocking task panicked or was cancelled — a bug
    // here, not in the request. It goes to stderr as well as to the caller,
    // because the caller only gets one line and the panic message is the part
    // worth keeping.
    eprintln!("kglite-visual: {what} task failed: {err}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": format!("{what} task failed") })),
    )
        .into_response()
}
