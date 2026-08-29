//! Per-type property statistics, and one node's stored properties (plan D12).
//!
//! **Sampled statistics are marked, and the mark survives to the screen.**
//! kglite scans a type exhaustively up to
//! `EXACT_PROPERTY_STATS_MAX_NODES` and samples above it, setting `approx` on
//! every stat it could not enumerate. Presenting a sampled distinct-count as an
//! exact one is the failure Phase 0 named: a color-by menu offering "3 distinct
//! values" for a property with 40 000 is not a smaller truth, it is a wrong
//! one. The flag rides all the way to the UI text.

use kglite::api::introspection::{compute_property_stats, EXACT_PROPERTY_STATS_MAX_NODES};
use kglite::api::{DirGraph, NodeIndex, CANONICAL_NODE_COLUMNS};
use serde::Serialize;
use ts_rs::TS;

use crate::error::CoreError;
use crate::protocol::PROTOCOL_VERSION;
use crate::values::{value_to_display, value_to_json};

/// Distinct values kglite enumerates before it gives up and reports a lower
/// bound. Also the cardinality ceiling for calling a property *categorical*:
/// a color-by menu with more entries than this is a legend nobody reads.
pub const MAX_DISTINCT_VALUES: usize = 32;

/// Nodes sampled when a type is above the exact-scan ceiling.
///
/// kglite's own sampler takes every `total/n`-th node, so this is a stride, not
/// a random draw — deterministic, which is what a committed baseline needs, and
/// blind to any ordering correlation in the type index, which is what it costs.
pub const SAMPLE_SIZE: usize = 20_000;

/// What an appearance channel can be driven by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum AppearanceRole {
    /// Numeric and non-constant: usable for size-by and for a continuous ramp.
    Numeric,
    /// Few enough distinct values to give each one a colour.
    Categorical,
    /// Neither. Listed anyway, with the reason — a property missing from the
    /// menu with no explanation reads as a bug in the menu.
    Unsuitable,
}

/// One property's statistics.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct PropertyStat {
    pub name: String,
    /// kglite's own type string for the column.
    pub value_type: String,
    /// Nodes where this property is set.
    pub non_null: u32,
    /// Distinct values. A **lower bound** when `approx` is true.
    pub unique: u32,
    /// The distinct values, when there were few enough to enumerate.
    #[ts(type = "unknown[]")]
    pub values: Vec<serde_json::Value>,
    /// One example, when there were not.
    #[ts(type = "unknown | null")]
    pub sample: Option<serde_json::Value>,
    /// True when `unique` and `values` are not exhaustive — the population was
    /// sampled, or the distinct-value set hit its cap. The UI must say
    /// "approximate"; presenting this as exact is the failure this field exists
    /// to prevent.
    pub approx: bool,
    pub role: AppearanceRole,
}

/// Per-type property statistics for the appearance menus.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct PropertyStatsResponse {
    pub protocol_version: u32,
    pub node_type: String,
    pub node_count: u32,
    /// True when kglite sampled rather than scanned — i.e. `node_count` is
    /// above its exact ceiling. Carried at the response level as well as per
    /// stat so a UI can label the whole panel once.
    pub sampled: bool,
    /// The ceiling that decision was made against, so the label can say why.
    pub exact_scan_ceiling: u32,
    pub properties: Vec<PropertyStat>,
    /// Properties suitable for size-by, largest `non_null` first.
    pub numeric_candidates: Vec<String>,
    /// Properties suitable for color-by.
    pub categorical_candidates: Vec<String>,
}

/// Compute the statistics for one node type.
pub fn property_stats(
    graph: &DirGraph,
    node_type: &str,
) -> Result<PropertyStatsResponse, CoreError> {
    let node_count = graph
        .type_indices
        .get(node_type)
        .map(|nodes| nodes.len())
        .ok_or_else(|| {
            CoreError::Request(format!("no node type named '{node_type}' in this graph"))
        })?;

    let sampled = node_count > EXACT_PROPERTY_STATS_MAX_NODES;
    let sample_size = sampled.then_some(SAMPLE_SIZE);
    let raw = compute_property_stats(graph, node_type, MAX_DISTINCT_VALUES, sample_size)
        .map_err(CoreError::Request)?;

    let mut properties: Vec<PropertyStat> = raw
        .into_iter()
        .map(|stat| {
            let role = classify(
                &stat.property_name,
                &stat.type_string,
                stat.unique,
                stat.non_null,
                stat.approx,
            );
            PropertyStat {
                name: stat.property_name,
                value_type: stat.type_string,
                non_null: stat.non_null as u32,
                unique: stat.unique as u32,
                values: stat
                    .values
                    .unwrap_or_default()
                    .iter()
                    .map(value_to_json)
                    .collect(),
                sample: stat.sample.as_ref().map(value_to_json),
                approx: stat.approx,
                role,
            }
        })
        .collect();

    // Most-populated first; name breaks ties. The menus are drawn from this
    // order and an e2e assertion reads it, so an order derived from a hash map
    // would make both flap.
    properties.sort_by(|a, b| {
        b.non_null
            .cmp(&a.non_null)
            .then_with(|| a.name.cmp(&b.name))
    });

    let numeric_candidates = properties
        .iter()
        .filter(|p| p.role == AppearanceRole::Numeric)
        .map(|p| p.name.clone())
        .collect();
    let categorical_candidates = properties
        .iter()
        .filter(|p| p.role == AppearanceRole::Categorical)
        .map(|p| p.name.clone())
        .collect();

    Ok(PropertyStatsResponse {
        protocol_version: PROTOCOL_VERSION,
        node_type: node_type.to_string(),
        node_count: node_count as u32,
        sampled,
        exact_scan_ceiling: EXACT_PROPERTY_STATS_MAX_NODES as u32,
        properties,
        numeric_candidates,
        categorical_candidates,
    })
}

