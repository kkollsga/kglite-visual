//! Bounded saved-view catalog. Every read/validate/write transaction holds one
//! OS lock, including readers in other viewer processes. Payload semantics
//! belong to core; this store owns names, quotas and durable replacement only.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_VIEWS: usize = 20;
pub const MAX_VIEW_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CATALOG_BYTES: usize = 40 * 1024 * 1024;
const MAX_NAME_BYTES: usize = 128;
const LOCK_WAIT: Duration = Duration::from_secs(2);
const CATALOG: &str = "catalog.json";
const STAGING: &str = "catalog.staging";
const LOCK: &str = "catalog.lock";

#[derive(Debug)]
pub enum ViewStoreError {
    Refused(String),
    Io(std::io::Error),
}
impl std::fmt::Display for ViewStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(message) => f.write_str(message),
            Self::Io(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for ViewStoreError {}
impl From<std::io::Error> for ViewStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
fn refuse(message: impl Into<String>) -> ViewStoreError {
    ViewStoreError::Refused(message.into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {
    pub name: String,
    pub saved_at: u64,
    pub bookmark: Value,
}
#[derive(Debug, Clone, Serialize)]
pub struct SavedViewSummary {
    pub name: String,
    pub saved_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    format_version: u32,
    views: Vec<SavedView>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            views: Vec::new(),
        }
    }
}

/// Three owned files maximum: catalog, reusable staging and lock. A stale
/// staging file is ignored by reads and overwritten by the next transaction.
#[derive(Debug)]
pub struct ViewStore {
    root: Option<PathBuf>,
}
impl Default for ViewStore {
    fn default() -> Self {
        Self::open()
    }
}
impl ViewStore {
    #[cfg(test)]
    pub(crate) fn at(root: PathBuf) -> Self {
        Self { root: Some(root) }
    }
    pub fn open() -> Self {
        let base = std::env::var_os(crate::queries::CONFIG_DIR_ENV)
            .map(PathBuf::from)
            .or_else(|| dirs::config_dir().map(|path| path.join("kglite-visual")));
        Self {
            root: base.map(|path| path.join("views")),
        }
    }
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
    pub fn list(&self) -> Result<Vec<SavedViewSummary>, ViewStoreError> {
        self.transaction(|catalog| {
            Ok((
                catalog
                    .views
                    .iter()
                    .map(|view| SavedViewSummary {
                        name: view.name.clone(),
                        saved_at: view.saved_at,
                    })
                    .collect(),
                false,
            ))
        })
    }
    pub fn get(&self, name: &str) -> Result<SavedView, ViewStoreError> {
        validate_name(name)?;
        self.transaction(|catalog| {
            catalog
                .views
                .iter()
                .find(|view| view.name == name)
                .cloned()
                .map(|view| (view, false))
                .ok_or_else(|| refuse(format!("saved view {name:?} does not exist")))
        })
    }
    /// Core must validate/capture the typed bookmark before calling save.
    /// Replacing a named view is explicit; no quota path evicts another name.
    pub fn save(
        &self,
        name: &str,
        bookmark: Value,
        replace: bool,
    ) -> Result<SavedViewSummary, ViewStoreError> {
        self.save_validated(name, bookmark, replace, |_| Ok(()))
    }
    /// Validation of an existing payload happens while the catalog lock is held.
    pub fn save_validated(
        &self,
        name: &str,
        bookmark: Value,
        replace: bool,
        validate_existing: impl FnOnce(&Value) -> Result<(), ViewStoreError>,
    ) -> Result<SavedViewSummary, ViewStoreError> {
        validate_name(name)?;
        let saved = SavedView {
            name: name.into(),
            saved_at: now_secs(),
            bookmark,
        };
        bounded_json(&saved, MAX_VIEW_BYTES, "saved view exceeds 4 MiB")?;
        let summary = SavedViewSummary {
            name: saved.name.clone(),
            saved_at: saved.saved_at,
        };
        self.transaction(|catalog| {
            if let Some(existing) = catalog.views.iter_mut().find(|view| view.name == name) {
                if !replace {
                    return Err(refuse(format!(
                        "saved view {name:?} already exists; explicit replacement is required"
                    )));
                }
                validate_existing(&existing.bookmark)?;
                *existing = saved;
            } else {
                if catalog.views.len() >= MAX_VIEWS {
                    return Err(refuse("saved-view catalog already holds 20 views"));
                }
                catalog.views.push(saved);
            }
            catalog.views.sort_by(|a, b| a.name.cmp(&b.name));
            Ok((summary, true))
        })
    }
    pub fn delete(&self, name: &str) -> Result<(), ViewStoreError> {
        validate_name(name)?;
        self.transaction(|catalog| {
            let index = catalog
                .views
                .iter()
                .position(|view| view.name == name)
                .ok_or_else(|| refuse(format!("saved view {name:?} does not exist")))?;
            catalog.views.remove(index);
            Ok(((), true))
        })
    }
    fn transaction<T>(
        &self,
        action: impl FnOnce(&mut Catalog) -> Result<(T, bool), ViewStoreError>,
    ) -> Result<T, ViewStoreError> {
        let root = self
            .root
            .as_deref()
            .ok_or_else(|| refuse("no config directory is available for saved views"))?;
        std::fs::create_dir_all(root)?;
        let _lock = CatalogLock::acquire(root)?;
        let mut catalog = read_catalog(root)?;
        #[cfg(test)]
        pause_child_after_read(root);
        let (result, changed) = action(&mut catalog)?;
        if changed {
            validate_catalog(&catalog)?;
            let bytes = bounded_json(
                &catalog,
                MAX_CATALOG_BYTES,
                "saved-view catalog exceeds 40 MiB",
            )?;
            replace_catalog(root, &bytes)?;
        }
        Ok(result)
    }
}

/// One bounded catalog per server lifetime; no filesystem or source label keys.
#[derive(Debug, Default)]
pub struct SessionViewStore(std::sync::Mutex<Catalog>);
impl SessionViewStore {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Catalog>, ViewStoreError> {
        self.0
            .lock()
            .map_err(|_| refuse("session catalog lock is poisoned"))
    }
    pub fn list(&self) -> Result<Vec<SavedViewSummary>, ViewStoreError> {
        Ok(self
            .lock()?
            .views
            .iter()
            .map(|v| SavedViewSummary {
                name: v.name.clone(),
                saved_at: v.saved_at,
            })
            .collect())
    }
    pub fn get(&self, name: &str) -> Result<SavedView, ViewStoreError> {
        validate_name(name)?;
        self.lock()?
            .views
            .iter()
            .find(|v| v.name == name)
            .cloned()
            .ok_or_else(|| refuse(format!("saved view {name:?} does not exist")))
    }
    pub fn save_validated(
        &self,
        name: &str,
        bookmark: Value,
        replace: bool,
        validate_existing: impl FnOnce(&Value) -> Result<(), ViewStoreError>,
    ) -> Result<SavedViewSummary, ViewStoreError> {
        validate_name(name)?;
        let saved = SavedView {
            name: name.into(),
            saved_at: now_secs(),
            bookmark,
        };
        bounded_json(&saved, MAX_VIEW_BYTES, "saved view exceeds 4 MiB")?;
        let mut catalog = self.lock()?;
        let existing = catalog.views.iter().position(|v| v.name == name);
        if let Some(index) = existing {
            if !replace {
                return Err(refuse(
                    "saved view already exists; explicit replacement is required",
                ));
            }
            validate_existing(&catalog.views[index].bookmark)?;
        } else if catalog.views.len() >= MAX_VIEWS {
            return Err(refuse("saved-view catalog already holds 20 views"));
        }
        // Serialize borrowed candidates before mutating: a quota refusal keeps
        // the old catalog intact without cloning up to 40 MiB of payloads.
        #[derive(Serialize)]
        struct Candidate<'a> {
            format_version: u32,
            views: Vec<&'a SavedView>,
        }
        let mut entries: Vec<_> = catalog.views.iter().filter(|v| v.name != name).collect();
        entries.push(&saved);
        bounded_json(
            &Candidate {
                format_version: FORMAT_VERSION,
                views: entries,
            },
            MAX_CATALOG_BYTES,
            "saved-view catalog exceeds 40 MiB",
        )?;
        let result = SavedViewSummary {
            name: saved.name.clone(),
            saved_at: saved.saved_at,
        };
        if let Some(index) = existing {
            catalog.views[index] = saved;
        } else {
            catalog.views.push(saved);
        }
        catalog.views.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }
    pub fn delete(&self, name: &str) -> Result<(), ViewStoreError> {
        validate_name(name)?;
        let mut catalog = self.lock()?;
        let index = catalog
            .views
            .iter()
            .position(|v| v.name == name)
            .ok_or_else(|| refuse(format!("saved view {name:?} does not exist")))?;
        catalog.views.remove(index);
        Ok(())
    }
}

