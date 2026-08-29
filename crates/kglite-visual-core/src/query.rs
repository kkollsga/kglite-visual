//! Cypher, and the server-side search built on it.
//!
//! **Synchronous, and deliberately so.** `execute_read` is a blocking call and
//! this crate is transport-agnostic: it cannot know whether its caller has an
//! async runtime, a notebook kernel or a desktop event loop. The caller runs
//! this off its own reactor — the CLI in `tokio::task::spawn_blocking` — and
//! must do it on a thread with at least [`QUERY_THREAD_STACK_BYTES`] of stack.
//! That is not a tuning knob: kglite's Cypher parser overflows tokio's 2 MiB
//! default, and the failure is a stack overflow inside a blocking task, which
//! arrives as a process abort with no message naming the query.
//!
//! **`lazy_eligible` stays false.** With it true the executor hands back a
//! `LazyResultDescriptor` and an *empty* `rows`, and a caller with no lazy
//! materializer reports a successful query with no results — the documented
//! bolt-server bug. There is one construction site for `ExecuteOptions` in this
//! file for exactly that reason.
//!
//! **Search is server-side, always** (plan D7). No client-side index: an index
//! the browser builds is an index over what the browser already has, which on a
//! 100M-node graph is nothing worth searching.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use kglite::api::param::json_value_to_kglite_value;
use kglite::api::session::{execute_read, ExecuteOptions};
use kglite::api::{DirGraph, NodeIndex, Value};
use serde::Serialize;
use ts_rs::TS;

use crate::bound::{Bound, BoundInfo};
use crate::error::CoreError;
use crate::protocol::PROTOCOL_VERSION;
use crate::request::{CypherRequest, SearchMode, SearchRequest};
use crate::values::{value_to_display, value_to_json};

/// Stack size a Cypher-executing thread needs, taken from the engine rather
/// than restated. kglite ships the number as `QUERY_THREAD_STACK_SIZE` and
/// every embedder is expected to honour it; a second literal here would be a
/// copy that stops moving when the engine's does.
pub const QUERY_THREAD_STACK_BYTES: usize = kglite::api::session::QUERY_THREAD_STACK_SIZE;

/// Rows one query may return.
///
/// **Provisional; P4 sets the final number.** The results table is HTML, and a
/// browser building 100 000 rows of DOM is the same freeze the whole
/// progressive-disclosure design exists to avoid.
pub const MAX_QUERY_ROWS: usize = 5_000;

/// Serialized ceiling for one result table. Four protocol chunks.
pub const MAX_QUERY_BYTES: usize = 2 * 1024 * 1024;

/// Work-unit budget handed to the executor as `ExecuteOptions::max_rows`.
///
/// **`max_rows` is not a row cap.** Measured against kglite 0.16.13, not read
/// off the field name: it is the executor's *work* budget — "the maximum
/// materialized row-set cardinality and the maximum number of collection items
/// a single expanding operator may emit" (`cypher/executor/budget.rs`) — and
/// exceeding it **errors** rather than truncating. Setting it to the caller's
/// requested row count therefore turns an ordinary query into a failure:
/// `MATCH (p:Person)-[:WORKS_AT]->(c) RETURN c.title, count(p)` with
/// `max_rows: 4` returns *"Query produced 118 work units … exceeding max_rows
/// limit of 4"*, not four rows. The response bound is applied by
/// [`to_table`] instead, where it can truncate and say so.
///
/// So this number is what it says: a runaway guard. Well under kglite's own
/// 10 000 000 unbounded backstop, because that backstop is sized for a batch
/// job and this is an interactive viewer — a query that has materialised two
/// million rows has already lost the user's attention, and the memory it costs
/// comes out of the browser's host process.
pub const MAX_QUERY_WORK_UNITS: usize = 2_000_000;

/// Hits one search may return. Small on purpose: a search result is a list a
/// human reads, and "load into view" is the bounded path from it into the
/// graph.
pub const MAX_SEARCH_HITS: usize = 200;

