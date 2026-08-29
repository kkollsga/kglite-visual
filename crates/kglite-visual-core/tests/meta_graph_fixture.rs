//! L1 core correctness against the committed fixture (test-plan §L1).
//!
//! Every number below is asserted exactly. A meta-graph test that only checked
//! "more than zero types" would pass on a graph whose type index came back
//! empty for the wrong reason — which is precisely the failure the whole entry
//! screen rests on not happening.

use std::path::{Path, PathBuf};

use kglite_visual_core::meta_graph::{self, DetailTier};
use kglite_visual_core::{load_graph, GraphSource, Session, View};

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("meta.kgl")
}

fn open_fixture() -> Session {
    let graph = load_graph(GraphSource::Path(&fixture_path())).expect("fixture loads");
    Session::open(graph, "meta.kgl")
}

#[test]
fn the_fixture_meta_graph_has_the_shape_the_e2e_test_asserts() {
    let session = open_fixture();
    let meta = session.meta_graph();

    assert_eq!(meta.meta.tier, DetailTier::Full, "5 core types is tier 1");
    assert_eq!(meta.meta.stats.core_type_count, 5);
    assert_eq!(meta.meta.stats.node_count, 118);
    assert_eq!(meta.meta.stats.edge_count, 560);
    assert_eq!(meta.meta.stats.node_type_count, 5);
    assert_eq!(meta.meta.stats.relationship_type_count, 7);

    let names: Vec<&str> = meta.meta.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Person", "Project", "Skill", "City", "Company"],
        "largest type first, name breaking ties — the order slots follow"
    );
    let counts: Vec<u32> = meta.meta.nodes.iter().map(|n| n.count).collect();
    assert_eq!(counts, vec![60, 20, 20, 10, 8]);

    let slots: Vec<u32> = meta.meta.nodes.iter().map(|n| n.slot).collect();
    assert_eq!(slots, vec![0, 1, 2, 3, 4], "slots are dense from zero");

    assert_eq!(meta.points.len(), 10, "one (x, y) pair per node");
    assert_eq!(
        meta.links.len(),
        meta.meta.edges.len() * 2,
        "one (src, tgt) pair per edge"
    );
    assert!(
        !meta.meta.node_bound.truncated && !meta.meta.edge_bound.truncated,
        "the fixture is far inside every bound"
    );
    assert_eq!(meta.meta.node_bound.returned, 5);
    assert_eq!(meta.meta.node_bound.total, 5);
}

#[test]
fn every_link_points_at_a_slot_that_was_sent() {
    let session = open_fixture();
    let meta = session.meta_graph();
    let slot_count = meta.meta.nodes.len() as f32;
    assert!(!meta.links.is_empty(), "the fixture has relationships");
    for index in &meta.links {
        assert_eq!(index.fract(), 0.0, "a slot index must be integral in f32");
        assert!(
            *index >= 0.0 && *index < slot_count,
            "link index {index} is outside the {slot_count} slots that were sent"
        );
    }
}

#[test]
fn relationship_types_and_their_counts_are_exact() {
    let session = open_fixture();
    let mut rows: Vec<(String, u32)> = session
        .meta_graph()
        .meta
        .edges
        .iter()
        .map(|e| (e.name.clone(), e.count))
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("CONTRIBUTES_TO".to_string(), 93),
            ("DEPENDS_ON".to_string(), 23),
            ("HAS_SKILL".to_string(), 180),
            ("KNOWS".to_string(), 180),
            ("LOCATED_IN".to_string(), 8),
            ("OWNS".to_string(), 16),
            ("WORKS_AT".to_string(), 60),
        ]
    );
    let total: u32 = rows.iter().map(|(_, c)| c).sum();
    assert_eq!(total, 560, "the meta-graph accounts for every edge");
}

#[test]
fn capability_badges_survive_the_kgl_round_trip() {
    // The fixture declares a spatial config on City. If `.kgl` dropped it on
    // save, the badge would silently disappear — the badge is only useful if
    // it describes the file a user actually opens.
    let session = open_fixture();
    let city = session
        .meta_graph()
        .meta
        .nodes
        .iter()
        .find(|n| n.name == "City")
        .expect("City is in the meta-graph");
    assert_eq!(city.capabilities, vec!["loc".to_string()]);

    let person = session
        .meta_graph()
        .meta
        .nodes
        .iter()
        .find(|n| n.name == "Person")
        .expect("Person is in the meta-graph");
    assert!(
        person.capabilities.is_empty(),
        "Person carries no declared capability in this fixture"
    );
}