struct CatalogLock(File);
impl CatalogLock {
    fn acquire(root: &Path) -> Result<Self, ViewStoreError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(LOCK))?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self(file)),
                Err(error)
                    if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    if Instant::now() >= deadline {
                        return Err(refuse(
                            "saved-view catalog is busy; lock wait exceeded 2 seconds",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}
impl Drop for CatalogLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn read_catalog(root: &Path) -> Result<Catalog, ViewStoreError> {
    let file = match File::open(root.join(CATALOG)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Catalog::default()),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > MAX_CATALOG_BYTES as u64 {
        return Err(refuse(
            "saved-view catalog exceeds 40 MiB; existing work was not overwritten",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CATALOG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(refuse(
            "saved-view catalog exceeds 40 MiB; existing work was not overwritten",
        ));
    }
    let catalog: Catalog = serde_json::from_slice(&bytes).map_err(|error| {
        refuse(format!(
            "saved-view catalog is unreadable; existing work was not overwritten: {error}"
        ))
    })?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}
fn validate_catalog(catalog: &Catalog) -> Result<(), ViewStoreError> {
    if catalog.format_version != FORMAT_VERSION {
        return Err(refuse(format!(
            "unsupported saved-view catalog version {}; supported version is 1",
            catalog.format_version
        )));
    }
    if catalog.views.len() > MAX_VIEWS {
        return Err(refuse("saved-view catalog exceeds 20 views"));
    }
    let mut names = HashSet::new();
    for view in &catalog.views {
        validate_name(&view.name)?;
        if !names.insert(&view.name) {
            return Err(refuse("saved-view catalog has duplicate names"));
        }
        bounded_json(view, MAX_VIEW_BYTES, "saved view exceeds 4 MiB")?;
    }
    Ok(())
}
fn validate_name(name: &str) -> Result<(), ViewStoreError> {
    if name.trim().is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(refuse(
            "saved-view name must contain 1–128 bytes and no control characters",
        ));
    }
    Ok(())
}
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0)
}

struct CappedBytes {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for CappedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("serialized size exceeds limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_json(
    value: &impl Serialize,
    limit: usize,
    message: &str,
) -> Result<Vec<u8>, ViewStoreError> {
    let mut writer = CappedBytes {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| refuse(message))?;
    Ok(writer.bytes)
}
fn replace_catalog(root: &Path, bytes: &[u8]) -> Result<(), ViewStoreError> {
    let mut staging = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(root.join(STAGING))?;
    staging.write_all(bytes)?;
    staging.sync_all()?;
    drop(staging);
    #[cfg(test)]
    if std::env::var_os("KGLV_VIEW_STORE_CRASH_BEFORE_REPLACE").is_some() {
        std::process::exit(73);
    }
    std::fs::rename(root.join(STAGING), root.join(CATALOG))?;
    #[cfg(unix)]
    File::open(root)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
fn pause_child_after_read(root: &Path) {
    if std::env::var_os("KGLV_VIEW_STORE_PAUSE_READ").is_none() {
        return;
    }
    std::fs::write(root.join("read-entered"), b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("read-release").exists() {
        assert!(
            Instant::now() < deadline,
            "test parent did not release child read"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_payload_validation_is_transactional() {
        let dir = tempfile::tempdir().unwrap();
        let store = ViewStore {
            root: Some(dir.path().into()),
        };
        store
            .save("future", serde_json::json!({"version":999}), false)
            .unwrap();
        let before = std::fs::read(dir.path().join(CATALOG)).unwrap();
        let refused =
            store.save_validated("future", serde_json::json!({"version":1}), true, |_| {
                Err(refuse("future schema"))
            });
        assert!(refused.is_err());
        assert_eq!(std::fs::read(dir.path().join(CATALOG)).unwrap(), before);
    }
    #[test]
    fn session_catalog_refuses_quota_without_losing_old_payload() {
        let store = SessionViewStore::default();
        for index in 0..MAX_VIEWS {
            store
                .save_validated(
                    &format!("view-{index}"),
                    serde_json::json!({"value":index}),
                    false,
                    |_| Ok(()),
                )
                .unwrap();
        }
        assert!(store
            .save_validated("extra", Value::Null, false, |_| Ok(()))
            .is_err());
        assert!(store
            .save_validated("view-0", Value::Null, false, |_| Ok(()))
            .is_err());
        assert!(store
            .save_validated("view-0", Value::Null, true, |_| Err(refuse("unsupported")))
            .is_err());
        assert_eq!(store.get("view-0").unwrap().bookmark["value"], 0);
        assert_eq!(store.list().unwrap().len(), MAX_VIEWS);
        assert!(SessionViewStore::default().list().unwrap().is_empty());
    }

    #[test]
    fn session_aggregate_limit_preserves_all_existing_entries() {
        let store = SessionViewStore::default();
        let payload = Value::String("x".repeat(MAX_VIEW_BYTES - 1024));
        for index in 0..10 {
            store
                .save_validated(
                    &format!("large-{index}"),
                    payload.clone(),
                    false,
                    |_| Ok(()),
                )
                .unwrap();
        }
        assert!(store
            .save_validated("overflow", payload, false, |_| Ok(()))
            .is_err());
        assert_eq!(store.list().unwrap().len(), 10);
        assert_eq!(
            store
                .get("large-0")
                .unwrap()
                .bookmark
                .as_str()
                .unwrap()
                .len(),
            MAX_VIEW_BYTES - 1024
        );
    }

    use std::process::{Child, Command, Stdio};

    fn store(root: &Path) -> ViewStore {
        ViewStore {
            root: Some(root.into()),
        }
    }
    fn payload() -> Value {
        serde_json::json!({"format_version":1,"source":{"kind":"session-only"},"nodes":[]})
    }
    fn child(root: &Path, name: &str, pause: bool, crash: bool) -> Child {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", "views::tests::child_writer"])
            .env("KGLV_VIEW_STORE_CHILD_ROOT", root)
            .env("KGLV_VIEW_STORE_CHILD_NAME", name)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if pause {
            command.env("KGLV_VIEW_STORE_PAUSE_READ", "1");
        }
        if crash {
            command.env("KGLV_VIEW_STORE_CRASH_BEFORE_REPLACE", "1");
        }
        command.spawn().unwrap()
    }
    fn wait_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "child did not create {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "subprocess helper, invoked by cross-process and interrupted-write tests"]
    fn child_writer() {
        let Some(root) = std::env::var_os("KGLV_VIEW_STORE_CHILD_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let name = std::env::var("KGLV_VIEW_STORE_CHILD_NAME").unwrap();
        std::fs::write(root.join(format!("started-{name}")), b"started").unwrap();
        store(&root).save(&name, payload(), false).unwrap();
    }
    #[test]
    fn save_replace_delete_and_read_keep_one_bounded_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        assert!(store.list().unwrap().is_empty());
        store
            .save("human name / is not a path", payload(), false)
            .unwrap();
        assert!(store
            .save(
                "human name / is not a path",
                serde_json::json!({"other-source":true}),
                false
            )
            .unwrap_err()
            .to_string()
            .contains("explicit replacement"));
        assert_eq!(
            store.get("human name / is not a path").unwrap().bookmark,
            payload()
        );
        store
            .save(
                "human name / is not a path",
                serde_json::json!({"changed":true}),
                true,
            )
            .unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        let listing = serde_json::to_value(store.list().unwrap()).unwrap();
        assert!(listing[0].get("bookmark").is_none());
        assert!(listing.to_string().len() < 256);
        assert_eq!(
            store.get("human name / is not a path").unwrap().bookmark,
            serde_json::json!({"changed":true})
        );
        store.delete("human name / is not a path").unwrap();
        assert!(store.list().unwrap().is_empty());
        let files: HashSet<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(files.len(), 2);
        assert!(files.contains(std::ffi::OsStr::new(CATALOG)));
        assert!(files.contains(std::ffi::OsStr::new(LOCK)));
    }
    #[test]
    fn unsupported_corrupt_and_oversized_catalogs_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        for input in [
            b"broken".as_slice(),
            br#"{"format_version":99,"views":[]}"#.as_slice(),
            br#"{"format_version":1,"views":[],"unknown":"data"}"#.as_slice(),
        ] {
            std::fs::write(dir.path().join(CATALOG), input).unwrap();
            assert!(store.save("new", payload(), false).is_err());
            assert_eq!(std::fs::read(dir.path().join(CATALOG)).unwrap(), input);
        }
        let file = File::create(dir.path().join(CATALOG)).unwrap();
        file.set_len(MAX_CATALOG_BYTES as u64 + 1).unwrap();
        assert!(store.list().unwrap_err().to_string().contains("40 MiB"));
        assert_eq!(file.metadata().unwrap().len(), MAX_CATALOG_BYTES as u64 + 1);
    }
    #[test]
    fn quotas_refuse_without_evicting_a_saved_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        for index in 0..MAX_VIEWS {
            store
                .save(&format!("view-{index}"), payload(), false)
                .unwrap();
        }
        let before = std::fs::read(dir.path().join(CATALOG)).unwrap();
        assert!(store
            .save("overflow", payload(), false)
            .unwrap_err()
            .to_string()
            .contains("20 views"));
        assert_eq!(std::fs::read(dir.path().join(CATALOG)).unwrap(), before);
        assert!(store
            .save("view-0", Value::String("x".repeat(MAX_VIEW_BYTES)), true)
            .unwrap_err()
            .to_string()
            .contains("4 MiB"));
        assert_eq!(std::fs::read(dir.path().join(CATALOG)).unwrap(), before);
        store
            .save("view-0", serde_json::json!({"replaced":true}), true)
            .unwrap();
        assert_eq!(store.list().unwrap().len(), MAX_VIEWS);
    }
    #[test]
    fn aggregate_byte_quota_refuses_before_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let large = Value::String("x".repeat(MAX_VIEW_BYTES - 1024));
        let views = (0..10)
            .map(|index| SavedView {
                name: format!("view-{index}"),
                saved_at: 0,
                bookmark: large.clone(),
            })
            .collect();
        let catalog = Catalog {
            format_version: FORMAT_VERSION,
            views,
        };
        let bytes = bounded_json(&catalog, MAX_CATALOG_BYTES, "test catalog must fit").unwrap();
        std::fs::write(dir.path().join(CATALOG), &bytes).unwrap();
        assert!(store
            .save("overflow", large, false)
            .unwrap_err()
            .to_string()
            .contains("40 MiB"));
        assert_eq!(std::fs::read(dir.path().join(CATALOG)).unwrap(), bytes);
    }
    #[test]
    fn two_processes_preserve_both_updates_under_one_os_lock() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = child(dir.path(), "first", true, false);
        wait_file(&dir.path().join("read-entered"));
        let mut second = child(dir.path(), "second", false, false);
        wait_file(&dir.path().join("started-second"));
        std::thread::sleep(Duration::from_millis(200));
        let second_escaped = second.try_wait().unwrap().is_some();
        std::fs::write(dir.path().join("read-release"), b"release").unwrap();
        assert!(first.wait().unwrap().success());
        assert!(second.wait().unwrap().success());
        assert!(
            !second_escaped,
            "a second process saved while the first held the catalog transaction"
        );
        let names: Vec<_> = store(dir.path())
            .list()
            .unwrap()
            .into_iter()
            .map(|view| view.name)
            .collect();
        assert_eq!(names, ["first", "second"]);
    }
    #[test]
    fn interrupted_staging_preserves_the_catalog_and_reuses_one_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("original", payload(), false).unwrap();
        let before = std::fs::read(dir.path().join(CATALOG)).unwrap();
        let mut interrupted = child(dir.path(), "interrupted", false, true);
        assert_eq!(interrupted.wait().unwrap().code(), Some(73));
        assert_eq!(std::fs::read(dir.path().join(CATALOG)).unwrap(), before);
        assert!(dir.path().join(STAGING).exists());
        assert_eq!(store.list().unwrap().len(), 1);
        store.save("after", payload(), false).unwrap();
        assert!(!dir.path().join(STAGING).exists());
        assert_eq!(store.list().unwrap().len(), 2);
    }
    #[test]
    fn lock_wait_is_bounded_and_does_not_touch_the_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("original", payload(), false).unwrap();
        let before = std::fs::read(dir.path().join(CATALOG)).unwrap();
        let _lock = CatalogLock::acquire(dir.path()).unwrap();
        let started = Instant::now();
        assert!(store
            .save("blocked", payload(), false)
            .unwrap_err()
            .to_string()
            .contains("lock wait exceeded"));
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_eq!(std::fs::read(dir.path().join(CATALOG)).unwrap(), before);
    }
}
