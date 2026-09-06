use std::collections::BTreeMap;
use std::sync::Arc;

use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::{CurrentSelection, DirGraph, GraphRead, NodeIndex, Value};
use kglite_visual_core::control::{AppearanceRequest, FocusRequest};
use kglite_visual_core::output::{
    CaptureOutputRequest, CapturedOutput, OutputScope, RenderOutputSettings,
};
use kglite_visual_core::presentation::{PresentationSettings, StyleRequest};
use kglite_visual_core::query_provenance::{LoadEntitiesRequest, RelationHandle};
use kglite_visual_core::records::{BrowseTypeRequest, RecordsRequest, TypedValue};
use kglite_visual_core::request::Request;
use kglite_visual_core::subset::{SubsetFilter, SubsetPredicate, SubsetRequest};
use kglite_visual_core::{CoreError, ExportFormat, Session};

fn fixture() -> Session {
    let mut graph = DirGraph::new();
    execute_mut(&mut graph, "CREATE (a:P {id:9007199254740993,title:'A',score:9007199254740993,category:2}) CREATE (b:P {id:9007199254740993,title:'B',score:9007199254740994,category:'2'}) CREATE (c:P {id:null,title:'C',score:9007199254740995}) CREATE (a)-[:R {weight:2}]->(b) CREATE (a)-[:R {weight:3}]->(b) CREATE (b)-[:S]->(b) CREATE (a)-[:OMITTED]->(b)", &ExecuteOptions::eager(&Default::default())).unwrap();
    let session = Session::open(Arc::new(graph), "output");
    session
        .handle(&Request::BrowseType(BrowseTypeRequest {
            node_type: "P".into(),
            limit: Some(3),
        }))
        .unwrap();
    let relationships = (0..3)
        .map(|edge_id| {
            let (source, target) = session
                .graph()
                .graph
                .edge_endpoints(kglite::api::EdgeIndex::new(edge_id))
                .unwrap();
            RelationHandle {
                generation: session.generation().into(),
                edge_id: edge_id as u32,
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
    session
}
fn request(session: &Session, scope: OutputScope) -> CaptureOutputRequest {
    let snapshot = session.snapshot_shared();
    CaptureOutputRequest {
        scope,
        expected: snapshot.meta.stamp,
        subset_revision: snapshot.meta.subset_revision,
    }
}

#[test]
fn exact_relation_multiset_and_export_local_identity_order_survive_all_formats() {
    let session = fixture();
    let capture = session
        .capture_output(&request(&session, OutputScope::Visible))
        .unwrap();
    assert_eq!(capture.identity().nodes.len(), 3);
    assert_eq!(capture.identity().edge_ids.len(), 3);
    let expected = capture
        .identity()
        .edge_ids
        .iter()
        .map(|id| match id {
            0 => "R",
            1 => "R",
            2 => "S",
            _ => panic!("omitted source edge leaked"),
        })
        .collect::<Vec<_>>();
    let json = capture.export(ExportFormat::Json).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&json.bytes).unwrap();
    let links = json["links"].as_array().unwrap();
    assert_eq!(links.len(), 3);
    assert_eq!(
        links
            .iter()
            .map(|edge| edge["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(links.iter().any(|edge| edge["source"] == edge["target"]));
    for format in [ExportFormat::Graphml, ExportFormat::Gexf] {
        let artifact = capture.export(format).unwrap();
        let text = String::from_utf8(artifact.bytes).unwrap();
        assert_eq!(text.matches("<edge ").count(), 3);
        assert!(!text.contains("OMITTED"));
    }
    let csv = String::from_utf8(capture.export(ExportFormat::CsvEdges).unwrap().bytes).unwrap();
    assert_eq!(csv.lines().skip(1).count(), 3);
    let induced = session
        .capture_output(&request(&session, OutputScope::LoadedInduced))
        .unwrap();
    assert_eq!(induced.preview(ExportFormat::Graphml).unwrap().edges, 4);
}

#[test]
fn relation_filter_is_exact_and_stale_preview_refuses_while_capture_stays_immutable() {
    let session = fixture();
    let old = request(&session, OutputScope::Visible);
    let capture = session.capture_output(&old).unwrap();
    session
        .handle(&Request::Subset(SubsetRequest {
            predicates: vec![SubsetFilter {
                id: "relations".into(),
                enabled: true,
                predicate: SubsetPredicate::Relation {
                    names: vec!["S".into()],
                },
            }],
        }))
        .unwrap();
    assert!(matches!(
        session.capture_output(&old),
        Err(CoreError::Conflict(_))
    ));
    let current = session
        .capture_output(&request(&session, OutputScope::Visible))
        .unwrap();
    assert_eq!(current.preview(ExportFormat::Json).unwrap().edges, 1);
    assert_eq!(capture.preview(ExportFormat::Json).unwrap().edges, 3);
    let svg = String::from_utf8(
        capture
            .render(&RenderOutputSettings::default())
            .unwrap()
            .rendered
            .bytes,
    )
    .unwrap();
    assert!(svg.contains("3 relation records"));
    assert!(svg.contains("self-loop"));
}

#[test]
fn preview_digest_covers_effective_settings_scope_and_revision() {
    let session = fixture();
    let original = request(&session, OutputScope::Visible);
    let capture = session.capture_output(&original).unwrap();
    let settings = RenderOutputSettings::default();
    let preview = capture.preview_render(&settings).unwrap();
    let defaulted: RenderOutputSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(
        preview.preview_digest,
        capture.preview_render(&defaulted).unwrap().preview_digest
    );
    CapturedOutput::check_preview_digest(&preview, &preview.preview_digest).unwrap();
    let changed = capture
        .preview_render(&RenderOutputSettings {
            width: 800,
            ..settings
        })
        .unwrap();
    assert!(matches!(
        CapturedOutput::check_preview_digest(&changed, &preview.preview_digest),
        Err(CoreError::Conflict(_))
    ));
    assert!(CapturedOutput::check_preview_digest(&preview, "bad").is_err());
    session
        .handle(&Request::Focus(FocusRequest { slots: Vec::new() }))
        .unwrap();
    assert!(matches!(
        session.capture_output(&original),
        Err(CoreError::Conflict(_))
    ));
}

#[test]
fn canonical_mapping_preserves_typed_categories_and_adjacent_large_integer_sizes() {
    let session = fixture();
    session
        .handle(&Request::Appearance(AppearanceRequest {
            color_by: Some("category".into()),
            size_by: Some("score".into()),
        }))
        .unwrap();
    let before = session.snapshot_shared();
    let mapping = &before.meta.appearance_mapping;
    assert_eq!(mapping.categories.len(), 2);
    assert_ne!(mapping.categories[0].value, mapping.categories[1].value);
    assert_eq!(
        mapping.size_min,
        Some(TypedValue::Int64("9007199254740993".into()))
    );
    assert_eq!(
        mapping.size_max,
        Some(TypedValue::Int64("9007199254740995".into()))
    );
    let radii: BTreeMap<_, _> = mapping
        .nodes
        .iter()
        .map(|node| (node.handle.node_id, node.radius.unwrap()))
        .collect();
    assert_eq!(radii[&0], 4.0);
    assert_eq!(radii[&2], 22.0);
    assert!((radii[&1] - (4.0 + 18.0 * 0.5f32.powf(0.25))).abs() < 0.0001);
    session
        .handle(&Request::Focus(FocusRequest { slots: Vec::new() }))
        .unwrap();
    assert_eq!(session.snapshot_shared().meta.appearance_mapping, *mapping);
    let capture = session
        .capture_output(&request(&session, OutputScope::Visible))
        .unwrap();
    let text = String::from_utf8(
        capture
            .render(&RenderOutputSettings::default())
            .unwrap()
            .rendered
            .bytes,
    )
    .unwrap();
    assert!(text.contains("Color: category"));
}

#[test]
fn presentation_is_atomic_preserves_channels_and_captures_legend_visibility() {
    let session = fixture();
    session
        .handle(&Request::Appearance(AppearanceRequest {
            color_by: Some("category".into()),
            size_by: None,
        }))
        .unwrap();
    let changed = PresentationSettings {
        label_density: 0.0,
        edge_opacity: 0.0,
        legend_visible: false,
        ..Default::default()
    };
    session
        .handle(&Request::Presentation(changed.clone()))
        .unwrap();
    let before = session.snapshot_shared();
    assert_eq!(before.meta.appearance.color_by.as_deref(), Some("category"));
    let bad = Request::Style(StyleRequest {
        appearance: Some(AppearanceRequest {
            color_by: None,
            size_by: None,
        }),
        presentation: Some(PresentationSettings {
            node_size_min: 60.0,
            node_size_max: 2.0,
            ..changed
        }),
    });
    assert!(session.handle(&bad).is_err());
    assert_eq!(before, session.snapshot_shared());
    let capture = session
        .capture_output(&request(&session, OutputScope::Visible))
        .unwrap();
    let text = String::from_utf8(
        capture
            .render(&RenderOutputSettings::default())
            .unwrap()
            .rendered
            .bytes,
    )
    .unwrap();
    assert!(!text.contains("Color: category"));
    assert!(text.contains("stroke-opacity=\"0\""));
}

#[test]
fn pinned_d3_reserved_relation_property_overwrites_topology_and_viewer_refuses_it() {
    let mut graph = DirGraph::new();
    execute_mut(&mut graph, "CREATE (a:P {id:1}) CREATE (b:P {id:2}) CREATE (a)-[:R {source:99,target:98,type:'WRONG'}]->(b)", &ExecuteOptions::eager(&Default::default())).unwrap();
    let mut selection = CurrentSelection::new();
    selection
        .get_level_mut(0)
        .unwrap()
        .add_selection(None, vec![NodeIndex::new(0), NodeIndex::new(1)]);
    let original = kglite::api::io::to_d3_json(&graph, Some(&selection)).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&original).unwrap();
    assert_eq!(parsed["links"][0]["source"], 99);
    assert_eq!(parsed["links"][0]["target"], 98);
    assert_eq!(parsed["links"][0]["type"], "WRONG");
    assert!(kglite_visual_core::export::export_nodes(
        &graph,
        &[NodeIndex::new(0), NodeIndex::new(1)],
        ExportFormat::Json,
        "unsafe"
    )
    .is_err());
    let graphml = kglite_visual_core::export::export_nodes(
        &graph,
        &[NodeIndex::new(0), NodeIndex::new(1)],
        ExportFormat::Graphml,
        "safe",
    )
    .unwrap();
    let graphml = String::from_utf8(graphml.bytes).unwrap();
    assert!(graphml.contains("&quot;source&quot;:99"));
    assert!(graphml.contains("&quot;target&quot;:98"));
    assert!(graphml.contains("&quot;type&quot;:&quot;WRONG&quot;"));
    let session = Session::open(Arc::new(graph), "collision");
    session
        .handle(&Request::BrowseType(BrowseTypeRequest {
            node_type: "P".into(),
            limit: Some(2),
        }))
        .unwrap();
    session
        .handle(&Request::LoadEntities(LoadEntitiesRequest {
            nodes: Vec::new(),
            relationships: vec![RelationHandle {
                generation: session.generation().into(),
                edge_id: 0,
                source: session.node_handle(0),
                target: session.node_handle(1),
            }],
        }))
        .unwrap();
    let capture = session
        .capture_output(&request(&session, OutputScope::Visible))
        .unwrap();
    assert!(capture
        .preview(ExportFormat::Json)
        .unwrap_err()
        .to_string()
        .contains("GraphML"));
    assert!(capture.preview(ExportFormat::Graphml).is_ok());
}

#[test]
fn huge_columnar_string_refuses_output_and_records_return_only_bounded_preview() {
    let mut graph = DirGraph::new();
    let mut params = std::collections::HashMap::new();
    params.insert("text".into(), Value::String("x".repeat(9 * 1024 * 1024)));
    execute_mut(
        &mut graph,
        "CREATE (:P {id:1,title:'A',huge:$text})",
        &ExecuteOptions::eager(&params),
    )
    .unwrap();
    let session = Session::open(Arc::new(graph), "large");
    session
        .handle(&Request::BrowseType(BrowseTypeRequest {
            node_type: "P".into(),
            limit: Some(1),
        }))
        .unwrap();
    let before = session.snapshot_shared();
    assert!(session
        .capture_output(&request(&session, OutputScope::Visible))
        .is_err());
    let records = session
        .records(&RecordsRequest {
            handles: vec![session.node_handle(0)],
            fields: vec!["huge".into()],
            offset: 0,
            limit: 1,
        })
        .unwrap();
    assert!(
        matches!(&records.rows[0].cells[0], kglite_visual_core::records::RecordCell::Truncated { preview, .. } if preview.len() <= 256)
    );
    assert_eq!(before, session.snapshot_shared());
}

#[test]
fn view_state_settings_come_from_the_acknowledged_stamp() {
    let session = fixture();
    session
        .handle(&Request::Style(StyleRequest {
            appearance: Some(AppearanceRequest {
                color_by: Some("category".into()),
                size_by: None,
            }),
            presentation: Some(PresentationSettings {
                edge_opacity: 0.3,
                ..Default::default()
            }),
        }))
        .unwrap();
    let state = session.view_state();
    let snapshot = session.snapshot_shared();
    assert_eq!(state.stamp, snapshot.meta.stamp);
    assert_eq!(state.appearance, snapshot.meta.appearance);
    assert_eq!(state.presentation, snapshot.meta.presentation);
    assert_eq!(state.caption_by, snapshot.meta.caption_by);
}