/// Decide what appearance channel a property can drive.
///
/// Pure, and separated from the kglite call, so the `approx` path has a unit
/// test that needs no 200 000-node graph: a synthetic stat is enough to pin
/// that a sampled distinct-count never becomes a categorical palette. Colouring
/// by a property whose value *set* is a lower bound gives some nodes no colour
/// at all, silently.
pub(crate) fn classify(
    name: &str,
    value_type: &str,
    unique: usize,
    non_null: usize,
    approx: bool,
) -> AppearanceRole {
    // kglite's canonical identity columns. Numeric or short-stringed they may
    // be, but "size the nodes by their id" is a channel that encodes nothing,
    // and `type` is constant within a type by construction. Named from
    // `CANONICAL_NODE_COLUMNS` rather than guessed.
    if CANONICAL_NODE_COLUMNS.contains(&name) {
        return AppearanceRole::Unsuitable;
    }
    if non_null == 0 || unique <= 1 {
        // A property every node shares, or none has, draws one flat colour.
        return AppearanceRole::Unsuitable;
    }
    // The exact strings kglite emits, observed from `compute_property_stats`
    // against the fixture rather than inferred from the `Value` variant names:
    // the columnar types come back capitalised and the canonical ones do not.
    let numeric = matches!(value_type, "Int64" | "Float64" | "uniqueid");
    if numeric {
        return AppearanceRole::Numeric;
    }
    if !approx && unique <= MAX_DISTINCT_VALUES {
        return AppearanceRole::Categorical;
    }
    AppearanceRole::Unsuitable
}

/// One node's stored properties.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct NodeDetail {
    pub protocol_version: u32,
    pub slot: u32,
    pub node_id: u32,
    pub node_type: String,
    pub title: String,
    /// `(key, value)` pairs, sorted by key. Sorted rather than in storage
    /// order: a properties panel that reorders itself between two nodes of the
    /// same type is unreadable.
    #[ts(type = "[string, unknown][]")]
    pub properties: Vec<(String, serde_json::Value)>,
}

/// Read one node's properties.
pub fn node_detail(graph: &DirGraph, slot: u32, node_id: u32) -> Result<NodeDetail, CoreError> {
    // The arena guard the disk backend needs for materialised reads; a no-op
    // on memory and mapped graphs.
    let _guard = graph.begin_read_pass();
    let view = graph
        .node_view(NodeIndex::new(node_id as usize))
        .ok_or_else(|| CoreError::Request(format!("node {node_id} is not in this graph")))?;

    let mut properties: Vec<(String, serde_json::Value)> = view
        .property_pairs_named(&graph.interner)
        .into_iter()
        .map(|(key, value)| (key, value_to_json(&value)))
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(NodeDetail {
        protocol_version: PROTOCOL_VERSION,
        slot,
        node_id,
        node_type: view.node_type_str(&graph.interner).to_string(),
        title: value_to_display(&view.title()),
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sampled_string_property_never_becomes_a_colour_palette() {
        // The synthetic-stats path D12 requires a test for: 12 distinct values
        // is inside the categorical ceiling, but the count is a LOWER BOUND, so
        // colouring by it would leave the values nobody sampled uncoloured.
        assert_eq!(
            classify("bio", "String", 12, 500_000, true),
            AppearanceRole::Unsuitable
        );
        assert_eq!(
            classify("bio", "String", 12, 500, false),
            AppearanceRole::Categorical,
            "the same shape scanned exhaustively is fine"
        );
    }

    #[test]
    fn a_sampled_numeric_property_is_still_usable_for_size() {
        // Size-by reads each node's own value, not the distinct set, so
        // sampling the *statistics* does not make the channel wrong — only the
        // range hint is approximate, and that is what `approx` labels.
        assert_eq!(
            classify("score", "Float64", 900_000, 1_000_000, true),
            AppearanceRole::Numeric
        );
    }

    #[test]
    fn constant_and_empty_properties_are_not_offered() {
        assert_eq!(
            classify("flag", "Int64", 1, 100, false),
            AppearanceRole::Unsuitable
        );
        assert_eq!(
            classify("bio", "String", 0, 0, false),
            AppearanceRole::Unsuitable
        );
    }

    #[test]
    fn kglites_identity_columns_are_never_appearance_channels() {
        // `id` is numeric and `type` is a string with one value per type, so
        // both would otherwise pass the shape tests and appear in a menu where
        // they encode nothing.
        for name in CANONICAL_NODE_COLUMNS {
            assert_eq!(
                classify(name, "Int64", 60, 60, false),
                AppearanceRole::Unsuitable,
                "{name} must not be offered as an appearance channel"
            );
        }
    }

    #[test]
    fn a_high_cardinality_string_is_not_categorical() {
        assert_eq!(
            classify("bio", "String", MAX_DISTINCT_VALUES + 1, 1_000, false),
            AppearanceRole::Unsuitable
        );
        assert_eq!(
            classify("bio", "String", MAX_DISTINCT_VALUES, 1_000, false),
            AppearanceRole::Categorical
        );
    }
}
