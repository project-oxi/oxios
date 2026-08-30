//! Integration tests for Foundation bootstrap (RFC-048 §2).
//!
//! These tests cover:
//! - fresh bootstrap with no binary present
//! - idempotent rerun
//! - explicit brain-dir override
//! - binary presence probe reports actionable state
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

use std::path::Path;

#[tokio::test]
async fn bootstrap_creates_foundation_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = oxios_kernel::foundation::bootstrap::BootstrapConfig {
        home: tmp.path().to_path_buf(),
        brain_dir: None,
        may_install: false,
        installer: None,
    };
    let report = oxios_kernel::foundation::bootstrap::bootstrap(&cfg)
        .await
        .unwrap();
    assert!(report.foundation_dir.is_dir());
    // No oxibrain binary in a fresh tmpdir → spawn fails → Unavailable.
    assert!(matches!(
        report.brain.state,
        oxios_kernel::foundation::DaemonState::Unavailable
    ));
    assert!(!report.idempotent);
}

#[tokio::test]
async fn bootstrap_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = oxios_kernel::foundation::bootstrap::BootstrapConfig {
        home: tmp.path().to_path_buf(),
        brain_dir: None,
        may_install: false,
        installer: None,
    };
    let first = oxios_kernel::foundation::bootstrap::bootstrap(&cfg)
        .await
        .unwrap();
    // Drop a sentinel so the second run sees a non-empty Foundation dir.
    std::fs::write(first.foundation_dir.join(".sentinel"), "x").unwrap();
    let second = oxios_kernel::foundation::bootstrap::bootstrap(&cfg)
        .await
        .unwrap();
    assert!(second.idempotent);
}

#[tokio::test]
async fn bootstrap_respects_explicit_brain_dir_override() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("custom-brain");
    let cfg = oxios_kernel::foundation::bootstrap::BootstrapConfig {
        home: tmp.path().to_path_buf(),
        brain_dir: Some(dir.clone()),
        may_install: false,
        installer: None,
    };
    let report = oxios_kernel::foundation::bootstrap::bootstrap(&cfg)
        .await
        .unwrap();
    assert_eq!(report.brain.dir, dir);
    assert!(matches!(
        report.brain.state,
        oxios_kernel::foundation::DaemonState::Unavailable
    ));
}

#[test]
fn quick_probe_classifies_binary_presence() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("never-there");
    assert_eq!(
        oxios_kernel::foundation::bootstrap::quick_probe(&missing),
        oxios_kernel::foundation::DaemonState::Unavailable
    );
    let present = tmp.path().join("oxibrain");
    std::fs::write(&present, "x").unwrap();
    assert_eq!(
        oxios_kernel::foundation::bootstrap::quick_probe(&present),
        oxios_kernel::foundation::DaemonState::Compatible
    );
}

#[test]
fn default_paths_match_rfc_spec() {
    let home = Path::new("/tmp/example-home");
    assert_eq!(
        oxios_kernel::foundation::default_brain_dir(home),
        std::path::PathBuf::from("/tmp/example-home/.oxi/brain")
    );
    assert_eq!(
        oxios_kernel::foundation::versioned_root(home),
        std::path::PathBuf::from("/tmp/example-home/.oxi/foundation/v1")
    );
}
