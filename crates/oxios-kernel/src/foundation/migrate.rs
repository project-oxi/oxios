//! Foundation credential migration (RFC-048 §6).
//!
//! User-invoked migration from legacy `~/.oxios/auth.json` and the shared
//! `~/.oxicode/auth.json` stores into Keychain-backed profile locators.
//!
//! The migration is **explicit** — nothing is auto-deleted, and every
//! step is reversible up until the post-verification archival that the
//! CLI offers only after the operator confirms.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::foundation::profile::{Profile, ProfileRegistry};
use crate::foundation::{default_brain_socket, foundation_root};

/// One legacy credential discovered during the scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyCredential {
    /// Provider label (e.g. `anthropic`, `openai`).
    pub provider: String,
    /// Redacted secret preview — last 4 chars only.
    pub redacted: String,
    /// Source file path.
    pub source: PathBuf,
}

/// Outcome of a single migration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    /// Profiles migrated.
    pub migrated: Vec<String>,
    /// Profiles already present in the Keychain — left untouched.
    pub already_present: Vec<String>,
    /// Profiles that failed to migrate.
    pub failures: Vec<MigrationFailure>,
    /// When the migration completed.
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

/// A migration failure with an actionable cause.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationFailure {
    pub profile_id: String,
    pub provider: String,
    pub reason: String,
}

/// Strict migration errors.
#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("legacy credential source `{0}` is not readable: {1}")]
    LegacyUnreadable(String, String),
    #[error("legacy credential for provider `{0}` is malformed")]
    LegacyMalformed(String),
    #[error("keychain write for profile `{0}` failed: {1}")]
    KeychainWriteFailed(String, String),
    #[error("keychain read-back for profile `{0}` failed: {1}")]
    KeychainReadbackFailed(String, String),
    #[error("profile `{0}` declared by registry is missing from legacy sources")]
    ProfileMissing(String),
}

/// Trait implemented by the actual Keychain backend. The default
/// implementation ([`InMemoryKeychain`]) is used in tests; production
/// uses [`OsKeychain`] which lives in `credential.rs` and writes through
/// the existing secure credential helper.
pub trait KeychainBackend: Send + Sync {
    /// Write `secret` for the given `service`/`account` locator.
    fn write(&self, service: &str, account: &str, secret: &str) -> Result<(), String>;
    /// Read the secret stored under `service`/`account`.
    fn read(&self, service: &str, account: &str) -> Result<Option<String>, String>;
    /// Delete the entry. Used only by the explicit `foundation revoke`
    /// command — never invoked from a normal turn.
    fn delete(&self, service: &str, account: &str) -> Result<(), String>;
}

/// In-memory Keychain used for tests and offline operation. The whole
/// point of moving secrets out of `auth.json` is the OS-backed store; this
/// type is *not* a substitute in production.
#[derive(Default, Debug, Clone)]
pub struct InMemoryKeychain {
    inner: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<(String, String), String>>>,
}

impl KeychainBackend for InMemoryKeychain {
    fn write(&self, service: &str, account: &str, secret: &str) -> Result<(), String> {
        self.inner.lock().map_err(|e| e.to_string())?.insert(
            (service.to_string(), account.to_string()),
            secret.to_string(),
        );
        Ok(())
    }
    fn read(&self, service: &str, account: &str) -> Result<Option<String>, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|e| e.to_string())?
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

