//! Daemon lifecycle integration tests (RFC-040 C2).
//!
//! Tests DaemonManager's status resolution, stale pidfile cleanup,
//! and orphan detection — the multi-source liveness interpretation
//! that is the core of daemon reliability.
#![allow(clippy::unwrap_used)] // `.unwrap()` on setup ops is idiomatic in tests (workspace convention)

use oxios_kernel::{DaemonManager, DaemonStatus};

// ── PID file lifecycle ─────────────────────────────────────────────

/// Stale pidfile (dead PID) → Stale status → cleanup → Stopped.
#[test]
fn stale_pidfile_detected_and_cleaned() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("oxios.pid");
    // Write a PID that is virtually guaranteed not to exist.
    std::fs::write(&pid_file, "999999").unwrap();
    let dm = DaemonManager::new(pid_file.to_str().unwrap(), tmp.path().to_str().unwrap());
    assert!(
        matches!(dm.status(), DaemonStatus::Stale { .. }),
        "dead PID in pidfile should be Stale"
    );
    dm.cleanup().unwrap();
    assert!(
        matches!(dm.status(), DaemonStatus::Stopped),
        "after cleanup, should be Stopped"
    );
}

/// No pidfile → Stopped.
#[test]
fn no_pidfile_is_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let dm = DaemonManager::new(
        tmp.path().join("nonexistent.pid").to_str().unwrap(),
        tmp.path().to_str().unwrap(),
    );
    assert!(matches!(dm.status(), DaemonStatus::Stopped));
}

/// Fresh pidfile (current PID) → Running.
#[cfg(unix)]
#[test]
fn fresh_pidfile_reports_running() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("oxios.pid");
    std::fs::write(&pid_file, std::process::id().to_string()).unwrap();
    let dm = DaemonManager::new(pid_file.to_str().unwrap(), tmp.path().to_str().unwrap());
    assert!(
        matches!(dm.status(), DaemonStatus::Running { .. }),
        "current PID in pidfile should be Running"
    );
    dm.cleanup().unwrap();
}

// ── Orphan detection (Unix) ────────────────────────────────────────

/// Port probe detects a listener when no pidfile exists.
#[cfg(unix)]
#[test]
fn orphan_detection_finds_listener() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let tmp = tempfile::tempdir().unwrap();
    let dm = DaemonManager::new(
        tmp.path().join("oxios.pid").to_str().unwrap(),
        tmp.path().to_str().unwrap(),
    )
    .with_probe_port(port);
    // Hold the listener — don't drop until after status() returns.
    let status = dm.status();
    drop(listener);
    match status {
        DaemonStatus::Orphaned { port: p } => assert_eq!(p, port),
        other => panic!("expected Orphaned, got {other:?}"),
    }
}

/// No listener + no pidfile → Stopped (not Orphaned).
#[test]
fn no_listener_is_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    // Use a privileged port that will never be bound in tests.
    let dm = DaemonManager::new(
        tmp.path().join("oxios.pid").to_str().unwrap(),
        tmp.path().to_str().unwrap(),
    )
    .with_probe_port(1);
    assert!(matches!(dm.status(), DaemonStatus::Stopped));
}

// ── DaemonStatus Display ───────────────────────────────────────────

/// DaemonStatus Display impl produces human-readable strings.
#[test]
fn daemon_status_display() {
    assert_eq!(
        DaemonStatus::Running { pid: 42 }.to_string(),
        "running (PID 42)"
    );
    assert_eq!(
        DaemonStatus::Stale { pid: 99 }.to_string(),
        "stale (PID 99 dead)"
    );
    assert_eq!(DaemonStatus::Stopped.to_string(), "stopped");
    assert_eq!(
        DaemonStatus::Orphaned { port: 4200 }.to_string(),
        "orphaned (no pidfile, port 4200 in use)"
    );
}
