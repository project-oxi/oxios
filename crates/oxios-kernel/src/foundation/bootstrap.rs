//! Idempotent Foundation bootstrap (RFC-048 §2).
//!
//! The first-run path is:
//!   1. Ensure `~/.oxi/foundation/v1` exists.
//!   2. Locate an `oxibrain` binary (installing on demand when the caller
//!      allows it).
//!   3. Run a one-shot `oxibrain --dir <dir> describe` probe and classify
//!      the data plane as [`DaemonState::Compatible`] or
//!      [`DaemonState::Unavailable`].
//!   4. Report the result. Never write the lockfile from a turn — the lock
//!      file is owned by Foundation imports, not by agent execution.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use super::{DaemonState, default_brain_dir, versioned_root};
use crate::brain::BrainInstaller;

/// Upper bound on one probe process (`describe` is a read-only store open).
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Brain probe result. `protocol_version` is the envelope `api` field —
/// the contract version the binary speaks (spec `agent-first-cli-v1` §3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainProbe {
    pub state: DaemonState,
    /// Brain data directory the probe ran against.
    pub dir: PathBuf,
    /// Envelope protocol version; `None` when no envelope came back.
    pub protocol_version: Option<u32>,
}

/// Foundation bootstrap report. Returned from [`bootstrap`] so CLI
/// onboarding and `foundation status` can render the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapReport {
    pub foundation_dir: PathBuf,
    pub profiles_loaded: usize,
    pub brain: BrainProbe,
    /// `true` when the bootstrap was a no-op because everything was
    /// already in place. Lets the CLI distinguish a fresh install from a
    /// routine re-run.
    pub idempotent: bool,
}

/// Configuration for a single bootstrap run. The `brain_dir` mirrors
/// `FoundationConfig.brain_dir`; when empty, the Foundation default is used.
#[derive(Clone)]
pub struct BootstrapConfig {
    pub home: PathBuf,
    pub brain_dir: Option<PathBuf>,
    /// When `true`, the bootstrap step is allowed to (idempotently) install
    /// a missing compatible binary through the supplied `installer`.
    /// `false` (default) reports `Unavailable` and lets the user run
    /// `oxios brain install` explicitly.
    pub may_install: bool,
    /// Installer consulted to resolve — and, when `may_install` is set,
    /// fetch — the `oxibrain` binary. `None` (default) probes only the
    /// managed `~/.oxi/oxibrain/bin/oxibrain` launcher location.
    pub installer: Option<Arc<BrainInstaller>>,
}

impl std::fmt::Debug for BootstrapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapConfig")
            .field("home", &self.home)
            .field("brain_dir", &self.brain_dir)
            .field("may_install", &self.may_install)
            .field("installer", &self.installer.as_ref().map(|_| "<installer>"))
            .finish()
    }
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            home: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
            brain_dir: None,
            may_install: false,
            installer: None,
        }
    }
}

/// Run the idempotent bootstrap. Never panics; reports every step.
pub async fn bootstrap(cfg: &BootstrapConfig) -> Result<BootstrapReport> {
    let foundation_dir = versioned_root(&cfg.home);
    ensure_directory(&foundation_dir)
        .with_context(|| format!("create foundation directory {}", foundation_dir.display()))?;
    let dir = cfg
        .brain_dir
        .clone()
        .unwrap_or_else(|| default_brain_dir(&cfg.home));

    // The first time we see an empty Foundation dir we are *not* idempotent.
    let fresh = std::fs::read_dir(&foundation_dir)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    if fresh {
        info!(
            dir = %foundation_dir.display(),
            "fresh foundation directory created"
        );
    }

    let exe = brain_executable(&cfg.home, cfg.installer.as_deref());
    let brain = probe_brain_with(&exe, &dir, cfg.may_install, cfg.installer.as_deref()).await;
    match brain.state {
        DaemonState::Compatible => info!(
            dir = %brain.dir.display(),
            "brain probe ok"
        ),
        DaemonState::Unavailable => warn!(
            dir = %brain.dir.display(),
            "brain unavailable — degraded mode is expected (RFC-047)"
        ),
        DaemonState::Incompatible => warn!(
            dir = %brain.dir.display(),
            "brain protocol incompatible — install a compatible release"
        ),
    }

    Ok(BootstrapReport {
        foundation_dir,
        profiles_loaded: 0, // populated by the resolver; bootstrap only writes the dir.
        brain,
        idempotent: !fresh,
    })
}

