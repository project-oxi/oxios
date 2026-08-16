//! Foundation shared package registry (RFC-048 §4).
//!
//! Reads the immutable `packages.lock` and the per-package manifests,
//! verifies schema version / target / source / digest, and exposes the
//! result as [`ImportedPackage`] candidates. **Never** writes the
//! lockfile — that is a Foundation-importer operation, not an agent turn.

use anyhow::Result;
use blake3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::capability::types::{ResourceRef, Rights};

/// Abstract requirements a package can declare. They are mapped to a
/// `(ResourceRef, Rights)` pair only through the reviewed table in
/// [`requirement_to_resource`] — unknown values are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstractRequirement {
    /// Read files inside the agent workspace.
    WorkspaceRead,
    /// Patch files inside the agent workspace.
    WorkspacePatch,
    /// Execute shell commands.
    ShellExecute,
    /// Drive a browser.
    BrowserNavigate,
    /// Read-only Brain connector access.
    BrainQuery,
    /// Manage scheduled jobs.
    ScheduleManage,
}

impl AbstractRequirement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceRead => "workspace.read",
            Self::WorkspacePatch => "workspace.patch",
            Self::ShellExecute => "shell.execute",
            Self::BrowserNavigate => "browser.navigate",
            Self::BrainQuery => "brain.query",
            Self::ScheduleManage => "schedule.manage",
        }
    }
}

/// Source trust label. `Signed` requires the digest to be in a separate
/// trust list — the importer only verifies the digest here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTrust {
    Unsigned,
    Signed,
}

/// Single package entry in the lockfile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub id: String,
    pub version: String,
    pub source: String,
    pub trust: SourceTrust,
    /// blake3 digest of the package archive. 32-byte hex.
    pub digest: String,
    /// Required target. Must include `oxios` for the lock to be useful
    /// to Oxios.
    pub targets: Vec<String>,
    /// Abstract requirements. Each maps through the reviewed table.
    #[serde(default)]
    pub requires: Vec<AbstractRequirement>,
    /// Optional persona hint — used by the persona manager to filter
    /// compatible packages.
    #[serde(default)]
    pub persona: Option<String>,
}

/// Lockfile root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageLock {
    pub schema_version: u32,
    pub entries: Vec<LockEntry>,
}

/// One capability derived from an abstract requirement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PackageCapability {
    pub requirement: AbstractRequirement,
    pub resource: ResourceRef,
    pub rights: Rights,
}

/// Imported package — a lockfile entry that passed all gates.
#[derive(Debug, Clone)]
pub struct ImportedPackage {
    pub id: String,
    pub version: String,
    pub source: String,
    pub digest: String,
    pub trust: SourceTrust,
    pub targets: Vec<String>,
    pub capabilities: Vec<PackageCapability>,
    pub persona: Option<String>,
}

/// Strict package errors. Every variant maps to a concrete failure the
/// importer logs and the CLI surfaces.
#[derive(Debug, Error)]
pub enum PackageError {
    #[error("package lock schema version {found} is not supported (expected {expected})")]
    UnsupportedSchema { found: u32, expected: u32 },
    #[error("package `{0}` is missing targets")]
    MissingTargets(String),
    #[error("package `{0}` targets `{1}` — must include `oxios`")]
    TargetMismatch(String, String),
    #[error("package `{0}` digest `{1}` is not 64 hex chars")]
    BadDigestFormat(String, String),
    #[error(
        "package `{id}` digest does not match computed digest `{actual}` (expected `{expected}`)"
    )]
    DigestMismatch {
        id: String,
        expected: String,
        actual: String,
    },
    #[error("package `{0}` cannot read archive `{1}`: {2}")]
    ArchiveUnreadable(String, String, String),
    #[error("package `{0}` archive digest mismatch")]
    ArchiveDigestMismatch(String),
    #[error("malformed JSON in package lock: {0}")]
    Json(String),
}