/// Default wall-clock ceiling for one query.
///
/// A viewer is interactive, so an unbounded query is a hung tab. Configurable
/// at the CLI (`--query-timeout-secs`) because a deliberate analytical query on
/// a large graph is a legitimate thing to wait for; the default is what an
/// accidental cartesian product costs.
pub const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// Execution settings a session applies to every query it runs.
#[derive(Debug, Clone, Copy)]
pub struct QueryConfig {
    pub timeout: Duration,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_QUERY_TIMEOUT,
        }
    }
}

impl QueryConfig {
    pub(crate) fn deadline(&self) -> Option<Instant> {
        Instant::now().checked_add(self.timeout)
    }
}

/// A relationship a query result mentioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct QueryRelationship {
    pub source_id: u32,
    pub target_id: u32,
    pub name: String,
}

/// A Cypher result, columnar.
///
/// kglite returns `Vec<Vec<Value>>` — row-major, one `Vec` per row — and there
/// is no columnar accessor to ask for instead (plan D11's filed wish). The
/// transpose therefore happens exactly once, here, rather than on every
/// consumer: a results table reads columns, a typed-array appearance getter
/// reads columns, and a client transposing per render would pay O(rows × cols)
/// for a shape the server already had to walk.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct QueryTable {
    pub protocol_version: u32,
    pub columns: Vec<String>,
    /// One array per column, in `columns` order. `data[c][r]` is row `r` of
    /// column `c`.
    #[ts(type = "unknown[][]")]
    pub data: Vec<Vec<serde_json::Value>>,
    /// Rows returned vs rows the query produced before the bound (D5).
    pub bound: BoundInfo,
    pub elapsed_ms: u32,
    /// kglite node ids the result mentioned, deduplicated in first-seen order.
    /// What "show in graph" maps into the slot space.
    pub node_ids: Vec<u32>,
    /// Relationships the result mentioned, so a `RETURN n, r, m` draws edges
    /// rather than a dust cloud.
    pub relationships: Vec<QueryRelationship>,
    /// Whether the query was an `EXPLAIN`. The rows are a plan, not data, and a
    /// UI that offered "show in graph" for them would be offering nonsense.
    pub explain: bool,
}

/// Run a read-only Cypher query.
///
/// Errors carry kglite's own message. `KgError` is built to be diagnostic —
/// position, expected token, the schema name it could not resolve — and
/// replacing it with "query failed" throws away the only thing the user can act
/// on.
pub fn run_cypher(
    graph: &DirGraph,
    request: &CypherRequest,
    config: QueryConfig,
) -> Result<QueryTable, CoreError> {
    let params = bind_params(&request.params);
    let bound = Bound {
        max_items: request
            .limit
            .map(|n| n as usize)
            .unwrap_or(MAX_QUERY_ROWS)
            .min(MAX_QUERY_ROWS),
        max_bytes: MAX_QUERY_BYTES,
    };

    let started = Instant::now();
    let outcome = execute(graph, &request.query, &params, config)?;
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let mut table = to_table(outcome.result.columns, outcome.result.rows, bound);
    table.elapsed_ms = elapsed_ms;
    table.explain = outcome.explain;
    Ok(table)
}

/// The one `ExecuteOptions` construction site in this crate.
///
/// Every field that is not a default is a decision Phase 0 recorded:
/// `lazy_eligible` false (no lazy materializer here, and true would hand back
/// an empty `rows` that reads as a successful query with no results),
/// `deadline` set (an interactive tool cannot host an unbounded query),
/// `max_rows` set to [`MAX_QUERY_WORK_UNITS`] — a runaway guard, **not** the
/// response bound; see that constant for the measurement that separates them.
fn execute(
    graph: &DirGraph,
    query: &str,
    params: &HashMap<String, Value>,
    config: QueryConfig,
) -> Result<kglite::api::session::ExecuteOutcome, CoreError> {
    let mut opts = ExecuteOptions::eager(params);
    opts.deadline = config.deadline();
    opts.max_rows = Some(MAX_QUERY_WORK_UNITS);
    Ok(execute_read(graph, query, &opts)?)
}

/// JSON parameters into kglite `Value`s, through the engine's own converter.
fn bind_params(params: &BTreeMap<String, serde_json::Value>) -> HashMap<String, Value> {
    params
        .iter()
        .map(|(key, value)| (key.clone(), json_value_to_kglite_value(value)))
        .collect()
}

