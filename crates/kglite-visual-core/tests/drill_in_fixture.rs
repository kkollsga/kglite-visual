//! L1: the drill-in path against the committed fixture (test-plan §L1).
//!
//! Every number here is asserted exactly and every one was *observed* against
//! `meta.kgl` before it was written down, not derived from what the code ought
//! to do. The fixture is 60 Person, 20 Project, 20 Skill, 10 City, 8 Company
//! with 560 edges, so the whole neighbourhood of any type fits in memory and
//! the assertions can be exact rather than "more than zero".
//!
//! The bound tests are the ones that matter most: `R1` says a verification that
//! has never been seen failing is not a verification, and "the response bound
//! is enforced in core" is the claim this whole phase rests on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kglite_visual_core::expand::{self, PreviewScope, MAX_EXPANSION_NODES};
use kglite_visual_core::request::{
    CypherRequest, EdgeDirection, ExpandRequest, Request, SearchMode, SearchRequest, SlotRequest,
    TypeRequest,
};
use kglite_visual_core::session::response_frames;
use kglite_visual_core::view::SliceKind;
use kglite_visual_core::{
    decode_frame, load_graph, GraphSource, MessageType, Response, Session, PROTOCOL_VERSION,
};

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

/// Person is the largest type, so the meta-graph gives it slot 0.
const PERSON_SLOT: u32 = 0;

fn expand_request(relationship: &str, direction: EdgeDirection, limit: Option<u32>) -> Request {
    Request::Expand(ExpandRequest {
        slot: PERSON_SLOT,
        relationship: Some(relationship.to_string()),
        direction,
        limit,
    })
}

fn slice(session: &Session, request: &Request) -> kglite_visual_core::GraphSlice {
    match session.handle(request).expect("request succeeds") {
        Response::Slice(slice) => slice,
        other => panic!("expected a graph slice, got {other:?}"),
    }
}

#[test]
fn a_type_level_preview_counts_every_relationship_without_fetching_a_node() {
    let session = open_fixture();
    let preview = session.preview(PERSON_SLOT).expect("Person has a slot");

    assert_eq!(preview.scope, PreviewScope::Type);
    assert_eq!(preview.node_type, "Person");
    assert_eq!(preview.title, "", "a type node has no title of its own");

    let rows: Vec<(&str, EdgeDirection, &str, u32)> = preview
        .relationships
        .iter()
        .map(|r| (r.name.as_str(), r.direction, r.other_type.as_str(), r.count))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("HAS_SKILL", EdgeDirection::Out, "Skill", 180),
            ("KNOWS", EdgeDirection::Out, "Person", 180),
            ("KNOWS", EdgeDirection::In, "Person", 180),
            ("CONTRIBUTES_TO", EdgeDirection::Out, "Project", 93),
            ("WORKS_AT", EdgeDirection::Out, "Company", 60),
        ],
        "largest first, then name, then far type — the order the buttons draw in"
    );
    assert_eq!(preview.total_edges, 693);
    assert_eq!(preview.max_nodes, MAX_EXPANSION_NODES as u32);
    assert_eq!(
        session.info().slot_count,
        5,
        "a preview allocates nothing — that is the whole point of previewing"
    );
}

#[test]
fn a_node_level_preview_counts_that_nodes_own_edges() {
    let session = open_fixture();
    // Reach an instance slot the only way a client can: expand first.
    let slice = slice(
        &session,
        &expand_request("WORKS_AT", EdgeDirection::Out, Some(4)),
    );
    let person = slice
        .meta
        .nodes
        .iter()
        .find(|n| n.node_type == "Person")
        .expect("a WORKS_AT expansion admits Person nodes");

    let preview = session.preview(person.slot).expect("an instance slot");
    assert_eq!(preview.scope, PreviewScope::Node);
    assert_eq!(preview.node_type, "Person");
    assert!(!preview.title.is_empty(), "an instance carries its title");
    assert!(
        preview.total_edges > 0 && preview.total_edges < 693,
        "one person's edges, not the whole type's: {}",
        preview.total_edges
    );
    for relationship in &preview.relationships {
        assert!(
            relationship.count > 0,
            "a preview button promising zero edges is worse than no button"
        );
    }
}

