//! The JSON twin of the binary protocol (test-plan §2).
//!
//! Every answer the WebSocket can give has a plain-HTTP JSON rendering, from
//! the *same* response structs — one encoder, two serializers, so
//! twin-vs-binary divergence is a compile-time non-problem rather than a drift
//! bug class. The binary path is the performance path; the twin is how an
//! agent, or a human with `curl`, verifies server behaviour with no GPU
//! browser in the loop.

use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use kglite_visual_core::Session;

/// `GET /api/meta-graph` — the entry screen, positions and links included.
pub async fn meta_graph(State(session): State<Arc<Session>>) -> Response {
    // Serialization only; the meta-graph itself was computed once at open.
    Json(session.meta_graph()).into_response()
}

/// `GET /api/session` — protocol version, tier, slot space, whole-graph stats.
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
    let response = match tokio::task::spawn_blocking(move || session.describe()).await {
        Ok(response) => response,
        Err(err) => {
            eprintln!("kglite-visual: describe task failed: {err}");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "describe failed\n",
            )
                .into_response();
        }
    };
    Json(response).into_response()
}
