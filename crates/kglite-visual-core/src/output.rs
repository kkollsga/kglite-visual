//! Immutable, explicitly scoped output captures and preview identity.
use std::time::{Duration, Instant};

use kglite::api::{DirGraph, NodeIndex};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::export::ExportFormat;
use crate::presentation::PresentationSettings;
use crate::records::NodeHandle;
use crate::render::{RenderFormat, Rendered, Theme};
use crate::request::LayoutKernel;
use crate::shared::RevisionStamp;
use crate::CoreError;

pub const MAX_OUTPUT_PROPERTY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
pub const OUTPUT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub enum OutputScope {
    Visible,
    LoadedInduced,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CaptureOutputRequest {
    pub scope: OutputScope,
    pub expected: RevisionStamp,
    pub subset_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RenderOutputSettings {
    #[ts(type = "'svg' | 'png'")]
    pub format: RenderFormat,
    pub width: u32,
    pub height: u32,
    pub seed: u32,
    #[ts(type = "'dark' | 'light'")]
    pub theme: Theme,
    pub kernel: LayoutKernel,
}
impl Default for RenderOutputSettings {
    fn default() -> Self {
        Self {
            format: RenderFormat::Svg,
            width: crate::render::DEFAULT_WIDTH,
            height: crate::render::DEFAULT_HEIGHT,
            seed: 0,
            theme: Theme::default(),
            kernel: LayoutKernel::Auto,
        }
    }
}
impl RenderOutputSettings {
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        if !(200..=8000).contains(&self.width) || !(200..=8000).contains(&self.height) {
            return Err(refusal(
                "render width and height must be between 200 and 8000",
            ));
        }
        if self.kernel == LayoutKernel::Simulation {
            return Err(refusal("a server image requires a static layout kernel"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct OutputPreview {
    pub stamp: RevisionStamp,
    pub subset_revision: String,
    pub scope: OutputScope,
    pub nodes: u32,
    pub edges: u32,
    pub format: String,
    pub preview_digest: String,
    pub notes: Vec<String>,
}

/// Array positions are export-local identities, not durable source keys.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct OutputIdentity {
    pub nodes: Vec<NodeHandle>,
    pub edge_ids: Vec<u32>,
    pub semantics: String,
}

pub struct OutputArtifact {
    pub preview: OutputPreview,
    pub bytes: Vec<u8>,
    pub filename: String,
}
pub struct CapturedRendered {
    pub preview: OutputPreview,
    pub rendered: Rendered,
    pub filename: String,
}

/// No live Session or source graph is consulted after this value is constructed.
pub struct CapturedOutput {
    pub(crate) stamp: RevisionStamp,
    pub(crate) subset_revision: String,
    pub(crate) scope: OutputScope,
    pub(crate) graph: DirGraph,
    pub(crate) nodes: Vec<NodeIndex>,
    pub(crate) identity: OutputIdentity,
    pub(crate) scene: crate::render::Scene,
    pub(crate) presentation: PresentationSettings,
    pub(crate) encoded_bound: usize,
    pub(crate) json_collision: bool,
}

impl CapturedOutput {
    pub fn identity(&self) -> &OutputIdentity {
        &self.identity
    }

    pub fn preview(&self, format: ExportFormat) -> Result<OutputPreview, CoreError> {
        if format == ExportFormat::Json && self.json_collision {
            return Err(refusal("D3 JSON cannot preserve a relation property named source, target, or type; choose GraphML to retain those properties"));
        }
        if self.encoded_bound > MAX_OUTPUT_BYTES {
            return Err(refusal("export's conservative encoded size exceeds 16 MiB"));
        }
        let mut preview = self.preview_base(format.as_str(), &format)?;
        if matches!(
            format,
            ExportFormat::Gexf | ExportFormat::Csv | ExportFormat::CsvEdges
        ) {
            preview.notes.push("this format carries structural columns and labels but omits arbitrary source properties".into());
        }
        if format == ExportFormat::Json {
            preview.notes.push("D3 JSON writes source integers as JSON numbers; use a lossless integer parser for values beyond JavaScript's safe range".into());
            preview.notes.push("D3 JSON omits a genuine node property named type because that name is reserved for the structural node type".into());
        }
        Ok(preview)
    }

    pub fn preview_render(
        &self,
        settings: &RenderOutputSettings,
    ) -> Result<OutputPreview, CoreError> {
        settings.validate()?;
        let mut preview = self.preview_base(settings.format.extension(), settings)?;
        preview.notes.push(
            "deterministic server image; geometry is not the browser camera or simulation".into(),
        );
        Ok(preview)
    }

    pub fn check_preview_digest(preview: &OutputPreview, expected: &str) -> Result<(), CoreError> {
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(refusal(
                "preview_digest must contain 64 hexadecimal characters",
            ));
        }
        if preview.preview_digest != expected {
            return Err(CoreError::Conflict(Box::new(crate::shared::RevisionConflict {
                code: "preview-settings-conflict".into(), expected: preview.stamp.clone(), actual: preview.stamp.clone(),
                message: "preview settings or captured state changed; refresh the preview before downloading".into(),
            })));
        }
        Ok(())
    }

    fn preview_base(
        &self,
        format: &str,
        settings: &impl Serialize,
    ) -> Result<OutputPreview, CoreError> {
        let canonical = serde_json::to_vec(&(
            1u32,
            self.scope,
            &self.stamp,
            &self.subset_revision,
            settings,
            &self.presentation,
        ))
        .map_err(|error| refusal(&error.to_string()))?;
        Ok(OutputPreview {
            stamp: self.stamp.clone(),
            subset_revision: self.subset_revision.clone(),
            scope: self.scope,
            nodes: self.nodes.len() as u32,
            edges: self.identity.edge_ids.len() as u32,
            format: format.into(),
            preview_digest: crate::source_identity::hex_digest(&Sha256::digest(canonical)),
            notes: vec![match self.scope {
                OutputScope::Visible => "visible instance nodes and exactly their visible retained relation records",
                OutputScope::LoadedInduced => "loaded instance nodes and every source relation between them, including hidden or previously unretained relations",
            }.into()],
        })
    }

    pub fn export(&self, format: ExportFormat) -> Result<OutputArtifact, CoreError> {
        let preview = self.preview(format)?;
        let deadline = Instant::now() + OUTPUT_TIMEOUT;
        let exported =
            crate::export::export_nodes(&self.graph, &self.nodes, format, "captured-view")?;
        check_deadline(deadline)?;
        if exported.bytes.len() > MAX_OUTPUT_BYTES {
            return Err(refusal("formatted export exceeds 16 MiB"));
        }
        Ok(OutputArtifact {
            preview,
            bytes: exported.bytes,
            filename: self.filename(format.extension()),
        })
    }

    pub(crate) fn filename(&self, extension: &str) -> String {
        let scope = match self.scope {
            OutputScope::Visible => "visible",
            OutputScope::LoadedInduced => "loaded-induced",
        };
        format!("{scope}-r{}.{}", self.stamp.revision, extension)
    }
}

pub(crate) fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}
pub(crate) fn check_deadline(deadline: Instant) -> Result<(), CoreError> {
    if Instant::now() >= deadline {
        return Err(refusal(
            "output capture exceeded its 10 second work deadline",
        ));
    }
    Ok(())
}