#[test]
fn expanding_a_type_node_appends_instance_slots_to_the_same_space() {
    // The flagship drill-in (D4's identity contract): the type node keeps slot
    // 0 and the instances it expands into take 5, 6, 7 … from the very same
    // allocator, so a link between them is just a pair of indices.
    let session = open_fixture();
    let slice = slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Both, None),
    );

    assert_eq!(slice.meta.kind, SliceKind::Expand);
    assert_eq!(slice.meta.first_slot, 5, "the five type nodes keep 0..4");
    assert_eq!(
        slice.meta.nodes.len(),
        60,
        "every Person is KNOWS-connected"
    );
    assert_eq!(slice.meta.slot_count, 65);
    assert_eq!(slice.meta.tombstone_count, 0);
    assert!(!slice.meta.bound.truncated);
    assert_eq!(slice.meta.bound.returned, 60);
    assert_eq!(slice.meta.bound.total, 60);
    // Nothing was cut, and the link half of the bound says so too: the fixture
    // is far under the shared byte budget, so this is the ceiling staying quiet.
    assert_eq!(slice.meta.link_bound.returned, 180);
    assert_eq!(slice.meta.link_bound.total, 180);
    assert!(!slice.meta.link_bound.truncated);

    let slots: Vec<u32> = slice.meta.nodes.iter().map(|n| n.slot).collect();
    assert_eq!(slots, (5..65).collect::<Vec<u32>>(), "dense, appended");
    assert_eq!(
        slice.points.len(),
        60 * 2,
        "one (x, y) pair per NEW slot — expansion appends, it does not resend"
    );

    // Links are re-sent whole (D4), so the meta-graph's own seven edges are
    // still in there. `setLinks` replaces the buffer; a partial upload would
    // silently erase everything it omitted.
    assert!(
        slice.meta.edges.iter().filter(|e| e.meta).count() == 7,
        "the meta-graph's links survive an expansion"
    );
    assert_eq!(slice.links.len(), slice.meta.edges.len() * 2);
    for index in &slice.links {
        assert!(
            *index >= 0.0 && *index < slice.meta.slot_count as f32,
            "link index {index} points outside the slot space"
        );
    }
}

#[test]
fn the_expansion_bound_can_fail_and_says_so() {
    // R1: the bound has been seen firing. 60 Person nodes are KNOWS-reachable
    // and the request asks for 40, so the answer is 40 with `truncated` set and
    // the true total intact — "showing 40 of 60", never a bare "40".
    let session = open_fixture();
    let slice = slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Out, Some(40)),
    );

    assert_eq!(slice.meta.bound.returned, 40);
    assert_eq!(slice.meta.bound.total, 60);
    assert!(slice.meta.bound.truncated);
    assert_eq!(slice.meta.nodes.len(), 40);
    assert_eq!(slice.meta.slot_count, 45);

    // Every link sent has both endpoints in the slot space. The bound dropping
    // a node must drop the links to it too, or the client is handed an index
    // into something it was never given.
    for edge in &slice.meta.edges {
        assert!(edge.source_slot < 45 && edge.target_slot < 45, "{edge:?}");
    }

    // …and the response says how many it dropped doing so. Without this, 40
    // nodes carrying 108 of their 180 KNOWS edges read as a complete
    // neighbourhood, which is the node bound's own failure mode one level down.
    assert_eq!(slice.meta.link_bound.returned, 108);
    assert_eq!(slice.meta.link_bound.total, 180);
    assert!(slice.meta.link_bound.truncated);
}

#[test]
fn a_client_cannot_ask_for_an_unbounded_expansion() {
    // The other half of R1: there is no request that gets past the ceiling.
    // `u32::MAX` is clamped rather than honoured or rejected — a client asking
    // for too much wants as much as it can have, and `truncated` is how it
    // learns it did not get everything.
    let session = open_fixture();
    let slice = slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Both, Some(u32::MAX)),
    );
    assert_eq!(
        expand::effective_bound(Some(u32::MAX)).max_items,
        MAX_EXPANSION_NODES
    );
    assert_eq!(
        slice.meta.nodes.len(),
        60,
        "the fixture is smaller than the ceiling, so the ceiling is what did not fire"
    );
    assert!(slice.meta.nodes.len() <= MAX_EXPANSION_NODES);
}

