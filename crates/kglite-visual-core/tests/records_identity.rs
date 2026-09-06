use std::collections::BTreeMap;
use std::sync::Arc;

use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::{DirGraph, Value};
use kglite_visual_core::records::{
    self, BrowseTypeRequest, LoadNodesRequest, RecordCell, RecordsRequest, TypedValue,
};
use kglite_visual_core::request::{
    CypherRequest, EdgeDirection, ExpandRequest, Request, SlotRequest,
};
use kglite_visual_core::{Response, Session};

fn build_session(script: &str) -> Session {
    let mut graph = DirGraph::new();
    execute_mut(
        &mut graph,
        script,
        &ExecuteOptions::eager(&Default::default()),
    )
    .unwrap();
    Session::open(Arc::new(graph), "records-test")
}

fn query(text: &str) -> Request {
    Request::Cypher(CypherRequest {
        query: text.into(),
        params: BTreeMap::new(),
        limit: None,
        as_graph: true,
    })
}

#[test]
fn direct_records_keep_large_keys_zero_false_empty_missing_and_null() {
    let session = build_session("CREATE (:P {id: 9007199254740993, title: 'A', score: 0, flag: false, empty: ''}) CREATE (:Q {id:9007199254740993}) CREATE (:P {id:9007199254740993, title:'duplicate'}) CREATE (:P {title:'keyless'})");
    let slice = session
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: None,
        })
        .unwrap();
    let handle = slice
        .meta
        .nodes
        .iter()
        .find(|node| node.title == "A")
        .unwrap()
        .handle
        .clone();
    let table = session
        .records(&RecordsRequest {
            handles: vec![handle],
            fields: vec![
                "id".into(),
                "score".into(),
                "flag".into(),
                "empty".into(),
                "absent".into(),
            ],
            offset: 0,
            limit: 100,
        })
        .unwrap();
    assert_eq!(
        table.rows[0].cells,
        vec![
            RecordCell::Value {
                value: TypedValue::Int64("9007199254740993".into())
            },
            RecordCell::Value {
                value: TypedValue::Int64("0".into())
            },
            RecordCell::Value {
                value: TypedValue::Boolean(false)
            },
            RecordCell::Value {
                value: TypedValue::String("".into())
            },
            RecordCell::Missing,
        ]
    );
    assert_eq!(records::cell(&Value::Null), RecordCell::Null);
    assert_eq!(slice.meta.nodes.len(), 3);
    assert!(
        slice
            .meta
            .nodes
            .iter()
            .find(|node| node.title == "A")
            .unwrap()
            .key
            .is_none(),
        "legacy JSON must not expose a rounded identity"
    );
    let foreign = build_session("CREATE (:P {id:9007199254740993})");
    assert!(foreign
        .records(&RecordsRequest {
            handles: vec![table.rows[0].handle.clone()],
            fields: vec![],
            offset: 0,
            limit: 100
        })
        .unwrap_err()
        .to_string()
        .contains("generation"));
}

#[test]
fn parallel_relations_and_self_loops_survive_query_expand_and_repeat() {
    let session = build_session("CREATE (a:P {id:1}) CREATE (b:P {id:2}) CREATE (a)-[:R]->(b) CREATE (a)-[:R]->(b) CREATE (a)-[:R]->(a)");
    let Response::Slice(slice) = session
        .handle(&query("MATCH (a)-[r:R]->(b) RETURN a,r,b"))
        .unwrap()
    else {
        panic!()
    };
    let ids: std::collections::HashSet<_> = slice
        .meta
        .edges
        .iter()
        .filter_map(|edge| edge.edge_id)
        .collect();
    assert_eq!(ids.len(), 3);
    let request = Request::Expand(ExpandRequest {
        slot: session.slot_of_type("P").unwrap(),
        relationship: Some("R".into()),
        direction: EdgeDirection::Both,
        limit: None,
    });
    session.handle(&request).unwrap();
    session.handle(&request).unwrap();
    let edges = session.sync_slice().meta.edges;
    assert_eq!(edges.iter().filter(|edge| !edge.meta).count(), 3);
    assert_eq!(
        edges
            .iter()
            .filter(|edge| !edge.meta && edge.source_slot == edge.target_slot)
            .count(),
        1
    );
    let relation_only = build_session("CREATE (a:P) CREATE (b:P) CREATE (a)-[:R]->(b)");
    let Response::Slice(slice) = relation_only
        .handle(&query("MATCH ()-[r:R]->() RETURN r"))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        slice.meta.nodes.len(),
        2,
        "relationship values carry endpoint provenance"
    );
    assert_eq!(slice.meta.edges.iter().filter(|edge| !edge.meta).count(), 1);
}

#[test]
fn compaction_does_not_redirect_a_record_handle() {
    let session = build_session("UNWIND range(0,99) AS i CREATE (:P {id:i, title:toString(i)})");
    let slice = session
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: None,
        })
        .unwrap();
    let survivor = slice.meta.nodes.last().unwrap().handle.clone();
    let old_slot = slice.meta.nodes.last().unwrap().slot;
    for _ in 0..50 {
        let first = session.sync_slice().meta.nodes[0].slot;
        session
            .handle(&Request::Collapse(SlotRequest { slot: first }))
            .unwrap();
    }
    let table = session
        .records(&RecordsRequest {
            handles: vec![survivor.clone()],
            fields: vec!["id".into()],
            offset: 0,
            limit: 100,
        })
        .unwrap();
    assert_eq!(table.rows[0].handle, survivor);
    assert_ne!(table.rows[0].slot, Some(old_slot));
    assert_eq!(
        table.rows[0].cells[0],
        RecordCell::Value {
            value: TypedValue::Int64("99".into())
        }
    );
}

