//! Authoritative predicates over bounded loaded instance nodes and relation records.
use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use kglite::api::DirGraph;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::records::{self, NodeHandle, RecordCell, TypedValue};
use crate::view::{SlotEntry, View};
use crate::CoreError;

pub const MAX_PREDICATES: usize = 32;
pub const MAX_PREDICATE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FieldRef {
    Property {
        name: String,
    },
    Derived {
        calculation_id: String,
        column: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SubsetRequest {
    pub predicates: Vec<SubsetFilter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SubsetFilter {
    pub id: String,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    pub predicate: SubsetPredicate,
}
fn enabled_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SubsetPredicate {
    Type {
        node_types: Vec<String>,
    },
    Category {
        field: FieldRef,
        values: Vec<TypedValue>,
        #[serde(default)]
        include_null: bool,
        #[serde(default)]
        include_missing: bool,
    },
    NumericRange {
        field: FieldRef,
        min: Option<TypedValue>,
        max: Option<TypedValue>,
        #[serde(default)]
        include_null: bool,
        #[serde(default)]
        include_missing: bool,
    },
    Missing {
        field: FieldRef,
        #[serde(default)]
        include_null: bool,
        #[serde(default)]
        include_missing: bool,
    },
    Relation {
        names: Vec<String>,
    },
    HideIsolated,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SubsetCounts {
    pub loaded_nodes: u32,
    pub loaded_edges: u32,
    pub visible_nodes: u32,
    pub visible_edges: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SubsetSnapshot {
    pub predicates: Vec<SubsetFilter>,
    pub visible_nodes: Vec<NodeHandle>,
    pub visible_edge_ids: Vec<u32>,
    pub counts: SubsetCounts,
    pub distributions: Vec<PredicateDistribution>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CategoryCount {
    pub value: TypedValue,
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct PredicateDistribution {
    pub id: String,
    pub scope: String,
    pub input_nodes: u32,
    pub matching_nodes: u32,
    pub null: u32,
    pub missing: u32,
    pub unavailable: u32,
    pub min: Option<TypedValue>,
    pub max: Option<TypedValue>,
    pub categories: Vec<CategoryCount>,
    pub other_values: u32,
}

/// Derived values stay frozen until an explicit calculation replaces this field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrozenField {
    pub input_subset_revision: String,
    pub values: BTreeMap<u32, TypedValue>,
}
pub type FrozenFields = BTreeMap<(String, String), FrozenField>;

pub(crate) fn validate(filters: &[SubsetFilter]) -> Result<(), CoreError> {
    if filters.len() > MAX_PREDICATES {
        return Err(refusal("at most 32 predicates are allowed"));
    }
    let mut ids = HashSet::new();
    for filter in filters {
        if filter.id.is_empty() || filter.id.len() > 128 || !ids.insert(&filter.id) {
            return Err(refusal(
                "predicate ids must be unique and contain 1–128 bytes",
            ));
        }
        if let Some(field) = field(&filter.predicate) {
            let names = match field {
                FieldRef::Property { name } => vec![name],
                FieldRef::Derived {
                    calculation_id,
                    column,
                } => vec![calculation_id, column],
            };
            if names.iter().any(|name| name.is_empty() || name.len() > 256) {
                return Err(refusal("field names must contain 1–256 bytes"));
            }
        }
        match &filter.predicate {
            SubsetPredicate::Type { node_types }
            | SubsetPredicate::Relation { names: node_types } => {
                if node_types.len() > 256 || node_types.iter().any(|name| name.len() > 256) {
                    return Err(refusal("type selections exceed their limit"));
                }
            }
            SubsetPredicate::Category { values, .. } => {
                if values.len() > 128
                    || values.iter().any(|value| {
                        matches!(
                            value,
                            TypedValue::List(_) | TypedValue::Map(_) | TypedValue::Null
                        )
                    })
                {
                    return Err(refusal(
                        "category predicates accept at most 128 non-null scalar values",
                    ));
                }
            }
            SubsetPredicate::NumericRange { min, max, .. } => {
                if min
                    .iter()
                    .chain(max.iter())
                    .any(|value| numeric(value).is_none())
                {
                    return Err(refusal(
                        "numeric bounds must be finite numbers or exact integer strings",
                    ));
                }
                if min.as_ref().zip(max.as_ref()).is_some_and(|(min, max)| {
                    compare(min, max) == Some(std::cmp::Ordering::Greater)
                }) {
                    return Err(refusal("numeric minimum exceeds maximum"));
                }
            }
            _ => {}
        }
    }
    records::serialized_bytes(&filters, MAX_PREDICATE_BYTES)?;
    Ok(())
}
fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}
fn field(predicate: &SubsetPredicate) -> Option<&FieldRef> {
    match predicate {
        SubsetPredicate::Category { field, .. }
        | SubsetPredicate::NumericRange { field, .. }
        | SubsetPredicate::Missing { field, .. } => Some(field),
        _ => None,
    }
}
fn read(graph: &DirGraph, derived: &FrozenFields, id: u32, field: &FieldRef) -> RecordCell {
    match field {
        FieldRef::Property { name } => records::read_cell(graph, id, name),
        FieldRef::Derived {
            calculation_id,
            column,
        } => match derived.get(&(calculation_id.clone(), column.clone())) {
            Some(result) => result
                .values
                .get(&id)
                .cloned()
                .map(|value| {
                    if value == TypedValue::Null {
                        RecordCell::Null
                    } else {
                        RecordCell::Value { value }
                    }
                })
                .unwrap_or(RecordCell::Missing),
            None => RecordCell::Unavailable {
                reason: "calculation is not available".into(),
            },
        },
    }
}

pub(crate) fn evaluate(
    graph: &DirGraph,
    generation: &str,
    view: &View,
    filters: &[SubsetFilter],
    derived: &FrozenFields,
    deadline: Option<Instant>,
) -> Result<SubsetSnapshot, CoreError> {
    validate(filters)?;
    let _guard = graph.begin_read_pass();
    let nodes: Vec<_> = view
        .live_entries()
        .filter_map(|(slot, entry)| match entry {
            SlotEntry::Node {
                node_id, node_type, ..
            } => Some((slot, *node_id, node_type.as_str())),
            _ => None,
        })
        .collect();
    let mut truth = Vec::with_capacity(nodes.len());
    for (_, id, node_type) in &nodes {
        check_deadline(deadline)?;
        truth.push(
            filters
                .iter()
                .map(|filter| {
                    !filter.enabled
                        || node_matches(graph, derived, *id, node_type, &filter.predicate)
                })
                .collect::<Vec<_>>(),
        );
    }
    let (visible, edge_ids) = project(view, &nodes, &truth, filters, None);
    let mut distributions = Vec::new();
    for (index, filter) in filters
        .iter()
        .enumerate()
        .filter(|(_, filter)| filter.enabled)
    {
        let Some(field) = field(&filter.predicate) else {
            continue;
        };
        let (input, _) = project(view, &nodes, &truth, filters, Some(index));
        let mut distribution = PredicateDistribution {
            id: filter.id.clone(),
            scope: "loaded instances after every other enabled predicate; exact counts".into(),
            ..Default::default()
        };
        for (row, (_, id, _)) in nodes
            .iter()
            .enumerate()
            .filter(|(_, (slot, _, _))| input.contains(slot))
        {
            check_deadline(deadline)?;
            distribution.input_nodes += 1;
            distribution.matching_nodes += u32::from(truth[row][index]);
            observe(&mut distribution, read(graph, derived, *id, field));
        }
        distributions.push(distribution);
    }
    let visible_nodes = nodes
        .iter()
        .filter(|(slot, _, _)| visible.contains(slot))
        .map(|(_, id, _)| NodeHandle {
            generation: generation.into(),
            node_id: *id,
        })
        .collect::<Vec<_>>();
    Ok(SubsetSnapshot {
        predicates: filters.to_vec(),
        counts: SubsetCounts {
            loaded_nodes: nodes.len() as u32,
            loaded_edges: view.edges().iter().filter(|edge| !edge.meta).count() as u32,
            visible_nodes: visible_nodes.len() as u32,
            visible_edges: edge_ids.len() as u32,
        },
        visible_nodes,
        visible_edge_ids: edge_ids,
        distributions,
    })
}
fn check_deadline(deadline: Option<Instant>) -> Result<(), CoreError> {
    if deadline.is_some_and(|limit| Instant::now() > limit) {
        return Err(refusal("subset evaluation exceeded its deadline"));
    }
    Ok(())
}
fn node_matches(
    graph: &DirGraph,
    derived: &FrozenFields,
    id: u32,
    node_type: &str,
    predicate: &SubsetPredicate,
) -> bool {
    let Some(field) = field(predicate) else {
        return match predicate {
            SubsetPredicate::Type { node_types } => node_types.iter().any(|name| name == node_type),
            _ => true,
        };
    };
    let cell = read(graph, derived, id, field);
    match predicate {
        SubsetPredicate::Category {
            values,
            include_null,
            include_missing,
            ..
        } => match cell {
            RecordCell::Value { value } => values.contains(&value),
            RecordCell::Null => *include_null,
            RecordCell::Missing => *include_missing,
            _ => false,
        },
        SubsetPredicate::NumericRange {
            min,
            max,
            include_null,
            include_missing,
            ..
        } => match cell {
            RecordCell::Value { value } => {
                numeric(&value).is_some()
                    && min
                        .as_ref()
                        .is_none_or(|min| compare(&value, min).is_some_and(|order| !order.is_lt()))
                    && max
                        .as_ref()
                        .is_none_or(|max| compare(&value, max).is_some_and(|order| !order.is_gt()))
            }
            RecordCell::Null => *include_null,
            RecordCell::Missing => *include_missing,
            _ => false,
        },
        SubsetPredicate::Missing {
            include_null,
            include_missing,
            ..
        } => {
            matches!(cell, RecordCell::Null) && *include_null
                || matches!(cell, RecordCell::Missing) && *include_missing
        }
        _ => true,
    }
}
fn project(
    view: &View,
    nodes: &[(u32, u32, &str)],
    truth: &[Vec<bool>],
    filters: &[SubsetFilter],
    skip: Option<usize>,
) -> (HashSet<u32>, Vec<u32>) {
    let mut visible: HashSet<u32> = nodes
        .iter()
        .zip(truth)
        .filter(|(_, row)| {
            row.iter()
                .enumerate()
                .all(|(i, keep)| Some(i) == skip || *keep)
        })
        .map(|((slot, _, _), _)| *slot)
        .collect();
    let retained: Vec<_> = view
        .edges()
        .iter()
        .filter(|edge| {
            !edge.meta
                && visible.contains(&edge.source_slot)
                && visible.contains(&edge.target_slot)
                && filters.iter().enumerate().all(|(i, filter)| {
                    !filter.enabled
                        || Some(i) == skip
                        || match &filter.predicate {
                            SubsetPredicate::Relation { names } => names.contains(&edge.name),
                            _ => true,
                        }
                })
        })
        .collect();
    if filters.iter().enumerate().any(|(i, filter)| {
        filter.enabled
            && Some(i) != skip
            && matches!(filter.predicate, SubsetPredicate::HideIsolated)
    }) {
        let incident: HashSet<_> = retained
            .iter()
            .flat_map(|edge| [edge.source_slot, edge.target_slot])
            .collect();
        visible.retain(|slot| incident.contains(slot));
    }
    (
        visible,
        retained.iter().filter_map(|edge| edge.edge_id).collect(),
    )
}
fn observe(out: &mut PredicateDistribution, cell: RecordCell) {
    match cell {
        RecordCell::Null => out.null += 1,
        RecordCell::Missing => out.missing += 1,
        RecordCell::Unavailable { .. } | RecordCell::Truncated { .. } => out.unavailable += 1,
        RecordCell::Value { value } => {
            if numeric(&value).is_some() {
                if out
                    .min
                    .as_ref()
                    .is_none_or(|min| compare(&value, min).is_some_and(|order| order.is_lt()))
                {
                    out.min = Some(value.clone());
                }
                if out
                    .max
                    .as_ref()
                    .is_none_or(|max| compare(&value, max).is_some_and(|order| order.is_gt()))
                {
                    out.max = Some(value.clone());
                }
            }
            if let Some(category) = out
                .categories
                .iter_mut()
                .find(|category| category.value == value)
            {
                category.count += 1;
            } else if out.categories.len() < 64 {
                out.categories.push(CategoryCount { value, count: 1 });
            } else {
                out.other_values += 1;
            }
        }
    }
}
#[derive(Clone, Copy)]
enum Number {
    Integer(i64),
    Float(f64),
}
fn numeric(value: &TypedValue) -> Option<Number> {
    match value {
        TypedValue::Int64(value) => value.parse().ok().map(Number::Integer),
        TypedValue::UniqueId(value) => value
            .parse::<u32>()
            .ok()
            .map(|value| Number::Integer(value.into())),
        TypedValue::Float64(value) if value.is_finite() => Some(Number::Float(*value)),
        _ => None,
    }
}
pub(crate) fn compare(a: &TypedValue, b: &TypedValue) -> Option<std::cmp::Ordering> {
    Some(match (numeric(a)?, numeric(b)?) {
        (Number::Integer(a), Number::Integer(b)) => a.cmp(&b),
        (Number::Float(a), Number::Float(b)) => a.partial_cmp(&b)?,
        (Number::Integer(a), Number::Float(b)) => int_float(a, b),
        (Number::Float(a), Number::Integer(b)) => int_float(b, a).reverse(),
    })
}
fn int_float(a: i64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if b >= 9_223_372_036_854_775_808.0 {
        return Ordering::Less;
    }
    if b < -9_223_372_036_854_775_808.0 {
        return Ordering::Greater;
    }
    a.cmp(&(b as i64)).then_with(|| {
        if b.fract() > 0.0 {
            Ordering::Less
        } else if b.fract() < 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kglite::api::session::{execute_mut, ExecuteOptions};

    #[test]
    fn integer_float_comparison_never_rounds_the_integer() {
        use std::cmp::Ordering::*;
        assert_eq!(
            int_float(9_007_199_254_740_993, 9_007_199_254_740_992.0),
            Greater
        );
        assert_eq!(int_float(i64::MAX, 9_223_372_036_854_775_808.0), Less);
        assert_eq!(int_float(i64::MIN, -9_223_372_036_854_775_808.0), Equal);
        assert_eq!(int_float(-1, -1.5), Greater);
        assert_eq!(int_float(1, 1.5), Less);
        assert!(numeric(&TypedValue::Float64(f64::INFINITY)).is_none());
        assert!(numeric(&TypedValue::Int64("9223372036854775808".into())).is_none());
    }
    #[test]
    fn derived_fields_are_namespaced_and_frozen_missing_is_not_null() {
        let mut graph = DirGraph::new();
        execute_mut(
            &mut graph,
            "CREATE (:P {id:1,degree:9})",
            &ExecuteOptions::eager(&Default::default()),
        )
        .unwrap();
        let mut view = View::new();
        view.intern_node(0, "P", "");
        let derived = BTreeMap::from([(
            ("calc".into(), "degree".into()),
            FrozenField {
                input_subset_revision: "4".into(),
                values: BTreeMap::from([(0, TypedValue::Int64("2".into()))]),
            },
        )]);
        let source = FieldRef::Property {
            name: "degree".into(),
        };
        let computed = FieldRef::Derived {
            calculation_id: "calc".into(),
            column: "degree".into(),
        };
        assert_eq!(
            read(&graph, &derived, 0, &source),
            RecordCell::Value {
                value: TypedValue::Int64("9".into())
            }
        );
        assert_eq!(
            read(&graph, &derived, 0, &computed),
            RecordCell::Value {
                value: TypedValue::Int64("2".into())
            }
        );
        let filters = vec![SubsetFilter {
            id: "frozen".into(),
            enabled: true,
            predicate: SubsetPredicate::NumericRange {
                field: computed.clone(),
                min: None,
                max: Some(TypedValue::Int64("3".into())),
                include_null: false,
                include_missing: false,
            },
        }];
        assert_eq!(
            evaluate(&graph, "g", &view, &filters, &derived, None)
                .unwrap()
                .counts
                .visible_nodes,
            1
        );
        assert_eq!(derived.values().next().unwrap().input_subset_revision, "4");
        assert_eq!(read(&graph, &derived, 123, &computed), RecordCell::Missing);
        assert!(matches!(
            read(&graph, &FrozenFields::new(), 0, &computed),
            RecordCell::Unavailable { .. }
        ));
    }
    #[test]
    fn disabled_predicate_preserves_nodes_and_expired_work_refuses() {
        let graph = DirGraph::new();
        let mut view = View::new();
        view.intern_node(0, "P", "");
        let filters = vec![SubsetFilter {
            id: "disabled".into(),
            enabled: false,
            predicate: SubsetPredicate::Type { node_types: vec![] },
        }];
        assert_eq!(
            evaluate(&graph, "g", &view, &filters, &FrozenFields::new(), None)
                .unwrap()
                .counts
                .visible_nodes,
            1
        );
        assert!(evaluate(
            &graph,
            "g",
            &view,
            &filters,
            &FrozenFields::new(),
            Some(Instant::now() - std::time::Duration::from_secs(1))
        )
        .is_err());
    }
}
