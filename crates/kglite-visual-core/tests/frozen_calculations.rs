use std::sync::Arc;

use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::{DirGraph, EdgeIndex, GraphRead};
use kglite_visual_core::calculations::{CalculateRequest, CalculationKind, CalculationMeta};
use kglite_visual_core::control::FocusRequest;
use kglite_visual_core::query_provenance::{LoadEntitiesRequest, RelationHandle};
use kglite_visual_core::records::{LoadNodesRequest, RecordCell, RecordsRequest, TypedValue};
use kglite_visual_core::request::Request;
use kglite_visual_core::shared::SharedRequest;
use kglite_visual_core::subset::{FieldRef, SubsetFilter, SubsetPredicate, SubsetRequest};
use kglite_visual_core::{CoreError, Session};

fn graph() -> DirGraph {
    let mut graph = DirGraph::new();
    execute_mut(&mut graph, "CREATE (a:P {id:10,title:'A',total:99}) CREATE (b:P {id:20,title:'B',total:98}) CREATE (c:P {id:30,title:'C',total:97}) CREATE (d:Disconnected {id:40,title:'D',total:96}) CREATE (a)-[:R]->(b) CREATE (a)-[:R]->(b) CREATE (b)-[:S]->(b) CREATE (b)-[:OMITTED]->(c)", &ExecuteOptions::eager(&Default::default())).unwrap();
    graph
}
fn load(session: &Session) {
    session
        .handle(&Request::LoadNodes(LoadNodesRequest {
            handles: (0..4).map(|id| session.node_handle(id)).collect(),
        }))
        .unwrap();
    let relationships = (0..3)
        .map(|id| {
            let (source, target) = session
                .graph()
                .graph
                .edge_endpoints(EdgeIndex::new(id))
                .unwrap();
            RelationHandle {
                generation: session.generation().into(),
                edge_id: id as u32,
                source: session.node_handle(source.index() as u32),
                target: session.node_handle(target.index() as u32),
            }
        })
        .collect();
    session
        .handle(&Request::LoadEntities(LoadEntitiesRequest {
            nodes: Vec::new(),
            relationships,
        }))
        .unwrap();
}
fn fixture() -> Session {
    let session = Session::open(Arc::new(graph()), "calculations");
    load(&session);
    session
}
fn calculate(session: &Session, kind: CalculationKind, id: Option<String>) -> CalculationMeta {
    session
        .handle(&Request::Calculate(CalculateRequest {
            kind,
            calculation_id: id.clone(),
        }))
        .unwrap();
    let calculations = session.snapshot_shared().meta.calculations;
    id.and_then(|id| calculations.iter().find(|meta| meta.id == id).cloned())
        .unwrap_or_else(|| calculations.last().unwrap().clone())
}
fn field(meta: &CalculationMeta, column: &str) -> FieldRef {
    FieldRef::Derived {
        calculation_id: meta.id.clone(),
        column: column.into(),
    }
}
fn values(session: &Session, fields: Vec<FieldRef>) -> Vec<Vec<RecordCell>> {
    session
        .records(&RecordsRequest {
            handles: (0..4).map(|id| session.node_handle(id)).collect(),
            fields: Vec::new(),
            field_refs: Some(fields),
            offset: 0,
            limit: 100,
        })
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.cells)
        .collect()
}
fn int(value: u32) -> RecordCell {
    RecordCell::Value {
        value: TypedValue::Int64(value.to_string()),
    }
}
fn filters(session: &Session, predicates: Vec<SubsetPredicate>) {
    session
        .handle(&Request::Subset(SubsetRequest {
            predicates: predicates
                .into_iter()
                .enumerate()
                .map(|(i, predicate)| SubsetFilter {
                    id: i.to_string(),
                    enabled: true,
                    predicate,
                })
                .collect(),
        }))
        .unwrap();
}

