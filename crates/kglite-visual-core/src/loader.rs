//! Loading a graph, from either of the two sources the product supports.

use std::path::Path;
use std::sync::Arc;

use kglite::api::io::{load_file_with, load_kgl_bytes_with, LoadOptions};
use kglite::api::DirGraph;

use crate::error::CoreError;

/// What a caller is willing to spend on a load.
///
/// **One of kglite 0.16.15's three `LoadOptions` levers is here, and the
/// other two were measured and rejected.** Both rejections are recorded with
/// their numbers, because "we should try mapped loading" is the kind of idea a
/// fresh session re-proposes every time it reads an RSS figure:
///
/// * **`storage`** (`memory` / `mapped`) is not a memory lever. Measured on
///   sodir_graph.kgl (133.6 MB, 546 850 nodes, 765 373 edges) on 2026-08-30,
///   two runs each: default 626.4 / 626.7 MB peak RSS, `mapped` 626.6 /
///   626.8 MB, `memory` 626.6 / 626.6 MB. A 0.4 MB spread over three modes,
///   because columns of 256 KB or more spill and are mmap'd on every path
///   already. What the mode decides is the backend the graph *continues* in,
///   and a read-only viewer never reaches that question.
/// * **`defer_index_rebuild`** buys nothing here and costs the one thing this
///   product is. sodir declares five indexes — `Wellbore.title`,
///   `Block.title`, `Stratigraphy.title`, `Discovery.wlbName` and a range
///   index — and `estimate_load_memory` models rebuilding all five at
///   **174 KB**, 0.08% of the 219 MB settled footprint. Measured either way:
///   load 0.76–0.85 s eager vs 0.77 s deferred, peak RSS 626.7 vs 626.6 MB.
///   Those `title` properties are exactly what `/api/search` and a
///   title-equality lookup read, so the deferral would trade 0.1 MB for a scan
///   on the interactive path. Upstream's −42.8% figure is real and is about a
///   500k-row fixture whose indexes are large; sodir's are not.
///
/// The ceiling below is the lever that does something for a viewer: a `.kgl`
/// too big for the machine is refused from the metadata head, before a byte is
/// decompressed, instead of being loaded into swap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadLimits {
    /// Refuse the load when kglite estimates its peak above this many
    /// megabytes. `None` leaves the process-wide `KGLITE_MAX_LOAD_MB` default
    /// (or no ceiling at all) in charge — an explicit value outranks it.
    ///
    /// Megabytes, matching the CLI flag and the `show()` keyword; kglite's
    /// Rust field is bytes, and the conversion happens once, below.
    pub max_load_mb: Option<u64>,
}

impl LoadLimits {
    fn to_options(self) -> LoadOptions {
        LoadOptions {
            max_load_bytes: self.max_load_mb.map(|mb| mb.saturating_mul(1024 * 1024)),
            ..Default::default()
        }
    }
}

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
    load_graph_with(source, LoadLimits::default())
}

/// [`load_graph`] under an explicit ceiling.
///
/// Split the way kglite splits its own `load_file` / `load_file_with`, so the
/// two sides of this boundary read the same: the bare form is exactly the
/// default options, and the `_with` form is where a caller states a budget.
///
/// A refused load arrives as [`CoreError::Load`] carrying
/// `io::ErrorKind::OutOfMemory` and kglite's own message, which names the
/// estimate, the ceiling, the term breakdown and the two ways out. It is a
/// statement about this process's budget, not about the file — nothing was
/// decompressed and the graph is not corrupt — which is why the kind is not
/// `InvalidData` and why the wheel raises `MemoryError` rather than
/// `ValueError` for it.
pub fn load_graph_with(
    source: GraphSource<'_>,
    limits: LoadLimits,
) -> Result<Arc<DirGraph>, CoreError> {
    let options = limits.to_options();
    match source {
        // kglite's `load_file_with` takes `&str`, so a non-UTF-8 path cannot be
        // expressed. Report that as the I/O error it will become rather than
        // panicking on `to_str().unwrap()`.
        GraphSource::Path(path) => {
            let path_str = path.to_str().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("graph path is not valid UTF-8: {}", path.display()),
                )
            })?;
            Ok(load_file_with(path_str, &options)?)
        }
        GraphSource::Bytes(bytes) => Ok(load_kgl_bytes_with(bytes, &options)?),
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