/// Transpose kglite's rows into columns, applying the bound once.
fn to_table(columns: Vec<String>, rows: Vec<Vec<Value>>, bound: Bound) -> QueryTable {
    let total = rows.len();
    let kept = total.min(bound.max_items);

    let mut node_ids: Vec<u32> = Vec::new();
    let mut seen_nodes: HashSet<u32> = HashSet::new();
    let mut relationships: Vec<QueryRelationship> = Vec::new();
    let mut seen_rels: HashSet<u32> = HashSet::new();

    let mut data: Vec<Vec<serde_json::Value>> = vec![Vec::with_capacity(kept); columns.len()];
    let mut bytes = 0usize;
    let mut returned = 0usize;
    for row in rows.into_iter().take(kept) {
        let row_bytes: usize = row.iter().map(estimate_bytes).sum();
        // One row always crosses the wire, however large: answering a
        // legitimate query with an empty table and no way to make progress is
        // worse than answering it with one row and `truncated`.
        if returned > 0 && bytes + row_bytes > bound.max_bytes {
            break;
        }
        bytes += row_bytes;
        for (index, cell) in row.iter().enumerate() {
            collect_graph_refs(
                cell,
                &mut node_ids,
                &mut seen_nodes,
                &mut relationships,
                &mut seen_rels,
            );
            if let Some(column) = data.get_mut(index) {
                column.push(value_to_json(cell));
            }
        }
        returned += 1;
    }

    QueryTable {
        protocol_version: PROTOCOL_VERSION,
        columns,
        data,
        bound: BoundInfo {
            returned: returned as u32,
            total: total as u32,
            truncated: returned < total,
        },
        elapsed_ms: 0,
        node_ids,
        relationships,
        explain: false,
    }
}

/// Rough serialized size of one cell, for the byte half of the bound.
fn estimate_bytes(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len() + 3,
        Value::Node(node) => {
            64 + node.labels.iter().map(|l| l.len() + 4).sum::<usize>() + 48 * node.properties.len()
        }
        Value::Relationship(rel) => 96 + rel.rel_type.len(),
        Value::Path(path) => 64 * (path.nodes.len() + path.rels.len()),
        Value::List(items) => 2 + items.iter().map(estimate_bytes).sum::<usize>(),
        Value::Map(map) => {
            2 + map
                .iter()
                .map(|(k, v)| k.len() + estimate_bytes(v) + 4)
                .sum::<usize>()
        }
        _ => 24,
    }
}

/// Pull node and relationship identities out of a result cell.
///
/// Recursive because a `RETURN collect(n)` or a path buries them one and two
/// levels down, and "show in graph" going blank for the aggregate form of the
/// same query would read as a bug in the query.
fn collect_graph_refs(
    value: &Value,
    node_ids: &mut Vec<u32>,
    seen_nodes: &mut HashSet<u32>,
    relationships: &mut Vec<QueryRelationship>,
    seen_rels: &mut HashSet<u32>,
) {
    match value {
        Value::Node(node) => {
            if seen_nodes.insert(node.id) {
                node_ids.push(node.id);
            }
        }
        Value::Relationship(rel) => {
            if seen_rels.insert(rel.id) {
                relationships.push(QueryRelationship {
                    source_id: rel.start_id,
                    target_id: rel.end_id,
                    name: rel.rel_type.clone(),
                });
            }
        }
        Value::Path(path) => {
            for node in &path.nodes {
                if seen_nodes.insert(node.id) {
                    node_ids.push(node.id);
                }
            }
            for rel in &path.rels {
                if seen_rels.insert(rel.id) {
                    relationships.push(QueryRelationship {
                        source_id: rel.start_id,
                        target_id: rel.end_id,
                        name: rel.rel_type.clone(),
                    });
                }
            }
        }
        Value::List(items) => {
            for item in items {
                collect_graph_refs(item, node_ids, seen_nodes, relationships, seen_rels);
            }
        }
        Value::Map(map) => {
            for (_, item) in map.iter() {
                collect_graph_refs(item, node_ids, seen_nodes, relationships, seen_rels);
            }
        }
        _ => {}
    }
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SearchHit {
    pub node_id: u32,
    pub node_type: String,
    /// The matched value, as displayed.
    pub label: String,
    /// The slot this node already occupies, if it is on screen. `null` means
    /// the hit exists but is not loaded — the "load into view" case, and the
    /// reason the two are one response rather than two endpoints.
    pub slot: Option<u32>,
}

/// What a search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct SearchResponse {
    pub protocol_version: u32,
    pub query: String,
    /// The property actually searched — echoed because it defaults, and a
    /// search that found nothing because it looked at the wrong column must say
    /// which column that was.
    pub property: String,
    pub node_type: Option<String>,
    pub mode: SearchMode,
    pub hits: Vec<SearchHit>,
    pub bound: BoundInfo,
    pub elapsed_ms: u32,
}