#[test]
fn exact_visible_multiset_and_namespaces_reach_records() {
    let session = fixture();
    let input = session.snapshot_shared();
    let degree = calculate(&session, CalculationKind::Degree, None);
    assert_eq!(degree.input_stamp, input.meta.stamp);
    assert_eq!(degree.input_subset_revision, input.meta.subset_revision);
    assert_eq!((degree.node_count, degree.edge_count), (4, 3));
    assert_eq!(
        values(
            &session,
            vec![
                field(&degree, "in"),
                field(&degree, "out"),
                field(&degree, "total")
            ]
        ),
        vec![
            vec![int(0), int(2), int(2)],
            vec![int(3), int(1), int(4)],
            vec![int(0), int(0), int(0)],
            vec![int(0), int(0), int(0)]
        ]
    );
    let components = calculate(&session, CalculationKind::WeakComponents, None);
    assert_eq!(
        values(
            &session,
            vec![
                field(&components, "component_id"),
                field(&components, "component_size")
            ]
        ),
        vec![
            vec![int(1), int(2)],
            vec![int(1), int(2)],
            vec![int(2), int(1)],
            vec![int(3), int(1)]
        ]
    );
    let table = session
        .records(&RecordsRequest {
            handles: vec![session.node_handle(0)],
            fields: vec!["total".into()],
            field_refs: Some(vec![field(&degree, "total")]),
            offset: 0,
            limit: 1,
        })
        .unwrap();
    assert_eq!(
        table.columns[0].field,
        FieldRef::Property {
            name: "total".into()
        }
    );
    assert_eq!(table.columns[1].field, field(&degree, "total"));
    assert_eq!(table.rows[0].cells, vec![int(99), int(2)]);
    assert_eq!(session.graph().graph.edge_count(), 4);
}

#[test]
fn frozen_values_survive_filters_and_explicit_recompute_replaces_same_fields_once() {
    let session = fixture();
    let degree = calculate(&session, CalculationKind::Degree, None);
    filters(
        &session,
        vec![SubsetPredicate::Relation {
            names: vec!["R".into()],
        }],
    );
    assert_eq!(
        values(&session, vec![field(&degree, "total")])[1],
        vec![int(4)]
    );
    assert_eq!(session.snapshot_shared().meta.calculations[0], degree);
    let recomputed = calculate(&session, CalculationKind::Degree, Some(degree.id.clone()));
    assert_eq!(recomputed.id, degree.id);
    assert_eq!(recomputed.edge_count, 2);
    assert_eq!(
        values(&session, vec![field(&degree, "total")])[1],
        vec![int(2)]
    );
    filters(
        &session,
        vec![SubsetPredicate::NumericRange {
            field: field(&degree, "total"),
            min: Some(TypedValue::Int64("2".into())),
            max: None,
            include_null: false,
            include_missing: false,
        }],
    );
    assert_eq!(
        session.snapshot_shared().meta.subset.counts.visible_nodes,
        2
    );
    assert_eq!(session.snapshot_shared().meta.calculations.len(), 1);
}

#[test]
fn outside_frozen_input_is_unavailable_and_does_not_match_missing_predicate() {
    let session = fixture();
    filters(
        &session,
        vec![SubsetPredicate::Category {
            field: FieldRef::Property {
                name: "title".into(),
            },
            values: vec![TypedValue::String("A".into())],
            include_null: false,
            include_missing: false,
        }],
    );
    let degree = calculate(&session, CalculationKind::Degree, None);
    filters(&session, Vec::new());
    let cells = values(&session, vec![field(&degree, "total")]);
    assert_eq!(cells[0], vec![int(0)]);
    assert!(matches!(&cells[1][0], RecordCell::Unavailable {reason} if reason.contains("outside")));
    filters(
        &session,
        vec![SubsetPredicate::Missing {
            field: field(&degree, "total"),
            include_null: true,
            include_missing: true,
        }],
    );
    let snapshot = session.snapshot_shared();
    assert_eq!(snapshot.meta.subset.counts.visible_nodes, 0);
    assert_eq!(snapshot.meta.subset.distributions[0].unavailable, 3);
}

#[test]
fn prepared_calculation_conflict_does_not_publish_values_or_history() {
    let session = fixture();
    let request = SharedRequest {
        request: Request::Calculate(CalculateRequest {
            kind: CalculationKind::Degree,
            calculation_id: None,
        }),
        expected: Some(session.shared_stamp()),
        request_id: Some("calculation".into()),
    };
    let prepared = session.prepare_shared(&request).unwrap();
    session
        .handle(&Request::Focus(FocusRequest { slots: Vec::new() }))
        .unwrap();
    let before = session.snapshot_shared();
    assert!(matches!(
        session.commit_shared(prepared),
        Err(CoreError::Conflict(_))
    ));
    assert_eq!(session.snapshot_shared(), before);
    assert!(before.meta.calculations.is_empty());
}

