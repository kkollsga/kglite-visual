//! One view, every client: the fan-out that makes the browser follow an agent.
//!
//! **The gap this closes was already live.** The JSON twin's `POST /api/*`
//! endpoints have always mutated the server-side slot space, and the WebSocket
//! clients watching that space were never told: a `curl` expand moved the view
//! the browser was drawing from, and the browser kept drawing the old one until
//! something else happened to fetch. Two clients disagreeing about one slot
//! space is not a cosmetic defect — a slot the server has re-used or tombstoned
//! is an index into the wrong node on every screen that missed the message.
//!
//! So every view-mutating response now goes to **all** connected clients,
//! whoever asked for it. That is not damage control; it is the P10 feature: the
//! user watches the agent navigate.
//!
//! **Who gets what:**
//!
//! | initiator | view-mutating response | everything else |
//! |-----------|------------------------|-----------------|
//! | HTTP twin | broadcast, *and* the JSON body as before | JSON body |
//! | WebSocket | broadcast **only** | frames to that socket |
//!
//! The WebSocket initiator deliberately does not get a private copy: it is
//! subscribed, so a second copy would arrive as a second slice and be applied
//! twice — harmless for an append, wrong for a compaction, which renumbers
//! every slot exactly once. The HTTP caller is not subscribed, so its body is
//! not a duplicate of anything and the wire contract is unchanged.

use std::sync::Arc;

use kglite_visual_core::control::{control_frames, Command};
use kglite_visual_core::session::{response_frames, Response};
use kglite_visual_core::Session;

use crate::queries::QueryStore;
use tokio::sync::broadcast;

/// One already-framed response, shared by every socket that receives it.
///
/// `Arc` rather than a clone per subscriber: the frames are the same bytes for
/// everyone, and a 5 000-node slice is hundreds of kilobytes.
pub type Update = Arc<Vec<Vec<u8>>>;

/// Updates a client may fall behind by before it is dropped.
///
/// The channel is **bounded** on purpose. An unbounded fan-out lets one paused
/// client (a background tab, a debugger on a breakpoint) hold every message the
/// server has produced since it stopped reading, which is the same memory
/// exhaustion `max_write_buffer_size` exists to prevent, moved one layer up.
///
/// Sixty-four is far past anything a healthy client lags by — a slice is
/// applied in a single frame — and small enough that the worst case is bounded
/// by the response bound rather than by uptime.
pub const BUS_CAPACITY: usize = 64;

/// The fan-out channel.
///
/// **Slow-client policy: report, then disconnect — never drop silently.**
/// `tokio::sync::broadcast` evicts the oldest message when a receiver falls
/// `BUS_CAPACITY` behind and reports it as `Lagged(n)`. Continuing from there
/// would leave that client's slot space permanently wrong: it missed an
/// expansion's `first_slot`, or a compaction's remap, and every index it holds
/// afterwards names a different node. Silence is the one outcome this project
/// refuses (D4 — "a compaction the client did not hear about would silently
/// re-label every selection it holds"), so [`crate::ws`] sends the client an
/// error frame naming the number of updates it missed and then closes the
/// socket. A reconnect is a correct view; a survivor of a gap is not.
#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<Update>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Self {
        // `broadcast::channel` keeps the sender alive with no receivers, and
        // `send` on an empty bus is a cheap Err we ignore: a server with no
        // browser attached still mutates its view for `curl` and for MCP.
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Update> {
        self.tx.subscribe()
    }

    /// Clients currently attached. Used by tests and by nothing in the request
    /// path — a broadcast that reaches nobody is still a correct broadcast.
    pub fn client_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Push already-framed bytes to every subscriber.
    pub fn publish(&self, frames: Vec<Vec<u8>>) {
        let _ = self.tx.send(Arc::new(frames));
    }

    /// Push a steering command, and report how many clients heard it.
    ///
    /// The count is the whole answer a caller gets. A command carries no slot
    /// space and produces no slice, so "did anything happen" has exactly one
    /// observable: whether anyone was listening.
    pub fn publish_command(&self, command: &Command) -> usize {
        let clients = self.client_count();
        self.publish(control_frames(command));
        clients
    }

    /// Push a response if it moved the view, and say whether it did.
    ///
    /// **The list of what moves the view lives here, once.** A response type
    /// added to the session that mutates the slot space and is not matched here
    /// re-opens the exact divergence this module closes, so the match is
    /// exhaustive rather than a `if let`: a new variant fails to compile until
    /// somebody decides which side of the line it is on.
    pub fn publish_if_view_mutating(&self, response: &Response) -> bool {
        let mutating = match response {
            // A slice IS the view change: appended slots, tombstones, the whole
            // link list, and the compaction remap when one fired. Expansion,
            // collapse, "show in graph" and a search's load-into-view all
            // arrive as one.
            Response::Slice(_) => true,
            // A layout changes no slot — but it changes what every attached
            // screen looks like, and the mode it is in. Two clients disagreeing
            // about whether the simulation is running is the same divergence
            // this module exists to close, one level up from the slot space:
            // one of them would be dragging points the other believes are
            // pinned. So it broadcasts, exactly like a steering command (D14).
            Response::Layout(_) => true,
            // Answers *about* the graph, not changes to what is drawn. A query
            // table, an expansion preview, one node's properties, a search hit
            // list and a type's property statistics leave the slot space
            // exactly as they found it, and pushing them to every client would
            // put one user's search results in another user's panel.
            Response::Query(_)
            | Response::Preview(_)
            | Response::NodeDetail(_)
            | Response::Search(_)
            | Response::PropertyStats(_) => false,
        };
        if mutating {
            self.publish(response_frames(response));
        }
        mutating
    }
}