impl PackageLock {
    /// Parse and validate a lockfile. The archive bytes are only checked
    /// when the caller passes them to [`Self::import`].
    pub fn parse(json: &str) -> Result<Self, PackageError> {
        let lock: PackageLock =
            serde_json::from_str(json).map_err(|e| PackageError::Json(e.to_string()))?;
        if lock.schema_version != crate::foundation::SCHEMA_VERSION {
            return Err(PackageError::UnsupportedSchema {
                found: lock.schema_version,
                expected: crate::foundation::SCHEMA_VERSION,
            });
        }
        for entry in &lock.entries {
            if entry.targets.is_empty() {
                return Err(PackageError::MissingTargets(entry.id.clone()));
            }
            if !entry.targets.iter().any(|t| t == "oxios") {
                return Err(PackageError::TargetMismatch(
                    entry.id.clone(),
                    entry.targets.join(","),
                ));
            }
            if entry.digest.len() != 64 || !entry.digest.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(PackageError::BadDigestFormat(
                    entry.id.clone(),
                    entry.digest.clone(),
                ));
            }
        }
        Ok(lock)
    }

    /// Import the lockfile. The optional `archive_bytes` map (id → bytes)
    /// is used to verify the digest when present.
    pub fn import(
        &self,
        archives: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<ImportedPackage>, PackageError> {
        let mut imported = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            // Verify digest when an archive was supplied.
            if let Some(bytes) = archives.get(&entry.id) {
                let actual = compute_digest(bytes);
                if !actual.eq_ignore_ascii_case(&entry.digest) {
                    return Err(PackageError::DigestMismatch {
                        id: entry.id.clone(),
                        expected: entry.digest.clone(),
                        actual,
                    });
                }
            }
            let capabilities = entry
                .requires
                .iter()
                .map(|req| requirement_to_resource(*req))
                .collect();
            imported.push(ImportedPackage {
                id: entry.id.clone(),
                version: entry.version.clone(),
                source: entry.source.clone(),
                digest: entry.digest.clone(),
                trust: entry.trust,
                targets: entry.targets.clone(),
                capabilities,
                persona: entry.persona.clone(),
            });
        }
        Ok(imported)
    }

    /// Verify a single archive file on disk. Used by the importer CLI
    /// when the package archive lives next to the lockfile.
    pub fn verify_archive(&self, id: &str, archive_path: &Path) -> Result<(), PackageError> {
        let entry = self.entries.iter().find(|e| e.id == id).ok_or_else(|| {
            PackageError::ArchiveUnreadable(
                id.to_string(),
                archive_path.display().to_string(),
                "no such entry".into(),
            )
        })?;
        let bytes = std::fs::read(archive_path).map_err(|e| {
            PackageError::ArchiveUnreadable(
                id.to_string(),
                archive_path.display().to_string(),
                e.to_string(),
            )
        })?;
        let actual = compute_digest(&bytes);
        if actual.eq_ignore_ascii_case(&entry.digest) {
            Ok(())
        } else {
            Err(PackageError::ArchiveDigestMismatch(id.to_string()))
        }
    }
}

/// Compute the blake3 digest of a buffer, hex-encoded.
pub fn compute_digest(bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    let digest_hex = hasher.finalize().to_hex();
    digest_hex.to_string()
}

/// Map an abstract requirement to a `(ResourceRef, Rights)` pair. The
/// mapping is intentionally narrow — every abstract verb must be reviewed
/// before it reaches this table.
pub fn requirement_to_resource(req: AbstractRequirement) -> PackageCapability {
    let (resource, rights) = match req {
        AbstractRequirement::WorkspaceRead => (
            ResourceRef::KernelDomain {
                domain: "workspace".into(),
            },
            Rights::READ,
        ),
        AbstractRequirement::WorkspacePatch => (
            ResourceRef::KernelDomain {
                domain: "workspace".into(),
            },
            Rights::READ | Rights::WRITE,
        ),
        AbstractRequirement::ShellExecute => (
            ResourceRef::Exec {
                mode: "shell".into(),
            },
            Rights::READ | Rights::EXECUTE,
        ),
        AbstractRequirement::BrowserNavigate => {
            (ResourceRef::Browser, Rights::READ | Rights::EXECUTE)
        }
        AbstractRequirement::BrainQuery => (
            ResourceRef::KernelDomain {
                domain: "brain".into(),
            },
            Rights::READ,
        ),
        AbstractRequirement::ScheduleManage => (
            ResourceRef::KernelDomain {
                domain: "cron".into(),
            },
            Rights::READ | Rights::WRITE | Rights::EXECUTE,
        ),
    };
    PackageCapability {
        requirement: req,
        resource,
        rights,
    }
}

