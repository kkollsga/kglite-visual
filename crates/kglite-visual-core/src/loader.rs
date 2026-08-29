//! Loading a graph, from either of the two sources the product supports.

use std::path::Path;
use std::sync::Arc;

use kglite::api::introspection::compute_type_connectivity;
use kglite::api::io::{load_file, load_kgl_bytes};
use kglite::api::{DirGraph, GraphRead};

use crate::error::CoreError;

/// Where a graph's bytes come from.
///
/// The CLI always has a path; the Python wheel's `show(graph)` hands over
/// `KnowledgeGraph.to_bytes()`, because two cdylibs cannot share a
/// `&KnowledgeGraph` across the process boundary (plan D9).
#[derive(Debug)]
pub enum GraphSource<'a> {
    Path(&'a Path),
    /// A `.kgl` image already in memory.
    ///
    /// Not a pure in-memory path: kglite spills any column of 256 KB or more
    /// to `$TMPDIR` while decoding, so this needs a writable temp directory
    /// exactly as much as the file path does.
    Bytes(&'a [u8]),
}

/// Read a graph into the shared, immutable handle every consumer holds.
///
/// `Arc<DirGraph>` — never `Session`. A viewer performs no writes, and
/// `Arc<DirGraph>` is the handle kglite has verified `Send + Sync` for
/// concurrent readers; `Session` carries the copy-on-write mutation state a
/// read-only consumer must not own.
pub fn load_graph(source: GraphSource<'_>) -> Result<Arc<DirGraph>, CoreError> {
    match source {
        // kglite's `load_file` takes `&str`, so a non-UTF-8 path cannot be
        // expressed. Report that as the I/O error it will become rather than
        // panicking on `to_str().unwrap()`.
        GraphSource::Path(path) => {
            let path_str = path.to_str().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("graph path is not valid UTF-8: {}", path.display()),
                )
            })?;
            let graph = load_file(path_str)?;
            repair_zero_connectivity(&graph);
            Ok(graph)
        }
        GraphSource::Bytes(bytes) => {
            let graph = load_kgl_bytes(bytes)?;
            repair_zero_connectivity(&graph);
            Ok(graph)
        }
    }
}

/// Replace a fabricated all-zero connectivity cache with a real one.
///
/// **Upstream bug, kglite 0.16.13 (reported 2026-08-29, open).** `.kgl`
/// persists the type-connectivity cache only when the graph being saved
/// already holds one. Loading a file saved without it derives triples from
/// `connection_type_metadata` — the type *pairs* are right, the counts are all
/// zero — and then writes that derived set into the cache
/// (`graph/io/file.rs`, `apply_to_with`). `get_or_compute_type_connectivity()`
/// reads the cache first, so its O(E) fallback never fires and the poisoned
/// numbers are permanent for the session. Downstream the meta-graph draws every
/// relationship type claiming zero edges, and the expansion preview — which
/// skips zero-count triples — offers nothing to expand.
///
/// So: detect the poisoned shape and pay the O(E) scan once, at load, where
/// the cost is amortised into a load the user is already waiting for. Both
/// consumers (`meta_graph::compute`, `expand::preview_for_type`) then read a
/// healed cache and need no fallback of their own.
///
/// **Never silently.** A recomputed number is a different provenance from a
/// persisted one, and a viewer that quietly launders one into the other is the
/// thing P2 refused to build. The stderr line names the upstream bug so the
/// operator can tell "this file predates the fix" from "this viewer is
/// guessing".
fn repair_zero_connectivity(graph: &DirGraph) {
    let edges = graph.graph.edge_count();
    if edges == 0 {
        return;
    }
    let cached = graph.get_or_compute_type_connectivity();
    // An empty set is poisoned too: the derived-from-metadata path can produce
    // one, and an empty *cache* stops the O(E) fallback just as effectively as
    // a zero-filled one. A correct cache on a graph with edges can be neither.
    if !cached.iter().all(|t| t.count == 0) {
        return;
    }

    let started = std::time::Instant::now();
    let recomputed = compute_type_connectivity(graph);
    let total: usize = recomputed.iter().map(|t| t.count).sum();
    if total == 0 {
        // Every edge has an endpoint whose type the scan could not resolve.
        // That is a fact about the file, not something to paper over — leaving
        // the cache alone keeps the zeros attributable to the data.
        eprintln!(
            "kglite-visual: {edges} edges carry no resolvable endpoint types; \
             relationship counts stay at zero"
        );
        return;
    }
    let triples = recomputed.len();
    graph.set_type_connectivity(recomputed);
    eprintln!(
        "kglite-visual: this .kgl was saved without a type-connectivity cache, so kglite \
         0.16.13 loaded it with all-zero relationship counts (upstream bug, reported \
         2026-08-29). Recomputed {triples} triples covering {total} of {edges} edges \
         in {} ms.",
        started.elapsed().as_millis()
    );
}

/// Node counts per label, read from the persisted type index.
///
/// O(#types), not O(V): `type_indices` is maintained by the engine and
/// restored on load, so this stays cheap on a 100M-node graph — the property
/// the whole progressive-disclosure entry screen rests on.
pub fn node_counts_by_type(graph: &DirGraph) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = graph
        .type_indices
        .iter()
        .map(|(name, nodes)| (name.to_string(), nodes.len()))
        .collect();
    counts.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    counts
}
