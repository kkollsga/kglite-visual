//! Saved queries: a small, bounded, human-owned file store beside the config.
//!
//! **In `cli`, never in `core`.** `kglite-visual-core` is transport-agnostic by
//! rule, and a crate that opens files has decided something about its host. The
//! store is a property of *this* front door — the localhost server and the
//! wheel that lib-links it — and both reach it through [`crate::broadcast::AppState`],
//! which is why `show()` shares a graph's saved queries with the CLI without
//! either of them arranging it.
//!
//! ## Why a file store and not the browser
//!
//! `localStorage` is keyed by origin, and an origin includes the port.
//! `--port 0` is the documented default for every agent and CI invocation, so
//! the browser's own storage would hand a *different* store to every launch —
//! a data-loss mechanism wearing a persistence layer. A sidecar file beside the
//! `.kgl` is out for three independent reasons: `.kgl` files live in read-only
//! directories, `show(graph_bytes)` has no path to sit beside, and a query that
//! spans two graphs belongs to neither.
//!
//! ## Layout
//!
//! ```text
//! $KGLITE_VISUAL_CONFIG_DIR                (if set — tests and sandboxes)
//! else <config_dir>/kglite-visual/queries/
//!     <16 hex>.json    one graph, keyed by the hash of its absolute path
//!     _unbound.json    every session with no path on disk (show(bytes))
//! ```
//!
//! The hash keeps the filename short and free of separators; the file is
//! **self-describing** — it carries `graph_path` and `graph_label` inside — so
//! `kglite-visual queries list` can name what each file belongs to without
//! reversing a hash, and a stale file can be recognised rather than guessed at.
//!
//! **SHA-256, truncated to 16 hex, and not blake3.** The plan named blake3;
//! blake3 is not in this tree and adding it would be a new crate and a new
//! licence for a filename. `sha2` is already a runtime dependency twice over
//! (kglite depends on it, and so does rust-embed-utils), so this is the stable
//! hash already present. The property needed is stability across runs and
//! machines, which both have; nothing here is a security boundary.
//!
//! ## Bounds are refusals, not sweeps
//!
//! Every ceiling below is enforced at **write** time and reported as a refusal
//! that names the number it hit ([`StoreError::Refused`] → HTTP 400). Nothing
//! deletes on the owner's behalf: this is a durable tier, a durable tier is a
//! promise, and an age sweep over one is a scheduled data loss with a date on
//! it. `make prune` deliberately does not know this directory exists. The owner
//! is a human running `kglite-visual queries {list,rm,prune}`, and even `prune`
//! only offers files whose graph is gone from disk.
//!
//! ## Concurrency
//!
//! One `Mutex` per store serialises this process. Two servers on the same graph
//! are last-writer-wins, and the write is a temp file plus a rename, so a
//! concurrent reader sees the old file or the new one and never a half-written
//! one. Losing a save to a second server is a real outcome and an acceptable
//! one; a corrupted store would not be.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Environment override for the store's root. Set by the tests, and the escape
/// hatch for a sandbox with no writable config directory.
pub const CONFIG_DIR_ENV: &str = "KGLITE_VISUAL_CONFIG_DIR";

/// Saved queries one graph may hold.
///
/// Sixty-four is a list a person scrolls, not a database. The panel shows them
/// in one dropdown, and a dropdown with hundreds of entries is a search problem
/// this project has not agreed to solve.
pub const MAX_SAVED_PER_GRAPH: usize = 64;

/// Bytes one query's text may be.
///
/// Its own ceiling rather than a consequence of the file ceiling, because the
/// message differs: "this query is too long" is actionable and "the store file
/// would exceed 256 KB" is not. 64 KB is far past any Cypher a human writes and
/// far short of a pasted data dump.
pub const MAX_QUERY_BYTES: usize = 64 * 1024;

/// Bytes one graph's store file may be, serialized.
pub const MAX_STORE_FILE_BYTES: usize = 256 * 1024;

/// Store files the directory may hold.
///
/// One per graph ever opened, so this is a bound on *distinct graphs*, not on
/// usage. Reaching it means 512 different `.kgl` files have been viewed on this
/// machine and none of their stores has ever been pruned — which is exactly the
/// moment a human should look, and exactly what the refusal says.
pub const MAX_STORE_FILES: usize = 512;