#[test]
fn derived_appearance_is_canonical_and_legacy_conflicts_refuse() {
    let session = fixture();
    let degree = calculate(&session, CalculationKind::Degree, None);
    let request: Request = serde_json::from_value(serde_json::json!({"type":"appearance","color_field":field(&degree,"total"),"size_field":field(&degree,"total")})).unwrap();
    session.handle(&request).unwrap();
    let snapshot = session.snapshot_shared();
    assert_eq!(
        snapshot.meta.appearance.color_field,
        Some(field(&degree, "total"))
    );
    assert_eq!(snapshot.meta.appearance.color_by, None);
    assert_eq!(
        snapshot.meta.appearance_mapping.size_max,
        Some(TypedValue::Int64("4".into()))
    );
    assert_eq!(
        snapshot
            .meta
            .appearance_mapping
            .nodes
            .iter()
            .find(|row| row.handle.node_id == 1)
            .unwrap()
            .radius,
        Some(22.0)
    );
    for legacy in [serde_json::Value::Null, serde_json::json!("total")] {
        assert!(serde_json::from_value::<Request>(serde_json::json!({"type":"appearance","color_by":legacy,"color_field":field(&degree,"total")})).is_err());
    }
    assert_eq!(session.snapshot_shared(), snapshot);
}

#[test]
fn calculation_capacity_and_recompute_identity_refuse_without_partial_changes() {
    let session = fixture();
    let first = calculate(&session, CalculationKind::Degree, None);
    for _ in 1..8 {
        calculate(&session, CalculationKind::Degree, None);
    }
    let before = session.snapshot_shared();
    for request in [
        CalculateRequest {
            kind: CalculationKind::Degree,
            calculation_id: None,
        },
        CalculateRequest {
            kind: CalculationKind::Degree,
            calculation_id: Some("invented".into()),
        },
        CalculateRequest {
            kind: CalculationKind::WeakComponents,
            calculation_id: Some(first.id.clone()),
        },
    ] {
        assert!(session.handle(&Request::Calculate(request)).is_err());
        assert_eq!(session.snapshot_shared(), before);
    }
    calculate(&session, CalculationKind::Degree, Some(first.id));
    assert_eq!(session.snapshot_shared().meta.calculations.len(), 8);
}

#[test]
fn bookmark_v2_and_history_restore_frozen_values_without_recomputing() {
    let session = fixture();
    let degree = calculate(&session, CalculationKind::Degree, None);
    session
        .handle(
            &serde_json::from_value::<Request>(
                serde_json::json!({"type":"appearance","color_field":field(&degree,"total")}),
            )
            .unwrap(),
        )
        .unwrap();
    let capture = session.capture_bookmark(None).unwrap();
    assert_eq!(capture.bookmark.version, 2);
    assert_eq!(
        capture.bookmark.calculations.as_ref().unwrap(),
        &vec![degree.clone()]
    );
    filters(
        &session,
        vec![SubsetPredicate::Relation {
            names: vec!["R".into()],
        }],
    );
    calculate(&session, CalculationKind::Degree, Some(degree.id.clone()));
    assert_eq!(
        values(&session, vec![field(&degree, "total")])[1],
        vec![int(2)]
    );
    let checkpoint = session
        .history_state()
        .history
        .entries
        .last()
        .unwrap()
        .id
        .clone();
    session
        .commit_shared(
            session
                .prepare_history_restore(&checkpoint, None, None)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        values(&session, vec![field(&degree, "total")])[1],
        vec![int(4)]
    );
    assert_eq!(
        session.snapshot_shared().meta.calculations,
        vec![degree.clone()]
    );
    session
        .commit_shared(
            session
                .prepare_bookmark_restore(&capture.bookmark, None, None, None)
                .unwrap(),
        )
        .unwrap();
    let restored = session.snapshot_shared();
    assert_eq!(restored.meta.calculations, vec![degree.clone()]);
    assert_eq!(
        restored.meta.appearance.color_field,
        Some(field(&degree, "total"))
    );
    assert_eq!(restored.meta.appearance_mapping.size_max, None);
    let before = restored;
    let mut corrupt = capture.bookmark;
    corrupt.derived[0].values[0].1 = TypedValue::Int64("999".into());
    assert!(session
        .prepare_bookmark_restore(&corrupt, None, None, None)
        .is_err());
    assert_eq!(session.snapshot_shared(), before);
}

