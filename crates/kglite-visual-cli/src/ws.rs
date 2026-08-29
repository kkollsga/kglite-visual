//! The WebSocket endpoint — the binary protocol's first transport.
//!
//! Everything about the *messages* lives in core; this file only moves bytes.
//! That is the transport-agnostic rule in practice: if this file ever decides
//! what a frame contains, the Python wheel and a desktop shell will each have
//! to decide it again, differently.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use kglite_visual_core::session::{error_frames, response_frames};
use kglite_visual_core::{Request, Session};

/// Outbound buffer ceiling, in bytes.
///
/// axum's default is unbounded: a client that stops reading (a background tab,
/// a paused debugger) makes the server buffer every frame it produces until
/// the process dies of memory exhaustion. Four chunks' worth is enough that a
/// healthy client never notices back-pressure and a stalled one is disconnected
/// instead of accumulated.
const MAX_WRITE_BUFFER_BYTES: usize = 4 * kglite_visual_core::protocol::CHUNK_TARGET_BYTES;

pub async fn upgrade(ws: WebSocketUpgrade, State(session): State<Arc<Session>>) -> Response {
    ws
        // The default `on_failed_upgrade` drops the error on the floor, so a
        // handshake that fails leaves the server silent and the client with a
        // closed socket and no reason. Every "the graph never loaded" report
        // starts here.
        .on_failed_upgrade(|err| eprintln!("kglite-visual: websocket upgrade failed: {err}"))
        .max_write_buffer_size(MAX_WRITE_BUFFER_BYTES)
        .on_upgrade(move |socket| serve(socket, session))
}

async fn serve(socket: WebSocket, session: Arc<Session>) {
    let (mut sink, mut stream) = socket.split();

    // Session info first, then the meta-graph: a client that cannot decode
    // this server's protocol version learns it from the smallest possible
    // message rather than after buffering an entire meta-graph.
    for frames in [session.session_info_frames(), session.meta_graph_frames()] {
        for frame in frames {
            // `Message::Binary` takes `Bytes`, so this send is zero-copy from
            // here on; the one allocation is the frame the encoder built.
            if sink.send(Message::Binary(frame.into())).await.is_err() {
                return; // client went away mid-response; nothing to report
            }
        }
    }

    while let Some(incoming) = stream.next().await {
        let Ok(message) = incoming else { return };
        match message {
            // The request vocabulary. A request this server cannot parse or
            // cannot answer is replied to with an error frame, never dropped:
            // a client waiting forever for a response that was silently
            // discarded is the harder bug of the two.
            Message::Text(text) => {
                let reply = answer(&session, text.as_str()).await;
                for frame in reply {
                    if sink.send(Message::Binary(frame.into())).await.is_err() {
                        return;
                    }
                }
            }
            Message::Ping(payload) => {
                if sink.send(Message::Pong(payload)).await.is_err() {
                    return;
                }
            }
            Message::Close(_) => return,
            Message::Binary(_) | Message::Pong(_) => {}
        }
    }
}

/// Parse, dispatch and frame one request.
///
/// Every arm may run Cypher or walk the graph, so the work goes to
/// `spawn_blocking` — the runtime's blocking threads carry kglite's
/// `QUERY_THREAD_STACK_SIZE` (see `main.rs`), and running it here on the
/// reactor would stall every other socket for the length of the query.
async fn answer(session: &Arc<Session>, text: &str) -> Vec<Vec<u8>> {
    let request: Request = match serde_json::from_str(text) {
        Ok(request) => request,
        // The parse error names the offending field and offset, and it is the
        // only thing that can tell a client its message shape is wrong.
        Err(err) => return error_frames(format!("could not read that request: {err}")),
    };

    let session = Arc::clone(session);
    match tokio::task::spawn_blocking(move || session.handle(&request)).await {
        Ok(Ok(response)) => response_frames(&response),
        Ok(Err(err)) => error_frames(err.to_string()),
        Err(err) => {
            eprintln!("kglite-visual: request task failed: {err}");
            error_frames("the server failed while answering that request")
        }
    }
}