#[test]
fn expanding_without_a_relationship_names_every_link_it_returns() {
    // The slow walk: with no relationship named the type comes off each edge,
    // and a link the client cannot name is a link the user cannot interpret.
    let session = open_fixture();
    let slice = slice(
        &session,
        &Request::Expand(ExpandRequest {
            slot: PERSON_SLOT,
            relationship: None,
            direction: EdgeDirection::Out,
            limit: Some(30),
        }),
    );
    let names: std::collections::BTreeSet<&str> = slice
        .meta
        .edges
        .iter()
        .filter(|e| !e.meta)
        .map(|e| e.name.as_str())
        .collect();
    assert!(!names.is_empty());
    for name in &names {
        assert!(
            ["KNOWS", "HAS_SKILL", "CONTRIBUTES_TO", "WORKS_AT"].contains(name),
            "unexpected relationship {name}"
        );
    }
}

#[test]
fn collapse_tombstones_the_instances_and_keeps_the_type_node() {
    // 45 slots is under the compaction minimum, so this exercises the tombstone
    // path on its own — the NaN-position semantics cosmos.gl reads as absence.
    let session = open_fixture();
    slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Out, Some(40)),
    );

    let collapsed = slice(
        &session,
        &Request::Collapse(SlotRequest { slot: PERSON_SLOT }),
    );
    assert_eq!(collapsed.meta.kind, SliceKind::Collapse);
    assert_eq!(collapsed.meta.tombstones, (5..45).collect::<Vec<u32>>());
    assert_eq!(collapsed.meta.tombstone_count, 40);
    assert_eq!(
        collapsed.meta.slot_count, 45,
        "tombstoning never returns slots to the allocator"
    );
    assert!(
        collapsed.compaction.is_none(),
        "45 slots is under the compaction minimum: the remap costs more than the waste"
    );
    assert_eq!(
        collapsed.meta.edges.len(),
        7,
        "back to the meta-graph's own links; a link to a tombstone is an index into nothing"
    );
    assert!(collapsed.meta.edges.iter().all(|e| e.meta));

    let info = session.info();
    assert_eq!(info.slot_count, 45);
    assert_eq!(info.tombstone_count, 40);
}

#[test]
fn a_mostly_dead_view_compacts_and_carries_the_remap_with_it() {
    // 65 slots with 60 tombstones is 92% waste, over both the ratio and the
    // minimum, so the collapse response carries the old→new remap. The client
    // applies it to its id↔slot map before the next frame; a compaction it did
    // not hear about would silently re-label every selection it holds.
    let session = open_fixture();
    let expanded = slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Both, None),
    );
    assert_eq!(expanded.meta.slot_count, 65);

    let collapsed = slice(
        &session,
        &Request::Collapse(SlotRequest { slot: PERSON_SLOT }),
    );
    let remap = collapsed
        .compaction
        .as_ref()
        .expect("92% tombstones must compact");
    assert_eq!(remap.protocol_version, PROTOCOL_VERSION);
    assert_eq!(remap.reclaimed, 60);
    assert_eq!(remap.slot_count, 5);
    assert_eq!(
        remap.old_to_new.len(),
        65,
        "one entry per pre-compaction slot"
    );
    assert_eq!(
        remap.old_to_new[..5],
        [Some(0), Some(1), Some(2), Some(3), Some(4)],
        "the five type nodes keep their places, so the view does not reshuffle"
    );
    assert!(
        remap.old_to_new[5..].iter().all(Option::is_none),
        "every instance slot is gone"
    );

    assert_eq!(collapsed.meta.slot_count, 5);
    assert_eq!(collapsed.meta.tombstone_count, 0);
    assert_eq!(
        collapsed.meta.first_slot, 0,
        "after a compaction every slot moved, so the client cannot splice"
    );
    assert_eq!(collapsed.points.len(), 10, "the whole position array");
    assert_eq!(collapsed.meta.edges.len(), 7);
    assert_eq!(session.info().slot_count, 5);
}

