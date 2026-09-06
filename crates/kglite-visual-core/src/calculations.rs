//! Frozen, namespaced calculations over an acknowledged visible input.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use kglite::api::{DirGraph, EdgeIndex, GraphRead};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::calculation_kernels::{self, CalculationInput};
use crate::records::TypedValue;
use crate::shared::RevisionStamp;
use crate::subset::{FieldRef, FrozenField, FrozenFields, SubsetSnapshot};
use crate::view::View;
use crate::CoreError;

pub const MAX_CALCULATIONS: usize = 8;
pub const MAX_DERIVED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub enum CalculationKind {
    Degree,
    WeakComponents,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CalculateRequest {
    pub kind: CalculationKind,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub calculation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CalculationField {
    pub field: FieldRef,
    pub label: String,
    pub value_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CalculationMeta {
    pub id: String,
    pub kind: CalculationKind,
    pub input_stamp: RevisionStamp,
    pub input_subset_revision: String,
    pub scope: String,
    pub node_count: u32,
    pub edge_count: u32,
    pub fields: Vec<CalculationField>,
    pub status: String,
    pub elapsed_ms: f64,
    pub semantics: String,
}

impl CalculationKind {
    pub(crate) fn columns(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Degree => &[
                ("in", "In degree"),
                ("out", "Out degree"),
                ("total", "Total degree"),
            ],
            Self::WeakComponents => &[
                ("component_id", "Weak component"),
                ("component_size", "Component size"),
            ],
        }
    }
    pub(crate) fn semantics(self) -> &'static str {
        match self {
            Self::Degree => "directed visible relation records; parallel edges count separately; a self-loop adds one in, one out, and two total",
            Self::WeakComponents => "visible relation records with direction ignored; isolates included; component IDs ordered by minimum source node identity",
        }
    }
}

pub(crate) fn capture_input(
    graph: &DirGraph,
    view: &View,
    subset: &SubsetSnapshot,
    deadline: Option<Instant>,
) -> Result<CalculationInput, CoreError> {
    let node_ids: Vec<_> = subset
        .visible_nodes
        .iter()
        .map(|handle| handle.node_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let indices: HashMap<_, _> = node_ids
        .iter()
        .enumerate()
        .map(|(index, &id)| (id, index))
        .collect();
    let retained: BTreeSet<_> = view
        .edges()
        .iter()
        .filter_map(|edge| edge.edge_id)
        .collect();
    let mut edges = Vec::with_capacity(subset.visible_edge_ids.len());
    let mut seen = BTreeSet::new();
    for &id in &subset.visible_edge_ids {
        calculation_kernels::check_deadline(deadline)?;
        if !retained.contains(&id) {
            return Err(refusal("visible calculation relation is not retained"));
        }
        if !seen.insert(id) {
            continue;
        }
        let (source, target) = graph
            .graph
            .edge_endpoints(EdgeIndex::new(id as usize))
            .ok_or_else(|| refusal("calculation relation is absent from source"))?;
        let source = *indices
            .get(&(source.index() as u32))
            .ok_or_else(|| refusal("calculation relation source is outside visible input"))?;
        let target = *indices
            .get(&(target.index() as u32))
            .ok_or_else(|| refusal("calculation relation target is outside visible input"))?;
        edges.push((source, target));
    }
    Ok(CalculationInput { node_ids, edges })
}

pub(crate) fn run(
    input: &CalculationInput,
    kind: CalculationKind,
    id: String,
    stamp: RevisionStamp,
    subset_revision: String,
    deadline: Option<Instant>,
) -> Result<(CalculationMeta, FrozenFields), CoreError> {
    let started = Instant::now();
    let values: Vec<Vec<u32>> = match kind {
        CalculationKind::Degree => calculation_kernels::degree(input, deadline)?
            .into_iter()
            .map(Vec::from)
            .collect(),
        CalculationKind::WeakComponents => calculation_kernels::weak_components(input, deadline)?
            .into_iter()
            .map(Vec::from)
            .collect(),
    };
    let mut fields = FrozenFields::new();
    let mut definitions = Vec::new();
    for (index, &(column, label)) in kind.columns().iter().enumerate() {
        calculation_kernels::check_deadline(deadline)?;
        let values = input
            .node_ids
            .iter()
            .copied()
            .zip(&values)
            .map(|(node, values)| (node, TypedValue::Int64(values[index].to_string())))
            .collect::<BTreeMap<_, _>>();
        fields.insert(
            (id.clone(), column.into()),
            FrozenField {
                input_subset_revision: subset_revision.clone(),
                values,
            },
        );
        definitions.push(CalculationField {
            field: FieldRef::Derived {
                calculation_id: id.clone(),
                column: column.into(),
            },
            label: label.into(),
            value_type: "int64".into(),
        });
    }
    Ok((
        CalculationMeta {
            id,
            kind,
            input_stamp: stamp,
            input_subset_revision: subset_revision,
            scope: "visible".into(),
            node_count: input.node_ids.len() as u32,
            edge_count: input.edges.len() as u32,
            fields: definitions,
            status: "ready".into(),
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            semantics: kind.semantics().into(),
        },
        fields,
    ))
}

fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}

impl crate::Session {
    pub(crate) fn calculate_uncommitted(
        &self,
        request: &CalculateRequest,
    ) -> Result<crate::Response, CoreError> {
        let before = self.state_read().clone();
        let id = calculation_id(
            request,
            &before.calculations,
            self.generation(),
            before.revision,
        )?;
        let deadline = self.config().deadline();
        let input = {
            let _pass = self.graph().begin_read_pass();
            capture_input(self.graph(), &before.view, &before.subset, deadline)?
        };
        let (meta, fields) = run(
            &input,
            request.kind,
            id.clone(),
            before.stamp(self.generation()),
            before.subset_revision.to_string(),
            deadline,
        )?;
        let mut calculations = before.calculations;
        calculations.retain(|calculation| calculation.id != id);
        calculations.push(meta);
        let mut derived = before.derived;
        derived.retain(|(calculation, _), _| *calculation != id);
        derived.extend(fields);
        validate_results(&calculations, &derived)?;
        calculation_kernels::check_deadline(deadline)?;
        {
            let mut state = self.state_write();
            state.calculations = calculations;
            state.derived = derived;
        }
        Ok(crate::Response::Shared(Box::new(self.snapshot_direct())))
    }
}

fn calculation_id(
    request: &CalculateRequest,
    existing: &[CalculationMeta],
    generation: &str,
    revision: u64,
) -> Result<String, CoreError> {
    if let Some(id) = &request.calculation_id {
        let Some(meta) = existing.iter().find(|meta| &meta.id == id) else {
            return Err(refusal(
                "calculation_id must name an existing frozen calculation to recompute",
            ));
        };
        if meta.kind != request.kind {
            return Err(refusal(
                "recompute must preserve the existing calculation kind",
            ));
        }
        return Ok(id.clone());
    }
    if existing.len() >= MAX_CALCULATIONS {
        return Err(refusal(
            "at most eight calculations are allowed; recompute an existing calculation",
        ));
    }
    let revision = revision
        .checked_add(1)
        .ok_or_else(|| refusal("view revision exhausted"))?;
    Ok(format!("calc-{generation}-{revision}"))
}

pub(crate) fn validate_results(
    metadata: &[CalculationMeta],
    fields: &FrozenFields,
) -> Result<(), CoreError> {
    if metadata.len() > MAX_CALCULATIONS {
        return Err(refusal("at most eight frozen calculations are allowed"));
    }
    let mut ids = BTreeSet::new();
    let mut expected_fields = BTreeSet::new();
    for meta in metadata {
        validate_meta(meta)?;
        if !ids.insert(&meta.id) {
            return Err(refusal("calculation identities must be unique"));
        }
        let mut input_ids = None;
        for &(column, _) in meta.kind.columns() {
            let key = (meta.id.clone(), column.to_string());
            let field = fields
                .get(&key)
                .ok_or_else(|| refusal("calculation field values are missing"))?;
            expected_fields.insert(key);
            if field.input_subset_revision != meta.input_subset_revision
                || field.values.len() != meta.node_count as usize
            {
                return Err(refusal(
                    "calculation field input metadata does not match its frozen values",
                ));
            }
            let members: Vec<_> = field.values.keys().copied().collect();
            if input_ids.as_ref().is_some_and(|input| *input != members) {
                return Err(refusal(
                    "calculation columns must share exactly the same captured node identities",
                ));
            }
            input_ids = Some(members);
            for value in field.values.values() {
                if !matches!(value, TypedValue::Int64(value) if value.parse::<u32>().is_ok_and(|value| value <= 40_000))
                {
                    return Err(refusal(
                        "degree and weak-component values must be bounded nonnegative integers",
                    ));
                }
            }
        }
        validate_algorithm_values(meta, fields)?;
    }
    if fields.keys().any(|key| !expected_fields.contains(key)) {
        return Err(refusal(
            "derived values have no matching calculation metadata",
        ));
    }
    let serializable: Vec<_> = fields
        .iter()
        .map(|(key, field)| (key, &field.input_subset_revision, &field.values))
        .collect();
    crate::records::serialized_bytes(&(metadata, serializable), MAX_DERIVED_BYTES)?;
    Ok(())
}

fn validate_algorithm_values(
    meta: &CalculationMeta,
    fields: &FrozenFields,
) -> Result<(), CoreError> {
    let values = |column: &str| -> Vec<u32> {
        fields[&(meta.id.clone(), column.into())]
            .values
            .values()
            .map(|value| {
                let TypedValue::Int64(value) = value else {
                    unreachable!("validated scalar")
                };
                value.parse::<u32>().expect("validated integer")
            })
            .collect()
    };
    match meta.kind {
        CalculationKind::Degree => {
            let incoming = values("in");
            let outgoing = values("out");
            let total = values("total");
            if incoming.iter().sum::<u32>() != meta.edge_count
                || outgoing.iter().sum::<u32>() != meta.edge_count
                || incoming
                    .iter()
                    .zip(outgoing)
                    .zip(total)
                    .any(|((&a, b), total)| a + b != total)
            {
                return Err(refusal(
                    "frozen degree values do not match their captured relation count",
                ));
            }
        }
        CalculationKind::WeakComponents => {
            let labels = values("component_id");
            let sizes = values("component_size");
            let mut counts = BTreeMap::<u32, u32>::new();
            for &label in &labels {
                *counts.entry(label).or_default() += 1;
            }
            if (meta.node_count == 0 && meta.edge_count != 0)
                || meta.edge_count < meta.node_count - counts.len() as u32
            {
                return Err(refusal(
                    "frozen component relation count cannot connect its reported members",
                ));
            }
            // Restoring remaps member identities; frozen labels retain their
            // original input ordering rather than following the new handles.
            if counts.keys().copied().ne(1..=counts.len() as u32) {
                return Err(refusal(
                    "frozen component labels must be contiguous positive integers",
                ));
            }
            if labels
                .iter()
                .zip(sizes)
                .any(|(label, size)| counts[label] != size)
            {
                return Err(refusal(
                    "frozen component sizes do not match their captured members",
                ));
            }
        }
    }
    Ok(())
}

fn validate_meta(meta: &CalculationMeta) -> Result<(), CoreError> {
    if meta.id.is_empty()
        || meta.id.len() > 128
        || meta.input_stamp.generation.is_empty()
        || meta.input_stamp.generation.len() > 128
        || meta.input_stamp.revision.parse::<u64>().is_err()
        || meta.input_subset_revision.parse::<u64>().is_err()
        || meta.scope != "visible"
        || meta.status != "ready"
        || !meta.elapsed_ms.is_finite()
        || meta.elapsed_ms < 0.0
        || meta.node_count as usize > crate::records::MAX_LOADED_NODES
        || meta.edge_count as usize > crate::records::MAX_LOADED_EDGES
        || meta.semantics != meta.kind.semantics()
        || meta.fields.len() != meta.kind.columns().len()
    {
        return Err(refusal("calculation metadata is invalid or unsupported"));
    }
    for (field, &(column, label)) in meta.fields.iter().zip(meta.kind.columns()) {
        if field.value_type != "int64"
            || field.label != label
            || field.field
                != (FieldRef::Derived {
                    calculation_id: meta.id.clone(),
                    column: column.into(),
                })
        {
            return Err(refusal(
                "calculation field definition does not match its algorithm",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frozen(kind: CalculationKind) -> (CalculationMeta, FrozenFields) {
        run(
            &CalculationInput {
                node_ids: vec![3, 8, 11],
                edges: vec![(0, 1), (0, 1), (1, 1)],
            },
            kind,
            "calc-one".into(),
            RevisionStamp {
                generation: "input-generation".into(),
                revision: "7".into(),
            },
            "4".into(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn result_validation_keeps_input_stamp_and_rejects_corrupt_degrees() {
        let (meta, mut fields) = frozen(CalculationKind::Degree);
        assert_eq!(meta.input_stamp.generation, "input-generation");
        assert_eq!(meta.input_stamp.revision, "7");
        assert_eq!(meta.input_subset_revision, "4");
        validate_results(std::slice::from_ref(&meta), &fields).unwrap();
        fields
            .get_mut(&(meta.id.clone(), "total".into()))
            .unwrap()
            .values
            .insert(3, TypedValue::Int64("3".into()));
        assert!(validate_results(&[meta], &fields).is_err());
    }

    #[test]
    fn component_metadata_rejects_missing_members_and_wrong_sizes() {
        let (meta, fields) = frozen(CalculationKind::WeakComponents);
        validate_results(std::slice::from_ref(&meta), &fields).unwrap();
        let mut missing = fields.clone();
        missing
            .get_mut(&(meta.id.clone(), "component_size".into()))
            .unwrap()
            .values
            .remove(&11);
        assert!(validate_results(std::slice::from_ref(&meta), &missing).is_err());
        let mut wrong = fields;
        wrong
            .get_mut(&(meta.id.clone(), "component_size".into()))
            .unwrap()
            .values
            .insert(3, TypedValue::Int64("3".into()));
        assert!(validate_results(&[meta], &wrong).is_err());
    }

    #[test]
    fn namespaced_definitions_and_algorithm_semantics_are_strict() {
        let (mut meta, fields) = frozen(CalculationKind::Degree);
        meta.fields[0].field = FieldRef::Property { name: "in".into() };
        assert!(validate_results(&[meta], &fields).is_err());
        let (mut meta, fields) = frozen(CalculationKind::Degree);
        meta.semantics = "undirected".into();
        assert!(validate_results(&[meta], &fields).is_err());
    }
}
