//! Captured output modes share the HTTP capture/digest/bounds implementation.
use super::*;
use crate::output_api::{self, ExportOutputRequest, RenderOutputRequest};
use kglite_visual_core::output::{CaptureOutputRequest, OutputScope, RenderOutputSettings};

impl CapturedArgs {
    pub(super) fn has_capture_fields(&self) -> bool {
        self.expected.is_some()
            || self.subset_revision.is_some()
            || self.preview_digest.is_some()
            || self.include_identity
    }
    fn request(&self) -> Result<CaptureOutputRequest, CoreError> {
        let scope = self.scope.ok_or_else(|| {
            CoreError::Request("captured output requires an explicit scope".into())
        })?;
        let stamp = self.expected.as_ref().ok_or_else(|| {
            CoreError::Request("explicit output scope requires expected from view_state".into())
        })?;
        let subset_revision = self.subset_revision.as_ref().ok_or_else(|| {
            CoreError::Request(
                "explicit output scope requires subset_revision from view_state".into(),
            )
        })?;
        if stamp.generation.is_empty() || stamp.revision.is_empty() || subset_revision.is_empty() {
            return Err(CoreError::Request(
                "captured output stamps must not be empty".into(),
            ));
        }
        Ok(CaptureOutputRequest {
            scope: match scope {
                OutputScopeArg::Visible => OutputScope::Visible,
                OutputScopeArg::LoadedInduced => OutputScope::LoadedInduced,
            },
            expected: RevisionStamp {
                generation: stamp.generation.clone(),
                revision: stamp.revision.clone(),
            },
            subset_revision: subset_revision.clone(),
        })
    }
}
fn refused_dispatch(error: DispatchError) -> Result<CallToolResult, McpError> {
    match error {
        DispatchError::Core(error) => Ok(refused(&error)),
        DispatchError::Task(message) => Err(McpError::internal_error(message, None)),
    }
}
fn bounded_result(result: CallToolResult) -> Result<CallToolResult, DispatchError> {
    output_api::bounded_json(&result, output_api::MAX_MCP_RESULT_BYTES)?;
    Ok(result)
}

pub(super) async fn export_captured(
    control: &ViewControl,
    args: ExportArgs,
) -> Result<CallToolResult, McpError> {
    let capture = match args.captured.request() {
        Ok(capture) => capture,
        Err(error) => return Ok(refused(&error)),
    };
    let final_output = args.captured.preview_digest.is_some();
    let prepared = match output_api::prepare_export(
        control.state.clone(),
        ExportOutputRequest {
            capture,
            format: args.format.into(),
            include_identity: args.captured.include_identity,
            preview_digest: args.captured.preview_digest,
        },
        final_output,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return refused_dispatch(error),
    };
    match tokio::task::spawn_blocking(move || {
        let mut metadata = prepared.metadata;
        metadata["output_stage"] = if final_output { "final" } else { "preview" }.into();
        let mut contents = Vec::new();
        if let Some(artifact) = prepared.artifact {
            metadata["filename"] = artifact.filename.into();
            let text = String::from_utf8(artifact.bytes)
                .map_err(|error| DispatchError::Task(error.to_string()))?;
            contents.push(ContentBlock::text(text));
        }
        contents.insert(0, ContentBlock::text(metadata.to_string()));
        bounded_result(CallToolResult::success(contents))
    })
    .await
    .map_err(|error| McpError::internal_error(error.to_string(), None))?
    {
        Ok(result) => Ok(result),
        Err(error) => refused_dispatch(error),
    }
}