#[test]
fn a_slot_that_names_nothing_is_a_request_error_not_a_panic() {
    let session = open_fixture();
    for request in [
        Request::Preview(SlotRequest { slot: 9_999 }),
        Request::NodeDetail(SlotRequest { slot: 9_999 }),
        Request::Expand(ExpandRequest {
            slot: 9_999,
            relationship: None,
            direction: EdgeDirection::Both,
            limit: None,
        }),
    ] {
        let err = session
            .handle(&request)
            .expect_err("slot 9999 does not exist");
        assert!(
            err.to_string().contains("9999"),
            "the message must name the slot it refused: {err}"
        );
    }

    // A type node has no stored properties, and saying so beats an empty panel.
    let err = session
        .handle(&Request::NodeDetail(SlotRequest { slot: PERSON_SLOT }))
        .expect_err("a type node is not an instance");
    assert!(err.to_string().contains("not an instance node"), "{err}");
}

#[test]
fn a_cypher_result_arrives_columnar_and_bounded() {
    let session = open_fixture();
    let request = Request::Cypher(CypherRequest {
        query: "MATCH (p:Person)-[:WORKS_AT]->(c:Company) \
                RETURN c.title AS company, count(p) AS staff ORDER BY staff DESC"
            .to_string(),
        params: Default::default(),
        limit: Some(3),
        as_graph: false,
    });
    let Response::Query(table) = session.handle(&request).expect("the query runs") else {
        panic!("expected a table");
    };

    assert_eq!(table.columns, vec!["company", "staff"]);
    assert_eq!(table.data.len(), 2, "one array per column");
    assert_eq!(table.data[0].len(), 3);
    assert_eq!(table.data[0][0], serde_json::json!("Company_3"));
    assert_eq!(table.data[1][0], serde_json::json!(12));
    assert_eq!(table.bound.returned, 3);
    assert_eq!(table.bound.total, 8, "eight companies employ someone");
    assert!(table.bound.truncated);
    assert!(!table.explain);
}

#[test]
fn a_query_parameter_is_bound_not_interpolated() {
    let session = open_fixture();
    let mut params = std::collections::BTreeMap::new();
    // A value that would change the query's meaning if it were pasted in.
    params.insert("name".to_string(), serde_json::json!("Person_7' OR 1=1 --"));
    let request = Request::Cypher(CypherRequest {
        query: "MATCH (p:Person) WHERE p.title = $name RETURN p.title AS t".to_string(),
        params,
        limit: None,
        as_graph: false,
    });
    let Response::Query(table) = session.handle(&request).expect("the query runs") else {
        panic!("expected a table");
    };
    assert_eq!(
        table.bound.total, 0,
        "the parameter matched nothing, which is what a bound value does"
    );
}

#[test]
fn a_broken_query_returns_kglites_own_diagnostic() {
    // "query failed" would throw away the position and the expected token —
    // the only part of the message a user can act on.
    let session = open_fixture();
    let err = session
        .handle(&Request::Cypher(CypherRequest {
            query: "MATCH (p:Person RETURN p".to_string(),
            params: Default::default(),
            limit: None,
            as_graph: false,
        }))
        .expect_err("unbalanced parenthesis");
    let message = err.to_string();
    assert!(
        message.len() > 20,
        "a bare failure is not a diagnostic: {message}"
    );
    assert!(
        message.to_lowercase().contains("cypher") || message.contains("RETURN"),
        "{message}"
    );
}

#[test]
fn a_mutating_query_is_refused_by_the_read_only_path() {
    // kglite's `execute_read` rejects mutations up front. A viewer that could
    // write would be a viewer that can corrupt someone else's `.kgl`.
    let session = open_fixture();
    let err = session
        .handle(&Request::Cypher(CypherRequest {
            query: "CREATE (:Person {title: 'intruder'})".to_string(),
            params: Default::default(),
            limit: None,
            as_graph: false,
        }))
        .expect_err("a viewer performs no writes");
    assert!(!err.to_string().is_empty());
}

