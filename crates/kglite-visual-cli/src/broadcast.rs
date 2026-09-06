//! Ordered shared commits and coherent attachment for every transport.
//! The blocking closure owns commit and publication, even if its caller leaves.

use std::sync::{Arc, Mutex};

use kglite_visual_core::shared::{
    shared_frames, CommittedEvent, RevisionStamp, SharedRequest, SharedWireMeta,
};
use kglite_visual_core::CoreError;
use kglite_visual_core::Response;
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
/// `tokio::sync::broadcast` reports lag once a receiver falls `BUS_CAPACITY`
/// behind. Each event is a full snapshot, but transient focus commands are not
/// replayable. Report the gap and close; reconnect captures current durable
/// state together with a fresh subscription.
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
    fn publish(&self, frames: Vec<Vec<u8>>) {
        let _ = self.tx.send(Arc::new(frames));
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
    publication_gate: Arc<Mutex<()>>,
    #[cfg(test)]
    commit_hook: Arc<Mutex<Option<CommitHook>>>,
}

impl AppState {
    /// `graph_label` is the launch contract's `graph` field. See
    /// [`QueryStore::open`] for what it does with a label that is not a path.
    pub fn new(session: Arc<Session>, graph_label: &str) -> Self {
        Self {
            session,
            bus: Bus::new(),
            queries: Arc::new(QueryStore::open(graph_label)),
            publication_gate: Arc::new(Mutex::new(())),
            #[cfg(test)]
            commit_hook: Arc::new(Mutex::new(None)),
        }
    }
}

#[cfg(test)]
type CommitHook = Arc<dyn Fn(&RevisionStamp) + Send + Sync>;

#[derive(Debug)]
pub enum DispatchError {
    Core(CoreError),
    Task(String),
}

impl From<CoreError> for DispatchError {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

#[derive(Debug)]
pub struct Execution {
    pub response: Response,
    pub stamp: Option<RevisionStamp>,
    pub request_id: Option<String>,
    pub published: bool,
}

impl AppState {
    pub async fn execute(&self, request: SharedRequest) -> Result<Execution, DispatchError> {
        if request.request_id.as_ref().is_some_and(|id| id.len() > 128) {
            return Err(CoreError::Request("request_id exceeds 128 bytes".into()).into());
        }
        if !request.request.is_shared() {
            let session = Arc::clone(&self.session);
            return tokio::task::spawn_blocking(move || {
                session
                    .handle(&request.request)
                    .map(|response| Execution {
                        response,
                        stamp: None,
                        request_id: request.request_id,
                        published: false,
                    })
                    .map_err(DispatchError::Core)
            })
            .await
            .map_err(|error| DispatchError::Task(error.to_string()))?;
        }
        let session = Arc::clone(&self.session);
        let prepared = tokio::task::spawn_blocking(move || session.prepare_shared(&request))
            .await
            .map_err(|error| DispatchError::Task(error.to_string()))??;
        let state = self.clone();
        // The closure owns the guard and publication. Cancelling its caller
        // cannot detach a mutation from the broadcast that acknowledges it.
        tokio::task::spawn_blocking(move || {
            let _ordered = state
                .publication_gate
                .lock()
                .map_err(|_| DispatchError::Task("shared publication lock is poisoned".into()))?;
            let event = state.session.commit_shared(prepared)?;
            #[cfg(test)]
            state.after_commit(&event.snapshot.meta.stamp);
            state.bus.publish(frame_event(&event));
            Ok(Execution {
                stamp: Some(event.snapshot.meta.stamp.clone()),
                request_id: event.request_id,
                response: event.response,
                published: true,
            })
        })
        .await
        .map_err(|error| DispatchError::Task(error.to_string()))?
    }

    #[cfg(test)]
    fn after_commit(&self, stamp: &RevisionStamp) {
        let hook = self.commit_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(stamp);
        }
    }

    pub async fn attach(
        &self,
    ) -> Result<(tokio::sync::broadcast::Receiver<Update>, Vec<Vec<u8>>), DispatchError> {
        let state = self.clone();
        tokio::task::spawn_blocking(move || {
            let _ordered = state
                .publication_gate
                .lock()
                .map_err(|_| DispatchError::Task("shared publication lock is poisoned".into()))?;
            let receiver = state.bus.subscribe();
            let snapshot = state.session.snapshot_shared();
            let mut frames = state.session.session_info_frames();
            frames.extend(state.session.meta_graph_frames());
            let meta = SharedWireMeta {
                snapshot: snapshot.meta,
                request_id: None,
                focus: None,
                mutation_kind: None,
            };
            frames.extend(shared_frames(&meta, &snapshot.points, &snapshot.links));
            Ok((receiver, frames))
        })
        .await
        .map_err(|error| DispatchError::Task(error.to_string()))?
    }
}

