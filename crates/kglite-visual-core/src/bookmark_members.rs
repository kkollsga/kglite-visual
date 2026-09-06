//! Resolve durable member keys only through the immutable source snapshot.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use kglite::api::{DirGraph, EdgeIndex, GraphRead, NodeIndex};
use sha2::{Digest, Sha256};

use crate::bookmark::{BookmarkMember, BookmarkRelation};
use crate::records::{self, NodeHandle, RecordCell, TypedValue};
use crate::source_identity::check_deadline;
use crate::view::{SlotEntry, View};
use crate::CoreError;

pub(crate) fn session_members(view: &View, generation: &str) -> Vec<BookmarkMember> {
    view.live_entries()
        .filter_map(|(_, entry)| match entry {
            SlotEntry::Node { node_id, .. } => Some(BookmarkMember::Handle {
                handle: NodeHandle {
                    generation: generation.into(),
                    node_id: *node_id,
                },
            }),
            _ => None,
        })
        .collect()
}

pub(crate) fn durable_members(
    graph: &DirGraph,
    view: &View,
    deadline: Option<Instant>,
) -> Result<Vec<BookmarkMember>, CoreError> {
    let mut members = Vec::new();
    for (_, entry) in view.live_entries() {
        check_deadline(deadline)?;
        if let SlotEntry::Node {
            node_id, node_type, ..
        } = entry
        {
            let key = scalar_key(graph, *node_id)?;
            members.push(BookmarkMember::Key {
                node_type: node_type.clone(),
                key,
            });
        }
    }
    resolve_members(graph, &members, "", deadline)?;
    Ok(members)
}

pub(crate) fn resolve_members(
    graph: &DirGraph,
    members: &[BookmarkMember],
    generation: &str,
    deadline: Option<Instant>,
) -> Result<Vec<u32>, CoreError> {
    if members.len() > records::MAX_LOADED_NODES {
        return Err(refusal("bookmark exceeds 5000 members"));
    }
    let mut resolved = vec![None; members.len()];
    let mut requested: BTreeMap<&str, HashMap<String, usize>> = BTreeMap::new();
    for (position, member) in members.iter().enumerate() {
        check_deadline(deadline)?;
        match member {
            BookmarkMember::Handle { handle } => {
                if handle.generation != generation
                    || graph
                        .node_view(NodeIndex::new(handle.node_id as usize))
                        .is_none()
                {
                    return Err(refusal(
                        "session-only bookmark belongs to another source session",
                    ));
                }
                resolved[position] = Some(handle.node_id);
            }
            BookmarkMember::Key { node_type, key } => {
                validate_key(key)?;
                if node_type.len() > 256
                    || requested
                        .entry(node_type)
                        .or_default()
                        .insert(key_token(key)?, position)
                        .is_some()
                {
                    return Err(refusal("bookmark has duplicate or invalid member keys"));
                }
            }
        }
    }
    for (node_type, keys) in requested {
        let candidates = graph
            .type_indices
            .get(node_type)
            .ok_or_else(|| refusal("bookmark node type is absent"))?;
        for index in candidates.iter() {
            check_deadline(deadline)?;
            let Ok(key) = scalar_key(graph, index.index() as u32) else {
                continue;
            };
            if let Some(&position) = keys.get(&key_token(&key)?) {
                if resolved[position].replace(index.index() as u32).is_some() {
                    return Err(refusal(
                        "source keys are ambiguous; exact durable restoration is unavailable",
                    ));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    resolved
        .into_iter()
        .map(|id| {
            let id = id.ok_or_else(|| refusal("bookmark source member is absent"))?;
            if !seen.insert(id) {
                return Err(refusal("bookmark repeats a source member"));
            }
            Ok(id)
        })
        .collect()
}

pub(crate) fn scalar_key(graph: &DirGraph, id: u32) -> Result<TypedValue, CoreError> {
    match records::read_cell(graph, id, "id") {
        RecordCell::Value { value } => {
            validate_key(&value)?;
            Ok(value)
        }
        _ => Err(refusal(
            "a loaded member has no bounded non-null source key; bookmark is session-only",
        )),
    }
}
fn validate_key(key: &TypedValue) -> Result<(), CoreError> {
    if matches!(
        key,
        TypedValue::Null | TypedValue::List(_) | TypedValue::Map(_)
    ) {
        return Err(refusal(
            "durable member keys must be non-null scalar values",
        ));
    }
    records::serialized_bytes(key, records::MAX_CELL_BYTES)?;
    Ok(())
}
fn key_token(key: &TypedValue) -> Result<String, CoreError> {
    serde_json::to_string(key).map_err(|error| CoreError::Request(error.to_string()))
}

pub(crate) fn capture_relations(
    graph: &DirGraph,
    view: &View,
    node_members: &HashMap<u32, u32>,
    deadline: Option<Instant>,
) -> Result<Vec<BookmarkRelation>, CoreError> {
    let mut result = Vec::new();
    let mut bytes = 0;
    for edge in view.edges().iter().filter(|edge| !edge.meta) {
        check_deadline(deadline)?;
        let id = edge
            .edge_id
            .ok_or_else(|| refusal("loaded relation has no source identity"))?;
        let (source, target) = graph
            .graph
            .edge_endpoints(EdgeIndex::new(id as usize))
            .ok_or_else(|| refusal("source relation is absent"))?;
        result.push(BookmarkRelation {
            edge_id: id,
            source_member: *node_members
                .get(&(source.index() as u32))
                .ok_or_else(|| refusal("relation source is not a saved member"))?,
            target_member: *node_members
                .get(&(target.index() as u32))
                .ok_or_else(|| refusal("relation target is not a saved member"))?,
            name: edge.name.clone(),
            attributes_sha256: relation_fingerprint(graph, id, &mut bytes, deadline)?,
        });
    }
    Ok(result)
}

pub(crate) fn relation_fingerprint(
    graph: &DirGraph,
    edge_id: u32,
    bytes: &mut usize,
    deadline: Option<Instant>,
) -> Result<String, CoreError> {
    let edge = graph
        .graph
        .edge_weight(EdgeIndex::new(edge_id as usize))
        .ok_or_else(|| refusal("source relation is absent"))?;
    if edge.property_count() > 64 {
        return Err(refusal("relation fingerprint exceeds 64 properties"));
    }
    let mut properties = BTreeMap::new();
    for (key, value) in edge.property_iter(&graph.interner) {
        check_deadline(deadline)?;
        if key.len() > 4096 {
            return Err(refusal("relation property name exceeds 4096 bytes"));
        }
        let value = match records::cell(value) {
            RecordCell::Null => TypedValue::Null,
            RecordCell::Value { value } => value,
            _ => {
                return Err(refusal(
                    "relation property exceeds the exact fingerprint value bound",
                ))
            }
        };
        *bytes += records::serialized_bytes(&(key, &value), 32 * 1024)?;
        if *bytes > 8 * 1024 * 1024 {
            return Err(refusal(
                "relation fingerprints exceed 8 MiB of property values",
            ));
        }
        properties.insert(key, value);
    }
    let encoded =
        serde_json::to_vec(&properties).map_err(|error| CoreError::Request(error.to_string()))?;
    Ok(crate::source_identity::hex_digest(&Sha256::digest(encoded)))
}

fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}
