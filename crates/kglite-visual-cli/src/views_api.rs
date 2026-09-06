//! Catalog IO stays outside the shared publication gate. Durable saves remain
//! successful even when a peer changes the view before the clean-marker CAS.
use std::sync::Arc;

use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use kglite_visual_core::{
    bookmark::{Bookmark, BookmarkCaptureOptions, BookmarkName, BookmarkSource, BookmarkStorage},
    CoreError,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    api::MutationOptions,
    broadcast::{AppState, DispatchError},
    views::{
        SavedView, SavedViewSummary, ViewStoreError, MAX_CATALOG_BYTES, MAX_VIEWS, MAX_VIEW_BYTES,
    },
};

#[derive(Deserialize)]
pub struct SaveView {
    pub name: String,
    #[serde(default)]
    pub replace: bool,
    #[serde(flatten)]
    pub capture: BookmarkCaptureOptions,
    #[serde(flatten)]
    pub options: MutationOptions,
}
#[derive(Deserialize)]
pub struct NamedView {
    #[serde(flatten)]
    pub bookmark: BookmarkName,
    #[serde(flatten)]
    pub options: MutationOptions,
}
#[derive(Deserialize)]
pub struct RestoreHistory {
    pub id: String,
    #[serde(flatten)]
    pub options: MutationOptions,
}

fn store_error(error: ViewStoreError) -> DispatchError {
    match error {
        ViewStoreError::Refused(message) => CoreError::Request(message).into(),
        ViewStoreError::Io(error) => DispatchError::Task(error.to_string()),
    }
}
fn task_error(error: tokio::task::JoinError) -> DispatchError {
    DispatchError::Task(error.to_string())
}
fn validate_options(options: &MutationOptions) -> Result<(), DispatchError> {
    if options.request_id.as_ref().is_some_and(|id| id.len() > 128) {
        return Err(CoreError::Request("request_id exceeds 128 bytes".into()).into());
    }
    Ok(())
}
fn decode_bookmark(value: &Value) -> Result<Bookmark, ViewStoreError> {
    let bookmark = Bookmark::deserialize(value).map_err(|error| {
        ViewStoreError::Refused(format!(
            "saved bookmark schema is unsupported or malformed: {error}"
        ))
    })?;
    kglite_visual_core::bookmark::validate_bookmark(&bookmark)
        .map_err(|error| ViewStoreError::Refused(error.to_string()))?;
    Ok(bookmark)
}
fn validate_existing(value: &Value) -> Result<(), ViewStoreError> {
    decode_bookmark(value).map(|_| ())
}
fn named_summary(storage: BookmarkStorage, summary: SavedViewSummary) -> Value {
    json!({"storage": storage, "name": summary.name, "saved_at": summary.saved_at})
}
fn get_saved(state: &AppState, name: &BookmarkName) -> Result<SavedView, ViewStoreError> {
    match name.storage {
        BookmarkStorage::Durable => state.views.get(&name.name),
        BookmarkStorage::Session => state.session_views.get(&name.name),
    }
}
fn error_value(error: DispatchError) -> Value {
    match error {
        DispatchError::Core(CoreError::Conflict(conflict)) => json!(conflict),
        DispatchError::Core(error) => json!({"error":error.to_string()}),
        DispatchError::Task(message) => json!({"error":message}),
    }
}

pub async fn list_value(state: AppState) -> Result<Value, DispatchError> {
    tokio::task::spawn_blocking(move || {
        let mut views: Vec<_> = state.views.list().map_err(store_error)?.into_iter()
            .map(|summary| named_summary(BookmarkStorage::Durable, summary)).collect();
        views.extend(state.session_views.list().map_err(store_error)?.into_iter()
            .map(|summary| named_summary(BookmarkStorage::Session, summary)));
        let eligibility = state.session.bookmark_eligibility();
        Ok(json!({"views":views,"save_storage":eligibility.storage,"eligibility":eligibility,
            "limits":{"durable":{"max_views":MAX_VIEWS,"max_bytes":MAX_CATALOG_BYTES},
            "session":{"max_views":MAX_VIEWS,"max_bytes":MAX_CATALOG_BYTES},"max_view_bytes":MAX_VIEW_BYTES}}))
    }).await.map_err(task_error)?
}

