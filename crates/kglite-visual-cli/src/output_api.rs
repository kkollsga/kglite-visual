//! Private immutable output captures. Preview digests bind every effective
//! setting; downloads never reuse a newer live subset behind an older preview.
use std::io::Write;

use axum::{
    extract::State,
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use kglite_visual_core::{
    output::{
        CaptureOutputRequest, CapturedOutput, CapturedRendered, OutputArtifact, OutputPreview,
        RenderOutputSettings, MAX_OUTPUT_BYTES,
    },
    CoreError, ExportFormat,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::broadcast::{AppState, DispatchError};

pub const MAX_EXPORT_METADATA_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_IMAGE_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_PREVIEW_JSON_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_MCP_RESULT_BYTES: usize = 24 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct ExportOutputRequest {
    #[serde(flatten)]
    pub capture: CaptureOutputRequest,
    pub format: ExportFormat,
    #[serde(default)]
    pub include_identity: bool,
    #[serde(default)]
    pub preview_digest: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct RenderOutputRequest {
    #[serde(flatten)]
    pub capture: CaptureOutputRequest,
    #[serde(flatten)]
    pub settings: RenderOutputSettings,
    #[serde(default)]
    pub preview_digest: Option<String>,
}

pub struct PreparedExport {
    pub metadata: Value,
    pub metadata_bytes: Vec<u8>,
    pub artifact: Option<OutputArtifact>,
}
pub struct PreparedImage {
    pub metadata: Value,
    pub image: CapturedRendered,
}

fn request_error(message: impl Into<String>) -> DispatchError {
    CoreError::Request(message.into()).into()
}
fn task_error(error: tokio::task::JoinError) -> DispatchError {
    DispatchError::Task(error.to_string())
}
fn check_digest(preview: &OutputPreview, digest: Option<&str>) -> Result<(), DispatchError> {
    let digest = digest.ok_or_else(|| {
        request_error("preview_digest is required; refresh the preview before downloading")
    })?;
    CapturedOutput::check_preview_digest(preview, digest).map_err(Into::into)
}

pub async fn prepare_export(
    state: AppState,
    body: ExportOutputRequest,
    download: bool,
) -> Result<PreparedExport, DispatchError> {
    tokio::task::spawn_blocking(move || {
        let captured = state.session.capture_output(&body.capture)?;
        let preview = captured.preview(body.format)?;
        if download {
            check_digest(&preview, body.preview_digest.as_deref())?;
        }
        let mut metadata = json!(preview);
        if body.include_identity {
            metadata["identity"] = json!(captured.identity());
        }
        let metadata_bytes = bounded_json(&metadata, MAX_EXPORT_METADATA_BYTES)?;
        let artifact = if download {
            Some(captured.export(body.format)?)
        } else {
            None
        };
        Ok(PreparedExport {
            metadata,
            metadata_bytes,
            artifact,
        })
    })
    .await
    .map_err(task_error)?
}
pub async fn prepare_image(
    state: AppState,
    body: RenderOutputRequest,
    download: bool,
) -> Result<PreparedImage, DispatchError> {
    tokio::task::spawn_blocking(move || {
        let captured = state.session.capture_output(&body.capture)?;
        let preview = captured.preview_render(&body.settings)?;
        if download {
            check_digest(&preview, body.preview_digest.as_deref())?;
        }
        let image = captured.render(&body.settings)?;
        let metadata = json!({"preview":image.preview,"rendered":image.rendered});
        bounded_json(&metadata, MAX_IMAGE_METADATA_BYTES)?;
        if image.rendered.bytes.len() > MAX_OUTPUT_BYTES {
            return Err(request_error("rendered output exceeds 16 MiB"));
        }
        Ok(PreparedImage { metadata, image })
    })
    .await
    .map_err(task_error)?
}

/// Serialize against the cap while writing, before allocating an oversized reply.
pub(crate) fn bounded_json(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, DispatchError> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                return Err(std::io::Error::other(
                    "output JSON exceeds its response ceiling",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| request_error(error.to_string()))?;
    Ok(writer.bytes)
}

pub(crate) fn image_preview_json(prepared: PreparedImage) -> Result<Vec<u8>, DispatchError> {
    let metadata_bytes = bounded_json(&prepared.metadata, MAX_IMAGE_METADATA_BYTES)?.len();
    let encoded_bytes = prepared
        .image
        .rendered
        .bytes
        .len()
        .div_ceil(3)
        .checked_mul(4)
        .ok_or_else(|| request_error("image preview length overflow"))?;
    // Base64 contains no JSON-escaped characters. This reserve includes its
    // property name, delimiters, and the existing metadata object's braces.
    if metadata_bytes
        .saturating_add(encoded_bytes)
        .saturating_add(32)
        > MAX_PREVIEW_JSON_BYTES
    {
        return Err(request_error("base64 image preview exceeds 24 MiB"));
    }
    let mut value = prepared.metadata;
    value["image_base64"] = base64::engine::general_purpose::STANDARD
        .encode(&prepared.image.rendered.bytes)
        .into();
    bounded_json(&value, MAX_PREVIEW_JSON_BYTES)
}
fn json_response(bytes: Vec<u8>) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
}
fn failure(error: DispatchError) -> Response {
    crate::api::dispatch_error(error, None)
}

pub async fn export_preview(
    State(state): State<AppState>,
    Json(body): Json<ExportOutputRequest>,
) -> Response {
    match prepare_export(state, body, false).await {
        Ok(result) => json_response(result.metadata_bytes),
        Err(error) => failure(error),
    }
}

pub async fn export_download(
    State(state): State<AppState>,
    Json(body): Json<ExportOutputRequest>,
) -> Response {
    let format = body.format;
    match prepare_export(state, body, true).await {
        Ok(result) => {
            let artifact = result.artifact.expect("download prepares an artifact");
            artifact_response(
                artifact.bytes,
                format.content_type(),
                &artifact.filename,
                &artifact.preview,
            )
        }
        Err(error) => failure(error),
    }
}
pub async fn render_preview(
    State(state): State<AppState>,
    Json(body): Json<RenderOutputRequest>,
) -> Response {
    let prepared = match prepare_image(state, body, false).await {
        Ok(prepared) => prepared,
        Err(error) => return failure(error),
    };
    match tokio::task::spawn_blocking(move || image_preview_json(prepared))
        .await
        .map_err(task_error)
        .and_then(|result| result)
    {
        Ok(bytes) => json_response(bytes),
        Err(error) => failure(error),
    }
}

pub async fn render_download(
    State(state): State<AppState>,
    Json(body): Json<RenderOutputRequest>,
) -> Response {
    match prepare_image(state, body, true).await {
        Ok(result) => {
            let image = result.image;
            artifact_response(
                image.rendered.bytes,
                image.rendered.format.content_type(),
                &image.filename,
                &image.preview,
            )
        }
        Err(error) => failure(error),
    }
}
fn artifact_response(
    bytes: Vec<u8>,
    content_type: &'static str,
    filename: &str,
    preview: &OutputPreview,
) -> Response {
    let mut response = ([(header::CONTENT_TYPE, content_type)], bytes).into_response();
    for (name, value) in [
        (
            "content-disposition",
            crate::api::content_disposition(filename),
        ),
        ("x-kglv-generation", preview.stamp.generation.clone()),
        ("x-kglv-revision", preview.stamp.revision.clone()),
        ("x-kglv-subset-revision", preview.subset_revision.clone()),
        (
            "x-kglv-scope",
            match preview.scope {
                kglite_visual_core::output::OutputScope::Visible => "visible",
                kglite_visual_core::output::OutputScope::LoadedInduced => "loaded-induced",
            }
            .into(),
        ),
        ("x-kglv-nodes", preview.nodes.to_string()),
        ("x-kglv-edges", preview.edges.to_string()),
        ("x-kglv-preview-digest", preview.preview_digest.clone()),
    ] {
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite_visual_core::{
        query_provenance::LoadEntitiesRequest,
        request::{CypherRequest, Request},
        shared::{CaptionRequest, SharedRequest},
        GraphSource, Response as CoreResponse,
    };
    use std::sync::Arc;

    async fn loaded_state() -> AppState {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kglite-visual-core/tests/fixtures/viewer-identity.kgl");
        let session = kglite_visual_core::load_session_with(
            GraphSource::Path(&path),
            "output fixture",
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let query = |text: &str| {
            let CoreResponse::Query(table) = session
                .handle(&Request::Cypher(CypherRequest {
                    query: text.into(),
                    params: Default::default(),
                    limit: None,
                    as_graph: false,
                }))
                .unwrap()
            else {
                panic!("query table")
            };
            table
        };
        let nodes = query("MATCH (n:Person) RETURN n")
            .row_references
            .into_iter()
            .flat_map(|row| row.nodes)
            .collect();
        let mut relationships: Vec<_> = query("MATCH (a)-[r]->(b) RETURN r")
            .row_references
            .into_iter()
            .flat_map(|row| row.relationships)
            .collect();
        relationships.sort_by_key(|relation| relation.edge_id);
        relationships.truncate(3);
        let state = AppState::new(Arc::new(session), "output fixture");
        state
            .execute(SharedRequest::new(Request::LoadEntities(
                LoadEntitiesRequest {
                    nodes,
                    relationships,
                },
            )))
            .await
            .unwrap();
        state
    }
    fn export_body(state: &AppState) -> ExportOutputRequest {
        let view = state.session.view_state();
        ExportOutputRequest {
            capture: CaptureOutputRequest {
                scope: kglite_visual_core::output::OutputScope::Visible,
                expected: view.stamp,
                subset_revision: view.subset_revision,
            },
            format: ExportFormat::Gexf,
            include_identity: true,
            preview_digest: None,
        }
    }
    #[tokio::test]
    async fn preview_and_final_keep_exact_edges_identity_and_refuse_changed_settings_or_revision() {
        let state = loaded_state().await;
        let mut body = export_body(&state);
        let mut events = state.bus.subscribe();
        let before = serde_json::to_value(state.session.view_state()).unwrap();
        assert!(prepare_export(state.clone(), body.clone(), true)
            .await
            .is_err());
        let preview = prepare_export(state.clone(), body.clone(), false)
            .await
            .unwrap();
        assert!(preview.artifact.is_none());
        assert_eq!(preview.metadata["nodes"], 3);
        assert_eq!(preview.metadata["edges"], 3);
        let mut source_edges: Vec<u64> = preview.metadata["identity"]["edge_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_u64().unwrap())
            .collect();
        source_edges.sort_unstable();
        assert_eq!(source_edges, [0, 1, 2]);
        body.preview_digest = Some(preview.metadata["preview_digest"].as_str().unwrap().into());
        let final_output = prepare_export(state.clone(), body.clone(), true)
            .await
            .unwrap();
        let artifact = final_output.artifact.unwrap();
        let xml = String::from_utf8(artifact.bytes).unwrap();
        assert_eq!(xml.matches("<edge ").count(), 3);
        assert_eq!(artifact.preview.stamp, body.capture.expected);
        let legacy = state.session.export_view(ExportFormat::Gexf).unwrap();
        assert_eq!(
            String::from_utf8(legacy.bytes)
                .unwrap()
                .matches("<edge ")
                .count(),
            4
        );
        let mut changed_settings = body.clone();
        changed_settings.format = ExportFormat::Csv;
        assert!(matches!(
            prepare_export(state.clone(), changed_settings, true).await,
            Err(DispatchError::Core(CoreError::Conflict(_)))
        ));
        assert_eq!(
            serde_json::to_value(state.session.view_state()).unwrap(),
            before
        );
        assert!(events.try_recv().is_err());
        state
            .execute(SharedRequest::new(Request::Caption(CaptionRequest {
                caption_by: Some("id".into()),
            })))
            .await
            .unwrap();
        assert!(matches!(
            prepare_export(state, body, true).await,
            Err(DispatchError::Core(CoreError::Conflict(_)))
        ));
    }
    #[tokio::test]
    async fn image_preview_and_download_have_identical_captured_settings_and_bytes() {
        let state = loaded_state().await;
        let mut body = RenderOutputRequest {
            capture: export_body(&state).capture,
            settings: RenderOutputSettings {
                width: 400,
                height: 300,
                ..Default::default()
            },
            preview_digest: None,
        };
        let preview = prepare_image(state.clone(), body.clone(), false)
            .await
            .unwrap();
        body.preview_digest = Some(preview.image.preview.preview_digest.clone());
        let expected = preview.image.rendered.bytes.clone();
        let json_bytes = image_preview_json(preview).unwrap();
        assert!(json_bytes.len() <= MAX_PREVIEW_JSON_BYTES);
        let value: Value = serde_json::from_slice(&json_bytes).unwrap();
        assert_eq!(value["preview"]["edges"], 3);
        assert_eq!(value["rendered"]["width"], 400);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(value["image_base64"].as_str().unwrap())
                .unwrap(),
            expected
        );
        let final_image = prepare_image(state.clone(), body.clone(), true)
            .await
            .unwrap();
        assert_eq!(final_image.image.rendered.bytes, expected);
        body.settings.width = 401;
        assert!(matches!(
            prepare_image(state, body, true).await,
            Err(DispatchError::Core(CoreError::Conflict(_)))
        ));
    }
    #[test]
    fn flattened_render_input_preserves_capture_and_explicit_settings() {
        let body:RenderOutputRequest=serde_json::from_value(json!({"scope":"visible","expected":{"generation":"test","revision":"2"},"subset_revision":"1","format":"png","width":300,"height":400,"seed":123,"theme":"light","kernel":"force","preview_digest":"x"})).unwrap();
        assert_eq!(body.settings.width, 300);
        assert_eq!(body.settings.seed, 123);
        assert_eq!(body.capture.expected.revision, "2");
        assert_eq!(body.preview_digest.as_deref(), Some("x"));
    }
    #[test]
    fn response_ceiling_counts_json_escaping_before_writing() {
        assert!(bounded_json(&json!({"text":"\0".repeat(100)}), 100).is_err());
        assert!(bounded_json(&json!({"text":"small"}), 100).is_ok());
    }
}
