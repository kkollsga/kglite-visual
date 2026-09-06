use kglite::api::DirGraph;
use kglite_visual_core::request::{CypherRequest, Request};
use kglite_visual_core::{Response, Session};
use std::collections::BTreeMap;
use std::sync::Arc;

fn query(
    session: &Session,
    text: &str,
    params: BTreeMap<String, serde_json::Value>,
) -> kglite_visual_core::query::QueryTable {
    match session
        .handle(&Request::Cypher(CypherRequest {
            query: text.into(),
            params,
            limit: None,
            as_graph: false,
        }))
        .unwrap()
    {
        Response::Query(table) => table,
        _ => panic!("query table expected"),
    }
}
#[test]
fn first_query_row_never_bypasses_serialized_response_ceiling() {
    let session = Session::open(Arc::new(DirGraph::new()), "query-bounds");
    let table = query(
        &session,
        "RETURN $value AS value",
        BTreeMap::from([("value".into(), serde_json::json!("\0".repeat(400_000)))]),
    );
    assert!(
        serde_json::to_vec(&table).unwrap().len() <= kglite_visual_core::query::MAX_QUERY_BYTES
    );
}
#[test]
fn scalar_large_integers_cross_json_losslessly() {
    let session = Session::open(Arc::new(DirGraph::new()), "query-values");
    let table = query(
        &session,
        "RETURN 9007199254740993 AS value",
        BTreeMap::new(),
    );
    assert_eq!(table.data[0][0], serde_json::json!("9007199254740993"));
}

fn graph_session() -> Session {
    let mut graph = DirGraph::new();
    kglite::api::session::execute_mut(&mut graph,"CREATE (a:P {id:9007199254740993,title:'A',score:0}) CREATE (b:P {id:9007199254740993,title:'B'}) CREATE (:P {id:null,title:'Null key'}) CREATE (a)-[:R]->(b) CREATE (a)-[:R]->(b) CREATE (b)-[:R]->(b)",&kglite::api::session::ExecuteOptions::eager(&Default::default())).unwrap();
    Session::open(Arc::new(graph), "provenance")
}
#[test]
fn numeric_id_columns_never_acquire_entity_references() {
    let session = graph_session();
    let before = session.shared_stamp();
    let table = query(
        &session,
        "MATCH (n:P) RETURN id(n),n.score",
        BTreeMap::new(),
    );
    assert_eq!(table.stamp, Some(before.clone()));
    assert_eq!(table.row_references.len(), table.bound.returned as usize);
    assert!(table
        .row_references
        .iter()
        .all(|row| row.nodes.is_empty() && row.relationships.is_empty() && !row.truncated));
    assert_eq!(session.shared_stamp(), before);
    assert_eq!(session.snapshot_shared().meta.subset.counts.loaded_nodes, 0);
}
#[test]
fn nested_entities_and_relationships_carry_only_real_source_handles() {
    let session = graph_session();
    let table = query(
        &session,
        "MATCH (a:P)-[r:R]->(b:P) RETURN {nested:[a,b]},r",
        BTreeMap::new(),
    );
    assert_eq!(table.row_references.len(), 3);
    let ids = table
        .row_references
        .iter()
        .flat_map(|row| row.relationships.iter().map(|relation| relation.edge_id))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), 3);
    for row in &table.row_references {
        assert!(!row.truncated);
        assert!(row
            .nodes
            .iter()
            .all(|handle| handle.generation == session.generation() && handle.node_id < 3));
        assert!(row
            .relationships
            .iter()
            .all(|relation| relation.generation == session.generation()));
    }
    let first = &table.row_references[0];
    let result = session
        .handle(&Request::LoadEntities(
            kglite_visual_core::query_provenance::LoadEntitiesRequest {
                nodes: first.nodes.clone(),
                relationships: first.relationships.clone(),
            },
        ))
        .unwrap();
    let Response::Slice(slice) = result else {
        panic!("slice")
    };
    assert_eq!(
        slice.meta.edges.iter().filter(|edge| !edge.meta).count(),
        1,
        "admission must not induce the other parallel source relations"
    );
    for row in &table.row_references[1..] {
        session
            .handle(&Request::LoadEntities(
                kglite_visual_core::query_provenance::LoadEntitiesRequest {
                    nodes: row.nodes.clone(),
                    relationships: row.relationships.clone(),
                },
            ))
            .unwrap();
    }
    assert_eq!(session.snapshot_shared().meta.subset.counts.loaded_edges, 3);
}
#[test]
fn relation_endpoint_spoof_and_foreign_generation_refuse_without_partial_loading() {
    let session = graph_session();
    let table = query(
        &session,
        "MATCH ()-[r:R]->() RETURN r LIMIT 1",
        BTreeMap::new(),
    );
    let before = session.snapshot_shared();
    let mut bad = table.row_references[0].relationships[0].clone();
    bad.target.node_id = 2;
    assert!(session
        .handle(&Request::LoadEntities(
            kglite_visual_core::query_provenance::LoadEntitiesRequest {
                nodes: vec![],
                relationships: vec![bad]
            }
        ))
        .is_err());
    let mut foreign = table.row_references[0].relationships[0].clone();
    foreign.generation = "foreign".into();
    assert!(session
        .handle(&Request::LoadEntities(
            kglite_visual_core::query_provenance::LoadEntitiesRequest {
                nodes: vec![],
                relationships: vec![foreign]
            }
        ))
        .is_err());
    assert_eq!(session.snapshot_shared(), before);
}
#[test]
fn aggregate_row_references_are_bounded_without_losing_graph_admission_guard() {
    let mut graph = DirGraph::new();
    kglite::api::session::execute_mut(
        &mut graph,
        "UNWIND range(0,99) AS i CREATE (:P {id:i,title:'row'})",
        &kglite::api::session::ExecuteOptions::eager(&Default::default()),
    )
    .unwrap();
    let session = Session::open(Arc::new(graph), "aggregate");
    let table = query(&session, "MATCH (n:P) RETURN collect(n)", BTreeMap::new());
    assert_eq!(table.row_references[0].nodes.len(), 64);
    assert!(table.row_references[0].truncated);
    assert_eq!(table.node_ids.len(), 100);
    assert!(!table.graph_references_truncated);
    assert_eq!(table.data[0][0]["state"], "truncated");
}
#[test]
fn source_search_handles_keep_null_and_duplicate_keys_addressable() {
    let session = graph_session();
    let request: Request =
        serde_json::from_value(serde_json::json!({"type":"search","query":"key"})).unwrap();
    let Response::Search(result) = session.handle(&request).unwrap() else {
        panic!("search")
    };
    assert_eq!(result.stamp, Some(session.shared_stamp()));
    assert_eq!(result.hits.len(), 1);
    let handle = result.hits[0].handle.clone().unwrap();
    let loaded = session
        .load_nodes(&kglite_visual_core::records::LoadNodesRequest {
            handles: vec![handle],
        })
        .unwrap();
    assert_eq!(
        loaded.meta.nodes[0].typed_key,
        kglite_visual_core::records::RecordCell::Null
    );
}

