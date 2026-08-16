//! Integration tests for credential migration (RFC-048 §6).
//!
//! Covers successful migration, idempotent rerun, Keychain write failure
//! leaving the legacy file intact, redacted reports, and revocation.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use oxios_kernel::foundation::migrate::{
    KeychainBackend, MigrationError, redact, revoke, scan_legacy_credentials,
};
use oxios_kernel::foundation::profile::{
    KeychainLocator, ModelCapabilities, Profile, ProfileRegistry, ProfileRole, ProviderKind,
};

/// In-memory keychain that records every write. Used by the failure-mode
/// tests so the failure path can be inspected without touching the OS
/// Keychain.
#[derive(Clone, Default)]
struct RecordingKeychain {
    inner: Arc<Mutex<HashMap<(String, String), String>>>,
    fail_writes: Arc<Mutex<bool>>,
}

impl KeychainBackend for RecordingKeychain {
    fn write(&self, service: &str, account: &str, secret: &str) -> Result<(), String> {
        if *self.fail_writes.lock() {
            return Err("simulated write failure".into());
        }
        self.inner.lock().insert(
            (service.to_string(), account.to_string()),
            secret.to_string(),
        );
        Ok(())
    }
    fn read(&self, service: &str, account: &str) -> Result<Option<String>, String> {
        Ok(self
            .inner
            .lock()
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.inner
            .lock()
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

fn fixture(home: &std::path::Path) -> ProfileRegistry {
    ProfileRegistry {
        schema_version: 1,
        profiles: vec![Profile {
            id: "anthropic-coding".into(),
            provider: ProviderKind::Anthropic,
            endpoint: String::new(),
            model: "claude-sonnet-4-5".into(),
            capabilities: ModelCapabilities::default(),
            roles: vec![ProfileRole::CodingPrimary],
            keychain: KeychainLocator {
                service: "oxios.foundation".into(),
                account: "profile.anthropic-coding".into(),
            },
        }],
    }
}

#[test]
fn migration_round_trips_legacy_secret() {
    let home = tempfile::tempdir().unwrap();
    let auth_dir = home.path().join(".oxios");
    std::fs::create_dir_all(&auth_dir).unwrap();
    std::fs::write(
        auth_dir.join("auth.json"),
        r#"{"providers":{"anthropic-coding":"sk-ant-secret"}}"#,
    )
    .unwrap();
    let registry = fixture(home.path());
    let kc = RecordingKeychain::default();

    let report = oxios_kernel::foundation::migrate::migrate_registry(home.path(), &registry, &kc)
        .expect("migration runs");
    assert_eq!(report.migrated, vec!["anthropic-coding".to_string()]);
    assert!(report.failures.is_empty());
    let stored = kc
        .read("oxios.foundation", "profile.anthropic-coding")
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
        r#"{"providers":{"anthropic-coding":"sk-ant-secret"}}"#,
    )
    .unwrap();
    let registry = fixture(home.path());
    let kc = RecordingKeychain::default();

    oxios_kernel::foundation::migrate::migrate_registry(home.path(), &registry, &kc).unwrap();
    let second =
        oxios_kernel::foundation::migrate::migrate_registry(home.path(), &registry, &kc).unwrap();
    assert_eq!(second.already_present, vec!["anthropic-coding".to_string()]);
    assert!(second.migrated.is_empty());
}

#[test]
fn migration_failure_leaves_legacy_file_intact() {
    let home = tempfile::tempdir().unwrap();
    let auth_dir = home.path().join(".oxios");
    std::fs::create_dir_all(&auth_dir).unwrap();
    let legacy = auth_dir.join("auth.json");
    std::fs::write(
        &legacy,
        r#"{"providers":{"anthropic-coding":"sk-ant-secret"}}"#,
    )
    .unwrap();
    let registry = fixture(home.path());
    let kc = RecordingKeychain::default();
    *kc.fail_writes.lock() = true;

    let report = oxios_kernel::foundation::migrate::migrate_registry(home.path(), &registry, &kc)
        .expect("report even on failure");
    assert_eq!(report.failures.len(), 1);
    let after = std::fs::read_to_string(&legacy).unwrap();
    assert!(after.contains("sk-ant-secret"));
}

#[test]
fn scan_redacts_secret_in_report() {
    let home = tempfile::tempdir().unwrap();
    let auth_dir = home.path().join(".oxios");
    std::fs::create_dir_all(&auth_dir).unwrap();
    std::fs::write(
        auth_dir.join("auth.json"),
        r#"{"providers":{"anthropic-coding":"sk-ant-secret"}}"#,
    )
    .unwrap();
    let registry = fixture(home.path());
    let found = scan_legacy_credentials(home.path(), &registry).unwrap();
    assert_eq!(found.len(), 1);
    assert!(!found[0].redacted.contains("sk-ant-secret"));
    assert!(found[0].redacted.contains("cret"));
}

#[test]
fn revoke_clears_keychain_entry() {
    let home = tempfile::tempdir().unwrap();
    let registry = fixture(home.path());
    let kc = RecordingKeychain::default();
    kc.write("oxios.foundation", "profile.anthropic-coding", "x")
        .unwrap();
    revoke(&registry, "anthropic-coding", &kc).unwrap();
    assert!(
        kc.read("oxios.foundation", "profile.anthropic-coding")
            .unwrap()
            .is_none()
    );
}

#[test]
fn revoke_missing_profile_errors() {
    let home = tempfile::tempdir().unwrap();
    let registry = fixture(home.path());
    let kc = RecordingKeychain::default();
    let err = revoke(&registry, "ghost", &kc).unwrap_err();
    assert!(matches!(err, MigrationError::ProfileMissing(_)));
}

#[test]
fn redact_masks_secret() {
    assert_eq!(redact(""), "<empty>");
    assert!(redact("abcdefgh").starts_with("****"));
    assert!(redact("abcdefgh").ends_with("efgh"));
}