#[test]
fn positions_match_the_committed_baseline() {
    // The baseline is an exact gate (CLAUDE.md → "Gate honesty"). A red diff
    // here after a deliberate layout change is regenerated *with a reason*,
    // in the same commit; it is never regenerated to silence a diff.
    let session = open_fixture();
    let meta = session.meta_graph();

    let baseline = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("meta.positions.json"),
    )
    .expect("committed positions baseline");
    let doc: serde_json::Value = serde_json::from_str(&baseline).expect("baseline is JSON");
    let slots = doc["slots"].as_array().expect("slots array");

    assert_eq!(slots.len(), meta.meta.nodes.len());
    for (i, node) in meta.meta.nodes.iter().enumerate() {
        assert_eq!(slots[i]["slot"].as_u64().unwrap(), node.slot as u64);
        assert_eq!(slots[i]["name"].as_str().unwrap(), node.name);
        assert_eq!(
            slots[i]["x"].as_f64().unwrap() as f32,
            meta.points[i * 2],
            "x drifted for slot {}",
            node.slot
        );
        assert_eq!(
            slots[i]["y"].as_f64().unwrap() as f32,
            meta.points[i * 2 + 1],
            "y drifted for slot {}",
            node.slot
        );
    }
}

#[test]
fn the_binary_frames_and_the_json_twin_carry_the_same_answer() {
    // Test-plan §2's guarantee: one encoder, two serializers. The twin is how
    // an agent verifies server behaviour without a GPU in the loop, so a
    // divergence between the two would make every such verification a lie.
    use kglite_visual_core::{decode_frame, MessageType};

    let session = open_fixture();
    let frames = session.meta_graph_frames();
    assert_eq!(frames.len(), 3, "metadata, points, links");

    let decoded: Vec<_> = frames.iter().map(|f| decode_frame(f).unwrap()).collect();
    assert_eq!(decoded[0].msg_type, MessageType::MetaGraphMeta);
    assert_eq!(decoded[1].msg_type, MessageType::Points);
    assert_eq!(decoded[2].msg_type, MessageType::Links);
    assert!(!decoded[0].terminal && !decoded[1].terminal);
    assert!(decoded[2].terminal, "the last frame closes the response");

    let from_binary: serde_json::Value = serde_json::from_slice(&decoded[0].payload).unwrap();
    let from_twin = serde_json::to_value(&session.meta_graph().meta).unwrap();
    assert_eq!(from_binary, from_twin);

    let binary_points: Vec<f32> = decoded[1]
        .payload
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(binary_points, session.meta_graph().points);
}

#[test]
fn session_info_reports_the_tier_and_the_slot_space() {
    let session = open_fixture();
    let info = session.info();
    assert_eq!(info.protocol_version, kglite_visual_core::PROTOCOL_VERSION);
    assert_eq!(info.graph, "meta.kgl");
    assert_eq!(info.tier, DetailTier::Full);
    assert_eq!(info.slot_count, 5, "one slot per type node, so far");
    assert_eq!(info.stats.node_count, 118);
}

#[test]
fn describe_serves_kglites_own_schema_document_plus_the_tier() {
    let session = open_fixture();
    let describe = session.describe();
    assert_eq!(describe.tier, DetailTier::Full);
    assert_eq!(describe.core_type_count, 5);

    let json = serde_json::to_value(&describe).unwrap();
    let node_types = json["schema"]["node_types"].as_array().expect("node_types");
    assert_eq!(node_types.len(), 5);
    assert_eq!(json["schema"]["node_count"], 118);
    assert_eq!(json["schema"]["edge_count"], 560);
}

#[test]
fn a_graph_with_more_types_than_the_tier_allows_is_clipped_not_dropped() {
    // The fixture is tier 1, so the coarser tiers need their own input. This
    // builds one in memory: the tier decision, the bound and the truncation
    // metadata are the code path a 1000-type graph takes, and P2 must not ship
    // it untested just because no fixture reaches that size.
    use kglite::api::session::{execute_mut, ExecuteOptions};
    use kglite::api::DirGraph;

    let mut script = String::new();
    for i in 0..60 {
        script.push_str(&format!("CREATE (:T{i} {{n: {i}}})\n"));
    }
    let mut graph = DirGraph::new();
    let params = std::collections::HashMap::new();
    execute_mut(&mut graph, &script, &ExecuteOptions::eager(&params)).expect("build");

    let mut view = View::new();
    let meta = meta_graph::compute(&graph, &mut view);
    assert_eq!(
        meta.meta.tier,
        DetailTier::Compact,
        "60 core types is tier 2"
    );
    assert_eq!(meta.meta.nodes.len(), 60, "tier 2 still sends every type");
    assert!(!meta.meta.node_bound.truncated);
}

#[test]
fn the_top_types_tier_truncates_and_reports_it() {
    // Constructed directly rather than through a 201-type graph: the bound is
    // what is under test, and building 201 kglite types to reach it would be
    // testing kglite.
    use kglite_visual_core::bound::{apply, Bound};

    let types: Vec<u32> = (0..300).collect();
    let (kept, info) = apply(
        types,
        Bound {
            max_items: meta_graph::TOP_TYPES_LIMIT,
            max_bytes: meta_graph::MAX_META_BYTES,
        },
        |_| 100,
    );
    assert_eq!(kept.len(), 50);
    assert_eq!(info.returned, 50);
    assert_eq!(info.total, 300);
    assert!(info.truncated, "the UI must be told 250 types are missing");
}
