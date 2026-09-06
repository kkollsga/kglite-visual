//! Source identities extracted from actual engine entities before JSON conversion.
use crate::query::QueryRelationship;
use crate::records::{NodeHandle, MAX_LOADED_EDGES, MAX_LOADED_NODES};
use kglite::api::{RelValue, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use ts_rs::TS;

pub const MAX_ROW_REFERENCES: usize = 64;
const MAX_REFERENCE_WORK: usize = 100_000;
const MAX_REFERENCE_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RelationHandle {
    pub generation: String,
    pub edge_id: u32,
    pub source: NodeHandle,
    pub target: NodeHandle,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct QueryRowReferences {
    pub nodes: Vec<NodeHandle>,
    pub relationships: Vec<RelationHandle>,
    pub truncated: bool,
}

#[derive(Default)]
pub(crate) struct RawReferences {
    pub nodes: Vec<u32>,
    pub relationships: Vec<QueryRelationship>,
    pub incomplete: bool,
    seen_nodes: HashSet<u32>,
    seen_relationships: HashSet<u32>,
    work: usize,
    name_bytes: usize,
}
impl RawReferences {
    pub fn row(values: &[Value]) -> Self {
        let mut result = Self::default();
        for value in values {
            result.visit(value, 0);
            if result.incomplete {
                break;
            }
        }
        result
    }
    pub fn scoped(&self, generation: Option<&str>) -> QueryRowReferences {
        let Some(generation) = generation else {
            return QueryRowReferences {
                truncated: self.incomplete
                    || !self.nodes.is_empty()
                    || !self.relationships.is_empty(),
                ..Default::default()
            };
        };
        let handle = |node_id| NodeHandle {
            generation: generation.into(),
            node_id,
        };
        QueryRowReferences {
            nodes: self
                .nodes
                .iter()
                .take(MAX_ROW_REFERENCES)
                .copied()
                .map(handle)
                .collect(),
            relationships: self
                .relationships
                .iter()
                .take(MAX_ROW_REFERENCES)
                .map(|relation| RelationHandle {
                    generation: generation.into(),
                    edge_id: relation.edge_id,
                    source: handle(relation.source_id),
                    target: handle(relation.target_id),
                })
                .collect(),
            truncated: self.incomplete
                || self.nodes.len() > MAX_ROW_REFERENCES
                || self.relationships.len() > MAX_ROW_REFERENCES,
        }
    }
    fn node(&mut self, id: u32) {
        if self.seen_nodes.contains(&id) {
            return;
        }
        if self.nodes.len() > MAX_LOADED_NODES {
            self.incomplete = true;
            return;
        }
        self.seen_nodes.insert(id);
        self.nodes.push(id);
    }
    fn relation(&mut self, relation: &RelValue) {
        self.node(relation.start_id);
        self.node(relation.end_id);
        if self.seen_relationships.contains(&relation.id) {
            return;
        }
        if self.relationships.len() > MAX_LOADED_EDGES
            || relation.rel_type.len()
                > crate::query::MAX_QUERY_BYTES.saturating_sub(self.name_bytes)
        {
            self.incomplete = true;
            return;
        }
        self.seen_relationships.insert(relation.id);
        self.name_bytes += relation.rel_type.len();
        self.relationships.push(QueryRelationship {
            edge_id: relation.id,
            source_id: relation.start_id,
            target_id: relation.end_id,
            name: relation.rel_type.clone(),
        });
    }
    fn visit(&mut self, value: &Value, depth: usize) {
        self.work += 1;
        if self.work > MAX_REFERENCE_WORK || depth > MAX_REFERENCE_DEPTH {
            self.incomplete = true;
            return;
        }
        match value {
            Value::NodeRef(id) => self.node(*id),
            Value::Node(node) => self.node(node.id),
            Value::Relationship(relation) => self.relation(relation),
            Value::Path(path) => {
                for node in &path.nodes {
                    self.work += 1;
                    if self.work > MAX_REFERENCE_WORK {
                        self.incomplete = true;
                        break;
                    }
                    self.node(node.id);
                    if self.incomplete {
                        break;
                    }
                }
                for relation in &path.rels {
                    self.work += 1;
                    if self.work > MAX_REFERENCE_WORK {
                        self.incomplete = true;
                        break;
                    }
                    self.relation(relation);
                    if self.incomplete {
                        break;
                    }
                }
            }
            Value::List(items) => {
                for item in items {
                    self.visit(item, depth + 1);
                    if self.incomplete {
                        break;
                    }
                }
            }
            Value::Map(items) => {
                for (_, item) in items.iter() {
                    self.visit(item, depth + 1);
                    if self.incomplete {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct LoadEntitiesRequest {
    pub nodes: Vec<NodeHandle>,
    pub relationships: Vec<RelationHandle>,
}
