//! Loading a graph, from either of the two sources the product supports.

use std::path::Path;
use std::sync::Arc;

use kglite::api::io::{load_file, load_kgl_bytes};
use kglite::api::DirGraph;

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
            Ok(load_file(path_str)?)
        }
        GraphSource::Bytes(bytes) => Ok(load_kgl_bytes(bytes)?),
    }
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