#[test]
fn mixed_numeric_and_digit_string_cells_keep_their_source_types() {
    use kglite_visual_core::records::{RecordCell, TypedValue};
    let session = Session::open(Arc::new(DirGraph::new()), "typed-query");
    let table = query(
        &session,
        "UNWIND [2,9007199254740993,'9007199254740993'] AS v RETURN v",
        BTreeMap::new(),
    );
    assert_eq!(
        table.cells[0],
        vec![
            RecordCell::Value {
                value: TypedValue::Int64("2".into())
            },
            RecordCell::Value {
                value: TypedValue::Int64("9007199254740993".into())
            },
            RecordCell::Value {
                value: TypedValue::String("9007199254740993".into())
            }
        ]
    );
    assert_eq!(table.data[0][0], serde_json::json!(2));
    assert_eq!(table.data[0][1], table.data[0][2]);
}
#[test]
fn source_search_long_labels_are_explicit_bounded_previews() {
    let mut graph = DirGraph::new();
    let params = [(
        "title".into(),
        kglite::api::Value::String("row".repeat(100_000)),
    )]
    .into_iter()
    .collect();
    kglite::api::session::execute_mut(
        &mut graph,
        "CREATE (:P {id:1,title:$title})",
        &kglite::api::session::ExecuteOptions::eager(&params),
    )
    .unwrap();
    let session = Session::open(Arc::new(graph), "long-search");
    let request: Request =
        serde_json::from_value(serde_json::json!({"type":"search","query":"row"})).unwrap();
    let Response::Search(result) = session.handle(&request).unwrap() else {
        panic!("search")
    };
    assert_eq!(result.hits.len(), 1);
    assert!(result.hits[0].label_truncated);
    assert_eq!(result.hits[0].label.chars().count(), 256);
    assert!(serde_json::to_vec(&result).unwrap().len() < 4096);
}
