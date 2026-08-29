//! A `.kgl` saved without a cardinality cache must still report real counts.
//!
//! **What this used to pin, and what it pins now.** kglite 0.16.13 persisted
//! the type-connectivity cache only if the in-memory graph already held one; a
//! file saved without it loaded with triples *derived* from
//! `connection_type_metadata` whose counts were all zero, and that derived set
//! was then itself cached, so the O(E) walk that would have corrected it never
//! ran. The viewer drew every relationship type claiming zero edges on a graph
//! with 765 373 of them (hit live on a 546 850-node graph, 2026-08-29).
//! This project carried a load-time repair for it; **kglite 0.16.14 fixed it
//! at the source** — the load path leaves the cache cold when real counts are
//! absent and distrusts persisted all-zero triples on a graph that has edges —
//! and the repair was deleted in the same change that moved the floor.
//!
//! So this file no longer covers code of ours. It covers the engine
//! **contract** the whole entry screen rests on, on the exact save shape that
//! broke it: a graph built through Cypher `CREATE` and saved without touching
//! the connectivity accessor, which is what an ingest pipeline that never ran
//! `describe()` or a planner produces. Calling
//! `get_or_compute_type_connectivity()` before the save would *fill* the cache
//! and defeat the test. It is two seconds of test time against a
//! silent-wrong-answer class that has bitten this project once already; if a
//! future engine regresses here, the meta-graph goes quietly wrong again and
//! this is what says so.

use std::collections::HashMap;
use std::path::Path;

use kglite::api::io::{prepare_kgl_write, write_kgl};
use kglite::api::session::{execute_mut, ExecuteOptions};
use kglite::api::{DirGraph, GraphRead};
use kglite_visual_core::{load_graph, GraphSource, Session, View};

/// Nodes and edges with a known, asymmetric count per triple, so a fallback
/// that fabricated a uniform number would fail as loudly as one that returned
/// zeros.
const SCRIPT: &str = "
CREATE (a:Person {title: 'a'})
CREATE (b:Person {title: 'b'})
CREATE (c:Person {title: 'c'})
CREATE (x:Company {title: 'x'})
CREATE (y:Company {title: 'y'})
CREATE (a)-[:KNOWS]->(b)
CREATE (b)-[:KNOWS]->(c)
CREATE (c)-[:KNOWS]->(a)
CREATE (a)-[:WORKS_AT]->(x)
CREATE (b)-[:WORKS_AT]->(y)
";

/// Build the graph and save it, with the connectivity cache deliberately cold.
fn write_cacheless_graph(path: &Path) {
    let mut graph = DirGraph::new();
    let params = HashMap::new();
    execute_mut(&mut graph, SCRIPT, &ExecuteOptions::eager(&params)).expect("the script executes");
    let mut graph = std::sync::Arc::new(graph);
    prepare_kgl_write(&mut graph);
    write_kgl(&graph, path.to_str().expect("ASCII temp path")).expect("the graph saves");
}

#[test]
fn a_cacheless_kgl_still_reports_real_relationship_counts() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("cacheless.kgl");
    write_cacheless_graph(&path);

    let graph = load_graph(GraphSource::Path(&path)).expect("the saved graph loads");
    // The premise, asserted rather than assumed: a graph with no edges would
    // report zero counts honestly and this test would pass for the wrong
    // reason.
    assert_eq!(graph.graph.edge_count(), 5, "five edges were written");

    let session = Session::open(graph, "cacheless.kgl");
    let meta = session.meta_graph();

    let counts: HashMap<&str, u32> = meta
        .meta
        .edges
        .iter()
        .map(|e| (e.name.as_str(), e.count))
        .collect();
    assert_eq!(
        counts.get("KNOWS"),
        Some(&3),
        "three Person-KNOWS-Person edges, not a zero derived from the metadata"
    );
    assert_eq!(
        counts.get("WORKS_AT"),
        Some(&2),
        "two Person-WORKS_AT-Company edges"
    );
    assert!(
        meta.meta.edges.iter().all(|e| e.count > 0),
        "no meta edge may carry a fabricated zero: {:?}",
        meta.meta.edges
    );
}

#[test]
fn a_cacheless_kgl_previews_the_expansion_it_would_perform() {
    // The other consumer of the same cache. A type-level preview reads the
    // triples too, and it *skips* zero-count ones — so on a poisoned file the
    // drill-in panel is not merely wrong, it is empty.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("cacheless.kgl");
    write_cacheless_graph(&path);

    let graph = load_graph(GraphSource::Path(&path)).expect("the saved graph loads");
    let mut view = View::new();
    let _ = kglite_visual_core::meta_graph::compute(&graph, &mut view);
    let previews = kglite_visual_core::expand::preview_for_type(&graph, "Person");
    assert!(
        previews
            .iter()
            .any(|p| p.name == "WORKS_AT" && p.count == 2),
        "Person should preview two WORKS_AT edges, got {previews:?}"
    );
}