/// Type-scoped title/property search, via Cypher.
///
/// The needle is a **parameter**, never interpolated: a search box that
/// concatenated user text into a query would let a quote turn a search into a
/// different query. The property and label are identifiers rather than values,
/// so they cannot be parameters — they are validated against the graph's own
/// schema before they reach the query text, which is the only safe form of
/// identifier substitution.
pub fn search(
    graph: &DirGraph,
    request: &SearchRequest,
    config: QueryConfig,
) -> Result<SearchResponse, CoreError> {
    let property = request
        .property
        .clone()
        .unwrap_or_else(|| "title".to_string());
    validate_identifier(&property, "property")?;
    if let Some(node_type) = &request.node_type {
        validate_identifier(node_type, "node type")?;
        if !graph.type_indices.contains_key(node_type) {
            return Err(CoreError::Request(format!(
                "no node type named '{node_type}' in this graph"
            )));
        }
    }

    let limit = request
        .limit
        .map(|n| n as usize)
        .unwrap_or(MAX_SEARCH_HITS)
        .min(MAX_SEARCH_HITS);
    let predicate = match request.mode {
        SearchMode::Contains => "CONTAINS",
        SearchMode::StartsWith => "STARTS WITH",
    };
    let label = request
        .node_type
        .as_ref()
        .map(|t| format!(":{t}"))
        .unwrap_or_default();
    let cypher = format!(
        "MATCH (n{label}) WHERE toLower(toString(n.{property})) {predicate} $needle \
         RETURN n AS node, n.{property} AS matched LIMIT {}",
        limit + 1
    );

    let mut params: HashMap<String, Value> = HashMap::new();
    params.insert(
        "needle".to_string(),
        Value::String(request.query.to_lowercase()),
    );

    let started = Instant::now();
    let outcome = execute(graph, &cypher, &params, config)?;
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    let total = outcome.result.rows.len();
    let mut hits = Vec::with_capacity(total.min(limit));
    for row in outcome.result.rows.iter().take(limit) {
        let Some(Value::Node(node)) = row.first() else {
            continue;
        };
        let matched = row.get(1).map(value_to_display).unwrap_or_default();
        hits.push(SearchHit {
            node_id: node.id,
            node_type: node.labels.first().cloned().unwrap_or_default(),
            label: matched,
            slot: None,
        });
    }

    let returned = hits.len();
    Ok(SearchResponse {
        protocol_version: PROTOCOL_VERSION,
        query: request.query.clone(),
        property,
        node_type: request.node_type.clone(),
        mode: request.mode,
        hits,
        bound: BoundInfo {
            returned: returned as u32,
            total: total as u32,
            truncated: returned < total,
        },
        elapsed_ms,
    })
}

/// Refuse anything that is not a bare identifier.
///
/// The only place in this crate where caller text reaches a query as *syntax*
/// rather than as a parameter. Cypher has no bind form for a label or a
/// property key, so the substitution is unavoidable; making it total — letters,
/// digits and underscore, nothing else, never empty — is what stops it being an
/// injection point. A backtick-quoted identifier would still admit a backtick.
fn validate_identifier(name: &str, what: &str) -> Result<(), CoreError> {
    let ok = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(CoreError::Request(format!(
            "{what} '{name}' is not a plain identifier (letters, digits and underscore only)"
        )))
    }
}

