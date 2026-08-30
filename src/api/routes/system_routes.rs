//! Local system surface: native folder picker (`POST /api/system/pick-folders`).
//!
//! The picker opens the macOS native folder chooser via `osascript` and
//! returns the user-confirmed, server-canonicalized directory. Contract
//! (design §4.4 / §9):
//!
//! - `200 { "paths": ["/abs/canonical/dir"] }` — user picked one folder
//! - `200 { "paths": [], "cancelled": true }`  — user cancelled; nothing changes
//! - `403 { "error": "folder picker is only available on the local host" }`
//! - `501 { "error": "folder picker is not supported on this platform" }`
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use serde_json::json;

use crate::api::error::AppError;
use crate::api::server::AppState;

/// Whether the request originated from a loopback IP (`127.0.0.0/8`, `::1`).
///
/// `::` (unspecified) also counts as local: some proxy setups present the
/// daemon with `::` for same-host connections.
fn is_loopback_ip(addr: &SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// AppleScript: native single-folder chooser; prints the POSIX path on stdout.
const PICK_FOLDER_SCRIPT: &str =
    r#"POSIX path of (choose folder with prompt "Choose a folder for this project")"#;

/// Result of one picker invocation (child-process output or spawn error).
pub(crate) type PickResult = std::io::Result<std::process::Output>;

/// Injectable picker backend: returns the raw `osascript` process output.
pub(crate) type PickerRunner =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = PickResult> + Send>> + Send + Sync>;

/// Test-only override slot for the picker runner.
#[cfg(test)]
static PICKER_OVERRIDE: parking_lot::RwLock<Option<PickerRunner>> = parking_lot::RwLock::new(None);

/// Production runner: spawn `osascript` off the main thread.
async fn osascript_runner() -> PickResult {
    tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(PICK_FOLDER_SCRIPT)
        .kill_on_drop(true)
        .output()
        .await
}

/// The active runner: the test override when installed, else osascript.
#[cfg(test)]
fn active_runner() -> PickerRunner {
    let override_runner = PICKER_OVERRIDE.read().clone();
    override_runner.unwrap_or_else(|| {
        Arc::new(|| -> Pin<Box<dyn Future<Output = PickResult> + Send>> {
            Box::pin(osascript_runner())
        })
    })
}

#[cfg(not(test))]
fn active_runner() -> PickerRunner {
    Arc::new(|| -> Pin<Box<dyn Future<Output = PickResult> + Send>> {
        Box::pin(osascript_runner())
    })
}
/// Install a stub runner for the duration of the returned guard.
#[cfg(test)]
fn with_picker_override(runner: PickerRunner) -> impl Drop {
    *PICKER_OVERRIDE.write() = Some(runner);
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            *PICKER_OVERRIDE.write() = None;
        }
    }
    Guard
}

fn cancelled_body() -> serde_json::Value {
    json!({ "paths": [], "cancelled": true })
}

/// Run the picker and map the raw process output onto the wire contract.
async fn run_picker(runner: &PickerRunner) -> Result<serde_json::Value, AppError> {
    #[cfg(target_os = "macos")]
    {
        let output = runner()
            .await
            .map_err(|e| AppError::Internal(format!("folder picker failed to launch: {e}")))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            // osascript reports cancellation as error -128 ("User canceled").
            if stderr.contains("User canceled") || stderr.contains("-128") {
                return Ok(cancelled_body());
            }
            return Err(AppError::Internal(format!(
                "folder picker failed: {}",
                stderr.trim()
            )));
        }

        // Single-select per call: at most one path.
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            // Defensive: no selection means nothing changed.
            return Ok(cancelled_body());
        }
        let picked = PathBuf::from(&raw);
        if !picked.is_dir() {
            return Err(AppError::BadRequest(
                "selected path is not a directory".into(),
            ));
        }
        let canonical = picked
            .canonicalize()
            .map_err(|e| AppError::BadRequest(format!("selected folder is not accessible: {e}")))?;
        Ok(json!({ "paths": [canonical.display().to_string()] }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = runner;
        Err(AppError::NotImplemented(
            "folder picker is not supported on this platform".into(),
        ))
    }
}