/// Everything a request handler needs: the open graph and the fan-out.
///
/// One state type for both faces. The twin and the WebSocket used to share
/// `Arc<Session>`; they now share the bus as well, because "the twin mutates
/// and nobody hears" was exactly the shape of one handler holding half the
/// state.
#[derive(Clone)]
pub struct AppState {
    pub session: Arc<Session>,
    pub bus: Bus,
    /// This graph's saved queries, keyed by the graph the session was launched
    /// with. Shared by every face — the twin, the WebSocket and MCP — for the
    /// same reason the bus is: two of them holding two stores is two answers to
    /// "what have I saved".
    pub queries: Arc<QueryStore>,
}

impl AppState {
    /// `graph_label` is the launch contract's `graph` field. See
    /// [`QueryStore::open`] for what it does with a label that is not a path.
    pub fn new(session: Arc<Session>, graph_label: &str) -> Self {
        Self {
            session,
            bus: Bus::new(),
            queries: Arc::new(QueryStore::open(graph_label)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(byte: u8) -> Vec<Vec<u8>> {
        vec![vec![byte; 4]]
    }

    #[tokio::test]
    async fn every_subscriber_receives_one_publish() {
        // The whole feature in four lines: one mutation, N clients, N copies.
        let bus = Bus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        assert_eq!(bus.client_count(), 2);

        bus.publish(frame(7));

        assert_eq!(a.recv().await.unwrap().as_slice(), &[vec![7u8; 4]]);
        assert_eq!(b.recv().await.unwrap().as_slice(), &[vec![7u8; 4]]);
    }

    #[tokio::test]
    async fn a_publish_with_no_clients_is_not_an_error() {
        // `curl` against a server nobody has opened a browser on is the normal
        // agent case, and `broadcast::send` returns Err when the channel is
        // empty. Treating that as a failure would fail every headless request.
        let bus = Bus::new();
        bus.publish(frame(1));
        assert_eq!(bus.client_count(), 0);
    }

    #[tokio::test]
    async fn a_client_that_falls_behind_is_told_how_far() {
        // The slow-client policy's evidence: the channel does not grow, and the
        // receiver learns it lost messages rather than silently resuming in the
        // middle of a slot space it no longer understands.
        let bus = Bus::new();
        let mut slow = bus.subscribe();
        for i in 0..(BUS_CAPACITY + 3) {
            bus.publish(frame(i as u8));
        }

        let err = slow
            .recv()
            .await
            .expect_err("the receiver must report loss");
        let broadcast::error::RecvError::Lagged(missed) = err else {
            panic!("expected Lagged, got {err:?}");
        };
        assert_eq!(missed, 3, "exactly the overflow, named");

        // And it resumes at the oldest message still held, so a client that
        // chose to continue would be reading a real frame — which is precisely
        // why the socket is closed instead.
        assert!(slow.recv().await.is_ok());
    }
}
