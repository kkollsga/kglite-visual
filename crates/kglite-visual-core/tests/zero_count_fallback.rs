//! A `.kgl` saved without a cardinality cache must still report real counts.
//!
//! **The failure this pins.** kglite (0.16.13) persists the type-connectivity
//! cache only if the in-memory graph already holds one. A file saved without
//! it loads with triples *derived* from `connection_type_metadata` whose counts
//! are all zero, and that derived set is then itself cached — so
//! `get_or_compute_type_connectivity()` never performs the O(E) walk that would
//! correct it. The viewer's meta-graph then draws every relationship type
//! claiming zero edges on a graph with hundreds of thousands of them (hit live
//! on a 546 850-node / 765 373-edge graph, 2026-08-29).
//!
//! The graph below is built through Cypher `CREATE` and saved without touching
//! the connectivity accessor first, which is exactly the shape of a `.kgl`
//! produced by an ingest pipeline that never ran `describe()` or a planner.
//! Calling `get_or_compute_type_connectivity()` before the save would *fill*
//! the cache and defeat the test.

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
    // The premise, asserted rather than assumed: if kglite ever starts saving
    // (or recomputing) the cache on this path, the fallback becomes
    // unreachable and this test would otherwise pass for the wrong reason.
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