/// Recent queries kept per graph.
///
/// The panel's "recent" list. Small on purpose: history is a convenience for
/// getting back to what you just ran, and anything worth keeping past that is
/// worth *saving*, which is the other half of this file.
pub const MAX_HISTORY: usize = 20;

/// The file every session with no path on disk shares.
const UNBOUND_FILE: &str = "_unbound.json";

/// On-disk format version. Bumped only for a change a reader must branch on.
const FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
pub enum StoreError {
    /// A bound fired, or the caller asked for something that is not there. The
    /// message names the ceiling; the caller can act on it.
    Refused(String),
    Io(std::io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Refused(message) => write!(f, "{message}"),
            StoreError::Io(err) => write!(f, "the saved-query store could not be read: {err}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::Io(err)
    }
}

/// One query a person chose to keep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQuery {
    pub name: String,
    pub query: String,
    /// Unix seconds. Seconds and not a formatted string, because a formatted
    /// timestamp in a file is a locale and a timezone somebody has to agree
    /// with later; the caller formats.
    pub saved_at: u64,
}

/// One query that was run from the panel or by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub query: String,
    pub ran_at: u64,
}

/// One graph's file, as it sits on disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreFile {
    pub version: u32,
    /// The absolute path this file belongs to, or `null` for the unbound file.
    pub graph_path: Option<String>,
    /// What the session called the graph. For the unbound file this is the last
    /// label written, which is a hint rather than an identity.
    pub graph_label: String,
    #[serde(default)]
    pub saved: Vec<SavedQuery>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The store's root directory, or `None` when this machine offers none.
///
/// `None` is not an error here: a headless container with no `HOME` is a
/// legitimate place to run this server, and it should serve graphs rather than
/// refuse to start. Every *operation* on a disabled store refuses and says so.
pub fn store_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
        let dir = PathBuf::from(dir);
        if !dir.as_os_str().is_empty() {
            return Some(dir);
        }
    }
    dirs::config_dir().map(|dir| dir.join("kglite-visual").join("queries"))
}

/// The file name a graph's queries live in.
///
/// A launch string that resolves to a file on disk is a graph with an identity
/// that survives the process; anything else — `show(bytes)`, a label, a deleted
/// file — shares the unbound file. The discriminator is *existence*, checked
/// once at open, because that is the only thing that makes two sessions the
/// same graph.
fn file_name_for(graph_path: Option<&Path>) -> String {
    match graph_path {
        None => UNBOUND_FILE.to_string(),
        Some(path) => {
            let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
            let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
            // The `.json` matters beyond tidiness: `store_files` counts and
            // lists by that extension, so an extensionless name would make a
            // graph's own store invisible to the ceiling, to `queries list`
            // and to `prune` — which is what the prune test caught.
            format!("{hex}.json")
        }
    }
}

/// Resolve a launch string to an absolute path, or `None` if it names no file.
fn resolve_graph_path(label: &str) -> Option<PathBuf> {
    let path = Path::new(label);
    if !path.is_file() {
        return None;
    }
    path.canonicalize().ok()
}

/// One session's view of the store.
pub struct QueryStore {
    /// `None` when no root could be resolved. Every operation then refuses.
    file: Option<PathBuf>,
    root: Option<PathBuf>,
    graph_path: Option<String>,
    graph_label: String,
    /// Serialises this process's read-modify-write cycles. See the module
    /// header for what it does and does not promise.
    lock: Mutex<()>,
}

impl QueryStore {
    /// Open the store for the graph a session was launched with.
    ///
    /// `graph_label` is the launch contract's `graph` field — a path for
    /// `kglite-visual <file>` and for `show(path)`, a display name for
    /// `show(bytes)`.
    pub fn open(graph_label: &str) -> Self {
        let root = store_root();
        let graph_path = resolve_graph_path(graph_label);
        let file = root
            .as_ref()
            .map(|root| root.join(file_name_for(graph_path.as_deref())));
        Self {
            file,
            root,
            graph_path: graph_path.map(|p| p.display().to_string()),
            graph_label: graph_label.to_string(),
            lock: Mutex::new(()),
        }
    }