#[test]
fn v1_migration_requires_absent_new_fields_and_empty_derived_values() {
    let session = fixture();
    let mut v1 = serde_json::to_value(session.capture_bookmark(None).unwrap().bookmark).unwrap();
    v1["version"] = serde_json::json!(1);
    v1.as_object_mut().unwrap().remove("channels");
    v1.as_object_mut().unwrap().remove("calculations");
    let bookmark: kglite_visual_core::bookmark::Bookmark =
        serde_json::from_value(v1.clone()).unwrap();
    session
        .commit_shared(
            session
                .prepare_bookmark_restore(&bookmark, None, None, None)
                .unwrap(),
        )
        .unwrap();
    assert!(session.snapshot_shared().meta.calculations.is_empty());
    let before = session.snapshot_shared();
    let mut wrong = bookmark;
    wrong.calculations = Some(Vec::new());
    assert!(session
        .prepare_bookmark_restore(&wrong, None, None, None)
        .is_err());
    let mut null = v1.clone();
    null["channels"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<kglite_visual_core::bookmark::Bookmark>(null).is_err());
    let mut missing_v2 = v1;
    missing_v2["version"] = serde_json::json!(2);
    let missing =
        serde_json::from_value::<kglite_visual_core::bookmark::Bookmark>(missing_v2).unwrap();
    assert!(session
        .prepare_bookmark_restore(&missing, None, None, None)
        .is_err());
    assert_eq!(session.snapshot_shared(), before);
}

#[test]
fn durable_reopen_preserves_original_calculation_input_generation_and_values() {
    use kglite::api::io::{prepare_kgl_write, write_kgl};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("calculations.kgl");
    let mut source = Arc::new(graph());
    prepare_kgl_write(&mut source);
    write_kgl(&source, path.to_str().unwrap()).unwrap();
    let open = || {
        kglite_visual_core::load_session_with(
            kglite_visual_core::GraphSource::Path(&path),
            "durable",
            kglite_visual_core::LoadLimits::default(),
            kglite_visual_core::QueryConfig::default(),
        )
        .unwrap()
    };
    let first = open();
    load(&first);
    let degree = calculate(&first, CalculationKind::Degree, None);
    filters(
        &first,
        vec![SubsetPredicate::Relation {
            names: vec!["R".into()],
        }],
    );
    let bookmark = first.capture_bookmark(None).unwrap().bookmark;
    assert!(matches!(
        bookmark.source,
        kglite_visual_core::bookmark::BookmarkSource::Durable { .. }
    ));
    let reopened = open();
    assert_ne!(first.generation(), reopened.generation());
    reopened
        .commit_shared(
            reopened
                .prepare_bookmark_restore(&bookmark, None, None, None)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        reopened.snapshot_shared().meta.calculations,
        vec![degree.clone()]
    );
    assert_eq!(
        values(&reopened, vec![field(&degree, "total")])[1],
        vec![int(4)]
    );
    assert_eq!(
        reopened.snapshot_shared().meta.subset.counts.visible_edges,
        2
    );
}

#[test]
fn captured_render_explains_derived_channels_and_their_frozen_visible_input() {
    use kglite_visual_core::output::{CaptureOutputRequest, OutputScope, RenderOutputSettings};
    let session = fixture();
    let degree = calculate(&session, CalculationKind::Degree, None);
    let components = calculate(&session, CalculationKind::WeakComponents, None);
    filters(
        &session,
        vec![SubsetPredicate::Relation {
            names: vec!["R".into()],
        }],
    );
    session.handle(&serde_json::from_value::<Request>(serde_json::json!({"type":"appearance","color_field":field(&components,"component_id"),"size_field":field(&degree,"total")})).unwrap()).unwrap();
    let before = session.snapshot_shared();
    let capture = session
        .capture_output(&CaptureOutputRequest {
            scope: OutputScope::Visible,
            expected: before.meta.stamp.clone(),
            subset_revision: before.meta.subset_revision.clone(),
        })
        .unwrap();
    let rendered = capture.render(&RenderOutputSettings::default()).unwrap();
    let svg = String::from_utf8(rendered.rendered.bytes).unwrap();
    assert!(svg.contains("Color: Weak component"));
    assert!(svg.contains("Size: Total degree"));
    assert!(svg.contains("Visible subset · 4 nodes · 2 relation records"));
    for meta in [&degree, &components] {
        assert!(svg.contains(&format!(
            "frozen visible input · revision {} · subset {} · 4 nodes · 3 relation records",
            meta.input_stamp.revision, meta.input_subset_revision
        )));
        assert!(!svg.contains(&meta.id));
    }
    assert!(svg.contains("integer 4"));
    assert!(svg.contains("22 px"));
    assert_eq!(session.snapshot_shared(), before);
}