/// Resolve the `oxibrain` executable for probes: installer resolution
/// (explicit config → managed launcher → legacy location → `PATH`) with
/// the managed `~/.oxi/oxibrain/bin/oxibrain` location as the fallback, so
/// a post-install retry can succeed. The pre-standard
/// `~/.oxi/bin/oxibrain` still resolves when the launcher is absent.
pub fn brain_executable(home: &Path, installer: Option<&BrainInstaller>) -> PathBuf {
    if let Some(found) = installer.and_then(|i| i.locate_binary()) {
        return found;
    }
    let managed = home
        .join(".oxi")
        .join("oxibrain")
        .join("bin")
        .join("oxibrain");
    if managed.is_file() {
        return managed;
    }
    let legacy = home.join(".oxi").join("bin").join("oxibrain");
    if legacy.is_file() {
        return legacy;
    }
    managed
}

/// Run one `describe` probe and classify. A failed attempt (spawn error or
/// non-zero exit) triggers exactly one install pass when `may_install` is
/// set, then retries once; `ensure_binary` cannot fix a broken store, but
/// it does recover a partial/corrupt release.
pub async fn probe_brain_with(
    exe: &Path,
    dir: &Path,
    may_install: bool,
    installer: Option<&BrainInstaller>,
) -> BrainProbe {
    let mut probe = describe_probe(exe, dir).await;
    if probe.state == DaemonState::Compatible {
        return probe;
    }
    if may_install
        && let Some(installer) = installer
        && installer.ensure_binary(true).await.is_some()
    {
        probe = describe_probe(exe, dir).await;
    }
    probe
}

/// One `oxibrain --dir D describe` attempt with failures classified.
async fn describe_probe(exe: &Path, dir: &Path) -> BrainProbe {
    let unavailable = |dir: &Path| BrainProbe {
        state: DaemonState::Unavailable,
        dir: dir.to_path_buf(),
        protocol_version: None,
    };
    let Ok(out) = tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio::process::Command::new(exe)
            .arg("--dir")
            .arg(dir)
            .arg("describe")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    else {
        warn!(dir = %dir.display(), "brain describe probe timed out");
        return unavailable(dir);
    };
    let out = match out {
        Ok(out) => out,
        Err(e) => {
            warn!(
                dir = %dir.display(),
                error = %e,
                "brain probe spawn failed — degraded (RFC-047)"
            );
            return unavailable(dir);
        }
    };
    if !out.status.success() {
        warn!(
            dir = %dir.display(),
            stderr = %String::from_utf8_lossy(&out.stderr),
            "brain describe probe failed — degraded (RFC-047)"
        );
        return unavailable(dir);
    }
    let Ok(envelope) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        warn!(
            dir = %dir.display(),
            "brain describe probe printed no envelope — degraded"
        );
        return unavailable(dir);
    };
    BrainProbe {
        state: DaemonState::Compatible,
        dir: dir.to_path_buf(),
        protocol_version: envelope
            .get("api")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32),
    }
}

fn ensure_directory(dir: &Path) -> Result<()> {
    if dir.exists() {
        if !dir.is_dir() {
            anyhow::bail!(
                "foundation path {} exists but is not a directory",
                dir.display()
            );
        }
        return Ok(());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("create_dir_all {}", dir.display()))?;
    Ok(())
}