    /// Where this session's queries are kept, for the caller to report.
    pub fn path(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    fn file(&self) -> Result<&Path, StoreError> {
        self.file.as_deref().ok_or_else(|| {
            StoreError::Refused(format!(
                "no saved-query store on this machine: no config directory could be \
                 resolved. Set {CONFIG_DIR_ENV} to a writable directory to enable it."
            ))
        })
    }

    /// Read this graph's file, or an empty one.
    ///
    /// A file that cannot be parsed is **not** silently replaced: a corrupt or
    /// future-versioned store is somebody's saved work, and overwriting it with
    /// an empty one on the next save is the one failure that cannot be undone.
    fn read(&self) -> Result<StoreFile, StoreError> {
        let path = self.file()?;
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoreFile {
                    version: FORMAT_VERSION,
                    graph_path: self.graph_path.clone(),
                    graph_label: self.graph_label.clone(),
                    ..Default::default()
                })
            }
            Err(err) => return Err(err.into()),
        };
        serde_json::from_slice(&bytes).map_err(|err| {
            StoreError::Refused(format!(
                "{} is not a saved-query file this version can read ({err}). It has \
                 been left alone; move it aside to start a new one.",
                path.display()
            ))
        })
    }

    /// Serialize, check the file bound, and replace the file atomically.
    fn write(&self, mut file: StoreFile) -> Result<(), StoreError> {
        let path = self.file()?;
        let root = path.parent().ok_or_else(|| {
            StoreError::Refused("the saved-query store path has no directory".to_string())
        })?;

        // The distinct-graph ceiling applies to *creating* a file, never to
        // updating one: refusing to save into a store that already exists would
        // punish the graph that happened to be opened last.
        if !path.exists() {
            let existing = count_store_files(root)?;
            if existing >= MAX_STORE_FILES {
                return Err(StoreError::Refused(format!(
                    "the saved-query store already holds {existing} graphs (ceiling \
                     {MAX_STORE_FILES}) and this graph has none yet. Run \
                     `kglite-visual queries prune` to drop the ones whose graph is \
                     gone, or `kglite-visual queries rm <file>`."
                )));
            }
        }

        file.version = FORMAT_VERSION;
        file.graph_path = self.graph_path.clone();
        file.graph_label = self.graph_label.clone();
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|err| StoreError::Refused(format!("could not encode the store: {err}")))?;
        if bytes.len() > MAX_STORE_FILE_BYTES {
            return Err(StoreError::Refused(format!(
                "this graph's saved queries would be {} bytes, past the {MAX_STORE_FILE_BYTES} \
                 byte ceiling for one store file. Delete some before saving more.",
                bytes.len()
            )));
        }

        std::fs::create_dir_all(root)?;
        // Rename, not a truncate-and-write: a reader in another process sees
        // the old file or the new one, never a half-written one.
        let temp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&temp, &bytes)?;
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    /// Everything this graph has: saved queries and recent history.
    pub fn list(&self) -> Result<StoreFile, StoreError> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.read()
    }

    /// Save, or overwrite a save of the same name.
    ///
    /// Overwriting by name is deliberate: the alternative is a second entry
    /// called `wells (2)`, which is how a saved-query list becomes unreadable.
    pub fn save(&self, name: &str, query: &str) -> Result<SavedQuery, StoreError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Refused(
                "a saved query needs a name".to_string(),
            ));
        }
        if query.trim().is_empty() {
            return Err(StoreError::Refused(
                "a saved query needs a query".to_string(),
            ));
        }
        if query.len() > MAX_QUERY_BYTES {
            return Err(StoreError::Refused(format!(
                "that query is {} bytes, past the {MAX_QUERY_BYTES} byte ceiling for one \
                 saved query.",
                query.len()
            )));
        }

        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = self.read()?;
        let entry = SavedQuery {
            name: name.to_string(),
            query: query.to_string(),
            saved_at: now_secs(),
        };
        match file.saved.iter_mut().find(|saved| saved.name == name) {
            Some(existing) => *existing = entry.clone(),
            None => {
                if file.saved.len() >= MAX_SAVED_PER_GRAPH {
                    return Err(StoreError::Refused(format!(
                        "this graph already has {MAX_SAVED_PER_GRAPH} saved queries, which \
                         is the ceiling. Delete one before saving another."
                    )));
                }
                file.saved.push(entry.clone());
            }
        }
        self.write(file)?;
        Ok(entry)
    }

    /// Remove a saved query. `false` means there was nothing by that name.
    pub fn delete(&self, name: &str) -> Result<bool, StoreError> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = self.read()?;
        let before = file.saved.len();
        file.saved.retain(|saved| saved.name != name);
        if file.saved.len() == before {
            return Ok(false);
        }
        self.write(file)?;
        Ok(true)
    }

    /// One saved query's text.
    pub fn get(&self, name: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .list()?
            .saved
            .into_iter()
            .find(|saved| saved.name == name)
            .map(|saved| saved.query))
    }

    /// Record a query somebody asked to run.
    ///
    /// **Explicitly called, not inferred from the dispatch path**, and that is
    /// the boundary: the app itself runs Cypher the user never typed — the
    /// per-node values behind a colour-by choice, the id list behind "load into
    /// view" — and a history built by watching the executor would fill up with
    /// machine noise the same day. So the two callers are the query panel's Run
    /// button and the `run_saved_query` tool: the places where a query is
    /// somebody's question.
    ///
    /// Over the query-length ceiling it is dropped rather than refused. History
    /// is a convenience, and failing a *run* because its record would not fit
    /// would be the tail wagging the dog.
    pub fn record(&self, query: &str) -> Result<(), StoreError> {
        let query = query.trim();
        if query.is_empty() || query.len() > MAX_QUERY_BYTES {
            return Ok(());
        }
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut file = self.read()?;
        // Most recent first, deduplicated: re-running the same query should
        // move it up the list, not fill the list with it.
        file.history.retain(|entry| entry.query != query);
        file.history.insert(
            0,
            HistoryEntry {
                query: query.to_string(),
                ran_at: now_secs(),
            },
        );
        file.history.truncate(MAX_HISTORY);
        self.write(file)
    }

    /// The store's root, for the CLI subcommand's reports.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}