#[test]
fn show_in_graph_maps_a_result_into_the_slot_space() {
    let session = open_fixture();
    let request = Request::Cypher(CypherRequest {
        query: "MATCH (p:Person)-[r:WORKS_AT]->(c:Company) RETURN p, r, c".to_string(),
        params: Default::default(),
        limit: Some(5),
        as_graph: true,
    });
    let Response::Slice(slice) = session.handle(&request).expect("the query runs") else {
        panic!("as_graph must answer with a slice");
    };
    assert_eq!(slice.meta.kind, SliceKind::Query);
    assert!(!slice.meta.nodes.is_empty());
    assert!(
        slice.meta.edges.iter().any(|e| e.name == "WORKS_AT"),
        "the relationships the result named become links"
    );
    assert_eq!(slice.points.len(), slice.meta.nodes.len() * 2);
}

#[test]
fn search_is_type_scoped_bounded_and_reports_what_it_searched() {
    let session = open_fixture();
    let request = Request::Search(SearchRequest {
        query: "person_1".to_string(),
        node_type: Some("Person".to_string()),
        property: Some("title".to_string()),
        mode: SearchMode::Contains,
        limit: Some(4),
    });
    let Response::Search(found) = session.handle(&request).expect("the search runs") else {
        panic!("expected search hits");
    };

    assert_eq!(found.property, "title");
    assert_eq!(found.node_type.as_deref(), Some("Person"));
    assert_eq!(found.hits.len(), 4);
    assert_eq!(found.bound.returned, 4);
    assert!(
        found.bound.truncated,
        "Person_1, _10, _11, _12, _13 is five"
    );
    for hit in &found.hits {
        assert_eq!(hit.node_type, "Person");
        assert!(hit.label.to_lowercase().contains("person_1"));
        assert_eq!(hit.slot, None, "nothing is loaded yet");
    }
}

#[test]
fn a_search_hit_already_on_screen_reports_its_slot() {
    // The distinction only the session can make: an already-loaded hit is
    // highlighted in place, a cold one offers "load into view".
    let session = open_fixture();
    slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Both, None),
    );

    let Response::Search(found) = session
        .handle(&Request::Search(SearchRequest {
            query: "person_0".to_string(),
            node_type: Some("Person".to_string()),
            property: Some("title".to_string()),
            mode: SearchMode::StartsWith,
            limit: None,
        }))
        .expect("the search runs")
    else {
        panic!("expected search hits");
    };
    assert!(!found.hits.is_empty());
    assert!(
        found.hits.iter().all(|h| h.slot.is_some()),
        "every Person is loaded, so every hit has a slot"
    );
}

#[test]
fn a_search_property_that_is_not_an_identifier_is_refused() {
    // The one place caller text reaches a query as syntax rather than as a
    // parameter, so it is the one place that has to be total.
    let session = open_fixture();
    let err = session
        .handle(&Request::Search(SearchRequest {
            query: "x".to_string(),
            node_type: None,
            property: Some("title RETURN 1 //".to_string()),
            mode: SearchMode::Contains,
            limit: None,
        }))
        .expect_err("that is not a property name");
    assert!(err.to_string().contains("plain identifier"), "{err}");
}

#[test]
fn property_stats_offer_channels_and_mark_what_is_not_exact() {
    let session = open_fixture();
    let Response::PropertyStats(stats) = session
        .handle(&Request::PropertyStats(TypeRequest {
            node_type: "Person".to_string(),
        }))
        .expect("Person has properties")
    else {
        panic!("expected property stats");
    };

    assert_eq!(stats.node_count, 60);
    assert!(
        !stats.sampled,
        "60 nodes is far under kglite's {} exact-scan ceiling",
        stats.exact_scan_ceiling
    );
    assert_eq!(stats.exact_scan_ceiling, 200_000);
    assert_eq!(
        stats.numeric_candidates,
        vec!["active", "age", "gid", "joined_year", "score"]
    );
    assert_eq!(
        stats.categorical_candidates,
        vec!["city"],
        "ten cities across sixty people is a legend a human can read"
    );

    // kglite caps its distinct-value set at 32 and sets `approx` when it hits
    // the cap, so `age` is a lower bound even on a 60-node graph. That flag has
    // to survive to the UI text — a "33 distinct values" that is really "33 or
    // more" is a wrong number, not a smaller one.
    let age = stats
        .properties
        .iter()
        .find(|p| p.name == "age")
        .expect("Person has an age");
    assert!(age.approx, "33 > the 32-value cap");
    assert_eq!(age.unique, 33);

    let city = stats
        .properties
        .iter()
        .find(|p| p.name == "city")
        .expect("Person has a city");
    assert!(!city.approx);
    assert_eq!(city.unique, 10);
    assert_eq!(city.values.len(), 10, "an exact set is enumerated");

    let unknown = session.handle(&Request::PropertyStats(TypeRequest {
        node_type: "Nope".to_string(),
    }));
    assert!(unknown.is_err());
}