fn frame_event(event: &CommittedEvent) -> Vec<Vec<u8>> {
    let meta = event.wire_meta();
    shared_frames(&meta, &event.snapshot.points, &event.snapshot.links)
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

#[cfg(test)]
mod ordering_tests {
    use super::*;
    use kglite_visual_core::shared::CaptionRequest;
    use kglite_visual_core::Request;
    use std::sync::mpsc;
    use std::time::Duration;

    fn state() -> AppState {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kglite-visual-core/tests/fixtures/meta.kgl");
        let graph = kglite_visual_core::load_graph(kglite_visual_core::GraphSource::Path(&fixture))
            .unwrap();
        AppState::new(
            Arc::new(Session::open(graph, "ordering-test")),
            "ordering-test",
        )
    }

    fn caption(name: &str) -> SharedRequest {
        SharedRequest::new(Request::Caption(CaptionRequest {
            caption_by: Some(name.into()),
        }))
    }

    fn revision(frames: &[Vec<u8>]) -> String {
        frames
            .iter()
            .find_map(|frame| {
                let decoded = kglite_visual_core::decode_frame(frame).unwrap();
                if decoded.msg_type != kglite_visual_core::MessageType::SharedUpdate {
                    return None;
                }
                let payload: serde_json::Value = serde_json::from_slice(&decoded.payload).unwrap();
                Some(
                    payload["snapshot"]["stamp"]["revision"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                )
            })
            .expect("a shared event")
    }

    struct Release(Option<mpsc::Sender<()>>);
    impl Drop for Release {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    fn pause_first(state: &AppState) -> (mpsc::Receiver<()>, Release) {
        let (entered_tx, entered) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        let release = Mutex::new(release);
        *state.commit_hook.lock().unwrap() = Some(Arc::new(move |stamp| {
            if stamp.revision == "1" {
                entered_tx.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
            }
        }));
        (entered, Release(Some(release_tx)))
    }

    async fn entered(receiver: mpsc::Receiver<()>) {
        tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(5)).unwrap())
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publication_order_cannot_overtake_an_earlier_commit() {
        let state = state();
        let mut a_viewer = state.bus.subscribe();
        let mut b_viewer = state.bus.subscribe();
        let (paused, release) = pause_first(&state);
        let a_state = state.clone();
        let a = tokio::spawn(async move { a_state.execute(caption("id")).await });
        entered(paused).await;
        let b_state = state.clone();
        let mut b = tokio::spawn(async move { b_state.execute(caption("title")).await });
        let early = tokio::time::timeout(Duration::from_millis(250), &mut b).await;
        let overtook = early.is_ok();
        drop(release);
        a.await.unwrap().unwrap();
        if let Ok(result) = early {
            result.unwrap().unwrap();
        } else {
            b.await.unwrap().unwrap();
        }
        let a_revisions = [
            revision(&a_viewer.recv().await.unwrap()),
            revision(&a_viewer.recv().await.unwrap()),
        ];
        let b_revisions = [
            revision(&b_viewer.recv().await.unwrap()),
            revision(&b_viewer.recv().await.unwrap()),
        ];
        assert!(
            !overtook,
            "later mutation completed while the earlier publication was paused"
        );
        assert_eq!(a_revisions, ["1", "2"]);
        assert_eq!(b_revisions, ["1", "2"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_a_caller_cannot_detach_a_committed_update_from_publication() {
        let state = state();
        let mut viewer = state.bus.subscribe();
        let (paused, release) = pause_first(&state);
        let caller_state = state.clone();
        let caller = tokio::spawn(async move { caller_state.execute(caption("id")).await });
        entered(paused).await;
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        drop(release);
        let event = tokio::time::timeout(Duration::from_secs(5), viewer.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(revision(&event), "1");
        assert_eq!(state.session.shared_stamp().revision, "1");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attachment_captures_a_snapshot_without_replaying_its_delta() {
        let state = state();
        let (paused, release) = pause_first(&state);
        let a_state = state.clone();
        let a = tokio::spawn(async move { a_state.execute(caption("id")).await });
        entered(paused).await;
        let attach_state = state.clone();
        let mut attach = tokio::spawn(async move { attach_state.attach().await });
        let early = tokio::time::timeout(Duration::from_millis(250), &mut attach).await;
        let captured_early = early.is_ok();
        drop(release);
        a.await.unwrap().unwrap();
        let (mut receiver, frames) = if let Ok(result) = early {
            result.unwrap().unwrap()
        } else {
            attach.await.unwrap().unwrap()
        };
        assert!(
            !captured_early,
            "attachment escaped before the pending publication"
        );
        assert_eq!(revision(&frames), "1");
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        state.execute(caption("title")).await.unwrap();
        assert_eq!(revision(&receiver.recv().await.unwrap()), "2");
    }

    #[tokio::test]
    async fn conflicts_and_private_reads_do_not_publish_or_advance_revision() {
        let state = state();
        let mut viewer = state.bus.subscribe();
        let expected = state.session.shared_stamp();
        state.execute(caption("id")).await.unwrap();
        viewer.recv().await.unwrap();
        let mut stale = caption("title");
        stale.expected = Some(expected);
        assert!(matches!(
            state.execute(stale).await,
            Err(DispatchError::Core(CoreError::Conflict(_)))
        ));
        let read: SharedRequest = serde_json::from_value(serde_json::json!({
            "type":"records", "handles":[], "fields":["id"], "request_id":"private-1"
        }))
        .unwrap();
        let result = state.execute(read).await.unwrap();
        assert!(!result.published);
        assert_eq!(result.request_id.as_deref(), Some("private-1"));
        assert_eq!(state.session.shared_stamp().revision, "1");
        assert!(matches!(
            viewer.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }
}
