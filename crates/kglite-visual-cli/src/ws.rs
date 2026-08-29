//! The WebSocket endpoint — the binary protocol's first transport.
//!
//! Everything about the *messages* lives in core; this file only moves bytes.
//! That is the transport-agnostic rule in practice: if this file ever decides
//! what a frame contains, the Python wheel and a desktop shell will each have
//! to decide it again, differently.
//!
//! **A socket is two directions now.** It answers what this client asked for,
//! and it carries what *anything* did to the shared view — an agent's MCP call,
//! a `curl`, another tab. See [`crate::broadcast`] for who receives what and
//! why the initiating socket deliberately does not get a private copy of its
//! own slice.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use kglite_visual_core::session::{error_frames, response_frames};
use kglite_visual_core::Request;
use tokio::sync::broadcast::error::RecvError;

use crate::broadcast::{AppState, Update};

/// Outbound buffer ceiling, in bytes.
///
/// axum's default is unbounded: a client that stops reading (a background tab,
/// a paused debugger) makes the server buffer every frame it produces until
/// the process dies of memory exhaustion. Four chunks' worth is enough that a
/// healthy client never notices back-pressure and a stalled one is disconnected
/// instead of accumulated.
///
/// **This is the second of two ceilings, and it is the one that fires last.**
/// The bus (`broadcast::BUS_CAPACITY`) bounds how many *updates* a slow client
/// may fall behind by; this bounds how many *bytes* the socket underneath it
/// may hold. Broadcast made both load-bearing: before it, a client only ever
/// received what it had asked for, so it could not fall behind without being
/// idle.
const MAX_WRITE_BUFFER_BYTES: usize = 4 * kglite_visual_core::protocol::CHUNK_TARGET_BYTES;

pub async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws
        // The default `on_failed_upgrade` drops the error on the floor, so a
        // handshake that fails leaves the server silent and the client with a
        // closed socket and no reason. Every "the graph never loaded" report
        // starts here.
        .on_failed_upgrade(|err| eprintln!("kglite-visual: websocket upgrade failed: {err}"))
        .max_write_buffer_size(MAX_WRITE_BUFFER_BYTES)
        .on_upgrade(move |socket| serve(socket, state))
}

type Sink = SplitSink<WebSocket, Message>;

/// Send one response's frames. `false` means the client is gone.
async fn send_all(sink: &mut Sink, frames: &[Vec<u8>]) -> bool {
    for frame in frames {
        // `Message::Binary` takes `Bytes`, so this send is zero-copy from
        // here on; the one allocation is the frame the encoder built.
        if sink
            .send(Message::Binary(frame.clone().into()))
            .await
            .is_err()
        {
            return false; // client went away mid-response; nothing to report
        }
    }
    true
}

async fn serve(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // Subscribe BEFORE the opening messages go out. A slice published between
    // "the meta-graph was serialized" and "the subscription exists" would be
    // lost, and the client would draw a view one expansion out of date with no
    // way to know it — the exact silent divergence this whole module fixes.
    let mut updates = state.bus.subscribe();

    // Session info first, then the meta-graph: a client that cannot decode
    // this server's protocol version learns it from the smallest possible
    // message rather than after buffering an entire meta-graph.
    for frames in [
        state.session.session_info_frames(),
        state.session.meta_graph_frames(),
    ] {
        if !send_all(&mut sink, &frames).await {
            return;
        }
    }

    loop {
        tokio::select! {
            // Both arms are cancel-safe: `StreamExt::next` on a split stream
            // and `broadcast::Receiver::recv` both leave nothing half-consumed
            // when the other branch wins.
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else { return };
                if !handle_incoming(&state, &mut sink, message).await {
                    return;
                }
            }
            update = updates.recv() => {
                if !handle_update(&mut sink, update).await {
                    return;
                }
            }
        }
    }
}

/// One inbound message. `false` closes the socket.
async fn handle_incoming(state: &AppState, sink: &mut Sink, message: Message) -> bool {
    match message {
        // The request vocabulary. A request this server cannot parse or
        // cannot answer is replied to with an error frame, never dropped:
        // a client waiting forever for a response that was silently
        // discarded is the harder bug of the two.
        Message::Text(text) => {
            let reply = answer(state, text.as_str()).await;
            send_all(sink, &reply).await
        }
        Message::Ping(payload) => sink.send(Message::Pong(payload)).await.is_ok(),
        Message::Close(_) => false,
        Message::Binary(_) | Message::Pong(_) => true,
    }
}

/// One broadcast update. `false` closes the socket.
async fn handle_update(sink: &mut Sink, update: Result<Update, RecvError>) -> bool {
    match update {
        Ok(frames) => send_all(sink, &frames).await,
        // The slow-client policy, stated in `broadcast.rs`: a client that
        // missed `missed` view updates cannot be caught up by the ones still
        // in the channel — it lost a `first_slot`, or a compaction remap, and
        // every index it holds afterwards may name a different node. It is
        // told, in the same in-band error channel every other failure uses,
        // and then closed. Reconnecting yields a correct view; continuing does
        // not.
        Err(RecvError::Lagged(missed)) => {
            let frames = error_frames(format!(
                "this view moved {missed} update(s) ahead of this client and the \
                 skipped changes cannot be replayed; reload the page to resynchronise"
            ));
            send_all(sink, &frames).await;
            false
        }
        // Every sender dropped: the server is shutting down.
        Err(RecvError::Closed) => false,
    }
}

/// Parse, dispatch and frame one request.
///
/// Every arm may run Cypher or walk the graph, so the work goes to
/// `spawn_blocking` — the runtime's blocking threads carry kglite's
/// `QUERY_THREAD_STACK_SIZE` (see `main.rs`), and running it here on the
/// reactor would stall every other socket for the length of the query.
///
/// Returns the frames for **this** socket only. A view-mutating answer returns
/// none, because it has already gone to every subscriber — this one included.
async fn answer(state: &AppState, text: &str) -> Vec<Vec<u8>> {
    let request: Request = match serde_json::from_str(text) {
        Ok(request) => request,
        // The parse error names the offending field and offset, and it is the
        // only thing that can tell a client its message shape is wrong.
        Err(err) => return error_frames(format!("could not read that request: {err}")),
    };

    let session = std::sync::Arc::clone(&state.session);
    match tokio::task::spawn_blocking(move || session.handle(&request)).await {
        Ok(Ok(response)) => {
            if state.bus.publish_if_view_mutating(&response) {
                Vec::new()
            } else {
                response_frames(&response)
            }
        }
        Ok(Err(err)) => error_frames(err.to_string()),
        Err(err) => {
            eprintln!("kglite-visual: request task failed: {err}");
            error_frames("the server failed while answering that request")
        }
    }
}
