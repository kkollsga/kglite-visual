use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kglite::api::io::{prepare_kgl_write, write_kgl};
use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::{DirGraph, Value};
use kglite_visual_core::bookmark::{
    BookmarkCaptureOptions, BookmarkFocusRequest, BookmarkMember, BookmarkName, BookmarkSource,
    BookmarkStorage,
};
use kglite_visual_core::control::{AppearanceRequest, FocusRequest, HighlightRequest};
use kglite_visual_core::records::{BrowseTypeRequest, LoadNodesRequest};
use kglite_visual_core::request::{CypherRequest, LayoutKernel, LayoutRequest, Request};
use kglite_visual_core::shared::{SharedRequest, ViewReference};
use kglite_visual_core::subset::{SubsetFilter, SubsetPredicate, SubsetRequest};
use kglite_visual_core::{
    load_session_with, CoreError, GraphSource, LoadLimits, QueryConfig, Session,
};

fn graph() -> DirGraph {
    let mut graph = DirGraph::new();
    execute_mut(&mut graph, "CREATE (a:P {id:9007199254740993,title:'A',score:1}) CREATE (b:P {id:9007199254740994,title:'B',score:2}) CREATE (:Q {id:3,title:'Isolated'}) CREATE (a)-[:R {weight:2}]->(b) CREATE (a)-[:R {weight:3}]->(b) CREATE (b)-[:S]->(b)", &ExecuteOptions::eager(&Default::default())).unwrap();
    graph
}
fn write(path: &Path, graph: DirGraph) {
    let mut graph = Arc::new(graph);
    prepare_kgl_write(&mut graph);
    write_kgl(&graph, path.to_str().unwrap()).unwrap();
}

#[test]
fn corrupt_misspelled_persisted_field_is_not_silently_replaced_with_none() {
    let source = Session::open(Arc::new(graph()), "memory");
    load(&source);
    apply(
        &source,
        Request::Appearance(AppearanceRequest {
            color_field: None,
            size_field: None,
            color_by: Some("score".into()),
            size_by: None,
        }),
    );
    let capture = source.capture_bookmark(None).unwrap();
    let before = source.snapshot_shared();
    let mut encoded = serde_json::to_value(capture.bookmark).unwrap();
    let object = encoded.as_object_mut().unwrap();
    let color = object.remove("color_by").unwrap();
    object.insert("colour_by".into(), color);
    assert!(serde_json::from_value::<kglite_visual_core::bookmark::Bookmark>(encoded).is_err());
    assert_eq!(source.snapshot_shared(), before);
    let mut missing =
        serde_json::to_value(source.capture_bookmark(None).unwrap().bookmark).unwrap();
    missing.as_object_mut().unwrap().remove("color_by");
    assert!(serde_json::from_value::<kglite_visual_core::bookmark::Bookmark>(missing).is_err());
}

#[test]
fn actual_disk_publish_changes_generation_fingerprint_and_reopen_preserves_exact_edges() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.kgl");
    let mut writer = Arc::new(graph());
    kglite::api::io::materialize_disk_graph(&mut writer, path.to_str().unwrap()).unwrap();
    assert!(path.join("CURRENT").is_file());
    let source = open(&path);
    load(&source);
    let capture = source.capture_bookmark(None).unwrap();
    assert!(
        matches!(&capture.durability, BookmarkSource::Durable { source } if source.kind == kglite_visual_core::source_identity::SourceKind::PublishedGeneration)
    );
    let reopened = open(&path);
    reopened
        .commit_shared(
            reopened
                .prepare_bookmark_restore(&capture.bookmark, None, None, None)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        reopened.snapshot_shared().meta.subset.counts.loaded_edges,
        3
    );
    execute_mut(
        kglite::api::make_dir_graph_mut(&mut writer),
        "CREATE (:Q {id:4,title:'new generation'})",
        &ExecuteOptions::eager(&Default::default()),
    )
    .unwrap();
    kglite::api::io::save_graph(&mut writer, path.to_str().unwrap()).unwrap();
    assert!(source
        .prepare_bookmark_restore(&capture.bookmark, None, None, None)
        .is_err());
    let newest = open(&path);
    assert!(newest
        .prepare_bookmark_restore(&capture.bookmark, None, None, None)
        .is_err());
}