#[test]
fn node_detail_reads_the_stored_properties_in_a_stable_order() {
    let session = open_fixture();
    let slice = slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Out, Some(6)),
    );
    let first = &slice.meta.nodes[0];

    let Response::NodeDetail(detail) = session
        .handle(&Request::NodeDetail(SlotRequest { slot: first.slot }))
        .expect("an instance slot has properties")
    else {
        panic!("expected node detail");
    };
    assert_eq!(detail.slot, first.slot);
    assert_eq!(detail.node_id, first.node_id);
    assert_eq!(detail.node_type, "Person");
    assert_eq!(detail.title, first.title);

    let keys: Vec<&str> = detail.properties.iter().map(|(k, _)| k.as_str()).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "a panel that reorders itself is unreadable");
    assert!(keys.contains(&"age"));
}

#[test]
fn the_binary_frames_and_the_json_twin_carry_the_same_answer() {
    // Test-plan §2 for the messages P3 added: one encoder, two serializers.
    // A divergence here is a bug, not a nuance — the twin is what an agent
    // verifies the wire path with.
    let session = open_fixture();
    let response = session
        .handle(&expand_request("WORKS_AT", EdgeDirection::Out, Some(9)))
        .expect("the expansion runs");
    let Response::Slice(slice) = &response else {
        panic!("expected a slice");
    };

    let frames = response_frames(&response);
    let decoded: Vec<_> = frames
        .iter()
        .map(|f| decode_frame(f).expect("every frame decodes"))
        .collect();
    assert_eq!(
        decoded.iter().map(|d| d.msg_type).collect::<Vec<_>>(),
        vec![
            MessageType::GraphSlice,
            MessageType::Points,
            MessageType::Links
        ],
        "metadata first, then the arrays — the same order the meta-graph uses"
    );
    assert!(decoded.last().expect("frames").terminal);

    let meta: serde_json::Value =
        serde_json::from_slice(&decoded[0].payload).expect("the metadata frame is JSON");
    assert_eq!(
        meta,
        serde_json::to_value(&slice.meta).expect("the twin serializes the same struct")
    );

    let points: Vec<f32> = decoded[1]
        .payload
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(
        points, slice.points,
        "the arrays are the same bytes either way"
    );
}

#[test]
fn a_compaction_takes_a_frame_of_its_own_on_the_wire() {
    // The remap is its own message type, not a field a client might skip: a
    // client that missed it would keep an id↔slot map describing a space that
    // no longer exists.
    let session = open_fixture();
    session
        .handle(&expand_request("KNOWS", EdgeDirection::Both, None))
        .expect("the expansion runs");
    let response = session
        .handle(&Request::Collapse(SlotRequest { slot: PERSON_SLOT }))
        .expect("the collapse runs");

    let types: Vec<MessageType> = response_frames(&response)
        .iter()
        .map(|f| decode_frame(f).expect("decodes").msg_type)
        .collect();
    assert_eq!(
        types,
        vec![
            MessageType::GraphSlice,
            MessageType::Compaction,
            MessageType::Points,
            MessageType::Links
        ]
    );
}

#[test]
fn two_sessions_over_one_graph_keep_separate_slot_spaces() {
    // `Arc<DirGraph>` is the shared, read-only handle; the view is not shared.
    // Two browser tabs on one server would otherwise expand into each other's
    // indices.
    let graph = load_graph(GraphSource::Path(&fixture_path())).expect("fixture loads");
    let a = Session::open(Arc::clone(&graph), "meta.kgl");
    let b = Session::open(graph, "meta.kgl");

    slice(&a, &expand_request("KNOWS", EdgeDirection::Out, Some(10)));
    assert_eq!(a.info().slot_count, 15);
    assert_eq!(b.info().slot_count, 5, "b saw none of a's expansion");
}

