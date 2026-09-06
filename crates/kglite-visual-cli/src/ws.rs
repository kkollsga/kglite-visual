//! Binary requests and ordered shared events over WebSocket.
//! Attachment captures the greeting and receiver together under the dispatch gate.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use kglite_visual_core::session::error_frames;
use kglite_visual_core::shared::SharedRequest;
use tokio::sync::broadcast::error::RecvError;

use crate::broadcast::{AppState, DispatchError, Update};

/// Outbound buffer ceiling, in bytes.
///
/// axum's default is unbounded: a client that stops reading (a background tab,
/// a paused debugger) makes the server buffer every frame it produces until
/// the process dies of memory exhaustion. Two bounded events' worth is enough that a
/// healthy client never notices back-pressure and a stalled one is disconnected
/// instead of accumulated.
///
/// **This is the second of two ceilings, and it is the one that fires last.**
/// The bus (`broadcast::BUS_CAPACITY`) bounds how many *updates* a slow client
/// may fall behind by; this bounds how many *bytes* the socket underneath it
/// may hold. Broadcast made both load-bearing: before it, a client only ever
/// received what it had asked for, so it could not fall behind without being
/// idle.
const MAX_WRITE_BUFFER_BYTES: usize = 2 * kglite_visual_core::shared::MAX_SHARED_EVENT_BYTES;

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

    let (mut updates, frames) = match state.attach().await {
        Ok(attached) => attached,
        Err(error) => {
            send_all(&mut sink, &dispatch_error_frames(error, None)).await;
            return;
        }
    };
    if !send_all(&mut sink, &frames).await {
        return;
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
        // Durable state can be resnapshotted, but a missed focus command
        // cannot be replayed. Make the gap explicit before disconnecting.
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
    let request: SharedRequest = match serde_json::from_str(text) {
        Ok(request) => request,
        Err(err) => return error_frames(format!("could not read that request: {err}")),
    };
    let request_id = request.request_id.clone().filter(|id| id.len() <= 128);
    match state.execute(request).await {
        Ok(execution) if execution.published => Vec::new(),
        Ok(execution) => private_frames(&execution.response, execution.request_id.as_deref()),
        Err(error) => dispatch_error_frames(error, request_id),
    }
}

fn private_frames(
    response: &kglite_visual_core::Response,
    request_id: Option<&str>,
) -> Vec<Vec<u8>> {
    use kglite_visual_core::{MessageType, Response, ResponseEncoder};
    let message_type = match response {
        Response::Query(_) => MessageType::QueryTable,
        Response::Records(_) => MessageType::Records,
        Response::FieldDetail(_) => MessageType::FieldDetail,
        Response::Preview(_) => MessageType::ExpansionPreview,
        Response::NodeDetail(_) => MessageType::NodeDetail,
        Response::Search(_) => MessageType::SearchResult,
        Response::PropertyStats(_) => MessageType::PropertyStats,
        Response::Slice(_) | Response::Layout(_) | Response::Shared(_) => {
            return error_frames("a shared response escaped the ordered publication path");
        }
    };
    #[derive(serde::Serialize)]
    struct Correlated<'a> {
        #[serde(flatten)]
        response: &'a Response,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<&'a str>,
    }
    let payload = serde_json::to_string(&Correlated {
        response,
        request_id,
    })
    .expect("private replies contain serializable records");
    let mut encoder = ResponseEncoder::new();
    encoder.push_json(message_type, &payload);
    encoder.finish()
}

fn dispatch_error_frames(error: DispatchError, request_id: Option<String>) -> Vec<Vec<u8>> {
    use kglite_visual_core::protocol::{MessageType, ResponseEncoder};
    let mut payload = match error {
        DispatchError::Core(kglite_visual_core::CoreError::Conflict(conflict)) => {
            serde_json::json!(conflict)
        }
        DispatchError::Core(error) => serde_json::json!({"message": error.to_string()}),
        DispatchError::Task(message) => serde_json::json!({"message": message}),
    };
    if let Some(request_id) = request_id {
        payload["request_id"] = request_id.into();
    }
    let mut encoder = ResponseEncoder::new();
    encoder.push_json(MessageType::Error, &payload.to_string());
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_records_echo_correlation_without_changing_record_identity() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kglite-visual-core/tests/fixtures/meta.kgl");
        let graph = kglite_visual_core::load_graph(kglite_visual_core::GraphSource::Path(&fixture))
            .unwrap();
        let session = kglite_visual_core::Session::open(graph, "correlation-test");
        let request = serde_json::from_value(
            serde_json::json!({"type":"records","handles":[],"fields":["id"]}),
        )
        .unwrap();
        let response = session.handle(&request).unwrap();
        let frames = private_frames(&response, Some("records-7"));
        assert_eq!(frames.len(), 1);
        let decoded = kglite_visual_core::decode_frame(&frames[0]).unwrap();
        assert_eq!(decoded.msg_type, kglite_visual_core::MessageType::Records);
        assert!(decoded.terminal);
        let mut body: serde_json::Value = serde_json::from_slice(&decoded.payload).unwrap();
        assert_eq!(
            body.as_object_mut().unwrap().remove("request_id").unwrap(),
            "records-7"
        );
        assert_eq!(body, serde_json::json!(response));
    }
}