#[cfg(unix)]
#[test]
fn path_alias_can_retarget_without_rebinding_an_existing_session() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.kgl");
    let other = directory.path().join("other.kgl");
    let alias = directory.path().join("alias.kgl");
    write(&path, graph());
    write(&other, DirGraph::new());
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let source = open(&alias);
    load(&source);
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&other, &alias).unwrap();
    let capture = source.capture_bookmark(None).unwrap();
    let BookmarkSource::Durable {
        source: fingerprint,
    } = capture.durability
    else {
        panic!("verified source")
    };
    assert_eq!(
        fingerprint.canonical_path,
        path.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(capture.bookmark.members.len(), 2);
}
fn open(path: &Path) -> Session {
    load_session_with(
        GraphSource::Path(path),
        "arbitrary display label",
        LoadLimits::default(),
        QueryConfig::default(),
    )
    .unwrap()
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
}
fn apply(session: &Session, request: Request) {
    session
        .apply_shared(&SharedRequest {
            request,
            expected: Some(session.shared_stamp()),
            request_id: None,
        })
        .unwrap();
}
fn name(storage: BookmarkStorage) -> BookmarkName {
    BookmarkName {
        storage,
        name: "Investigation".into(),
    }
}

#[test]
fn durable_reopen_restores_exact_parallel_edges_self_loop_filters_and_schema_coordinates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.kgl");
    write(&path, graph());
    let source = open(&path);
    load(&source);
    apply(
        &source,
        Request::Subset(SubsetRequest {
            predicates: vec![SubsetFilter {
                id: "relations".into(),
                enabled: true,
                predicate: SubsetPredicate::Relation {
                    names: vec!["R".into()],
                },
            }],
        }),
    );
    apply(
        &source,
        Request::Appearance(AppearanceRequest {
            color_field: None,
            size_field: None,
            color_by: Some("score".into()),
            size_by: None,
        }),
    );
    apply(
        &source,
        Request::Layout(LayoutRequest {
            kernel: LayoutKernel::Force,
            seed_slot: None,
        }),
    );
    let captured = source.capture_bookmark(None).unwrap();
    assert!(matches!(
        captured.durability,
        BookmarkSource::Durable { .. }
    ));
    assert_eq!(captured.bookmark.relations.len(), 3);
    assert_eq!(captured.bookmark.members.len(), 2);
    assert!(captured.bookmark.layout.positions.iter().any(|position| matches!(&position.reference, kglite_visual_core::bookmark::BookmarkReference::Type { name } if name == "P")));
    let reopened = open(&path);
    assert_ne!(source.generation(), reopened.generation());
    let prepared = reopened
        .prepare_bookmark_restore(
            &captured.bookmark,
            Some(&name(BookmarkStorage::Durable)),
            Some(&reopened.shared_stamp()),
            None,
        )
        .unwrap();
    let event = reopened.commit_shared(prepared).unwrap();
    assert!(event.wire_meta().restored);
    assert_eq!(event.snapshot.meta.subset.counts.loaded_nodes, 2);
    assert_eq!(event.snapshot.meta.subset.counts.loaded_edges, 3);
    assert_eq!(event.snapshot.meta.subset.counts.visible_edges, 2);
    assert_eq!(
        event.snapshot.meta.appearance.color_by.as_deref(),
        Some("score")
    );
    assert!(!event.snapshot.meta.saved_view.as_ref().unwrap().dirty);
    assert_eq!(
        reopened.capture_bookmark(None).unwrap().bookmark.layout,
        captured.bookmark.layout
    );
    let focused = reopened
        .apply_shared(&SharedRequest::new(Request::Focus(FocusRequest {
            slots: vec![],
        })))
        .unwrap();
    assert!(!focused.wire_meta().restored);
}

#[test]
fn session_only_handles_do_not_escape_their_generation_or_borrow_a_display_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("real.kgl");
    write(&path, graph());
    let source = Session::open(Arc::new(graph()), path.display().to_string());
    load(&source);
    let captured = source.capture_bookmark(None).unwrap();
    assert!(matches!(
        captured.durability,
        BookmarkSource::SessionOnly { .. }
    ));
    assert!(captured
        .bookmark
        .members
        .iter()
        .all(|member| matches!(member, BookmarkMember::Handle { .. })));
    let other = open(&path);
    assert!(other
        .prepare_bookmark_restore(&captured.bookmark, None, None, None)
        .is_err());
    source
        .commit_shared(
            source
                .prepare_bookmark_restore(&captured.bookmark, None, None, None)
                .unwrap(),
        )
        .unwrap();
}

