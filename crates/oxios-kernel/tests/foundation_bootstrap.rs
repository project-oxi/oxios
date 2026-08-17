//! Integration tests for Foundation bootstrap (RFC-048 §2).
//!
//! These tests cover:
//! - fresh bootstrap with no daemon present
//! - idempotent rerun
//! - explicit endpoint override
//! - missing executable reported as actionable (not a silent fallback)
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

use std::path::Path;

#[tokio::test]
async fn bootstrap_creates_foundation_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = oxios_kernel::foundation::bootstrap::BootstrapConfig {
        home: tmp.path().to_path_buf(),
        socket_path: None,
        may_start_daemon: false,
    };
    let report = oxios_kernel::foundation::bootstrap::bootstrap(&cfg)
        .await
        .unwrap();
    assert!(report.foundation_dir.is_dir());
    // No daemon socket present in a fresh tmpdir → Unavailable.
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
        socket_path: None,
        may_start_daemon: false,
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
async fn bootstrap_respects_explicit_socket_override() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("custom-brain.sock");
    let cfg = oxios_kernel::foundation::bootstrap::BootstrapConfig {
        home: tmp.path().to_path_buf(),
        socket_path: Some(socket.clone()),
        may_start_daemon: false,
    };
    let report = oxios_kernel::foundation::bootstrap::bootstrap(&cfg)
        .await
        .unwrap();
    assert_eq!(report.brain.socket_path, socket);
    assert!(matches!(
        report.brain.state,
        oxios_kernel::foundation::DaemonState::Unavailable
    ));
}

#[tokio::test]
async fn missing_daemon_reports_unavailable_without_blocking() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("never-listening.sock");
    let state = oxios_kernel::foundation::bootstrap::quick_probe(&socket).await;
    assert_eq!(state, oxios_kernel::foundation::DaemonState::Unavailable);
}

#[tokio::test]
async fn default_paths_match_rfc_spec() {
    let home = Path::new("/tmp/example-home");
    assert_eq!(
        oxios_kernel::foundation::default_brain_socket(home),
        std::path::PathBuf::from("/tmp/example-home/.oxi/brain/oxibrain.sock")
    );
    assert_eq!(
        oxios_kernel::foundation::versioned_root(home),
        std::path::PathBuf::from("/tmp/example-home/.oxi/foundation/v1")
    );
}