pub async fn save_value(state: AppState, body: SaveView) -> Result<Value, DispatchError> {
    save_with_capture_hook(state, body, || {}).await
}

async fn save_with_capture_hook(
    state: AppState,
    body: SaveView,
    captured: impl FnOnce() + Send + 'static,
) -> Result<Value, DispatchError> {
    validate_options(&body.options)?;
    let worker = state.clone();
    tokio::task::spawn_blocking(move || {
        let capture = worker.session.capture_bookmark_with(&body.capture, body.options.expected.as_ref())?;
        captured();
        let storage = match capture.durability {
            BookmarkSource::Durable { .. } => BookmarkStorage::Durable,
            BookmarkSource::SessionOnly { .. } => BookmarkStorage::Session,
        };
        let payload = serde_json::to_value(&capture.bookmark).map_err(|error| DispatchError::Task(error.to_string()))?;
        let summary = match storage {
            BookmarkStorage::Durable => worker.views.save_validated(&body.name, payload, body.replace, validate_existing),
            BookmarkStorage::Session => worker.session_views.save_validated(&body.name, payload, body.replace, validate_existing),
        }.map_err(store_error)?;
        let name = BookmarkName {storage, name:body.name};
        let mut result = json!({"saved":named_summary(storage,summary),"captured_stamp":capture.captured_stamp,
            "content_revision":capture.content_revision,"durability":capture.durability,"request_id":body.options.request_id});
        // Both IO and the metadata tail belong to this owned job. A cancelled
        // HTTP/MCP waiter cannot leave a successful replacement unacknowledged.
        let marker = worker.mark_bookmark_saved_blocking(name, capture, body.options.request_id);
        match marker {
            Ok(execution) => {
                result["marker_applied"] = true.into();
                result["dirty"] = false.into();
                result["stamp"] = json!(execution.stamp);
            }
            Err(error) => {
                result["marker_applied"] = false.into();
                result["dirty"] = true.into();
                result["marker_error"] = error_value(error);
            }
        }
        Ok(result)
    }).await.map_err(task_error)?
}

pub async fn restore_value(state: AppState, body: NamedView) -> Result<Value, DispatchError> {
    validate_options(&body.options)?;
    let worker = state.clone();
    let name = body.bookmark.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let saved = get_saved(&worker, &body.bookmark).map_err(store_error)?;
        let bookmark = decode_bookmark(&saved.bookmark).map_err(store_error)?;
        worker
            .session
            .prepare_bookmark_restore(
                &bookmark,
                Some(&body.bookmark),
                body.options.expected.as_ref(),
                body.options.request_id,
            )
            .map_err(DispatchError::Core)
    })
    .await
    .map_err(task_error)??;
    let execution = state.commit_prepared(prepared).await?;
    Ok(json!({"restored":name,"stamp":execution.stamp,"request_id":execution.request_id}))
}