#[test]
fn ambiguous_null_and_large_keys_degrade_to_explicit_session_only() {
    for text in [
        "CREATE (:P {id:1,title:'a'}) CREATE (:P {id:1,title:'b'})",
        "CREATE (:P {id:null,title:'null'})",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keys.kgl");
        let mut graph = DirGraph::new();
        execute_mut(
            &mut graph,
            text,
            &ExecuteOptions::eager(&Default::default()),
        )
        .unwrap();
        write(&path, graph);
        let source = open(&path);
        source
            .browse_type(&BrowseTypeRequest {
                node_type: "P".into(),
                limit: None,
            })
            .unwrap();
        assert!(matches!(
            source.capture_bookmark(None).unwrap().durability,
            BookmarkSource::SessionOnly { .. }
        ));
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large-key.kgl");
    let mut graph = DirGraph::new();
    let params = std::collections::HashMap::from([("key".into(), Value::String("x".repeat(5000)))]);
    execute_mut(
        &mut graph,
        "CREATE (:P {id:$key,title:'big'})",
        &ExecuteOptions::eager(&params),
    )
    .unwrap();
    write(&path, graph);
    let source = open(&path);
    source
        .browse_type(&BrowseTypeRequest {
            node_type: "P".into(),
            limit: None,
        })
        .unwrap();
    assert!(matches!(
        source.capture_bookmark(None).unwrap().durability,
        BookmarkSource::SessionOnly { .. }
    ));
}

#[test]
fn failed_source_relation_and_version_restore_leave_content_and_history_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.kgl");
    write(&path, graph());
    let source = open(&path);
    load(&source);
    let bookmark = source.capture_bookmark(None).unwrap().bookmark;
    let before = source.snapshot_shared();
    let mut wrong = bookmark.clone();
    wrong.version += 1;
    assert!(source
        .prepare_bookmark_restore(&wrong, None, None, None)
        .is_err());
    wrong = bookmark.clone();
    wrong.relations[0].attributes_sha256 = "wrong".into();
    assert!(source
        .prepare_bookmark_restore(&wrong, None, None, None)
        .is_err());
    wrong = bookmark.clone();
    wrong.relations[0].target_member = 999;
    assert!(source
        .prepare_bookmark_restore(&wrong, None, None, None)
        .is_err());
    std::fs::remove_file(&path).unwrap();
    assert!(source
        .prepare_bookmark_restore(&bookmark, None, None, None)
        .is_err());
    assert_eq!(source.snapshot_shared(), before);
}

#[test]
fn stale_prepared_restore_cannot_replace_a_peer_change_or_its_history() {
    let source = Session::open(Arc::new(graph()), "memory");
    load(&source);
    let bookmark = source.capture_bookmark(None).unwrap().bookmark;
    apply(&source, Request::Reset);
    let prepared = source
        .prepare_bookmark_restore(&bookmark, None, Some(&source.shared_stamp()), None)
        .unwrap();
    apply(
        &source,
        Request::Appearance(AppearanceRequest {
            color_field: None,
            size_field: None,
            color_by: Some("score".into()),
            size_by: None,
        }),
    );
    let before = source.history_state();
    assert!(matches!(
        source.commit_shared(prepared),
        Err(CoreError::Conflict(_))
    ));
    assert_eq!(source.history_state(), before);
    assert_eq!(source.snapshot_shared().meta.subset.counts.loaded_nodes, 0);
}

#[test]
fn saved_local_selection_is_acknowledged_and_focus_does_not_dirty_it() {
    let source = Session::open(Arc::new(graph()), "memory");
    load(&source);
    let selected = vec![ViewReference::Node {
        handle: source.snapshot_shared().meta.subset.visible_nodes[0].clone(),
    }];
    let capture = source
        .capture_bookmark_with(
            &BookmarkCaptureOptions {
                selected: Some(selected.clone()),
                focus: Some(BookmarkFocusRequest::Fit),
            },
            Some(&source.shared_stamp()),
        )
        .unwrap();
    apply(&source, Request::Focus(FocusRequest { slots: Vec::new() }));
    source
        .commit_shared(
            source
                .prepare_bookmark_saved(&name(BookmarkStorage::Session), &capture, None, None)
                .unwrap(),
        )
        .unwrap();
    let saved = source.snapshot_shared().meta;
    assert_eq!(saved.selected, selected);
    assert_eq!(saved.saved_view.as_ref().unwrap().selected, selected);
    assert!(!saved.saved_view.unwrap().dirty);
    apply(&source, Request::Focus(FocusRequest { slots: Vec::new() }));
    assert_eq!(
        source.snapshot_shared().meta.content_revision,
        saved.content_revision
    );
    assert!(!source.snapshot_shared().meta.saved_view.unwrap().dirty);
    apply(
        &source,
        Request::Appearance(AppearanceRequest {
            color_field: None,
            size_field: None,
            color_by: Some("score".into()),
            size_by: None,
        }),
    );
    assert!(source.snapshot_shared().meta.saved_view.unwrap().dirty);
    assert!(matches!(
        source.prepare_bookmark_saved(&name(BookmarkStorage::Session), &capture, None, None),
        Err(CoreError::Conflict(_))
    ));
}

