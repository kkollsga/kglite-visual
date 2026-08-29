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
/// **It does not touch this session's view.** `core::render` opens a private
/// session over the same read-only graph, so a `POST /api/render` cannot move
/// the slot space of whatever browser tab is attached. (Rendering the *live*
/// view is a different request, and P10 is where it lands.)
pub async fn render(State(state): State<AppState>, Json(body): Json<RenderRequest>) -> Response {
    let graph = Arc::clone(state.session.graph());
    let name = state.session.info().graph;
    let config = state.session.config();
    // A render lays out a bounded slice and, for PNG, rasterises it: tens of
    // milliseconds, and on the async runtime that is the WebSocket feeding the
    // renderer stalling for all of them.
    match tokio::task::spawn_blocking(move || {
        kglite_visual_core::render(&graph, &name, config, &body)
    })
    .await
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