pub async fn delete_value(state: AppState, body: NamedView) -> Result<Value, DispatchError> {
    delete_with_io_hook(state, body, || {}).await
}
async fn delete_with_io_hook(
    state: AppState,
    body: NamedView,
    before_io: impl FnOnce() + Send + 'static,
) -> Result<Value, DispatchError> {
    validate_options(&body.options)?;
    tokio::task::spawn_blocking(move || {
        if let Some(expected) = body.options.expected.as_ref() {
            kglite_visual_core::shared::check_stamp(expected, &state.session.shared_stamp())?;
        }
        before_io();
        match body.bookmark.storage {
            BookmarkStorage::Durable => state.views.delete(&body.bookmark.name),
            BookmarkStorage::Session => state.session_views.delete(&body.bookmark.name),
        }
        .map_err(store_error)?;
        let mut result = json!({"deleted":body.bookmark,"request_id":body.options.request_id});
        // The file transaction has released its OS lock before metadata enters
        // the publication gate. Cancellation cannot skip this conditional tail.
        match state.clear_deleted_marker_blocking(body.bookmark, body.options.request_id) {
            Ok(execution) => {
                result["marker_applied"] = true.into();
                result["stamp"] = json!(execution.stamp);
            }
            Err(error) => {
                result["marker_applied"] = false.into();
                result["marker_error"] = error_value(error);
            }
        }
        Ok(result)
    })
    .await
    .map_err(task_error)?
}

pub async fn history_value(state: AppState) -> Result<Value, DispatchError> {
    let session = Arc::clone(&state.session);
    tokio::task::spawn_blocking(move || {
        serde_json::to_value(session.history_state())
            .map_err(|error| DispatchError::Task(error.to_string()))
    })
    .await
    .map_err(task_error)?
}
pub async fn history_restore_value(
    state: AppState,
    body: RestoreHistory,
) -> Result<Value, DispatchError> {
    validate_options(&body.options)?;
    let session = Arc::clone(&state.session);
    let id = body.id.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        session.prepare_history_restore(
            &body.id,
            body.options.expected.as_ref(),
            body.options.request_id,
        )
    })
    .await
    .map_err(task_error)??;
    let execution = state.commit_prepared(prepared).await?;
    Ok(json!({"restored_history":id,"stamp":execution.stamp,"request_id":execution.request_id}))
}