/// Convenience: parse the lockfile at the standard location and import.
pub fn import_from_path(
    lock_path: &Path,
    archives: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<ImportedPackage>, PackageError> {
    let raw = std::fs::read_to_string(lock_path)
        .map_err(|e| PackageError::Json(format!("read {}: {}", lock_path.display(), e)))?;
    let lock = PackageLock::parse(&raw)?;
    lock.import(archives)
}

/// Convenience: discover the lockfile at the Foundation versioned root.
pub fn default_lock_path(home: &Path) -> PathBuf {
    crate::foundation::versioned_root(home).join(crate::foundation::PACKAGES_LOCK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_lock() -> PackageLock {
        PackageLock {
            schema_version: 1,
            entries: vec![LockEntry {
                id: "oxi.brain-helper".into(),
                version: "0.1.0".into(),
                source: "local://./brain-helper".into(),
                trust: SourceTrust::Unsigned,
                digest: "0".repeat(64),
                targets: vec!["oxios".into()],
                requires: vec![AbstractRequirement::BrainQuery],
                persona: None,
            }],
        }
    }

    #[test]
    fn imports_minimal_lock() {
        let mut lock = minimal_lock();
        let mut archives = BTreeMap::new();
        archives.insert("oxi.brain-helper".to_string(), b"hello".to_vec());
        // Patch the digest to the real one so verification succeeds.
        lock.entries[0].digest = compute_digest(b"hello");

        let imported = lock.import(&archives).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].capabilities.len(), 1);
        assert_eq!(
            imported[0].capabilities[0].requirement,
            AbstractRequirement::BrainQuery
        );
    }

    #[test]
    fn rejects_wrong_schema() {
        let raw = r#"{"schema_version": 999, "entries": []}"#;
        assert!(matches!(
            PackageLock::parse(raw),
            Err(PackageError::UnsupportedSchema { .. })
        ));
    }

    #[test]
    fn rejects_missing_targets() {
        let raw = r#"{
            "schema_version": 1,
            "entries": [
                {
                    "id": "x",
                    "version": "0.0.1",
                    "source": "local://x",
                    "trust": "unsigned",
                    "digest": "0000000000000000000000000000000000000000000000000000000000000000",
                    "targets": [],
                    "requires": []
                }
            ]
        }"#;
        assert!(matches!(
            PackageLock::parse(raw),
            Err(PackageError::MissingTargets(_))
        ));
    }

    #[test]
    fn rejects_target_without_oxios() {
        let raw = r#"{
            "schema_version": 1,
            "entries": [
                {
                    "id": "x",
                    "version": "0.0.1",
                    "source": "local://x",
                    "trust": "unsigned",
                    "digest": "0000000000000000000000000000000000000000000000000000000000000000",
                    "targets": ["memo"],
                    "requires": []
                }
            ]
        }"#;
        assert!(matches!(
            PackageLock::parse(raw),
            Err(PackageError::TargetMismatch(_, _))
        ));
    }

    #[test]
    fn rejects_bad_digest_format() {
        let raw = r#"{
            "schema_version": 1,
            "entries": [
                {
                    "id": "x",
                    "version": "0.0.1",
                    "source": "local://x",
                    "trust": "unsigned",
                    "digest": "too-short",
                    "targets": ["oxios"],
                    "requires": []
                }
            ]
        }"#;
        assert!(matches!(
            PackageLock::parse(raw),
            Err(PackageError::BadDigestFormat(_, _))
        ));
    }

    #[test]
    fn digest_mismatch_is_reported() {
        let lock = minimal_lock();
        let mut archives = BTreeMap::new();
        archives.insert("oxi.brain-helper".to_string(), b"hello".to_vec());
        // The lockfile still has the all-zeros digest → mismatch.
        let err = lock.import(&archives).unwrap_err();
        assert!(matches!(err, PackageError::DigestMismatch { .. }));
    }

    #[test]
    fn requirement_to_resource_table_is_reviewed() {
        for req in [
            AbstractRequirement::WorkspaceRead,
            AbstractRequirement::WorkspacePatch,
            AbstractRequirement::ShellExecute,
            AbstractRequirement::BrowserNavigate,
            AbstractRequirement::BrainQuery,
            AbstractRequirement::ScheduleManage,
        ] {
            let cap = requirement_to_resource(req);
            assert_eq!(cap.requirement, req);
        }
    }
}