/// Scan legacy stores for credentials matching the profile registry.
///
/// The scan never returns the secret itself — only provider name and a
/// redacted preview.
pub fn scan_legacy_credentials(
    home: &Path,
    registry: &ProfileRegistry,
) -> Result<Vec<LegacyCredential>, MigrationError> {
    let mut out = Vec::new();
    for legacy in legacy_paths(home) {
        if !legacy.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&legacy).map_err(|e| {
            MigrationError::LegacyUnreadable(legacy.display().to_string(), e.to_string())
        })?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| MigrationError::LegacyMalformed(legacy.display().to_string()))?;
        // Two shapes are honoured: a top-level `{"providers": {…}}`
        // map (oxios's auth store) and a flat array (legacy cli shape).
        let providers: Vec<(String, String)> =
            if let Some(map) = value.get("providers").and_then(|v| v.as_object()) {
                map.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            } else if let Some(items) = value.get("keys").and_then(|v| v.as_array()) {
                items
                    .iter()
                    .filter_map(|item| {
                        let provider = item.get("provider")?.as_str()?.to_string();
                        let key = item.get("key")?.as_str()?.to_string();
                        Some((provider, key))
                    })
                    .collect()
            } else {
                continue;
            };
        for (provider, key) in providers {
            if registry
                .profiles
                .iter()
                .any(|p| profile_provider_label(p) == provider)
            {
                out.push(LegacyCredential {
                    provider,
                    redacted: redact(&key),
                    source: legacy.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// Migrate every profile in `registry` from the legacy stores into the
/// Keychain via `keychain`. Already-present entries are reported and
/// skipped. A failed write leaves the legacy file untouched.
pub fn migrate_registry(
    home: &Path,
    registry: &ProfileRegistry,
    keychain: &dyn KeychainBackend,
) -> Result<MigrationReport, MigrationError> {
    let mut report = MigrationReport {
        migrated: Vec::new(),
        already_present: Vec::new(),
        failures: Vec::new(),
        completed_at: chrono::Utc::now(),
    };

    let legacy = scan_legacy_credentials(home, registry)?;
    for profile in &registry.profiles {
        let label = profile_provider_label(profile);
        let match_ = legacy.iter().find(|c| c.provider == label);
        let secret = match match_ {
            Some(_) => match read_secret(&label, home) {
                Some(s) => s,
                None => {
                    report.failures.push(MigrationFailure {
                        profile_id: profile.id.clone(),
                        provider: label.clone(),
                        reason: "legacy credential discovered but not extractable".into(),
                    });
                    continue;
                }
            },
            None => {
                report.failures.push(MigrationFailure {
                    profile_id: profile.id.clone(),
                    provider: label.clone(),
                    reason: "no legacy credential for provider".into(),
                });
                continue;
            }
        };

        let service = &profile.keychain.service;
        let account = &profile.keychain.account;

        // Idempotent: already-present entries are reported, not re-written.
        match keychain.read(service, account) {
            Ok(Some(_)) => {
                report.already_present.push(profile.id.clone());
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                report.failures.push(MigrationFailure {
                    profile_id: profile.id.clone(),
                    provider: label.clone(),
                    reason: format!("keychain readback failed: {e}"),
                });
                continue;
            }
        }

        if let Err(e) = keychain.write(service, account, &secret) {
            report.failures.push(MigrationFailure {
                profile_id: profile.id.clone(),
                provider: label.clone(),
                reason: format!("keychain write failed: {e}"),
            });
            continue;
        }

        // Verify the round-trip.
        match keychain.read(service, account) {
            Ok(Some(readback)) if readback == secret => {
                report.migrated.push(profile.id.clone());
            }
            Ok(Some(_)) => {
                report.failures.push(MigrationFailure {
                    profile_id: profile.id.clone(),
                    provider: label.clone(),
                    reason: "keychain round-trip mismatch".into(),
                });
            }
            Ok(None) => {
                report.failures.push(MigrationFailure {
                    profile_id: profile.id.clone(),
                    provider: label.clone(),
                    reason: "keychain entry vanished after write".into(),
                });
            }
            Err(e) => {
                report.failures.push(MigrationFailure {
                    profile_id: profile.id.clone(),
                    provider: label.clone(),
                    reason: format!("keychain readback failed: {e}"),
                });
            }
        }
    }

    Ok(report)
}

/// Render a redacted preview of a secret.
pub fn redact(secret: &str) -> String {
    if secret.is_empty() {
        return "<empty>".into();
    }
    let visible = secret.chars().rev().take(4).collect::<Vec<_>>();
    let visible: String = visible.into_iter().rev().collect();
    format!("****{visible}")
}

/// Compute the Brain socket path the bootstrap step would use. Re-exported
/// so the CLI can render it in `foundation status`.
pub fn brain_socket_path(home: &Path) -> PathBuf {
    default_brain_socket(home)
}

/// Compute the Foundation root for the CLI. Re-exported so the CLI can
/// render it without depending on private paths.
pub fn foundation_dir(home: &Path) -> PathBuf {
    foundation_root(home)
}

fn legacy_paths(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".oxios").join("auth.json"),
        home.join(".oxicode").join("auth.json"),
    ]
}

fn profile_provider_label(profile: &Profile) -> String {
    // We treat the profile id as the operator-facing label for migration
    // because the legacy stores keyed secrets by provider, not by id.
    // The mapping is established by the registry's `id` field — when
    // the operator renames the registry, the rename is the breaking
    // change.
    profile.id.clone()
}

fn read_secret(provider: &str, home: &Path) -> Option<String> {
    for legacy in legacy_paths(home) {
        let raw = std::fs::read_to_string(&legacy).ok()?;
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        if let Some(map) = value.get("providers").and_then(|v| v.as_object()) {
            if let Some(v) = map.get(provider).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
        if let Some(items) = value.get("keys").and_then(|v| v.as_array()) {
            for item in items {
                if item.get("provider").and_then(|v| v.as_str()) == Some(provider) {
                    if let Some(k) = item.get("key").and_then(|v| v.as_str()) {
                        return Some(k.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Revoke (delete) the Keychain entry for a profile. Used by the explicit
/// `foundation revoke <id>` command — never invoked from a normal turn.
pub fn revoke(
    registry: &ProfileRegistry,
    profile_id: &str,
    keychain: &dyn KeychainBackend,
) -> Result<(), MigrationError> {
    let profile = registry
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| MigrationError::ProfileMissing(profile_id.to_string()))?;
    keychain
        .delete(&profile.keychain.service, &profile.keychain.account)
        .map_err(|e| MigrationError::KeychainWriteFailed(profile.id.clone(), e))?;
    Ok(())
}

/// Quick helper used by `foundation status` to render a redacted snapshot
/// of which profiles have a credential available.
pub fn availability(
    registry: &ProfileRegistry,
    keychain: &dyn KeychainBackend,
) -> Vec<(String, bool)> {
    registry
        .profiles
        .iter()
        .map(|p| {
            let present = keychain
                .read(&p.keychain.service, &p.keychain.account)
                .ok()
                .flatten()
                .is_some();
            (p.id.clone(), present)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::profile::{
        KeychainLocator, ModelCapabilities, ProfileRole, ProviderKind,
    };

    fn fixture(home: &Path) -> ProfileRegistry {
        ProfileRegistry {
            schema_version: 1,
            profiles: vec![Profile {
                id: "anthropic-default".into(),
                provider: ProviderKind::Anthropic,
                endpoint: String::new(),
                model: "claude-sonnet-4-5".into(),
                capabilities: ModelCapabilities::default(),
                roles: vec![ProfileRole::CodingPrimary],
                keychain: KeychainLocator {
                    service: "oxios.foundation".into(),
                    account: "profile.anthropic-default".into(),
                },
            }],
        }
    }

    #[test]
    fn redact_hides_secret() {
        assert_eq!(redact(""), "<empty>");
        assert_eq!(redact("abcd"), "****abcd");
        assert_eq!(redact("sk-ant-secret"), "****cret");
    }

    #[test]
    fn scan_finds_providers_in_oxios_auth() {
        let home = tempfile::tempdir().unwrap();
        let auth_dir = home.path().join(".oxios");
        std::fs::create_dir_all(&auth_dir).unwrap();
        std::fs::write(
            auth_dir.join("auth.json"),
            r#"{"providers": {"anthropic-default": "sk-ant-secret"}}"#,
        )
        .unwrap();
        let registry = fixture(home.path());
        let found = scan_legacy_credentials(home.path(), &registry).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].provider, "anthropic-default");
        assert!(found[0].redacted.ends_with("cret"));
        // Make sure we never expose the secret in the redacted field.
        assert!(!found[0].redacted.contains("sk-ant-secret"));
    }

    #[test]
    fn migration_round_trips_into_keychain() {
        let home = tempfile::tempdir().unwrap();
        let auth_dir = home.path().join(".oxios");
        std::fs::create_dir_all(&auth_dir).unwrap();
        std::fs::write(
            auth_dir.join("auth.json"),
            r#"{"providers": {"anthropic-default": "sk-ant-secret"}}"#,
        )
        .unwrap();
        let registry = fixture(home.path());
        let kc = InMemoryKeychain::default();
        let report = migrate_registry(home.path(), &registry, &kc).unwrap();
        assert_eq!(report.migrated, vec!["anthropic-default".to_string()]);
        assert!(report.failures.is_empty());
        let stored = kc
            .read("oxios.foundation", "profile.anthropic-default")
            .unwrap()
            .unwrap();
        assert_eq!(stored, "sk-ant-secret");
    }

    #[test]
    fn migration_is_idempotent() {
        let home = tempfile::tempdir().unwrap();
        let auth_dir = home.path().join(".oxios");
        std::fs::create_dir_all(&auth_dir).unwrap();
        std::fs::write(
            auth_dir.join("auth.json"),
            r#"{"providers": {"anthropic-default": "sk-ant-secret"}}"#,
        )
        .unwrap();
        let registry = fixture(home.path());
        let kc = InMemoryKeychain::default();
        migrate_registry(home.path(), &registry, &kc).unwrap();
        let report = migrate_registry(home.path(), &registry, &kc).unwrap();
        assert_eq!(
            report.already_present,
            vec!["anthropic-default".to_string()]
        );
        assert!(report.migrated.is_empty());
    }

    #[test]
    fn migration_failure_leaves_legacy_file() {
        let home = tempfile::tempdir().unwrap();
        let auth_dir = home.path().join(".oxios");
        std::fs::create_dir_all(&auth_dir).unwrap();
        let legacy_path = auth_dir.join("auth.json");
        std::fs::write(
            &legacy_path,
            r#"{"providers": {"anthropic-default": "sk-ant-secret"}}"#,
        )
        .unwrap();
        let registry = fixture(home.path());
        let kc = FailingKeychain;
        let report = migrate_registry(home.path(), &registry, &kc).unwrap();
        assert_eq!(report.failures.len(), 1);
        // Legacy file must remain untouched.
        let after = std::fs::read_to_string(&legacy_path).unwrap();
        assert!(after.contains("sk-ant-secret"));
    }

    /// A keychain backend whose writes always fail. Used to assert that
    /// the migration refuses to touch the legacy file when the Keychain
    /// round-trip cannot complete.
    struct FailingKeychain;
    impl KeychainBackend for FailingKeychain {
        fn write(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Err("simulated failure".into())
        }
        fn read(&self, _: &str, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), String> {
            Err("simulated failure".into())
        }
    }

    #[test]
    fn missing_profile_revoke_errors() {
        let home = tempfile::tempdir().unwrap();
        let registry = fixture(home.path());
        let kc = InMemoryKeychain::default();
        let err = revoke(&registry, "ghost", &kc).unwrap_err();
        assert!(matches!(err, MigrationError::ProfileMissing(_)));
    }

    #[test]
    fn availability_reports_each_profile() {
        let home = tempfile::tempdir().unwrap();
        let registry = fixture(home.path());
        let kc = InMemoryKeychain::default();
        kc.write("oxios.foundation", "profile.anthropic-default", "x")
            .unwrap();
        let avail = availability(&registry, &kc);
        assert_eq!(avail, vec![("anthropic-default".to_string(), true)]);
    }

    #[test]
    fn brain_socket_path_uses_default() {
        let home = Path::new("/tmp/example");
        assert_eq!(
            brain_socket_path(home),
            PathBuf::from("/tmp/example/.oxi/brain/oxibrain.sock")
        );
        // touch Result import to keep it in the public surface
        let _: Result<()> = Ok(());
    }
}