/// Store files in a directory. Nothing else in it is counted or touched.
fn store_files(root: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn count_store_files(root: &Path) -> Result<usize, StoreError> {
    Ok(store_files(root)?.len())
}

/// One row of `kglite-visual queries list`.
#[derive(Debug, Serialize)]
pub struct StoreFileReport {
    pub file: String,
    pub graph_path: Option<String>,
    pub graph_label: String,
    pub saved: usize,
    pub history: usize,
    pub bytes: u64,
    /// The file names a path and nothing is there any more — what `prune`
    /// offers to remove. `false` for the unbound file, which names no path and
    /// therefore can never be stale.
    pub graph_missing: bool,
}

/// Every store file, described from its own contents.
///
/// Unreadable files are reported rather than skipped: a file the reader cannot
/// parse is exactly what a human needs to be told about, and a listing that
/// silently omits it would leave `prune` looking like it had nothing to do.
pub fn list_store(root: &Path) -> Result<Vec<StoreFileReport>, StoreError> {
    let mut reports = Vec::new();
    for path in store_files(root)? {
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed: Option<StoreFile> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        match parsed {
            Some(file) => reports.push(StoreFileReport {
                file: name,
                graph_missing: file
                    .graph_path
                    .as_ref()
                    .is_some_and(|p| !Path::new(p).is_file()),
                graph_path: file.graph_path,
                graph_label: file.graph_label,
                saved: file.saved.len(),
                history: file.history.len(),
                bytes,
            }),
            None => reports.push(StoreFileReport {
                file: name,
                graph_path: None,
                graph_label: "<unreadable>".to_string(),
                saved: 0,
                history: 0,
                bytes,
                graph_missing: false,
            }),
        }
    }
    Ok(reports)
}

/// Delete one store file by its file name.
pub fn remove_store_file(root: &Path, file: &str) -> Result<bool, StoreError> {
    // A name, never a path: `rm ../../something.json` must not reach outside
    // the store, and a subcommand that takes a path from a shell is exactly
    // where that would happen.
    let allowed: HashSet<String> = store_files(root)?
        .iter()
        .filter_map(|path| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    if !allowed.contains(file) {
        return Ok(false);
    }
    std::fs::remove_file(root.join(file))?;
    Ok(true)
}

/// Remove the store files whose graph is no longer on disk.
///
/// The only collection this store has, and it is **not** an age sweep: a saved
/// query is durable until its graph is gone or a human says otherwise. `dry_run`
/// reports what it would take without taking it, because the first thing anyone
/// should do with a delete command is look.
pub fn prune_store(root: &Path, dry_run: bool) -> Result<Vec<String>, StoreError> {
    let mut removed = Vec::new();
    for report in list_store(root)? {
        if !report.graph_missing {
            continue;
        }
        if !dry_run {
            std::fs::remove_file(root.join(&report.file))?;
        }
        removed.push(report.file);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store rooted in a tempdir, with the env override pointed at it.
    ///
    /// `KGLITE_VISUAL_CONFIG_DIR` is process-wide, so these tests share one
    /// mutex rather than running in parallel with each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn store_in(dir: &Path, label: &str) -> QueryStore {
        std::env::set_var(CONFIG_DIR_ENV, dir);
        QueryStore::open(label)
    }

    #[test]
    fn a_save_round_trips_and_overwrites_by_name() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = store_in(tmp.path(), "<in-memory>");

        store
            .save("wells", "MATCH (w:Wellbore) RETURN w")
            .expect("saves");
        store
            .save("wells", "MATCH (w:Wellbore) RETURN w.title")
            .expect("overwrites");
        let file = store.list().expect("lists");
        assert_eq!(
            file.saved.len(),
            1,
            "same name replaces rather than appends"
        );
        assert_eq!(file.saved[0].query, "MATCH (w:Wellbore) RETURN w.title");
        assert_eq!(
            store.get("wells").expect("reads").as_deref(),
            Some("MATCH (w:Wellbore) RETURN w.title")
        );
        assert!(store.delete("wells").expect("deletes"));
        assert!(!store.delete("wells").expect("second delete is a no-op"));
    }

    /// R1 for the per-graph ceiling: it must be reachable and it must refuse.
    #[test]
    fn the_saved_query_ceiling_refuses_and_names_itself() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = store_in(tmp.path(), "<in-memory>");

        for i in 0..MAX_SAVED_PER_GRAPH {
            store
                .save(&format!("q{i}"), "RETURN 1")
                .expect("under the ceiling");
        }
        let err = store
            .save("one-too-many", "RETURN 1")
            .expect_err("the ceiling must fire");
        let message = err.to_string();
        assert!(
            message.contains(&MAX_SAVED_PER_GRAPH.to_string()),
            "the refusal must name the ceiling: {message}"
        );
        // ...and an overwrite of an existing name still works at the ceiling,
        // because it adds nothing.
        store
            .save("q0", "RETURN 2")
            .expect("overwrite at the ceiling");
    }

    #[test]
    fn an_oversized_query_is_refused_by_its_own_ceiling() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = store_in(tmp.path(), "<in-memory>");

        let huge = "x".repeat(MAX_QUERY_BYTES + 1);
        let message = store
            .save("huge", &huge)
            .expect_err("must refuse")
            .to_string();
        assert!(message.contains(&MAX_QUERY_BYTES.to_string()), "{message}");
        // The store is untouched by a refusal — no partial write.
        assert!(store.list().expect("lists").saved.is_empty());
    }

    #[test]
    fn the_file_ceiling_refuses_before_the_write_lands() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = store_in(tmp.path(), "<in-memory>");

        // Each query is under its own ceiling; together they cross the file's.
        let chunk = "x".repeat(MAX_QUERY_BYTES - 1);
        let mut refused = None;
        for i in 0..MAX_SAVED_PER_GRAPH {
            if let Err(err) = store.save(&format!("q{i}"), &chunk) {
                refused = Some(err.to_string());
                break;
            }
        }
        let message = refused.expect("the file ceiling must fire before the count ceiling");
        assert!(
            message.contains(&MAX_STORE_FILE_BYTES.to_string()),
            "the refusal must name the file ceiling: {message}"
        );
    }

    #[test]
    fn the_distinct_graph_ceiling_refuses_a_new_file_and_spares_existing_ones() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path()).expect("root");
        // Fill the directory to the ceiling with well-formed files.
        for i in 0..MAX_STORE_FILES {
            std::fs::write(
                tmp.path().join(format!("{i:016x}.json")),
                serde_json::to_vec(&StoreFile {
                    version: FORMAT_VERSION,
                    graph_path: None,
                    graph_label: "filler".into(),
                    saved: Vec::new(),
                    history: Vec::new(),
                })
                .expect("encodes"),
            )
            .expect("write");
        }

        let fresh = store_in(tmp.path(), "<in-memory>");
        let message = fresh
            .save("first", "RETURN 1")
            .expect_err("a new file must be refused at the ceiling")
            .to_string();
        assert!(message.contains(&MAX_STORE_FILES.to_string()), "{message}");

        // An existing file is still writable: the ceiling is on creating a
        // store, not on using one.
        std::fs::rename(
            tmp.path().join("0000000000000000.json"),
            tmp.path().join(UNBOUND_FILE),
        )
        .expect("rename");
        fresh
            .save("first", "RETURN 1")
            .expect("existing store still accepts writes");
    }

    #[test]
    fn history_is_capped_most_recent_first_and_deduplicated() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = store_in(tmp.path(), "<in-memory>");

        for i in 0..MAX_HISTORY + 5 {
            store.record(&format!("RETURN {i}")).expect("records");
        }
        let file = store.list().expect("lists");
        assert_eq!(file.history.len(), MAX_HISTORY);
        assert_eq!(file.history[0].query, format!("RETURN {}", MAX_HISTORY + 4));

        store.record("RETURN 0").expect("records");
        let file = store.list().expect("lists");
        assert_eq!(file.history.len(), MAX_HISTORY, "still capped");
        assert_eq!(
            file.history[0].query, "RETURN 0",
            "a re-run moves to the top"
        );
        assert_eq!(
            file.history
                .iter()
                .filter(|e| e.query == "RETURN 0")
                .count(),
            1,
            "and does not appear twice"
        );
    }

    #[test]
    fn a_path_backed_graph_and_an_unbound_one_get_different_files() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let graph = tmp.path().join("a.kgl");
        std::fs::write(&graph, b"not really a graph").expect("write");

        let bound = store_in(tmp.path(), &graph.display().to_string());
        let unbound = store_in(tmp.path(), "<in-memory graph>");
        assert_ne!(bound.path(), unbound.path());
        assert_eq!(
            unbound.path().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new(UNBOUND_FILE))
        );
        // Same file for the same path, whatever spelling reached the launcher.
        let again = store_in(tmp.path(), &graph.display().to_string());
        assert_eq!(bound.path(), again.path());
    }

    #[test]
    fn an_unreadable_store_is_refused_rather_than_overwritten() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = store_in(tmp.path(), "<in-memory>");
        store.save("keep", "RETURN 1").expect("saves");

        let path = store.path().expect("has a path").to_path_buf();
        std::fs::write(&path, b"{ this is not json").expect("corrupt it");
        assert!(store.list().is_err(), "a corrupt store is reported");
        assert!(store.save("new", "RETURN 2").is_err(), "and not replaced");
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            b"{ this is not json",
            "somebody's saved work is left alone"
        );
    }

    #[test]
    fn prune_removes_only_the_files_whose_graph_is_gone() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("store");
        let graph = tmp.path().join("live.kgl");
        std::fs::write(&graph, b"graph").expect("write");

        let live = store_in(&root, &graph.display().to_string());
        live.save("q", "RETURN 1").expect("saves");
        let unbound = store_in(&root, "<in-memory>");
        unbound.save("q", "RETURN 1").expect("saves");

        assert!(prune_store(&root, false).expect("prunes").is_empty());
        std::fs::remove_file(&graph).expect("delete the graph");

        let would = prune_store(&root, true).expect("dry run");
        assert_eq!(would.len(), 1, "only the orphan: {would:?}");
        assert_eq!(
            store_files(&root).expect("files").len(),
            2,
            "dry run took nothing"
        );

        let removed = prune_store(&root, false).expect("prunes");
        assert_eq!(removed.len(), 1);
        let left = list_store(&root).expect("lists");
        assert_eq!(left.len(), 1);
        assert_eq!(
            left[0].file, UNBOUND_FILE,
            "the unbound file is never stale"
        );
    }

    #[test]
    fn rm_refuses_a_name_that_reaches_outside_the_store() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("store");
        let outside = tmp.path().join("precious.json");
        std::fs::write(&outside, b"{}").expect("write");
        store_in(&root, "<in-memory>")
            .save("q", "RETURN 1")
            .expect("saves");

        assert!(
            !remove_store_file(&root, "../precious.json").expect("no error, just a refusal"),
            "a traversal must not be treated as a store file"
        );
        assert!(outside.is_file(), "and must not delete anything");
        assert!(remove_store_file(&root, UNBOUND_FILE).expect("removes"));
    }
}
