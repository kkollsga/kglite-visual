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
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kglite_visual_core::error::CoreError;
use kglite_visual_core::request::{
    CypherRequest, ExpandRequest, Request, SearchRequest, SlotRequest, TypeRequest,
};
use kglite_visual_core::Session;

/// `GET /api/meta-graph` — the entry screen, positions and links included.
pub async fn meta_graph(State(session): State<Arc<Session>>) -> Response {
    // Serialization only; the meta-graph itself was computed once at open.
    Json(session.meta_graph()).into_response()
}

/// `GET /api/session` — protocol version, tier, slot space, bounds, stats.
pub async fn session_info(State(session): State<Arc<Session>>) -> Response {
    Json(session.info()).into_response()
}

/// `GET /api/describe` — kglite's own schema document plus this session's tier
/// (plan D12): the same progressive-disclosure schema an agent gets from
/// kglite's MCP server, served by the running viz session.
pub async fn describe(State(session): State<Arc<Session>>) -> Response {
    // `compute_schema` walks per-type metadata rather than nodes, but it is
    // still the heaviest synchronous call in this file and it scales with the
    // schema, not with the request. Off the async runtime it goes: one slow
    // /api/describe must not stall the WebSocket feeding the renderer.
    match tokio::task::spawn_blocking(move || session.describe()).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => task_failed("describe", &err),
    }
}

/// `POST /api/cypher` — `{"query": "...", "params": {...}, "limit": n,
/// "as_graph": bool}`.
pub async fn cypher(state: State<Arc<Session>>, Json(body): Json<CypherRequest>) -> Response {
    dispatch(state, Request::Cypher(body)).await
}

/// `POST /api/search` — `{"query": "...", "node_type": "...",
/// "property": "...", "mode": "contains"|"starts-with", "limit": n}`.
pub async fn search(state: State<Arc<Session>>, Json(body): Json<SearchRequest>) -> Response {
    dispatch(state, Request::Search(body)).await
}

/// `POST /api/preview` — `{"slot": n}`. Per-relationship counts, no fetch.
pub async fn preview(state: State<Arc<Session>>, Json(body): Json<SlotRequest>) -> Response {
    dispatch(state, Request::Preview(body)).await
}

/// `POST /api/expand` — `{"slot": n, "relationship": "...",
/// "direction": "out"|"in"|"both", "limit": n}`.
pub async fn expand(state: State<Arc<Session>>, Json(body): Json<ExpandRequest>) -> Response {
    dispatch(state, Request::Expand(body)).await
}

/// `POST /api/collapse` — `{"slot": n}`.
pub async fn collapse(state: State<Arc<Session>>, Json(body): Json<SlotRequest>) -> Response {
    dispatch(state, Request::Collapse(body)).await
}

/// `POST /api/node` — `{"slot": n}`. One node's stored properties.
pub async fn node_detail(state: State<Arc<Session>>, Json(body): Json<SlotRequest>) -> Response {
    dispatch(state, Request::NodeDetail(body)).await
}

/// `POST /api/property-stats` — `{"node_type": "..."}`.
pub async fn property_stats(state: State<Arc<Session>>, Json(body): Json<TypeRequest>) -> Response {
    dispatch(state, Request::PropertyStats(body)).await
}

/// The one place a request becomes a response on the HTTP side.
///
/// Named handlers above rather than a single `/api/request` endpoint because a
/// `curl` line that says what it is asking for is the whole value of the twin;
/// they all funnel here so there is still exactly one dispatch.
async fn dispatch(State(session): State<Arc<Session>>, request: Request) -> Response {
    match tokio::task::spawn_blocking(move || session.handle(&request)).await {
        Ok(Ok(response)) => Json(response).into_response(),
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
