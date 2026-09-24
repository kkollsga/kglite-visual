//! Engine fixes adopted with a kglite floor move, pinned through the viewer
//! routes they reach: the query route (`Session::handle` → `Request::Cypher`),
//! which the query panel, the `--cypher` render route and the MCP `cypher` tool
//! all take, and the property statistics that pick appearance channels.
//!
//! Each case is a silent wrong answer or a spurious refusal on the engine
//! release before the floor that adopted it, so a compile alone proves
//! nothing about it; every test here was run against that release and seen
//! red first.

use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::DirGraph;
use kglite_visual_core::query::QueryTable;
use kglite_visual_core::request::{CypherRequest, Request};
use kglite_visual_core::{Response, Session};
use serde_json::json;
use std::sync::Arc;

fn session(fixture: &str) -> Session {
    let mut graph = DirGraph::new();
    execute_mut(
        &mut graph,
        fixture,
        &ExecuteOptions::eager(&Default::default()),
    )
    .expect("fixture builds");
    Session::open(Arc::new(graph), "engine-fixes")
}

fn query(session: &Session, text: &str) -> QueryTable {
    match session
        .handle(&Request::Cypher(CypherRequest {
            query: text.into(),
            params: Default::default(),
            limit: None,
            as_graph: false,
        }))
        .unwrap_or_else(|e| panic!("{text}: {e:?}"))
    {
        Response::Query(table) => table,
        _ => panic!("query table expected"),
    }
}

/// kglite 0.17.11: a `CALL { }` body whose pattern reads the imported variable
/// through a property map ran as a graph-global fused scan with no per-row
/// seed, so the count answered 0 while the rows existed.
#[test]
fn call_body_counts_through_an_imported_property_map() {
    let s = session(
        "CREATE (:Note {id: 'n1'}) CREATE (:Chunk {note_id: 'n1'}) \
         CREATE (:Chunk {note_id: 'n1'})",
    );
    let table = query(
        &s,
        "MATCH (n:Note) CALL { WITH n MATCH (c:Chunk {note_id: n.id}) \
         RETURN count(c) AS k } RETURN k",
    );
    assert_eq!(table.data[0], [json!(2)]);
}

/// kglite 0.18.0: on a MATCH variable with no stored `id` / `start` / `end`,
/// `r.id`, `r.start` and `` r.`end` `` answered null; they now fall back to the
/// relationship's envelope, the rule `n.type` already follows on a node.
#[test]
fn relationship_envelope_keys_fall_back_on_a_match_variable() {
    let s = session("CREATE (:P {id: 'a'})-[:KNOWS]->(:P {id: 'b'})");
    let table = query(
        &s,
        "MATCH (a)-[r:KNOWS]->(b) \
         RETURN r.id = id(r) AS rid, r.start = id(a) AS rs, r.`end` = id(b) AS re",
    );
    assert_eq!(table.columns, ["rid", "rs", "re"]);
    for column in &table.data {
        assert_eq!(column, &[json!(true)], "{:?}", table.data);
    }
}

/// kglite 0.18.0: `labels(x)[i]` answered null for a node carried as a value
/// (here `startNode(r)`) while `head(labels(x))` was right.
#[test]
fn labels_index_reads_a_node_value() {
    let s = session("CREATE (:P {id: 'a'})-[:KNOWS]->(:P {id: 'b'})");
    let table = query(
        &s,
        "MATCH ()-[r:KNOWS]->() WITH startNode(r) AS s RETURN labels(s)[0] AS l",
    );
    assert_eq!(table.data[0], [json!("P")]);
}

/// kglite 0.18.0: `type()` and `startNode()` returned null on a relationship
/// that arrived as a value (`collect(r)[0]`) rather than a MATCH binding.
#[test]
fn relationship_functions_read_a_relationship_value() {
    let s = session("CREATE (:P {id: 'a'})-[:KNOWS]->(:P {id: 'b'})");
    let table = query(
        &s,
        "MATCH ()-[r:KNOWS]->() WITH collect(r)[0] AS r \
         RETURN type(r) AS t, startNode(r).id AS s",
    );
    assert_eq!(table.data[0], [json!("KNOWS")]);
    assert_eq!(table.data[1], [json!("a")]);
}

/// kglite 0.18.0: `shortestPath` ignored its hop bounds, so `*..2` returned
/// a three-hop path. A written maximum now bounds the search.
#[test]
fn shortest_path_honours_its_maximum_hop_bound() {
    let s = session(
        "CREATE (:S {id: 'a'})-[:R]->(:S {id: 'b'})-[:R]->(:S {id: 'c'})-[:R]->(:S {id: 'd'})",
    );
    let table = query(
        &s,
        "MATCH p = shortestPath((x:S {id: 'a'})-[:R*..2]->(y:S {id: 'd'})) \
         RETURN length(p) AS hops",
    );
    assert_eq!(
        table.data[0],
        Vec::<serde_json::Value>::new(),
        "{:?}",
        table.data
    );
}

/// kglite 0.18.0: a MATCH inline property map accepts any expression a
/// `CREATE` map does. `{id: row[0]}` failed to parse with "Expected property
/// key or '}'".
#[test]
fn match_property_map_accepts_an_index_expression() {
    let s = session("CREATE (:D {id: 'x'}) CREATE (:D {id: 'y'})");
    let table = query(
        &s,
        "UNWIND [['y']] AS row MATCH (d:D {id: row[0]}) RETURN d.id AS id",
    );
    assert_eq!(table.data[0], [json!("y")]);
}

/// kglite 0.18.0: a property's recorded type followed its last write, so a
/// string column that one later `SET` gave an integer reported `Int64` and
/// this viewer offered it as a numeric size/colour channel whose domain was the
/// one integer. It now reports `mixed`, which is not numeric, so the column is
/// offered as the categorical channel its values are.
#[test]
fn a_property_holding_two_value_types_is_not_a_numeric_channel() {
    use kglite_visual_core::stats::{property_stats, AppearanceRole};

    let mut graph = DirGraph::new();
    for statement in [
        "CREATE (:T {id: 1, p: 'a'}) CREATE (:T {id: 2, p: 'b'}) CREATE (:T {id: 3, p: 'c'})",
        "MATCH (n:T {id: 3}) SET n.p = 42",
    ] {
        execute_mut(
            &mut graph,
            statement,
            &ExecuteOptions::eager(&Default::default()),
        )
        .expect("fixture builds");
    }
    let stats = property_stats(&graph, "T").expect("stats compute");
    let p = stats
        .properties
        .iter()
        .find(|stat| stat.name == "p")
        .expect("p is listed");
    assert_ne!(p.value_type, "Int64", "{p:?}");
    assert_eq!(p.role, AppearanceRole::Categorical, "{p:?}");
    assert!(!stats.numeric_candidates.contains(&"p".to_string()));
}