/// `POST /api/system/pick-folders` — open the native folder chooser.
pub(crate) async fn handle_pick_folders(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !is_loopback_ip(&peer) {
        tracing::warn!(peer = %peer, "Rejected /api/system/pick-folders from non-loopback client");
        return Err(AppError::Forbidden(
            "folder picker is only available on the local host".into(),
        ));
    }

    let runner = active_runner();
    let result = run_picker(&runner).await;

    // Audit the outcome only — the picked path is never persisted server-side.
    match &result {
        Ok(body) if body.get("cancelled").is_some() => {
            state.kernel.security.audit(
                "web-system",
                oxicode_sdk::AuditAction::Other {
                    detail: "pick_folders:cancelled".into(),
                },
                "system",
            );
        }
        Ok(_) => {
            state.kernel.security.audit(
                "web-system",
                oxicode_sdk::AuditAction::Other {
                    detail: "pick_folders:completed count=1".into(),
                },
                "system",
            );
        }
        Err(_) => {}
    }

    result.map(Json)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // `.unwrap()` on setup ops is idiomatic in tests

    use super::*;
    use crate::api::routes::test_support;

    #[cfg(unix)]
    fn make_output(status_success: bool, stdout: &str, stderr: &str) -> PickResult {
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(if status_success { 0 } else { 1 }),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    fn stub_runner(status_success: bool, stdout: String, stderr: String) -> PickerRunner {
        Arc::new(move || {
            Box::pin(std::future::ready(make_output(
                status_success,
                &stdout,
                &stderr,
            ))) as Pin<Box<dyn Future<Output = PickResult> + Send>>
        })
    }

    #[test]
    fn loopback_detection_covers_v4_v6_and_rejects_remote() {
        assert!(is_loopback_ip(&"127.0.0.1:9000".parse().unwrap()));
        assert!(is_loopback_ip(&"127.9.0.1:9000".parse().unwrap()));
        assert!(is_loopback_ip(&"[::1]:9000".parse().unwrap()));
        assert!(!is_loopback_ip(&"192.168.1.10:9000".parse().unwrap()));
        assert!(!is_loopback_ip(&"10.0.0.2:9000".parse().unwrap()));
    }

    #[tokio::test]
    async fn cancelled_osascript_maps_to_cancelled_response() {
        let runner = stub_runner(
            false,
            String::new(),
            "execution error: User canceled. (-128)\n".into(),
        );
        let body = run_picker(&runner).await.unwrap();
        assert_eq!(body, serde_json::json!({ "paths": [], "cancelled": true }));
    }

    #[tokio::test]
    async fn successful_pick_returns_canonicalized_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = format!("{}/\n", tmp.path().display());
        let runner = stub_runner(true, raw, String::new());
        let body = run_picker(&runner).await.unwrap();
        let canonical = tmp.path().canonicalize().unwrap();
        assert_eq!(
            body,
            serde_json::json!({ "paths": [canonical.display().to_string()] })
        );
    }

    #[tokio::test]
    async fn non_directory_selection_is_rejected_in_english() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("plain.txt");
        std::fs::write(&file, "x").unwrap();
        let runner = stub_runner(true, format!("{}\n", file.display()), String::new());
        let err = run_picker(&runner).await.unwrap_err();
        assert!(
            err.to_string().contains("not a directory"),
            "English error expected, got: {err}"
        );
    }

    #[tokio::test]
    async fn other_stderr_failure_maps_to_internal_error() {
        let runner = stub_runner(false, String::new(), "some unexpected failure\n".into());
        let err = run_picker(&runner).await.unwrap_err();
        assert!(err.to_string().contains("folder picker failed"), "{err}");
        assert!(err.to_string().contains("some unexpected failure"));
    }

    #[tokio::test]
    async fn remote_clients_are_rejected_with_local_only_error() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());
        let remote: SocketAddr = "203.0.113.7:51000".parse().unwrap();

        let result = handle_pick_folders(State(app.state.clone()), ConnectInfo(remote)).await;
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("folder picker is only available on the local host"),
            "exact contract message expected, got: {err}"
        );
    }

    #[tokio::test]
    async fn loopback_client_reaches_picker_and_override_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_support::test_app(tmp.path());

        let inner = tmp.path().join("picked");
        std::fs::create_dir_all(&inner).unwrap();
        let _guard = with_picker_override(stub_runner(
            true,
            format!("{}\n", inner.display()),
            String::new(),
        ));

        let local: SocketAddr = test_support::loopback_peer();
        let response = handle_pick_folders(State(app.state.clone()), ConnectInfo(local))
            .await
            .unwrap();
        let canonical = inner.canonicalize().unwrap();
        assert_eq!(
            response.0,
            serde_json::json!({ "paths": [canonical.display().to_string()] })
        );
    }
}
