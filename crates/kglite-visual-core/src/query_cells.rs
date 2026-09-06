//! Bounded previews around the engine's canonical JSON conversion.
use crate::records::{self, RecordCell, MAX_CELL_BYTES, MAX_COLLECTION_ITEMS, MAX_VALUE_DEPTH};
use kglite::api::Value;

pub(crate) fn preview(value: &Value) -> (serde_json::Value, RecordCell) {
    let mut remaining = MAX_CELL_BYTES;
    if let Err(reason) = preflight(value, 0, &mut remaining) {
        return omitted_pair(value, reason);
    }
    let mut json = crate::values::value_to_json(value);
    records::preserve_integers(&mut json);
    if records::serialized_bytes(&json, MAX_CELL_BYTES).is_err() {
        return omitted_pair(value, "serialized cell exceeds 4096 bytes");
    }
    let cell = records::cell(&entities_as_maps(value));
    (json, cell)
}
fn omitted_pair(value: &Value, reason: &str) -> (serde_json::Value, RecordCell) {
    let cell = if reason == "non-finite value" {
        RecordCell::Unavailable {
            reason: reason.into(),
        }
    } else {
        let preview = match value {
            Value::String(text) => text.chars().take(256).collect(),
            Value::List(items) => format!("{} items", items.len()),
            Value::Map(items) => format!("{} fields", items.len()),
            Value::Node(_) => "node record; inspect source fields through its row reference".into(),
            Value::Relationship(_) => "relationship record".into(),
            Value::Path(_) => "path record".into(),
            _ => "value omitted".into(),
        };
        RecordCell::Truncated {
            preview,
            reason: reason.into(),
        }
    };
    (
        serde_json::to_value(&cell).expect("record preview serializes"),
        cell,
    )
}
// This conversion runs only after preflight has bounded depth, entries and bytes.
fn entities_as_maps(value: &Value) -> Value {
    match value {
        Value::NodeRef(id) => Value::UniqueId(*id),
        Value::Node(node) => node_map(node),
        Value::Relationship(relation) => relation_map(relation),
        Value::Path(path) => Value::Map(
            [
                (
                    "nodes",
                    Value::List(path.nodes.iter().map(node_map).collect()),
                ),
                (
                    "relationships",
                    Value::List(path.rels.iter().map(relation_map).collect()),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        Value::List(items) => Value::List(items.iter().map(entities_as_maps).collect()),
        Value::Map(items) => Value::Map(
            items
                .iter()
                .map(|(key, value)| (key, entities_as_maps(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}
fn node_map(node: &kglite::api::NodeValue) -> Value {
    Value::Map(
        [
            ("id", Value::UniqueId(node.id)),
            (
                "labels",
                Value::List(
                    node.labels
                        .iter()
                        .map(|label| Value::String(label.clone()))
                        .collect(),
                ),
            ),
            (
                "properties",
                Value::Map(
                    node.properties
                        .iter()
                        .map(|(key, value)| (key, entities_as_maps(value)))
                        .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    )
}
fn relation_map(relation: &kglite::api::RelValue) -> Value {
    Value::Map(
        [
            ("id", Value::UniqueId(relation.id)),
            ("start", Value::UniqueId(relation.start_id)),
            ("end", Value::UniqueId(relation.end_id)),
            ("type", Value::String(relation.rel_type.clone())),
            (
                "properties",
                Value::Map(
                    relation
                        .properties
                        .iter()
                        .map(|(key, value)| (key, entities_as_maps(value)))
                        .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    )
}
fn charge(bytes: usize, remaining: &mut usize) -> Result<(), &'static str> {
    *remaining = remaining
        .checked_sub(bytes)
        .ok_or("cell exceeds 4096 byte preview budget")?;
    Ok(())
}
fn text(text: &str, remaining: &mut usize) -> Result<(), &'static str> {
    if text.len() > *remaining {
        return Err("cell exceeds 4096 byte preview budget");
    }
    let escaped = text
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>();
    charge(escaped + 2, remaining)
}
fn preflight(value: &Value, depth: usize, remaining: &mut usize) -> Result<(), &'static str> {
    if depth > MAX_VALUE_DEPTH {
        return Err("cell exceeds depth 8");
    }
    charge(32, remaining)?;
    match value {
        Value::String(value) => text(value, remaining),
        Value::Float64(value) if !value.is_finite() => Err("non-finite value"),
        Value::Point { lat, lon } if !lat.is_finite() || !lon.is_finite() => {
            Err("non-finite value")
        }
        Value::List(items) => {
            if items.len() > MAX_COLLECTION_ITEMS {
                return Err("collection exceeds 64 preview entries");
            }
            for item in items {
                preflight(item, depth + 1, remaining)?;
            }
            Ok(())
        }
        Value::Map(items) => {
            if items.len() > MAX_COLLECTION_ITEMS {
                return Err("collection exceeds 64 preview entries");
            }
            for (key, item) in items.iter() {
                text(key, remaining)?;
                preflight(item, depth + 1, remaining)?;
            }
            Ok(())
        }
        Value::Node(node) => preflight_node(node, depth, remaining),
        Value::Relationship(relation) => preflight_relation(relation, depth, remaining),
        Value::Path(path) => {
            if path.nodes.len() + path.rels.len() > MAX_COLLECTION_ITEMS {
                return Err("path exceeds 64 preview entities");
            }
            for node in &path.nodes {
                preflight_node(node, depth, remaining)?;
            }
            for relation in &path.rels {
                preflight_relation(relation, depth, remaining)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
fn preflight_node(
    node: &kglite::api::NodeValue,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), &'static str> {
    if node.labels.len() > MAX_COLLECTION_ITEMS {
        return Err("node exceeds 64 preview labels");
    }
    charge(96, remaining)?;
    for label in &node.labels {
        text(label, remaining)?;
    }
    preflight_properties(&node.properties, depth, remaining)
}
fn preflight_relation(
    relation: &kglite::api::RelValue,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), &'static str> {
    charge(128, remaining)?;
    text(&relation.rel_type, remaining)?;
    preflight_properties(&relation.properties, depth, remaining)
}
fn preflight_properties(
    properties: &kglite::api::PropMap,
    depth: usize,
    remaining: &mut usize,
) -> Result<(), &'static str> {
    if properties.len() > MAX_COLLECTION_ITEMS {
        return Err("record exceeds 64 preview fields");
    }
    for (key, value) in properties.iter() {
        text(key, remaining)?;
        preflight(value, depth + 1, remaining)?;
    }
    Ok(())
}
