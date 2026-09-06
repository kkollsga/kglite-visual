use std::collections::BTreeMap;
use std::sync::Arc;

use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::DirGraph;
use kglite_visual_core::control::{
    AppearanceRequest, FocusRequest, HighlightConcept, HighlightRequest,
};
use kglite_visual_core::records::{
    BrowseTypeRequest, LoadNodesRequest, RecordsRequest, TypedValue,
};
use kglite_visual_core::request::{
    CypherRequest, LayoutKernel, LayoutRequest, Request, SlotRequest,
};
use kglite_visual_core::shared::{CaptionRequest, SharedRequest, ViewReference};
use kglite_visual_core::subset::{FieldRef, SubsetFilter, SubsetPredicate, SubsetRequest};
use kglite_visual_core::{CoreError, Session};

fn session() -> Session {
    let mut graph = DirGraph::new();
    execute_mut(&mut graph, "CREATE (a:P {id:9007199254740993,title:'A',score:0,flag:false}) CREATE (b:P {id:9007199254740992,title:'B',score:2,flag:true}) CREATE (c:Q {title:'isolated'}) CREATE (a)-[:R]->(b) CREATE (a)-[:R]->(b) CREATE (b)-[:S]->(b)", &ExecuteOptions::eager(&Default::default())).unwrap();
    Session::open(Arc::new(graph), "shared-test")
}
fn load(session: &Session) {
    session
        .handle(&Request::Cypher(CypherRequest {
            query: "MATCH (a)-[r]->(b) RETURN a,r,b".into(),
            params: BTreeMap::new(),
            limit: None,
            as_graph: true,
        }))
        .unwrap();
    session
        .browse_type(&BrowseTypeRequest {
            node_type: "Q".into(),
            limit: None,
        })
        .unwrap();
}
fn apply(session: &Session, request: Request) {
    session
        .apply_shared(&SharedRequest {
            request,
            expected: Some(session.shared_stamp()),
            request_id: Some("test".into()),
        })
        .unwrap();
}
fn predicate(id: &str, predicate: SubsetPredicate) -> SubsetFilter {
    SubsetFilter {
        id: id.into(),
        enabled: true,
        predicate,
    }
}
fn subset(session: &Session, predicates: Vec<SubsetFilter>) {
    apply(session, Request::Subset(SubsetRequest { predicates }));
}
fn field(name: &str) -> FieldRef {
    FieldRef::Property { name: name.into() }
}

#[test]
fn stale_prepared_layout_cannot_overwrite_newer_membership_or_settings() {
    let session = session();
    load(&session);
    let before = session.shared_stamp();
    let old = session
        .prepare_shared(&SharedRequest {
            request: Request::Layout(LayoutRequest {
                kernel: LayoutKernel::Force,
                seed_slot: None,
            }),
            expected: Some(before.clone()),
            request_id: None,
        })
        .unwrap();
    apply(
        &session,
        Request::Appearance(AppearanceRequest {
            color_by: Some("score".into()),
            size_by: None,
        }),
    );
    let committed = session.snapshot_shared();
    assert!(matches!(
        session.commit_shared(old),
        Err(CoreError::Conflict(_))
    ));
    assert_eq!(session.snapshot_shared(), committed);
    assert!(matches!(
        session.prepare_shared(&SharedRequest {
            request: Request::Reset,
            expected: Some(before),
            request_id: None
        }),
        Err(CoreError::Conflict(_))
    ));
}

#[test]
fn two_preparations_from_one_base_admit_exactly_one() {
    let session = Arc::new(session());
    let a = session
        .prepare_shared(&SharedRequest::new(Request::Caption(CaptionRequest {
            caption_by: Some("title".into()),
        })))
        .unwrap();
    let b = session
        .prepare_shared(&SharedRequest::new(Request::Appearance(
            AppearanceRequest {
                color_by: Some("score".into()),
                size_by: None,
            },
        )))
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let workers = [a, b]
        .into_iter()
        .map(|prepared| {
            let session = session.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                session.commit_shared(prepared)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(CoreError::Conflict(_))))
            .count(),
        1
    );
    assert_eq!(session.shared_stamp().revision, "1");
}

#[test]
fn direct_mutators_commit_but_records_and_sync_do_not() {
    let session = session();
    let slice = session
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: Some(1),
        })
        .unwrap();
    assert_eq!(session.shared_stamp().revision, "1");
    session
        .load_nodes(&LoadNodesRequest {
            handles: vec![slice.meta.nodes[0].handle.clone()],
        })
        .unwrap();
    assert_eq!(session.shared_stamp().revision, "2");
    let records = session
        .records(&RecordsRequest {
            handles: vec![slice.meta.nodes[0].handle.clone()],
            fields: vec!["id".into()],
            offset: 0,
            limit: 10,
        })
        .unwrap();
    assert_eq!(records.stamp, session.shared_stamp());
    assert!(records.rows[0].visible);
    session.sync_slice();
    session.view_state();
    session.snapshot_shared();
    assert_eq!(session.shared_stamp().revision, "2");
    session.reset().unwrap();
    assert_eq!(session.shared_stamp().revision, "3");
    assert_eq!(session.snapshot_shared().meta.subset.counts.loaded_nodes, 0);
}