#[test]
fn an_expansion_that_triggers_a_compaction_still_reports_the_nodes_it_added() {
    // The defect this pins was found by driving the running server, not by a
    // test: expanding into a view that is already sparse enough to compact used
    // to drop the `nodes` list, so the sixty nodes the expansion had just added
    // arrived with no slot, no id, no title and no way to select them. Every
    // list in the slice metadata is in the PRE-compaction space and the client
    // applies the remap last, so `nodes` belongs there like everything else.
    let session = open_fixture();
    // 45 slots, 40 of them tombstoned — under the 64-slot compaction minimum,
    // so the view sits sparse rather than reclaiming.
    slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Out, Some(40)),
    );
    let collapsed = slice(
        &session,
        &Request::Collapse(SlotRequest { slot: PERSON_SLOT }),
    );
    assert!(collapsed.compaction.is_none());
    assert_eq!(collapsed.meta.tombstone_count, 40);

    // Now expand all 60: 105 slots with 40 dead is 38%, over both thresholds.
    let expanded = slice(
        &session,
        &expand_request("KNOWS", EdgeDirection::Both, None),
    );
    let remap = expanded
        .compaction
        .as_ref()
        .expect("38% tombstones across 105 slots must compact");
    assert_eq!(remap.reclaimed, 40);

    assert_eq!(
        expanded.meta.nodes.len(),
        60,
        "the nodes the expansion added must survive the compaction that followed it"
    );
    // Pre-compaction slots, because the client remaps last.
    let slots: Vec<u32> = expanded.meta.nodes.iter().map(|n| n.slot).collect();
    assert_eq!(slots, (45..105).collect::<Vec<u32>>());
    for node in &expanded.meta.nodes {
        assert!(!node.title.is_empty(), "a node arrived without its label");
        assert_eq!(
            remap.old_to_new[node.slot as usize],
            Some(node.slot - 40),
            "every added node has a live destination in the remap"
        );
    }
    assert_eq!(expanded.meta.first_slot, 0);
    assert_eq!(
        expanded.points.len(),
        65 * 2,
        "the whole array, from slot zero"
    );
}

/// The resync a newly connected client is handed, against a view that has
/// already moved (G5 commit 0).
///
/// The defect it replaces was structural rather than arithmetical: the greeting
/// described the session's *opening* state, so a client joining a drilled-into
/// view was given a slot space whose middle it had never been told about, and
/// the next broadcast's `first_slot` spliced positions in over the top of the
/// gap. Every assertion below is about the newcomer having no gap to fall into.
#[test]
fn a_resync_describes_the_whole_view_including_its_holes() {
    let session = open_fixture();
    // Expand 60 Person, then collapse 40 of them by tombstoning City's members
    // — a view with instances AND holes, which is the shape a naive "send the
    // slices again" resync gets wrong.
    slice(&session, &expand_request("KNOWS", EdgeDirection::Out, None));
    let city_slot = session
        .slot_of_type("City")
        .expect("City is on the meta-graph");
    slice(
        &session,
        &expand_request("LIVES_IN", EdgeDirection::Out, None),
    );
    slice(
        &session,
        &Request::Collapse(SlotRequest { slot: city_slot }),
    );

    let before = session.view_state();
    let sync = session.sync_slice();

    assert_eq!(sync.meta.kind, SliceKind::Sync);
    assert_eq!(
        sync.meta.first_slot, 0,
        "a newcomer has nothing to splice into"
    );
    assert_eq!(sync.meta.slot_count, before.slot_count);
    assert_eq!(
        sync.points.len(),
        before.slot_count as usize * 2,
        "positions for the whole space, meta-graph slots included"
    );
    assert!(sync.compaction.is_none(), "a resync reclaims nothing");
    assert_eq!(sync.meta.tombstones.len(), before.tombstone_count as usize);
    assert_eq!(
        sync.meta.nodes.len() as u32,
        before.live_count - before.types.len() as u32,
        "every live instance is named; the type nodes came with the meta-graph"
    );
    for node in &sync.meta.nodes {
        assert!(
            !sync.meta.tombstones.contains(&node.slot),
            "slot {} is named and a hole at the same time",
            node.slot
        );
        assert!(!node.title.is_empty(), "slot {} arrived unnamed", node.slot);
    }
    assert_eq!(sync.meta.edges.len(), before.link_count as usize);

    // …and it changed nothing. `last_slice` is what the bound last did, and a
    // client connecting is not something that happened to the view.
    let after = session.view_state();
    assert_eq!(after.slot_count, before.slot_count);
    assert_eq!(after.tombstone_count, before.tombstone_count);
    assert_eq!(
        after.last_slice.map(|s| s.kind),
        Some(SliceKind::Collapse),
        "the resync overwrote the report of the collapse that preceded it"
    );
}