#[test]
fn frozen_member_without_key_makes_bookmark_session_only_even_after_it_is_unloaded() {
    use kglite::api::io::{prepare_kgl_write, write_kgl};
    let mut graph = DirGraph::new();
    execute_mut(
        &mut graph,
        "CREATE (:P {id:1,title:'Keyed'}) CREATE (:P {id:null,title:'Null key'})",
        &ExecuteOptions::eager(&Default::default()),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("null-key.kgl");
    let mut graph = Arc::new(graph);
    prepare_kgl_write(&mut graph);
    write_kgl(&graph, path.to_str().unwrap()).unwrap();
    let session = kglite_visual_core::load_session_with(
        kglite_visual_core::GraphSource::Path(&path),
        "nullable",
        kglite_visual_core::LoadLimits::default(),
        kglite_visual_core::QueryConfig::default(),
    )
    .unwrap();
    session
        .handle(&Request::LoadNodes(LoadNodesRequest {
            handles: vec![session.node_handle(0), session.node_handle(1)],
        }))
        .unwrap();
    let table = session
        .records(&RecordsRequest {
            handles: vec![session.node_handle(1)],
            fields: vec!["id".into()],
            field_refs: None,
            offset: 0,
            limit: 1,
        })
        .unwrap();
    assert_eq!(table.rows[0].cells, vec![RecordCell::Null]);
    calculate(&session, CalculationKind::Degree, None);
    session.reset().unwrap();
    session
        .handle(&Request::LoadNodes(LoadNodesRequest {
            handles: vec![session.node_handle(0)],
        }))
        .unwrap();
    assert_eq!(
        session.bookmark_eligibility().storage,
        kglite_visual_core::bookmark::BookmarkStorage::Session
    );
    let capture = session.capture_bookmark(None).unwrap();
    assert!(matches!(
        capture.bookmark.source,
        kglite_visual_core::bookmark::BookmarkSource::SessionOnly { .. }
    ));
    session
        .commit_shared(
            session
                .prepare_bookmark_restore(&capture.bookmark, None, None, None)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(session.snapshot_shared().meta.calculations[0].node_count, 2);
}

#[test]
fn recompute_reevaluates_its_own_filter_once_without_recursive_recalculation() {
    let session = fixture();
    let degree = calculate(&session, CalculationKind::Degree, None);
    filters(
        &session,
        vec![SubsetPredicate::NumericRange {
            field: field(&degree, "total"),
            min: Some(TypedValue::Int64("3".into())),
            max: None,
            include_null: false,
            include_missing: false,
        }],
    );
    assert_eq!(
        session.snapshot_shared().meta.subset.counts.visible_nodes,
        1
    );
    let recomputed = calculate(&session, CalculationKind::Degree, Some(degree.id.clone()));
    assert_eq!((recomputed.node_count, recomputed.edge_count), (1, 1));
    assert_eq!(
        session.snapshot_shared().meta.subset.counts.visible_nodes,
        0
    );
    assert_eq!(
        values(&session, vec![field(&degree, "total")])[1],
        vec![int(2)]
    );
}
#[test]
fn weak_component_bookmark_refuses_impossible_edge_counts_without_mutation() {
    for session in [fixture(), Session::open(Arc::new(graph()), "empty input")] {
        calculate(&session, CalculationKind::WeakComponents, None);
        let mut bookmark = session.capture_bookmark(None).unwrap().bookmark;
        let meta = &mut bookmark.calculations.as_mut().unwrap()[0];
        meta.edge_count = if meta.node_count == 0 { 1 } else { 0 };
        let before = session.snapshot_shared();
        assert!(
            session
                .prepare_bookmark_restore(&bookmark, None, None, None)
                .is_err(),
            "impossible frozen component input metadata must refuse"
        );
        assert_eq!(session.snapshot_shared(), before);
    }
}