#[test]
fn relation_filters_preserve_parallel_records_and_isolates_until_explicitly_hidden() {
    let session = session();
    load(&session);
    let all = session.snapshot_shared();
    assert_eq!(all.meta.subset.counts.loaded_nodes, 3);
    assert_eq!(all.meta.subset.counts.loaded_edges, 3);
    subset(
        &session,
        vec![predicate(
            "rel",
            SubsetPredicate::Relation {
                names: vec!["R".into()],
            },
        )],
    );
    let filtered = session.snapshot_shared();
    assert_eq!(filtered.meta.subset.counts.visible_nodes, 3);
    assert_eq!(filtered.meta.subset.counts.visible_edges, 2);
    assert_eq!(filtered.meta.topology_revision, all.meta.topology_revision);
    subset(
        &session,
        vec![
            predicate(
                "rel",
                SubsetPredicate::Relation {
                    names: vec!["S".into()],
                },
            ),
            predicate("isolate", SubsetPredicate::HideIsolated),
        ],
    );
    let loop_only = session.snapshot_shared();
    assert_eq!(loop_only.meta.subset.counts.visible_nodes, 1);
    assert_eq!(loop_only.meta.subset.counts.visible_edges, 1);
    assert_eq!(loop_only.meta.subset.counts.loaded_nodes, 3);
}

#[test]
fn exact_numeric_zero_false_and_missing_filters_have_conditioned_counts() {
    let session = session();
    load(&session);
    subset(
        &session,
        vec![predicate(
            "large",
            SubsetPredicate::NumericRange {
                field: field("id"),
                min: Some(TypedValue::Int64("9007199254740993".into())),
                max: None,
                include_null: false,
                include_missing: false,
            },
        )],
    );
    let exact = session.snapshot_shared();
    assert_eq!(exact.meta.subset.counts.visible_nodes, 1);
    assert_eq!(exact.meta.subset.distributions[0].input_nodes, 3);
    assert_eq!(
        exact.meta.subset.distributions[0].min,
        Some(TypedValue::UniqueId("2".into()))
    );
    subset(
        &session,
        vec![
            predicate(
                "zero",
                SubsetPredicate::NumericRange {
                    field: field("score"),
                    min: Some(TypedValue::Int64("0".into())),
                    max: Some(TypedValue::Float64(0.0)),
                    include_null: false,
                    include_missing: false,
                },
            ),
            predicate(
                "false",
                SubsetPredicate::Category {
                    field: field("flag"),
                    values: vec![TypedValue::Boolean(false)],
                    include_null: false,
                    include_missing: false,
                },
            ),
        ],
    );
    let zero = session.snapshot_shared();
    assert_eq!(zero.meta.subset.counts.visible_nodes, 1);
    assert_eq!(zero.meta.subset.distributions[0].input_nodes, 1);
    subset(
        &session,
        vec![predicate(
            "missing",
            SubsetPredicate::Missing {
                field: field("score"),
                include_null: false,
                include_missing: true,
            },
        )],
    );
    assert_eq!(
        session.snapshot_shared().meta.subset.counts.visible_nodes,
        1
    );
}

#[test]
fn appearance_highlight_and_caption_snapshot_are_coherent_and_hide_retains_identity() {
    let session = session();
    load(&session);
    let initial = session.snapshot_shared();
    let node = initial.meta.slice.nodes[0].clone();
    apply(
        &session,
        Request::Highlight(HighlightRequest {
            slots: vec![node.slot],
            concept: HighlightConcept::Highlighted,
        }),
    );
    apply(
        &session,
        Request::Caption(CaptionRequest {
            caption_by: Some("title".into()),
        }),
    );
    apply(
        &session,
        Request::Appearance(AppearanceRequest {
            color_by: Some("score".into()),
            size_by: Some("score".into()),
        }),
    );
    apply(
        &session,
        Request::Focus(FocusRequest {
            slots: vec![node.slot],
        }),
    );
    let decorated = session.snapshot_shared();
    assert_eq!(decorated.meta.subset_revision, initial.meta.subset_revision);
    assert_eq!(
        decorated.meta.topology_revision,
        initial.meta.topology_revision
    );
    assert_eq!(decorated.meta.caption_by.as_deref(), Some("title"));
    assert_eq!(decorated.meta.appearance.color_by.as_deref(), Some("score"));
    subset(
        &session,
        vec![predicate(
            "none",
            SubsetPredicate::Type { node_types: vec![] },
        )],
    );
    assert_eq!(
        session.snapshot_shared().meta.highlighted,
        vec![ViewReference::Node {
            handle: node.handle
        }]
    );
    session
        .handle(&Request::Collapse(SlotRequest { slot: node.slot }))
        .unwrap();
    assert!(session.snapshot_shared().meta.highlighted.is_empty());
}