/// The node indices a query result named, as kglite node handles.
pub fn node_indices(table: &QueryTable) -> Vec<NodeIndex> {
    table
        .node_ids
        .iter()
        .map(|id| NodeIndex::new(*id as usize))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_transpose_into_columns_once() {
        let table = to_table(
            vec!["a".into(), "b".into()],
            vec![
                vec![Value::Int64(1), Value::String("x".into())],
                vec![Value::Int64(2), Value::String("y".into())],
            ],
            Bound {
                max_items: 10,
                max_bytes: 10_000,
            },
        );
        assert_eq!(table.columns, vec!["a", "b"]);
        assert_eq!(table.data.len(), 2, "one array per column");
        assert_eq!(
            table.data[0],
            vec![serde_json::json!(1), serde_json::json!(2)]
        );
        assert_eq!(
            table.data[1],
            vec![serde_json::json!("x"), serde_json::json!("y")]
        );
        assert!(!table.bound.truncated);
    }

    #[test]
    fn the_row_bound_can_fail_and_reports_the_true_total() {
        let rows: Vec<Vec<Value>> = (0..10).map(|i| vec![Value::Int64(i)]).collect();
        let table = to_table(
            vec!["n".into()],
            rows,
            Bound {
                max_items: 3,
                max_bytes: 10_000,
            },
        );
        assert_eq!(table.data[0].len(), 3);
        assert_eq!(
            table.bound,
            BoundInfo {
                returned: 3,
                total: 10,
                truncated: true
            }
        );
    }

    #[test]
    fn the_byte_bound_fires_independently_of_the_row_bound() {
        let rows: Vec<Vec<Value>> = (0..10)
            .map(|_| vec![Value::String("x".repeat(200))])
            .collect();
        let table = to_table(
            vec!["n".into()],
            rows,
            Bound {
                max_items: 1000,
                max_bytes: 500,
            },
        );
        assert!(
            table.bound.truncated,
            "10 rows of 200 bytes against a 500-byte ceiling"
        );
        assert!(table.bound.returned >= 1, "one row always crosses the wire");
        assert!(table.bound.returned < 10);
        assert_eq!(table.bound.total, 10);
    }

    #[test]
    fn no_row_limit_still_lands_on_the_ceiling() {
        // The clamp `run_cypher` applies, isolated: a client that names no
        // limit gets MAX_QUERY_ROWS, and one that names more gets the same.
        for requested in [None, Some(u32::MAX), Some(MAX_QUERY_ROWS as u32 + 1)] {
            let effective = requested
                .map(|n| n as usize)
                .unwrap_or(MAX_QUERY_ROWS)
                .min(MAX_QUERY_ROWS);
            assert_eq!(effective, MAX_QUERY_ROWS);
        }
    }

    #[test]
    fn identifiers_that_could_escape_the_query_are_refused() {
        // The injection surface: a property name reaches the query as syntax.
        for bad in [
            "name`",
            "name RETURN 1",
            "n.name",
            "",
            "1name",
            "name'",
            "name)-[:X]->(m",
        ] {
            assert!(
                validate_identifier(bad, "property").is_err(),
                "{bad:?} must be refused"
            );
        }
        for good in ["title", "name", "_private", "col_2"] {
            assert!(validate_identifier(good, "property").is_ok(), "{good:?}");
        }
    }

    #[test]
    fn graph_refs_are_pulled_out_of_lists_and_paths_too() {
        use kglite::api::{NodeValue, PathValue, RelValue};
        let node = |id: u32| NodeValue {
            id,
            labels: vec!["Person".into()],
            properties: Default::default(),
        };
        let rel = RelValue {
            id: 9,
            start_id: 1,
            end_id: 2,
            rel_type: "KNOWS".into(),
            properties: Default::default(),
        };
        let rows = vec![vec![
            Value::List(vec![Value::Node(Box::new(node(1)))]),
            Value::Path(Box::new(PathValue {
                nodes: vec![node(1), node(2)],
                rels: vec![rel],
            })),
        ]];
        let table = to_table(
            vec!["c".into(), "p".into()],
            rows,
            Bound {
                max_items: 10,
                max_bytes: 10_000,
            },
        );
        assert_eq!(table.node_ids, vec![1, 2], "deduplicated, first-seen order");
        assert_eq!(table.relationships.len(), 1);
        assert_eq!(table.relationships[0].name, "KNOWS");
    }
}