#[test]
fn save_refuses_unloaded_selection_and_accepts_hidden_loaded_selection() {
    let source = Session::open(Arc::new(graph()), "memory");
    load(&source);
    let handle = source.snapshot_shared().meta.subset.visible_nodes[0].clone();
    apply(
        &source,
        Request::Subset(SubsetRequest {
            predicates: vec![SubsetFilter {
                id: "hidden".into(),
                enabled: true,
                predicate: SubsetPredicate::Type {
                    node_types: vec!["Q".into()],
                },
            }],
        }),
    );
    source
        .capture_bookmark_with(
            &BookmarkCaptureOptions {
                selected: Some(vec![ViewReference::Node { handle }]),
                focus: None,
            },
            None,
        )
        .unwrap();
    assert!(source
        .capture_bookmark_with(
            &BookmarkCaptureOptions {
                selected: Some(vec![ViewReference::Node {
                    handle: source.node_handle(999)
                }]),
                focus: None
            },
            None
        )
        .unwrap_err()
        .to_string()
        .contains("load it or clear"));
}

#[test]
fn recovery_rolls_at_twenty_and_transient_actions_do_not_fill_it() {
    let source = Session::open(Arc::new(graph()), "memory");
    load(&source);
    let original = source.history_state().history.entries[0].id.clone();
    for i in 0..25 {
        apply(
            &source,
            Request::Appearance(AppearanceRequest {
                color_field: None,
                size_field: None,
                color_by: Some(format!("field{i}")),
                size_by: None,
            }),
        );
    }
    let state = source.history_state();
    assert_eq!(state.history.entries.len(), 20);
    assert_eq!(state.history.evicted_count, "6");
    assert!(source
        .prepare_history_restore(&original, None, None)
        .is_err());
    apply(&source, Request::Focus(FocusRequest { slots: vec![] }));
    apply(
        &source,
        Request::Highlight(HighlightRequest {
            slots: vec![source.slot_of_type("P").unwrap()],
            concept: Default::default(),
        }),
    );
    assert_eq!(source.history_state().history, state.history);
    let id = &state.history.entries.last().unwrap().id;
    let prepared = source
        .prepare_history_restore(id, Some(&source.shared_stamp()), None)
        .unwrap();
    source.commit_shared(prepared).unwrap();
    assert_eq!(
        source.snapshot_shared().meta.appearance.color_by.as_deref(),
        Some("field23")
    );
}

#[test]
fn typed_loader_preserves_budgets_and_bytes_remain_session_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.kgl");
    write(&path, graph());
    let bytes = std::fs::read(&path).unwrap();
    let session = load_session_with(
        GraphSource::Bytes(&bytes),
        "bytes",
        LoadLimits::default(),
        QueryConfig {
            timeout: Duration::from_secs(7),
        },
    )
    .unwrap();
    assert_eq!(session.config().timeout, Duration::from_secs(7));
    assert!(matches!(
        session.capture_bookmark(None).unwrap().durability,
        BookmarkSource::SessionOnly { .. }
    ));
    assert!(load_session_with(
        GraphSource::Path(&path),
        "limited",
        LoadLimits {
            max_load_mb: Some(0)
        },
        QueryConfig::default()
    )
    .is_err());
}

#[test]
fn deletion_clears_only_matching_catalog_association_without_content_change() {
    let source = Session::open(Arc::new(graph()), "memory");
    source
        .load_nodes(&LoadNodesRequest {
            handles: vec![source.node_handle(0)],
        })
        .unwrap();
    let capture = source.capture_bookmark(None).unwrap();
    source
        .commit_shared(
            source
                .prepare_bookmark_saved(&name(BookmarkStorage::Session), &capture, None, None)
                .unwrap(),
        )
        .unwrap();
    let content = source.snapshot_shared().meta.content_revision;
    source
        .commit_shared(
            source
                .prepare_bookmark_deleted(&name(BookmarkStorage::Durable), None, None)
                .unwrap(),
        )
        .unwrap();
    assert!(source.snapshot_shared().meta.saved_view.is_some());
    source
        .commit_shared(
            source
                .prepare_bookmark_deleted(&name(BookmarkStorage::Session), None, None)
                .unwrap(),
        )
        .unwrap();
    assert!(source.snapshot_shared().meta.saved_view.is_none());
    assert_eq!(source.snapshot_shared().meta.content_revision, content);
}
