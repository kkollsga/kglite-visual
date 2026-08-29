//! Proves the pinned kglite dependency is more than a manifest line.
//!
//! "It compiles" is a weak test (CLAUDE.md → "A reported status is not the
//! result"): a dependency floor can compile and still misbehave, and this one
//! crosses a data boundary — the `.kgl` file. So the test writes a real graph
//! through the engine, reads it back through *our* loader, and checks the
//! numbers survived the round trip.

use std::collections::HashMap;
use std::sync::Arc;

use kglite::api::io::{prepare_kgl_write, write_kgl};
use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::DirGraph;
use kglite_visual_core::{load_graph, node_counts_by_type, GraphSource};

/// Two Person nodes, one City, one relationship — small enough to assert
/// exactly, wide enough that a type index collapsing to a single bucket would
/// show up.
const FIXTURE_CYPHER: &str = "
    CREATE (a:Person {name: 'ada'})
    CREATE (b:Person {name: 'linus'})
    CREATE (c:City {name: 'oslo'})
    CREATE (a)-[:KNOWS]->(b)
    CREATE (a)-[:LIVES_IN]->(c)
";

fn build_fixture() -> Arc<DirGraph> {
    let mut graph = DirGraph::new();
    let params = HashMap::new();
    let opts = ExecuteOptions::eager(&params);
    execute_mut(&mut graph, FIXTURE_CYPHER, &opts).expect("fixture Cypher must execute");
    Arc::new(graph)
}

#[test]
fn tiny_graph_round_trips_through_a_kgl_file() {
    let mut graph = build_fixture();
    assert_eq!(
        node_counts_by_type(&graph),
        vec![("City".to_string(), 1), ("Person".to_string(), 2)],
        "in-memory build must land in the type index before it is ever saved"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("fixture.kgl");
    let path_str = path.to_str().unwrap();

    prepare_kgl_write(&mut graph);
    write_kgl(&graph, path_str).expect("write .kgl");

    let reloaded = load_graph(GraphSource::Path(&path)).expect("load .kgl through our wrapper");
    assert_eq!(
        node_counts_by_type(&reloaded),
        vec![("City".to_string(), 1), ("Person".to_string(), 2)],
        "type index must be restored on load, not silently empty"
    );

    let bytes = std::fs::read(&path).expect("read .kgl bytes");
    let from_bytes = load_graph(GraphSource::Bytes(&bytes)).expect("load .kgl bytes");
    assert_eq!(
        node_counts_by_type(&from_bytes),
        node_counts_by_type(&reloaded),
        "the bytes handover (the Python show(graph) path) must agree with the file path"
    );
}

#[test]
fn a_missing_file_is_a_load_error_not_a_panic() {
    // `DirGraph` is not `Debug`, so `expect_err` (which formats the Ok value)
    // is unavailable — match the Result instead.
    match load_graph(GraphSource::Path(std::path::Path::new(
        "/nonexistent/kglite-visual/never.kgl",
    ))) {
        Err(kglite_visual_core::CoreError::Load(_)) => {}
        Err(other) => panic!("expected the io::Error family, got {other:?}"),
        Ok(_) => panic!("a missing file must fail"),
    }
}

/// The engine's half of the `row_limit` contract, pinned at the floor.
///
/// This file exists because "it compiles" is a weak test of a dependency
/// floor, and `row_limit` is now load-bearing for the response bound: the
/// banner's total comes from `QueryDiagnostics::total_rows`, and the rows above
/// the cap are supposed never to be built. Both halves are asserted against
/// the engine directly, so a floor move that quietly changed either — a cap
/// that stopped biting, a total that became a lower bound — arrives here as a
/// failure rather than as a wrong number in a picture.
///
/// It is a row-count proxy, not a memory measurement: nothing this side of the
/// engine can weigh the `Vec` that was never allocated. What it does establish
/// is that the rows above the cap do not exist in the value we are handed.
/// **Our** wiring of the cap is proved separately, in `query.rs` — this test
/// constructs its own `ExecuteOptions` and so cannot see that wiring at all.
#[test]
fn the_engine_stops_retaining_rows_at_the_cap_and_still_counts_them_all() {
    const NODES: usize = 500;
    const CAP: usize = 7;

    let mut graph = DirGraph::new();
    let params = HashMap::new();
    let opts = ExecuteOptions::eager(&params);
    let script: String = (0..NODES)
        .map(|i| format!("CREATE (:Item {{n: {i}}})\n"))
        .collect();
    execute_mut(&mut graph, &script, &opts).expect("fixture Cypher must execute");

    let mut capped = ExecuteOptions::eager(&params);
    capped.row_limit = Some(CAP);
    let outcome = kglite::api::session::execute_read(&graph, "MATCH (n:Item) RETURN n.n", &capped)
        .expect("the query runs");

    assert_eq!(
        outcome.result.rows.len(),
        CAP,
        "row_limit did not reach the executor: it handed back every row"
    );
    let diagnostics = outcome
        .result
        .diagnostics
        .expect("a capped execution reports diagnostics");
    assert_eq!(
        diagnostics.row_limit,
        Some(CAP),
        "the cap in force is echoed whether or not it bit"
    );
    assert_eq!(
        diagnostics.total_rows,
        Some(NODES as u64),
        "the pre-truncation total must be exact — it is what the banner prints"
    );
}
