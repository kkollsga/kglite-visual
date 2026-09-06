//! One bounded mapping drives attached clients and immutable server images.
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use kglite::api::DirGraph;
use serde::Serialize;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::records::{self, NodeHandle, RecordCell, TypedValue};
use crate::shared::SharedViewState;
use crate::view::SlotEntry;
use crate::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub enum MappingValueState {
    Value,
    Null,
    Missing,
    Unavailable,
    Truncated,
    NonNumeric,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct AppearanceNode {
    pub handle: NodeHandle,
    pub color: Option<[f32; 4]>,
    pub radius: Option<f32>,
    pub color_state: Option<MappingValueState>,
    pub size_state: Option<MappingValueState>,
}
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct AppearanceCategory {
    pub value: TypedValue,
    pub color: [f32; 4],
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct AppearanceMapping {
    pub scope: String,
    pub nodes: Vec<AppearanceNode>,
    pub categories: Vec<AppearanceCategory>,
    pub other_categories: u32,
    pub size_min: Option<TypedValue>,
    pub size_max: Option<TypedValue>,
}
impl Default for AppearanceMapping {
    fn default() -> Self {
        Self {
            scope: "loaded".into(),
            nodes: Vec::new(),
            categories: Vec::new(),
            other_categories: 0,
            size_min: None,
            size_max: None,
        }
    }
}

const COLORS: [[f32; 4]; 8] = [
    [0.38, 0.65, 0.98, 0.95],
    [0.98, 0.55, 0.28, 0.95],
    [0.35, 0.82, 0.53, 0.95],
    [0.85, 0.45, 0.85, 0.95],
    [0.95, 0.82, 0.35, 0.95],
    [0.45, 0.85, 0.85, 0.95],
    [0.92, 0.44, 0.52, 0.95],
    [0.65, 0.60, 0.92, 0.95],
];
const UNSET: [f32; 4] = [0.42, 0.47, 0.55, 0.75];

pub(crate) fn compute(
    graph: &DirGraph,
    state: &SharedViewState,
    generation: &str,
    deadline: Option<Instant>,
) -> Result<AppearanceMapping, CoreError> {
    let mut mapping = AppearanceMapping::default();
    if state.appearance.color_field.is_none() && state.appearance.size_field.is_none() {
        return Ok(mapping);
    }
    let _guard = graph.begin_read_pass();
    let ids: Vec<u32> = state
        .view
        .live_entries()
        .filter_map(|(_, entry)| match entry {
            SlotEntry::Node { node_id, .. } => Some(*node_id),
            _ => None,
        })
        .collect();
    mapping.nodes = ids
        .iter()
        .map(|&node_id| AppearanceNode {
            handle: NodeHandle {
                generation: generation.into(),
                node_id,
            },
            color: None,
            radius: None,
            color_state: None,
            size_state: None,
        })
        .collect();
    if let Some(field) = &state.appearance.color_field {
        color(graph, field, state, &mut mapping, deadline)?;
    }
    if let Some(field) = &state.appearance.size_field {
        size(graph, field, state, &mut mapping, deadline)?;
    }
    records::serialized_bytes(&mapping, 2 * 1024 * 1024)?;
    Ok(mapping)
}

fn color(
    graph: &DirGraph,
    field: &crate::subset::FieldRef,
    state: &SharedViewState,
    mapping: &mut AppearanceMapping,
    deadline: Option<Instant>,
) -> Result<(), CoreError> {
    let mut categories = BTreeMap::<String, (TypedValue, u32)>::new();
    let mut tokens = Vec::with_capacity(mapping.nodes.len());
    let mut bytes = 0usize;
    for node in &mut mapping.nodes {
        crate::source_identity::check_deadline(deadline)?;
        let cell = crate::subset::read(graph, &state.derived, node.handle.node_id, field);
        node.color_state = Some(cell_state(&cell));
        node.color = Some(UNSET);
        let token = if let RecordCell::Value { value } = cell {
            let key = serde_json::to_string(&value)
                .map_err(|error| CoreError::Request(error.to_string()))?;
            let hash = crate::source_identity::hex_digest(&Sha256::digest(key.as_bytes()));
            if let Some((_, count)) = categories.get_mut(&key) {
                *count += 1;
            } else {
                bytes += key.len() * 2 + 128;
                if bytes > 1024 * 1024 {
                    return Err(CoreError::Request(
                        "appearance categories exceed 1 MiB".into(),
                    ));
                }
                categories.insert(key, (value, 1));
            }
            Some(hash)
        } else {
            None
        };
        tokens.push(token);
    }
    let colors: HashMap<_, _> = categories
        .keys()
        .enumerate()
        .map(|(index, key)| {
            (
                crate::source_identity::hex_digest(&Sha256::digest(key.as_bytes())),
                COLORS[index % COLORS.len()],
            )
        })
        .collect();
    for (node, token) in mapping.nodes.iter_mut().zip(tokens) {
        if let Some(token) = token {
            node.color = colors.get(&token).copied();
        }
    }
    mapping.other_categories = categories.len().saturating_sub(8) as u32;
    mapping.categories = categories
        .into_values()
        .enumerate()
        .take(8)
        .map(|(index, (value, count))| AppearanceCategory {
            value,
            count,
            color: COLORS[index],
        })
        .collect();
    Ok(())
}

fn size(
    graph: &DirGraph,
    field: &crate::subset::FieldRef,
    state: &SharedViewState,
    mapping: &mut AppearanceMapping,
    deadline: Option<Instant>,
) -> Result<(), CoreError> {
    let mut values = Vec::with_capacity(mapping.nodes.len());
    for node in &mut mapping.nodes {
        crate::source_identity::check_deadline(deadline)?;
        let cell = crate::subset::read(graph, &state.derived, node.handle.node_id, field);
        node.size_state = Some(cell_state(&cell));
        let value = match cell {
            RecordCell::Value { value } if crate::subset::compare(&value, &value).is_some() => {
                Some(value)
            }
            RecordCell::Value { .. } => {
                node.size_state = Some(MappingValueState::NonNumeric);
                None
            }
            _ => None,
        };
        if let Some(value) = &value {
            if mapping.size_min.as_ref().is_none_or(|min| {
                crate::subset::compare(value, min).is_some_and(|order| order.is_lt())
            }) {
                mapping.size_min = Some(value.clone());
            }
            if mapping.size_max.as_ref().is_none_or(|max| {
                crate::subset::compare(value, max).is_some_and(|order| order.is_gt())
            }) {
                mapping.size_max = Some(value.clone());
            }
        }
        values.push(value);
    }
    let (Some(low), Some(high)) = (&mapping.size_min, &mapping.size_max) else {
        return Ok(());
    };
    let min = f64::from(state.presentation.node_size_min);
    let span = f64::from(state.presentation.node_size_max) - min;
    for (node, value) in mapping.nodes.iter_mut().zip(values) {
        node.radius = value
            .and_then(|value| normalize(&value, low, high))
            .map(|t| (min + span * t.powf(0.25)) as f32);
    }
    Ok(())
}

fn cell_state(cell: &RecordCell) -> MappingValueState {
    match cell {
        RecordCell::Value { .. } => MappingValueState::Value,
        RecordCell::Null => MappingValueState::Null,
        RecordCell::Missing => MappingValueState::Missing,
        RecordCell::Unavailable { .. } => MappingValueState::Unavailable,
        RecordCell::Truncated { .. } => MappingValueState::Truncated,
    }
}

fn integer(value: &TypedValue) -> Option<i128> {
    match value {
        TypedValue::Int64(value) | TypedValue::UniqueId(value) => value.parse().ok(),
        _ => None,
    }
}
fn float(value: &TypedValue) -> Option<f64> {
    match value {
        TypedValue::Float64(value) => Some(*value),
        _ => integer(value).map(|value| value as f64),
    }
}
fn normalize(value: &TypedValue, low: &TypedValue, high: &TypedValue) -> Option<f64> {
    if let (Some((vi, vf)), Some((li, lf)), Some((hi, hf))) =
        (parts(value), parts(low), parts(high))
    {
        if let (Some(numerator), Some(denominator)) = (vi.checked_sub(li), hi.checked_sub(li)) {
            let denominator = denominator as f64 + (hf - lf);
            return Some(if denominator == 0.0 {
                0.0
            } else {
                ((numerator as f64 + (vf - lf)) / denominator).clamp(0.0, 1.0)
            });
        }
    }
    let (value, low, high) = (float(value)?, float(low)?, float(high)?);
    let scale = low.abs().max(high.abs()).max(1.0);
    Some(if high == low {
        0.0
    } else {
        ((value / scale - low / scale) / (high / scale - low / scale)).clamp(0.0, 1.0)
    })
}

fn parts(value: &TypedValue) -> Option<(i128, f64)> {
    if let Some(value) = integer(value) {
        return Some((value, 0.0));
    }
    match value {
        TypedValue::Float64(value) if value.is_finite() && value.abs() < i128::MAX as f64 => {
            Some((value.trunc() as i128, value.fract()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_float_integer_domain_never_rounds_distinct_extrema_together() {
        let low = TypedValue::Float64(9_007_199_254_740_992.0);
        let high = TypedValue::Int64("9007199254740993".into());
        assert_eq!(normalize(&low, &low, &high), Some(0.0));
        assert_eq!(normalize(&high, &low, &high), Some(1.0));
        let low = TypedValue::Int64(i64::MIN.to_string());
        let high = TypedValue::Int64(i64::MAX.to_string());
        assert_eq!(normalize(&high, &low, &high), Some(1.0));
        let low = TypedValue::Float64(-1e308);
        let high = TypedValue::Float64(1e308);
        assert_eq!(normalize(&TypedValue::Float64(0.0), &low, &high), Some(0.5));
    }
}