pub(super) async fn render_captured(
    control: &ViewControl,
    args: RenderArgs,
) -> Result<CallToolResult, McpError> {
    if !matches!(args.target, RenderTargetArg::LiveView)
        || args.query.is_some()
        || args.params.is_some()
        || args.captured.include_identity
    {
        return Ok(refused_text("captured images use the explicit instance scope; query targets and identity mappings are not image settings"));
    }
    let capture = match args.captured.request() {
        Ok(capture) => capture,
        Err(error) => return Ok(refused(&error)),
    };
    let final_output = args.captured.preview_digest.is_some();
    let prepared = match output_api::prepare_image(
        control.state.clone(),
        RenderOutputRequest {
            capture,
            preview_digest: args.captured.preview_digest,
            settings: RenderOutputSettings {
                format: match args.format {
                    FormatArg::Png => RenderFormat::Png,
                    FormatArg::Svg => RenderFormat::Svg,
                },
                width: args
                    .width
                    .unwrap_or(kglite_visual_core::render::DEFAULT_WIDTH),
                height: args
                    .height
                    .unwrap_or(kglite_visual_core::render::DEFAULT_HEIGHT),
                seed: args.seed,
                theme: match args.theme {
                    ThemeArg::Dark => Theme::Dark,
                    ThemeArg::Light => Theme::Light,
                },
                kernel: args
                    .layout
                    .map(Into::into)
                    .unwrap_or(kglite_visual_core::request::LayoutKernel::Auto),
            },
        },
        final_output,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return refused_dispatch(error),
    };
    match tokio::task::spawn_blocking(move || {
        let mut metadata = prepared.metadata;
        metadata["output_stage"] = if final_output { "final" } else { "preview" }.into();
        metadata["filename"] = prepared.image.filename.into();
        let rendered = prepared.image.rendered;
        let image = match rendered.format {
            RenderFormat::Png => ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(&rendered.bytes),
                "image/png",
            ),
            RenderFormat::Svg => ContentBlock::text(
                String::from_utf8(rendered.bytes)
                    .map_err(|error| DispatchError::Task(error.to_string()))?,
            ),
        };
        bounded_result(CallToolResult::success(vec![
            ContentBlock::text(metadata.to_string()),
            image,
        ]))
    })
    .await
    .map_err(|error| McpError::internal_error(error.to_string(), None))?
    {
        Ok(result) => Ok(result),
        Err(error) => refused_dispatch(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn presentation_only_preserves_channels_and_mixed_null_is_one_atomic_change() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kglite-visual-core/tests/fixtures/meta.kgl");
        let graph = kglite_visual_core::load_graph(kglite_visual_core::GraphSource::Path(&fixture))
            .unwrap();
        let state = AppState::new(
            Arc::new(kglite_visual_core::Session::open(graph, "mcp-style")),
            "mcp-style",
        );
        let control = ViewControl::new(state.clone());
        let args =
            serde_json::from_value(serde_json::json!({"color_by":"group","size_by":"score","presentation":{"label_density":0.6,"legend_visible":false}}))
                .unwrap();
        assert!(!control
            .set_appearance(Parameters(args))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        let mut events = state.bus.subscribe();
        let read = serde_json::to_value(control.view_state().await.unwrap()).unwrap();
        let current: serde_json::Value =
            serde_json::from_str(read["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(current["appearance"]["color_by"], "group");
        assert!(current.get("appearance_mapping").is_none());
        let mut presentation = current["presentation"].clone();
        presentation["edge_opacity"] = serde_json::json!(0.4);
        let args = serde_json::from_value(
            serde_json::json!({"presentation":presentation,"expected":current["stamp"]}),
        )
        .unwrap();
        assert!(!control
            .set_appearance(Parameters(args))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        events.try_recv().unwrap();
        assert!(events.try_recv().is_err());
        let preserved = state.session.snapshot_shared();
        assert_eq!(preserved.meta.appearance.color_by.as_deref(), Some("group"));
        assert_eq!(preserved.meta.appearance.size_by.as_deref(), Some("score"));
        assert_eq!(preserved.meta.presentation.edge_opacity, 0.4);
        assert_eq!(preserved.meta.presentation.label_density, 0.6);
        assert!(!preserved.meta.presentation.legend_visible);
        let args=serde_json::from_value(serde_json::json!({"color_by":null,"presentation":{"label_density":0.3},"expected":preserved.meta.stamp})).unwrap();
        assert!(!control
            .set_appearance(Parameters(args))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        events.try_recv().unwrap();
        assert!(events.try_recv().is_err());
        let mixed = state.session.snapshot_shared();
        assert!(mixed.meta.appearance.color_by.is_none());
        assert!(mixed.meta.appearance.size_by.is_none());
        assert_eq!(mixed.meta.presentation.label_density, 0.3);
        let args = serde_json::from_value(
            serde_json::json!({"presentation":{"edge_opacity":2.0},"expected":mixed.meta.stamp}),
        )
        .unwrap();
        assert!(control
            .set_appearance(Parameters(args))
            .await
            .unwrap()
            .is_error
            .unwrap_or(false));
        assert_eq!(state.session.snapshot_shared().meta, mixed.meta);
        assert!(events.try_recv().is_err());
    }
    #[test]
    fn explicit_scope_never_falls_back_when_capture_stamps_are_missing() {
        for value in [
            serde_json::json!({"scope":"visible"}),
            serde_json::json!({"scope":"visible","expected":{"generation":"","revision":""},"subset_revision":""}),
        ] {
            let args: CapturedArgs = serde_json::from_value(value).unwrap();
            assert!(args.request().is_err());
        }
    }
}
