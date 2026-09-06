//! Shared readability settings retain structural sizes and mandatory schema labels.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::CoreError;

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct StyleRequest {
    pub appearance: Option<crate::control::AppearanceRequest>,
    pub presentation: Option<PresentationSettings>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct PresentationSettings {
    /// One preserves the existing instance label sampler; schema labels are unaffected.
    pub label_density: f32,
    pub prioritize_selected_labels: bool,
    pub prioritize_hovered_labels: bool,
    pub edge_opacity: f32,
    /// Numeric size-by range only; structural instance and schema radii remain intact.
    pub node_size_min: f32,
    pub node_size_max: f32,
    pub legend_visible: bool,
}
impl Default for PresentationSettings {
    fn default() -> Self {
        Self {
            label_density: 1.0,
            prioritize_selected_labels: true,
            prioritize_hovered_labels: false,
            edge_opacity: 1.0,
            node_size_min: 4.0,
            node_size_max: 22.0,
            legend_visible: true,
        }
    }
}
impl PresentationSettings {
    pub(crate) fn validate(&self) -> Result<(), CoreError> {
        if !self.label_density.is_finite()
            || !(0.0..=1.0).contains(&self.label_density)
            || !self.edge_opacity.is_finite()
            || !(0.0..=1.0).contains(&self.edge_opacity)
            || !self.node_size_min.is_finite()
            || !self.node_size_max.is_finite()
            || self.node_size_min <= 0.0
            || self.node_size_min > self.node_size_max
            || self.node_size_max > 64.0
        {
            return Err(CoreError::Request("presentation requires density and opacity in 0–1 and a finite numeric size-by range 0 < min <= max <= 64".into()));
        }
        Ok(())
    }
}
