//! Provenance belongs to the loader, never to a caller-provided display label.
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use kglite::api::io::GraphFileIdentity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::CoreError;

pub const ENGINE_VERSION: &str = "0.16.22";
const MAX_CURRENT_BYTES: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    FileSha256,
    PublishedGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(deny_unknown_fields)]
pub struct SourceFingerprint {
    pub canonical_path: String,
    pub kind: SourceKind,
    pub engine_version: String,
    pub fingerprint: String,
}

#[derive(Debug)]
pub(crate) struct SourceIdentity {
    path: PathBuf,
    loaded: GraphFileIdentity,
    kind: Option<SourceKind>,
    fingerprint: Mutex<Option<SourceFingerprint>>,
}

impl SourceIdentity {
    pub(crate) fn has_durable_identity(&self) -> bool {
        self.kind.is_some()
    }
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn capture(path: &Path) -> Result<Self, CoreError> {
        let path = path.canonicalize()?;
        let metadata = path.metadata()?;
        let kind = if metadata.is_file() {
            Some(SourceKind::FileSha256)
        } else if metadata.is_dir() && path.join("CURRENT").is_file() {
            Some(SourceKind::PublishedGeneration)
        } else {
            None
        };
        let loaded = GraphFileIdentity::capture(&path)?;
        Ok(Self {
            path,
            loaded,
            kind,
            fingerprint: Mutex::new(None),
        })
    }

    pub(crate) fn verify_loaded(&self) -> Result<(), CoreError> {
        if GraphFileIdentity::capture(&self.path)? != self.loaded {
            return Err(refusal("source changed since this graph was loaded"));
        }
        Ok(())
    }

    pub(crate) fn durable_fingerprint(
        &self,
        deadline: Option<Instant>,
        verify_bytes: bool,
    ) -> Result<Option<SourceFingerprint>, CoreError> {
        let Some(kind) = &self.kind else {
            return Ok(None);
        };
        self.verify_loaded()?;
        let cached = self.fingerprint.lock().unwrap().clone();
        if !verify_bytes {
            if let Some(cached) = cached {
                return Ok(Some(cached));
            }
        }
        let fingerprint = self.hash_source(kind, deadline)?;
        self.verify_loaded()?;
        let result = SourceFingerprint {
            canonical_path: self
                .path
                .to_str()
                .ok_or_else(|| refusal("source path is not UTF-8"))?
                .into(),
            kind: kind.clone(),
            engine_version: ENGINE_VERSION.into(),
            fingerprint,
        };
        let mut current = self.fingerprint.lock().unwrap();
        if current.as_ref().is_some_and(|previous| previous != &result) {
            return Err(refusal(
                "source bytes changed since the verified graph snapshot",
            ));
        }
        *current = Some(result.clone());
        Ok(Some(result))
    }

    fn hash_source(
        &self,
        kind: &SourceKind,
        deadline: Option<Instant>,
    ) -> Result<String, CoreError> {
        let path = match kind {
            SourceKind::FileSha256 => self.path.clone(),
            SourceKind::PublishedGeneration => self.path.join("CURRENT"),
        };
        let mut file = File::open(path)?;
        let mut remaining = match kind {
            SourceKind::FileSha256 => u64::MAX,
            // CURRENT is opaque. Its immutable-generation contract is public;
            // generation paths and individual disk-format files are not.
            SourceKind::PublishedGeneration => MAX_CURRENT_BYTES,
        };
        if file.metadata()?.len() > remaining {
            return Err(refusal("disk graph CURRENT exceeds 4096 bytes"));
        }
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            check_deadline(deadline)?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if count as u64 > remaining {
                return Err(refusal("source fingerprint exceeded its byte limit"));
            }
            remaining -= count as u64;
            digest.update(&buffer[..count]);
        }
        Ok(hex_digest(&digest.finalize()))
    }
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn check_deadline(deadline: Option<Instant>) -> Result<(), CoreError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(refusal("bookmark preparation exceeded its deadline"));
    }
    Ok(())
}
fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_engine_version_matches_the_actual_exact_manifest_pin() {
        let manifest = include_str!("../Cargo.toml");
        let declarations: Vec<_> = manifest
            .lines()
            .filter(|line| line.starts_with("kglite = "))
            .collect();
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0], format!("kglite = \"={ENGINE_VERSION}\""));
    }

    #[test]
    fn file_fingerprint_streams_and_detects_changed_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.kgl");
        std::fs::write(&path, b"abc").unwrap();
        let source = SourceIdentity::capture(&path).unwrap();
        let fingerprint = source.durable_fingerprint(None, false).unwrap().unwrap();
        assert_eq!(
            fingerprint.fingerprint,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(fingerprint.kind, SourceKind::FileSha256);
        std::fs::write(&path, b"different graph").unwrap();
        assert!(source.durable_fingerprint(None, false).is_err());
    }

    #[test]
    fn byte_verification_catches_same_size_rewrite_with_preserved_mtime() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.kgl");
        std::fs::write(&path, b"abc").unwrap();
        let modified = path.metadata().unwrap().modified().unwrap();
        let source = SourceIdentity::capture(&path).unwrap();
        source.durable_fingerprint(None, false).unwrap();
        std::fs::write(&path, b"xyz").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert!(source.durable_fingerprint(None, true).is_err());
    }

    #[test]
    fn current_is_opaque_generation_signal_and_scratch_is_irrelevant() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("CURRENT"), b"opaque engine bytes").unwrap();
        let source = SourceIdentity::capture(directory.path()).unwrap();
        let before = source.durable_fingerprint(None, false).unwrap().unwrap();
        std::fs::write(directory.path().join(".writer-scratch"), b"noise").unwrap();
        assert_eq!(
            source.durable_fingerprint(None, true).unwrap(),
            Some(before)
        );
        std::fs::write(directory.path().join("CURRENT"), b"new opaque generation").unwrap();
        assert!(source.durable_fingerprint(None, false).is_err());
    }

    #[test]
    fn legacy_directory_is_not_durable_and_missing_path_refuses() {
        let directory = tempfile::tempdir().unwrap();
        let source = SourceIdentity::capture(directory.path()).unwrap();
        assert_eq!(source.durable_fingerprint(None, false).unwrap(), None);
        assert!(SourceIdentity::capture(&directory.path().join("absent")).is_err());
    }

    #[test]
    fn fingerprint_obeys_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.kgl");
        std::fs::write(&path, b"abc").unwrap();
        let source = SourceIdentity::capture(&path).unwrap();
        assert!(source
            .durable_fingerprint(Some(Instant::now()), false)
            .is_err());
    }
}