/// The layout half of the resync: what the newcomer is told about *where*.
///
/// `None` under the simulation is the load-bearing case — the server does not
/// know where anything is there, and shipping a remembered static arrangement
/// to a client whose peers are all simulating would put it in a picture nobody
/// else is looking at.
#[test]
fn the_remembered_layout_is_offered_only_while_the_server_owns_the_geometry() {
    use kglite_visual_core::request::{LayoutKernel, LayoutRequest};

    let session = open_fixture();
    assert!(
        session.last_layout().is_none(),
        "a session starts under the viewer's GPU"
    );

    session
        .handle(&Request::Layout(LayoutRequest {
            kernel: LayoutKernel::Islands,
            seed_slot: None,
        }))
        .expect("a layout over the meta-graph succeeds");
    let held = session
        .last_layout()
        .expect("a static kernel is remembered");
    assert!(held.meta.kernel_chosen.is_static());
    assert_eq!(
        held.points.len(),
        session.view_state().slot_count as usize * 2
    );

    session
        .handle(&Request::Layout(LayoutRequest {
            kernel: LayoutKernel::Simulation,
            seed_slot: None,
        }))
        .expect("handing the layout back succeeds");
    assert!(
        session.last_layout().is_none(),
        "the arrangement is the viewer's again, so the server has none to offer"
    );
}

/// The export's scope is the view, and the whole-graph path is not reachable
/// from it (plan E8, pre-mortem #4).
///
/// The structural half of that claim is a signature — `export_nodes` takes a
/// slice, not kglite's `Option<&CurrentSelection>` — and a signature cannot go
/// red. This is the observable half: the fixture holds 118 nodes, an expansion
/// puts 60 of them on screen, and the file that comes out has 60. A handler
/// that reached the engine's `None` would produce 118 and this test is what
/// says so.
#[test]
fn exporting_the_live_view_writes_the_view_and_never_the_whole_graph() {
    use kglite_visual_core::ExportFormat;

    let session = open_fixture();

    // The entry screen holds no instance nodes at all. A whole-graph dump would
    // be the *easiest* thing to answer here and is exactly what must not
    // happen, so the empty view is refused by name.
    let err = session
        .export_view(ExportFormat::Csv)
        .expect_err("an entry screen has nothing to export");
    assert!(err.to_string().contains("nothing to export"), "{err}");

    slice(&session, &expand_request("KNOWS", EdgeDirection::Out, None));
    let on_screen = session.view_state().live_count - session.view_state().types.len() as u32;
    assert_eq!(
        on_screen, 60,
        "the fixture's KNOWS expansion loads 60 Person"
    );

    let exported = session
        .export_view(ExportFormat::Csv)
        .expect("a loaded view exports");
    assert_eq!(exported.nodes, on_screen);
    let text = String::from_utf8(exported.bytes).expect("CSV is UTF-8");
    assert_eq!(
        text.lines().count() as u32,
        on_screen + 1,
        "one header and one row per node on screen — never the graph's 118"
    );
    assert!(
        !text.contains("Company_"),
        "a type nobody expanded reached the file: this is the whole-graph path"
    );
    assert!(
        exported.filename.ends_with("-view.csv"),
        "{}",
        exported.filename
    );
}