#[test]
fn aggregate_and_repeated_additions_cannot_bypass_the_loaded_node_ceiling() {
    let session = build_session("UNWIND range(0,5000) AS i CREATE (:P {id:i})");
    let before = session.sync_slice();
    assert!(session
        .handle(&query("MATCH (n:P) RETURN collect(n)"))
        .unwrap_err()
        .to_string()
        .contains("5000"));
    assert_eq!(session.sync_slice(), before);
    let first = session
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: Some(5000),
        })
        .unwrap();
    let remaining = (0..5001)
        .find(|id| !first.meta.nodes.iter().any(|node| node.node_id == *id))
        .unwrap();
    let before = session.sync_slice();
    assert!(session
        .load_nodes(&LoadNodesRequest {
            handles: vec![session.node_handle(remaining)]
        })
        .is_err());
    assert_eq!(session.sync_slice(), before);
}

#[test]
fn cell_preview_and_record_page_limits_are_explicit() {
    assert!(matches!(
        records::cell(&Value::String("x".repeat(5000))),
        RecordCell::Truncated { .. }
    ));
    assert!(matches!(
        records::cell(&Value::List(vec![Value::Int64(0); 65])),
        RecordCell::Truncated { .. }
    ));
    assert!(matches!(
        records::cell(&Value::Float64(f64::NAN)),
        RecordCell::Unavailable { .. }
    ));
    let session = build_session("UNWIND range(0,599) AS i CREATE (:P {id:i})");
    let slice = session
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: None,
        })
        .unwrap();
    let request = RecordsRequest {
        handles: slice
            .meta
            .nodes
            .into_iter()
            .map(|node| node.handle)
            .collect(),
        fields: vec!["id".into()],
        offset: 0,
        limit: u32::MAX,
    };
    let table = session.records(&request).unwrap();
    assert_eq!(table.rows.len(), 500);
    assert_eq!(table.next_offset, Some(500));
    assert!(table.bound.truncated);
    assert!(serde_json::to_vec(&table).unwrap().len() <= records::MAX_RECORD_BYTES);
    let mut wide = request;
    wide.fields = vec!["id".into(); 33];
    assert!(session.records(&wide).is_err());
}

#[test]
fn serialized_view_byte_refusal_leaves_membership_unchanged() {
    let mut graph = DirGraph::new();
    let params = [("title".to_string(), Value::String("\"".repeat(2500)))]
        .into_iter()
        .collect();
    execute_mut(
        &mut graph,
        "UNWIND range(0,499) AS i CREATE (:P {id:i, title:$title})",
        &ExecuteOptions::eager(&params),
    )
    .unwrap();
    let session = Session::open(Arc::new(graph), "escaped-titles");
    let before = session.sync_slice();
    let error = session
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: Some(500),
        })
        .unwrap_err();
    assert!(error.to_string().contains("byte limit"), "{error}");
    assert_eq!(session.sync_slice(), before);
}

#[test]
fn relation_count_refusal_leaves_membership_unchanged() {
    let session = build_session("CREATE (a:P {id:1}) CREATE (b:P {id:2}) WITH a,b UNWIND range(0,20000) AS i CREATE (a)-[:R]->(b)");
    let before = session.sync_slice();
    let error = session
        .handle(&query("MATCH (a)-[r:R]->(b) RETURN a,b,collect(r)"))
        .unwrap_err();
    assert!(error.to_string().contains("20000"), "{error}");
    assert_eq!(session.sync_slice(), before);
}

#[test]
fn legacy_detail_bounds_wide_and_long_properties_and_keeps_nested_integers_exact() {
    let mut graph = DirGraph::new();
    let fields = (0..40)
        .map(|i| format!("p{i}:{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let params = [("blob".to_string(), Value::String("x".repeat(20_000)))]
        .into_iter()
        .collect();
    execute_mut(
        &mut graph,
        &format!("CREATE (:P {{id:1, blob:$blob, numbers:[9007199254740993], {fields}}})"),
        &ExecuteOptions::eager(&params),
    )
    .unwrap();
    let session = Session::open(Arc::new(graph), "wide");
    let slice = session
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: None,
        })
        .unwrap();
    let Response::NodeDetail(detail) = session
        .handle(&Request::NodeDetail(SlotRequest {
            slot: slice.meta.nodes[0].slot,
        }))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(detail.properties.len(), 32);
    assert!(detail.property_bound.truncated);
    let blob = &detail
        .properties
        .iter()
        .find(|(name, _)| name == "blob")
        .unwrap()
        .1;
    assert_eq!(blob["state"], "truncated");
    let numbers = &detail
        .properties
        .iter()
        .find(|(name, _)| name == "numbers")
        .unwrap()
        .1;
    assert_eq!(numbers, &serde_json::json!(["9007199254740993"]));
    assert!(serde_json::to_vec(&detail).unwrap().len() < records::MAX_RECORD_BYTES);
}