/// Quick non-spawning readiness probe used by `foundation status`.
///
/// Process-per-operation readiness is binary presence: `Compatible` means a
/// spawnable `oxibrain` exists at `binary`. The full classification comes
/// from [`probe_brain_with`].
pub fn quick_probe(binary: &Path) -> DaemonState {
    if binary.is_file() {
        DaemonState::Compatible
    } else {
        DaemonState::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_directory_creates_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("nested/foundation/v1");
        ensure_directory(&target).unwrap();
        assert!(target.is_dir());
        // Idempotent
        ensure_directory(&target).unwrap();
    }

    #[test]
    fn ensure_directory_rejects_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let err = ensure_directory(&file).unwrap_err().to_string();
        assert!(err.contains("not a directory"), "got: {err}");
    }

    /// Probing a path with no binary degrades to `Unavailable` without
    /// ever running anything.
    #[tokio::test]
    async fn probe_unavailable_when_binary_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let hs = probe_brain_with(
            &tmp.path().join("missing-oxibrain"),
            &tmp.path().join("brain"),
            false,
            None,
        )
        .await;
        assert_eq!(hs.state, DaemonState::Unavailable);
        assert_eq!(hs.protocol_version, None);
        assert_eq!(hs.dir, tmp.path().join("brain"));
    }

    #[cfg(unix)]
    fn write_stub(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A stub `oxibrain` shell script: counts runs in `counter`, exits
    /// non-zero on the first run, answers `describe` after.
    #[cfg(unix)]
    fn flaky_stub(dir: &Path) -> (PathBuf, PathBuf) {
        let stub = dir.join("oxibrain");
        let counter = dir.join("runs");
        write_stub(
            &stub,
            &format!(
                "#!/bin/sh\n\
                 n=$(cat {counter} 2>/dev/null || echo 0)\n\
                 n=$((n+1))\n\
                 echo $n > {counter}\n\
                 if [ \"$n\" -eq 1 ]; then exit 1; fi\n\
                 printf '%s' '{{\"api\":1,\"ok\":true,\"op\":\"describe\",\"data\":{{}}}}'\n",
                counter = counter.display()
            ),
        );
        (stub, counter)
    }

    #[cfg(unix)]
    fn stub_installer(dir: &Path, stub: &Path) -> Arc<BrainInstaller> {
        let section = crate::config::BrainSection {
            binary_path: stub.display().to_string(),
            ..Default::default()
        };
        Arc::new(BrainInstaller::from_brain_section(dir, &section))
    }

    /// The installer hook runs exactly once when `may_install` is set:
    /// `ensure_binary` resolves the (already present) stub without any
    /// network access, and the single retry completes the probe — exactly
    /// two runs in total.
    #[cfg(unix)]
    #[tokio::test]
    async fn probe_retries_once_after_installer_hook() {
        let tmp = tempfile::tempdir().unwrap();
        let (stub, counter) = flaky_stub(tmp.path());
        let installer = stub_installer(tmp.path(), &stub);
        let hs = probe_brain_with(&stub, &tmp.path().join("brain"), true, Some(&installer)).await;
        assert_eq!(hs.state, DaemonState::Compatible);
        assert_eq!(hs.protocol_version, Some(1));
        assert_eq!(std::fs::read_to_string(&counter).unwrap().trim(), "2");
    }

    /// Without `may_install` there is no hook and no retry: one run, then
    /// `Unavailable`.
    #[cfg(unix)]
    #[tokio::test]
    async fn probe_does_not_retry_without_may_install() {
        let tmp = tempfile::tempdir().unwrap();
        let (stub, counter) = flaky_stub(tmp.path());
        let installer = stub_installer(tmp.path(), &stub);
        let hs = probe_brain_with(&stub, &tmp.path().join("brain"), false, Some(&installer)).await;
        assert_eq!(hs.state, DaemonState::Unavailable);
        assert_eq!(std::fs::read_to_string(&counter).unwrap().trim(), "1");
    }

    /// A healthy stub classifies `Compatible` on the first run.
    #[cfg(unix)]
    #[tokio::test]
    async fn probe_compatible_on_describe_envelope() {
        let tmp = tempfile::tempdir().unwrap();
        let stub = tmp.path().join("oxibrain");
        write_stub(
            &stub,
            "#!/bin/sh\nprintf '%s' '{\"api\":1,\"ok\":true,\"op\":\"describe\",\"data\":{}}'\n",
        );
        let hs = probe_brain_with(&stub, &tmp.path().join("brain"), false, None).await;
        assert_eq!(hs.state, DaemonState::Compatible);
        assert_eq!(hs.protocol_version, Some(1));
        assert_eq!(hs.dir, tmp.path().join("brain"));
    }
}
