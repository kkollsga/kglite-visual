//! Bounded record reads use engine identities, independently of renderer slots.

use std::collections::BTreeSet;

use kglite::api::{DirGraph, InternedKey, NodeIndex, Value};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{BoundInfo, CoreError};

pub const MAX_RECORD_HANDLES: usize = 5_000;
pub const MAX_RECORD_FIELDS: usize = 32;
pub const MAX_RECORD_ROWS: usize = 500;
pub const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CELL_BYTES: usize = 4096;
pub const MAX_COLLECTION_ITEMS: usize = 64;
pub const MAX_VALUE_DEPTH: usize = 8;
pub const MAX_LOADED_NODES: usize = 5_000;
pub const MAX_LOADED_EDGES: usize = 20_000;
pub const MAX_LOADED_BYTES: usize = 2 * 1024 * 1024;

/// Count JSON without allocating an oversized serialization on a refused view.
pub(crate) fn serialized_bytes(value: &impl Serialize, limit: usize) -> Result<usize, CoreError> {
    struct Meter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Meter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            if buffer.len() > self.limit.saturating_sub(self.bytes) {
                return Err(std::io::Error::other(format!(
                    "serialized value exceeds its {} byte limit",
                    self.limit
                )));
            }
            self.bytes += buffer.len();
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut meter = Meter { bytes: 0, limit };
    serde_json::to_writer(&mut meter, value)
        .map_err(|error| CoreError::Request(error.to_string()))?;
    Ok(meter.bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct NodeHandle {
    pub generation: String,
    pub node_id: u32,
}

/// Integer payloads are decimal strings so JavaScript never rounds an identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum TypedValue {
    UniqueId(String),
    Int64(String),
    Float64(f64),
    String(String),
    Boolean(bool),
    Date(String),
    Timestamp(String),
    Point {
        lat: f64,
        lon: f64,
    },
    Duration {
        months: i32,
        days: i32,
        seconds: String,
    },
    Null,
    List(Vec<TypedValue>),
    Map(Vec<(String, TypedValue)>),
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum RecordCell {
    Value { value: TypedValue },
    Null,
    Missing,
    Unavailable { reason: String },
    Truncated { preview: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RecordsRequest {
    pub handles: Vec<NodeHandle>,
    pub fields: Vec<String>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default = "default_page_size")]
    pub limit: u32,
}

fn default_page_size() -> u32 {
    100
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct BrowseTypeRequest {
    pub node_type: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LoadNodesRequest {
    pub handles: Vec<NodeHandle>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RecordColumn {
    pub name: String,
    pub types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RecordRow {
    pub handle: NodeHandle,
    pub slot: Option<u32>,
    pub visible: bool,
    pub cells: Vec<RecordCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RecordTable {
    pub stamp: crate::shared::RevisionStamp,
    pub subset_revision: String,
    pub generation: String,
    pub columns: Vec<RecordColumn>,
    pub rows: Vec<RecordRow>,
    pub bound: BoundInfo,
    pub next_offset: Option<u32>,
    /// Some storage backends erase source null/absence distinctions at ingestion.
    pub missing_semantics: String,
}

pub(crate) fn validate(request: &RecordsRequest) -> Result<(), CoreError> {
    if request.handles.len() > MAX_RECORD_HANDLES || request.fields.len() > MAX_RECORD_FIELDS {
        return Err(CoreError::Request(format!(
            "records allow at most {MAX_RECORD_HANDLES} handles and {MAX_RECORD_FIELDS} fields"
        )));
    }
    if request
        .fields
        .iter()
        .any(|field| field.is_empty() || field.len() > 256)
    {
        return Err(CoreError::Request(
            "record field names must contain 1–256 bytes".into(),
        ));
    }
    Ok(())
}

pub(crate) fn read_cell(graph: &DirGraph, node_id: u32, field: &str) -> RecordCell {
    let Some(node) = graph.node_view(NodeIndex::new(node_id as usize)) else {
        return RecordCell::Unavailable {
            reason: "node is absent from this source snapshot".into(),
        };
    };
    if field == "type" {
        return cell(&Value::String(node.node_type_str(&graph.interner).into()));
    }
    match node.get_field_ref(field) {
        Some(value) => cell(&value),
        None if node
            .property_key_set()
            .contains(&InternedKey::from_str(field)) =>
        {
            RecordCell::Null
        }
        None => RecordCell::Missing,
    }
}

pub fn cell(value: &Value) -> RecordCell {
    if matches!(value, Value::Null) {
        return RecordCell::Null;
    }
    match typed_value(value, 0) {
        Ok(value)
            if serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() <= MAX_CELL_BYTES) =>
        {
            RecordCell::Value { value }
        }
        Ok(_) => truncated(value, "serialized value exceeds 4096 bytes"),
        Err(reason) if reason == "non-finite or graph-reference value" => RecordCell::Unavailable {
            reason: reason.into(),
        },
        Err(reason) => truncated(value, reason),
    }
}

pub(crate) fn legacy_cell_json(value: &Value) -> serde_json::Value {
    match cell(value) {
        RecordCell::Value { .. } => {
            let mut json = crate::values::value_to_json(value);
            preserve_integers(&mut json);
            json
        }
        RecordCell::Null => serde_json::Value::Null,
        other => serde_json::to_value(other).expect("record cells serialize"),
    }
}

pub(crate) fn preserve_integers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number)
            if number
                .as_i64()
                .is_some_and(|n| n.unsigned_abs() > 9_007_199_254_740_991) =>
        {
            *value = serde_json::Value::String(number.to_string());
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(preserve_integers),
        serde_json::Value::Object(values) => values.values_mut().for_each(preserve_integers),
        _ => {}
    }
}

fn truncated(value: &Value, reason: &str) -> RecordCell {
    let preview = match value {
        Value::String(text) => text.chars().take(256).collect(),
        Value::List(items) => format!("{} items", items.len()),
        Value::Map(items) => format!("{} fields", items.len()),
        _ => "value omitted".into(),
    };
    RecordCell::Truncated {
        preview,
        reason: reason.into(),
    }
}

/// Also used for source keys; a refused key remains inspectable through its handle.
pub fn typed_value(value: &Value, depth: usize) -> Result<TypedValue, &'static str> {
    let mut budget = MAX_CELL_BYTES;
    typed_with_budget(value, depth, &mut budget)
}

fn typed_with_budget(
    value: &Value,
    depth: usize,
    remaining: &mut usize,
) -> Result<TypedValue, &'static str> {
    if depth > MAX_VALUE_DEPTH {
        return Err("value exceeds nesting limit");
    }
    let charge = match value {
        Value::String(text) => text.len().saturating_add(32),
        _ => 32,
    };
    *remaining = remaining
        .checked_sub(charge)
        .ok_or("value exceeds 4096 byte conversion budget")?;
    Ok(match value {
        Value::UniqueId(v) => TypedValue::UniqueId(v.to_string()),
        Value::Int64(v) => TypedValue::Int64(v.to_string()),
        Value::Float64(v) if v.is_finite() => TypedValue::Float64(*v),
        Value::Boolean(v) => TypedValue::Boolean(*v),
        Value::String(v) if v.len() <= MAX_CELL_BYTES => TypedValue::String(v.clone()),
        Value::String(_) => return Err("string exceeds 4096 bytes"),
        Value::DateTime(v) => TypedValue::Date(v.to_string()),
        Value::Timestamp(v) => TypedValue::Timestamp(v.to_string()),
        Value::Point { lat, lon } if lat.is_finite() && lon.is_finite() => TypedValue::Point {
            lat: *lat,
            lon: *lon,
        },
        Value::Duration {
            months,
            days,
            seconds,
        } => TypedValue::Duration {
            months: *months,
            days: *days,
            seconds: seconds.to_string(),
        },
        Value::Null => TypedValue::Null,
        Value::List(items) if items.len() <= MAX_COLLECTION_ITEMS => TypedValue::List(
            items
                .iter()
                .map(|v| typed_with_budget(v, depth + 1, remaining))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(items) if items.len() <= MAX_COLLECTION_ITEMS => TypedValue::Map(
            items
                .iter()
                .map(|(k, v)| {
                    *remaining = remaining
                        .checked_sub(k.len())
                        .ok_or("map key exceeds byte budget")?;
                    Ok::<_, &'static str>((
                        k.to_string(),
                        typed_with_budget(v, depth + 1, remaining)?,
                    ))
                })
                .collect::<Result<_, _>>()?,
        ),
        Value::List(_) | Value::Map(_) => return Err("collection exceeds 64 entries"),
        _ => return Err("non-finite or graph-reference value"),
    })
}

pub(crate) fn column_types(rows: &[RecordRow], column: usize) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match &row.cells[column] {
            RecordCell::Value { value } => serde_json::to_value(value)
                .ok()?
                .get("type")?
                .as_str()
                .map(str::to_owned),
            RecordCell::Null => Some("null".into()),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

impl crate::Session {
    pub fn records(&self, request: &RecordsRequest) -> Result<RecordTable, CoreError> {
        validate(request)?;
        self.check_handles(&request.handles)?;
        let offset = (request.offset as usize).min(request.handles.len());
        let limit = (request.limit as usize).min(MAX_RECORD_ROWS);
        let (stamp, subset_revision, slots) = {
            let state = self.state_read();
            let visible: std::collections::HashSet<_> = state
                .subset
                .visible_nodes
                .iter()
                .map(|handle| handle.node_id)
                .collect();
            let slots: Vec<_> = request
                .handles
                .iter()
                .skip(offset)
                .take(limit)
                .map(|handle| {
                    (
                        state.slot_of_node(handle.node_id),
                        visible.contains(&handle.node_id),
                    )
                })
                .collect();
            (
                state.stamp(self.generation()),
                state.subset_revision.to_string(),
                slots,
            )
        };
        let mut table = RecordTable {
            stamp, subset_revision,
            generation: self.generation().into(),
            columns: request.fields.iter().map(|name| RecordColumn { name: name.clone(), types: Vec::new() }).collect(),
            rows: Vec::new(),
            bound: BoundInfo::new(0, request.handles.len().saturating_sub(offset)),
            next_offset: None,
            missing_semantics: "missing means absent in this snapshot; storage may have erased source null/absence distinctions".into(),
        };
        let _guard = self.graph().begin_read_pass();
        let mut bytes = 16 * 1024;
        for (handle, (slot, visible)) in request.handles.iter().skip(offset).zip(slots) {
            let row = RecordRow {
                handle: handle.clone(),
                slot,
                visible,
                cells: request
                    .fields
                    .iter()
                    .map(|field| read_cell(self.graph(), handle.node_id, field))
                    .collect(),
            };
            let row_bytes = serde_json::to_vec(&row)
                .map_err(|e| CoreError::Request(e.to_string()))?
                .len()
                + 1;
            if bytes + row_bytes > MAX_RECORD_BYTES {
                break;
            }
            bytes += row_bytes;
            table.rows.push(row);
        }
        for (index, column) in table.columns.iter_mut().enumerate() {
            column.types = column_types(&table.rows, index);
        }
        let end = offset + table.rows.len();
        table.bound = BoundInfo::new(table.rows.len(), request.handles.len() - offset);
        table.next_offset = (end < request.handles.len() && end > offset).then_some(end as u32);
        if serde_json::to_vec(&table)
            .map_err(|e| CoreError::Request(e.to_string()))?
            .len()
            > MAX_RECORD_BYTES
        {
            return Err(CoreError::Request(
                "record response exceeds its byte ceiling".into(),
            ));
        }
        Ok(table)
    }
}