#[test]
fn topology_change_invalidates_static_layout_in_the_same_commit() {
    let session = session();
    load(&session);
    apply(
        &session,
        Request::Layout(LayoutRequest {
            kernel: LayoutKernel::Force,
            seed_slot: None,
        }),
    );
    assert!(session.snapshot_shared().meta.layout.is_some());
    session.reset().unwrap();
    let snapshot = session.snapshot_shared();
    assert_eq!(snapshot.meta.layout_kernel, LayoutKernel::Simulation);
    assert!(snapshot.meta.layout.is_none());
    assert_eq!(
        snapshot.points.len(),
        snapshot.meta.slice.slot_count as usize * 2
    );
}

#[test]
fn invalid_generation_predicate_and_request_id_refuse_without_partial_mutation() {
    let session = session();
    load(&session);
    let before = session.snapshot_shared();
    let mut foreign = session.shared_stamp();
    foreign.generation = "foreign".into();
    assert!(matches!(
        session.apply_shared(&SharedRequest {
            request: Request::Reset,
            expected: Some(foreign),
            request_id: None
        }),
        Err(CoreError::Conflict(_))
    ));
    assert!(session
        .apply_shared(&SharedRequest {
            request: Request::Reset,
            expected: None,
            request_id: Some("x".repeat(129))
        })
        .is_err());
    assert!(session
        .handle(&Request::Subset(SubsetRequest {
            predicates: vec![predicate(
                "bad",
                SubsetPredicate::NumericRange {
                    field: field("score"),
                    min: Some(TypedValue::Int64("8".into())),
                    max: Some(TypedValue::Int64("2".into())),
                    include_null: false,
                    include_missing: false
                }
            )]
        }))
        .is_err());
    assert_eq!(session.snapshot_shared(), before);
}

#[test]
fn browser_fixtures_really_have_null_keys_parallel_records_and_summary_tier() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let identity =
        kglite::api::io::load_file(root.join("viewer-identity.kgl").to_str().unwrap()).unwrap();
    let identity = Session::open(identity, "identity-fixture");
    let people = identity
        .browse_type(&BrowseTypeRequest {
            node_type: "Person".into(),
            limit: None,
        })
        .unwrap();
    assert_eq!(people.meta.nodes.len(), 3);
    assert_eq!(
        people
            .meta
            .nodes
            .iter()
            .find(|node| node.title == "Null key")
            .unwrap()
            .typed_key,
        kglite_visual_core::records::RecordCell::Null
    );
    assert_eq!(
        people
            .meta
            .nodes
            .iter()
            .filter(|node| node.typed_key
                == kglite_visual_core::records::RecordCell::Value {
                    value: TypedValue::Int64("9007199254740993".into())
                })
            .count(),
        2
    );
    let summary =
        kglite::api::io::load_file(root.join("viewer-summary.kgl").to_str().unwrap()).unwrap();
    let summary = Session::open(summary, "summary-fixture");
    assert_eq!(summary.info().tier, kglite_visual_core::DetailTier::Summary);
    assert_eq!(summary.info().stats.core_type_count, 5001);
    assert_eq!(
        summary
            .browse_type(&BrowseTypeRequest {
                node_type: "Type5000".into(),
                limit: None
            })
            .unwrap()
            .meta
            .nodes
            .len(),
        1
    );
}

#[test]
fn large_conditioned_distributions_refuse_the_whole_shared_event_atomically() {
    let mut graph = DirGraph::new();
    let params = [(
        "prefix".into(),
        kglite::api::Value::String("x".repeat(3900)),
    )]
    .into_iter()
    .collect();
    for kind in 0..32 {
        execute_mut(&mut graph,&format!("UNWIND range(0,63) AS i CREATE (:T{kind} {{id:i,title:'row',field{kind}:$prefix + toString(i)}})"),&ExecuteOptions::eager(&params)).unwrap();
    }
    let session = Session::open(Arc::new(graph), "distribution-bound");
    for kind in 0..32 {
        session
            .browse_type(&BrowseTypeRequest {
                node_type: format!("T{kind}"),
                limit: None,
            })
            .unwrap();
    }
    let before = session.snapshot_shared();
    let predicates = (0..32)
        .map(|kind| {
            predicate(
                &format!("missing{kind}"),
                SubsetPredicate::Missing {
                    field: field(&format!("field{kind}")),
                    include_null: false,
                    include_missing: true,
                },
            )
        })
        .collect();
    let error = session
        .handle(&Request::Subset(SubsetRequest { predicates }))
        .unwrap_err();
    assert!(error.to_string().contains("byte limit"), "{error}");
    assert_eq!(session.snapshot_shared(), before);
}