fn reply(result: Result<Value, DispatchError>, request_id: Option<String>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => crate::api::dispatch_error(error, request_id.filter(|id| id.len() <= 128)),
    }
}
pub async fn list(State(state): State<AppState>) -> Response {
    reply(list_value(state).await, None)
}
pub async fn save(State(state): State<AppState>, Json(body): Json<SaveView>) -> Response {
    let id = body.options.request_id.clone();
    reply(save_value(state, body).await, id)
}
pub async fn restore(State(state): State<AppState>, Json(body): Json<NamedView>) -> Response {
    let id = body.options.request_id.clone();
    reply(restore_value(state, body).await, id)
}
pub async fn delete(State(state): State<AppState>, Json(body): Json<NamedView>) -> Response {
    let id = body.options.request_id.clone();
    reply(delete_value(state, body).await, id)
}
pub async fn history(State(state): State<AppState>) -> Response {
    reply(history_value(state).await, None)
}
pub async fn history_restore(
    State(state): State<AppState>,
    Json(body): Json<RestoreHistory>,
) -> Response {
    let id = body.options.request_id.clone();
    reply(history_restore_value(state, body).await, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite_visual_core::{
        shared::{CaptionRequest, SharedRequest},
        Request, Session,
    };
    use std::sync::mpsc;
    use std::time::Duration;

    fn make_state() -> AppState {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kglite-visual-core/tests/fixtures/meta.kgl");
        let graph = kglite_visual_core::load_graph(kglite_visual_core::GraphSource::Path(&fixture))
            .unwrap();
        AppState::new(
            Arc::new(Session::open(graph, "memory-saved-views")),
            "memory-saved-views",
        )
    }
    fn save_request(
        name: &str,
        expected: Option<kglite_visual_core::shared::RevisionStamp>,
    ) -> SaveView {
        serde_json::from_value(json!({"name":name,"expected":expected,"request_id":"save-test"}))
            .unwrap()
    }
    fn named(name: &str) -> NamedView {
        serde_json::from_value(json!({"storage":"session","name":name})).unwrap()
    }
    fn caption(name: &str) -> SharedRequest {
        SharedRequest::new(Request::Caption(CaptionRequest {
            caption_by: Some(name.into()),
        }))
    }

    #[tokio::test]
    async fn session_save_restore_delete_and_history_use_ordered_events() {
        let state = make_state();
        state.execute(caption("id")).await.unwrap();
        let mut events = state.bus.subscribe();
        let saved = save_value(
            state.clone(),
            save_request("first", Some(state.session.shared_stamp())),
        )
        .await
        .unwrap();
        assert_eq!(saved["saved"]["storage"], "session");
        assert_eq!(saved["marker_applied"], true);
        events.try_recv().unwrap();
        state.execute(caption("title")).await.unwrap();
        events.try_recv().unwrap();
        let restored = restore_value(state.clone(), named("first")).await.unwrap();
        assert_eq!(restored["stamp"], json!(state.session.shared_stamp()));
        assert_eq!(
            state.session.snapshot_shared().meta.caption_by.as_deref(),
            Some("id")
        );
        events.try_recv().unwrap();
        let history = history_value(state.clone()).await.unwrap();
        let id = history["history"]["entries"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        history_restore_value(
            state.clone(),
            serde_json::from_value(json!({"id":id,"expected":state.session.shared_stamp()}))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            state.session.snapshot_shared().meta.caption_by.as_deref(),
            Some("title")
        );
        events.try_recv().unwrap();
        delete_value(state.clone(), named("first")).await.unwrap();
        assert!(state.session_views.list().unwrap().is_empty());
        events.try_recv().unwrap();
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn stale_catalog_actions_and_missing_history_preserve_content_and_entries() {
        let state = make_state();
        let old = state.session.shared_stamp();
        save_value(state.clone(), save_request("kept", None))
            .await
            .unwrap();
        let before = serde_json::to_value(state.session.snapshot_shared().meta).unwrap();
        assert!(matches!(
            save_value(state.clone(), save_request("stale", Some(old.clone()))).await,
            Err(DispatchError::Core(CoreError::Conflict(_)))
        ));
        let stale: NamedView =
            serde_json::from_value(json!({"storage":"session","name":"kept","expected":old}))
                .unwrap();
        assert!(matches!(
            delete_value(state.clone(), stale).await,
            Err(DispatchError::Core(CoreError::Conflict(_)))
        ));
        assert_eq!(state.session_views.list().unwrap().len(), 1);
        assert!(history_restore_value(
            state.clone(),
            serde_json::from_value(json!({"id":"missing"})).unwrap()
        )
        .await
        .is_err());
        assert_eq!(
            serde_json::to_value(state.session.snapshot_shared().meta).unwrap(),
            before
        );
        assert!(make_state().session_views.list().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_catalog_jobs_still_publish_their_metadata_tail() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kglite-visual-core/tests/fixtures/meta.kgl");
        let session = kglite_visual_core::load_session_with(
            kglite_visual_core::GraphSource::Path(&fixture),
            "display label is not provenance",
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut state = AppState::new(Arc::new(session), "cancelled-save");
        state.views = Arc::new(crate::views::ViewStore::at(root.path().into()));
        for deleting in [false, true] {
            let (entered_tx, entered) = mpsc::channel();
            let (release_tx, release) = mpsc::channel();
            let hook = move || {
                entered_tx.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(10)).unwrap();
            };
            let worker = state.clone();
            let mut events = state.bus.subscribe();
            let job = tokio::spawn(async move {
                if deleting {
                    delete_with_io_hook(
                        worker,
                        serde_json::from_value(json!({"storage":"durable","name":"cancelled"}))
                            .unwrap(),
                        hook,
                    )
                    .await
                } else {
                    save_with_capture_hook(worker, save_request("cancelled", None), hook).await
                }
            });
            tokio::task::spawn_blocking(move || {
                entered.recv_timeout(Duration::from_secs(5)).unwrap()
            })
            .await
            .unwrap();
            job.abort();
            assert!(job.await.unwrap_err().is_cancelled());
            release_tx.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(state.views.list().unwrap().len(), usize::from(!deleting));
            assert_eq!(
                state.session.snapshot_shared().meta.saved_view.is_some(),
                !deleting
            );
            assert!(events.try_recv().is_err());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn older_replacement_cannot_leave_a_clean_marker_for_newer_content() {
        let state = make_state();
        let (entered_tx, entered) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        let worker = state.clone();
        let saving = tokio::spawn(async move {
            let mut body = save_request("same-name", None);
            body.replace = true;
            save_with_capture_hook(worker, body, move || {
                entered_tx.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(10)).unwrap();
            })
            .await
        });
        tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(5)).unwrap())
            .await
            .unwrap();
        state.execute(caption("id")).await.unwrap();
        assert_eq!(
            save_value(state.clone(), save_request("same-name", None))
                .await
                .unwrap()["marker_applied"],
            true
        );
        release_tx.send(()).unwrap();
        let result = saving.await.unwrap().unwrap();
        assert_eq!(result["dirty"], true);
        assert!(state.session_views.get("same-name").unwrap().bookmark["caption_by"].is_null());
        assert!(
            state
                .session
                .snapshot_shared()
                .meta
                .saved_view
                .as_ref()
                .is_none_or(|marker| marker.dirty),
            "catalog now contains older content, so the newer shared marker cannot remain clean"
        );
    }

    #[tokio::test]
    async fn conflicted_save_preserves_a_different_clean_named_association() {
        let state = make_state();
        let capture = state.session.capture_bookmark(None).unwrap();
        state.execute(caption("id")).await.unwrap();
        save_value(state.clone(), save_request("other-name", None))
            .await
            .unwrap();
        state
            .session_views
            .save_validated(
                "older-name",
                json!(capture.bookmark),
                true,
                validate_existing,
            )
            .unwrap();
        assert!(state
            .mark_bookmark_saved_blocking(
                BookmarkName {
                    storage: BookmarkStorage::Session,
                    name: "older-name".into(),
                },
                capture,
                None
            )
            .is_err());
        let marker = state.session.snapshot_shared().meta.saved_view.unwrap();
        assert_eq!(marker.bookmark.name, "other-name");
        assert!(!marker.dirty);
        assert_eq!(
            state.session.snapshot_shared().meta.caption_by.as_deref(),
            Some("id")
        );
    }

    async fn save_race(content_change: bool) {
        let state = make_state();
        let before = state.session.shared_stamp();
        let (entered_tx, entered) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        let worker = state.clone();
        let saving = tokio::spawn(async move {
            save_with_capture_hook(worker, save_request("racing", Some(before)), move || {
                entered_tx.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(10)).unwrap();
            })
            .await
        });
        tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(5)).unwrap())
            .await
            .unwrap();
        if content_change {
            state.execute(caption("id")).await.unwrap();
        } else {
            state
                .execute(SharedRequest::new(Request::Focus(
                    kglite_visual_core::control::FocusRequest { slots: Vec::new() },
                )))
                .await
                .unwrap();
        }
        release_tx.send(()).unwrap();
        let result = saving.await.unwrap().unwrap();
        assert_eq!(result["marker_applied"], !content_change);
        assert_eq!(result["dirty"], content_change);
        let stored = state.session_views.get("racing").unwrap();
        assert!(
            stored.bookmark["caption_by"].is_null(),
            "persisted the immutable captured exploration"
        );
        assert_eq!(
            state.session.snapshot_shared().meta.caption_by.as_deref(),
            content_change.then_some("id")
        );
        if content_change {
            assert_eq!(result["marker_error"]["code"], "saved-content-conflict");
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn focus_during_save_keeps_clean_marker() {
        save_race(false).await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn content_change_during_save_persists_capture_and_reports_dirty() {
        save_race(true).await;
    }
}
