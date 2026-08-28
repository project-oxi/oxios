//! Idempotent Foundation bootstrap (RFC-048 §2).
//!
//! The first-run path is:
//!   1. Ensure `~/.oxi/foundation/v1` exists.
//!   2. Locate a compatible `oxibrain` binary (installing on demand when
//!      the caller allows it).
//!   3. Spawn a short-lived session child, perform the capability
//!      handshake, and classify the server as [`DaemonState::Compatible`]
//!      / [`DaemonState::Unavailable`] / [`DaemonState::Incompatible`].
//!   4. Report the result. Never write the lockfile from a turn — the lock
//!      file is owned by Foundation imports, not by agent execution.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

use super::{DaemonState, default_brain_dir, versioned_root};
use crate::brain::BrainInstaller;

/// Minimum protocol version accepted from the oxibrain server.
pub const MIN_BRAIN_PROTOCOL_VERSION: u32 = 1;
/// Maximum protocol version accepted from the oxibrain server.
pub const MAX_BRAIN_PROTOCOL_VERSION: u32 = 2;

/// Identity advertised in the handshake hello (`ClientHello::client_version`).
const CLIENT_VERSION: &str = concat!("oxios ", env!("CARGO_PKG_VERSION"));

/// Brain handshake result. Carries the server's negotiated protocol version
/// so a future bump can give actionable diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainHandshake {
    pub state: DaemonState,
    /// Brain data directory the endpoint serves (the child's `--dir`).
    pub dir: PathBuf,
    /// Wire protocol version negotiated with the server; `None` when no
    /// handshake completed.
    pub protocol_version: Option<u32>,
    /// Server identity and version from the handshake, when it got that far.
    pub daemon_build: Option<String>,
}

/// Foundation bootstrap report. Returned from [`bootstrap`] so CLI
/// onboarding and `foundation status` can render the same shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapReport {
    pub foundation_dir: PathBuf,
    pub profiles_loaded: usize,
    pub brain: BrainHandshake,
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
    /// managed `~/.oxi/bin/oxibrain` location.
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

    let endpoint = brain_endpoint(&cfg.home, dir, cfg.installer.as_deref());
    let brain = handshake_brain_with(&endpoint, cfg.may_install, cfg.installer.clone()).await?;
    match brain.state {
        DaemonState::Compatible => info!(
            dir = %brain.dir.display(),
            "brain handshake ok"
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

/// Build the spawn endpoint for a brain data directory. Resolution follows
/// the installer (explicit config → managed root → `PATH`); with no
/// installer — or no binary locatable yet — the managed `~/.oxi/bin/oxibrain`
/// location is used so a post-install retry can succeed.
pub fn brain_endpoint(
    home: &Path,
    dir: PathBuf,
    installer: Option<&BrainInstaller>,
) -> oxibrain_client::LocalProcessEndpoint {
    let executable = installer
        .and_then(|i| i.locate_binary())
        .unwrap_or_else(|| home.join(".oxi").join("bin").join("oxibrain"));
    oxibrain_client::LocalProcessEndpoint::new(executable, dir)
}

/// How one handshake attempt failed. Drives both the classification and
/// whether an installer pass is worth attempting.
enum HandshakeFailure {
    /// The session child could not be spawned (missing binary, EACCES…).
    Spawn(anyhow::Error),
    /// The server answered but rejected our hello with a typed handshake
    /// error (protocol range, store format…).
    Rejected(oxibrain_client::HandshakeError),
    /// Transport failed after a successful spawn (child exited early, IO).
    Transport(anyhow::Error),
}

impl HandshakeFailure {
    /// Degrade the failure into a report. Logs once — bootstrap itself
    /// never errors out on a failed handshake (RFC-047).
    fn into_handshake(self, endpoint: &oxibrain_client::LocalProcessEndpoint) -> BrainHandshake {
        let state = match &self {
            HandshakeFailure::Spawn(e) => {
                warn!(
                    error = %e,
                    dir = %endpoint.dir.display(),
                    "brain spawn failed — degraded (RFC-047)"
                );
                DaemonState::Unavailable
            }
            HandshakeFailure::Rejected(typed) => {
                warn!(
                    error = %typed,
                    dir = %endpoint.dir.display(),
                    "brain rejected the handshake — incompatible"
                );
                DaemonState::Incompatible
            }
            HandshakeFailure::Transport(e) => {
                warn!(
                    error = %e,
                    dir = %endpoint.dir.display(),
                    "brain handshake transport failed — degraded (RFC-047)"
                );
                DaemonState::Unavailable
            }
        };
        BrainHandshake {
            state,
            dir: endpoint.dir.clone(),
            protocol_version: None,
            daemon_build: None,
        }
    }
}

/// Connect (spawn a short-lived session child), negotiate capabilities,
/// and classify. `may_install` + `installer` trigger one binary install
/// pass when the first attempt fails before completing a handshake; a
/// successful pass is retried exactly once. A typed rejection is never
/// retried — installing cannot fix a protocol mismatch.
pub async fn handshake_brain_with(
    endpoint: &oxibrain_client::LocalProcessEndpoint,
    may_install: bool,
    installer: Option<Arc<BrainInstaller>>,
) -> Result<BrainHandshake> {
    let failure = match attempt(endpoint).await {
        Ok(handshake) => return Ok(handshake),
        Err(failure) => failure,
    };
    if !matches!(failure, HandshakeFailure::Rejected(_))
        && may_install
        && let Some(installer) = &installer
        && installer.ensure_binary(true).await.is_some()
        && let Ok(handshake) = attempt(endpoint).await
    {
        return Ok(handshake);
    }
    Ok(failure.into_handshake(endpoint))
}

/// One spawn + bring-up attempt, with failures classified for the caller.
async fn attempt(
    endpoint: &oxibrain_client::LocalProcessEndpoint,
) -> Result<BrainHandshake, HandshakeFailure> {
    let mut client = oxibrain_client::BrainClient::spawn_local(endpoint.clone())
        .await
        .map_err(HandshakeFailure::Spawn)?;
    let caps = client
        .bring_up(None, oxibrain_client::default_client_hello(CLIENT_VERSION))
        .await
        .map_err(
            |e| match e.downcast_ref::<oxibrain_client::HandshakeError>() {
                Some(typed) => HandshakeFailure::Rejected(typed.clone()),
                None => HandshakeFailure::Transport(e),
            },
        )?;
    Ok(classify(endpoint, caps))
}

/// Classify negotiated capabilities against the accepted protocol range.
fn classify(
    endpoint: &oxibrain_client::LocalProcessEndpoint,
    caps: oxibrain_client::BrainCapabilities,
) -> BrainHandshake {
    let version = caps.protocol_version.0;
    let in_range = (MIN_BRAIN_PROTOCOL_VERSION..=MAX_BRAIN_PROTOCOL_VERSION).contains(&version);
    BrainHandshake {
        state: if in_range {
            DaemonState::Compatible
        } else {
            DaemonState::Incompatible
        },
        dir: endpoint.dir.clone(),
        protocol_version: Some(version),
        daemon_build: Some(format!("{} {}", caps.server_name, caps.server_version)),
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
/// Daemonless readiness is binary presence: `Compatible` means a spawnable
/// `oxibrain` exists at `binary`. The full protocol classification still
/// comes from [`handshake_brain_with`].
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

    /// Spawning an endpoint whose executable does not exist fails without
    /// any server involvement and degrades to `Unavailable`.
    #[tokio::test]
    async fn handshake_unavailable_when_binary_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let ep = oxibrain_client::LocalProcessEndpoint::new(
            tmp.path().join("missing-oxibrain"),
            tmp.path().join("brain"),
        );
        let hs = handshake_brain_with(&ep, false, None).await.unwrap();
        assert_eq!(hs.state, DaemonState::Unavailable);
        assert_eq!(hs.protocol_version, None);
        assert_eq!(hs.dir, tmp.path().join("brain"));
    }

    #[test]
    fn classify_marks_out_of_range_version_incompatible() {
        let ep = oxibrain_client::LocalProcessEndpoint::new("oxibrain", "/brain");
        let caps = oxibrain_client::BrainCapabilities {
            protocol_version: oxibrain_client::BrainProtocolVersion::new(
                MAX_BRAIN_PROTOCOL_VERSION + 1,
            ),
            store_format_version: 1,
            supported_operations: vec![],
            server_name: "stub".into(),
            server_version: "0".into(),
        };
        let hs = classify(&ep, caps);
        assert_eq!(hs.state, DaemonState::Incompatible);
        assert_eq!(hs.protocol_version, Some(MAX_BRAIN_PROTOCOL_VERSION + 1));
        assert_eq!(hs.daemon_build.as_deref(), Some("stub 0"));
    }

    #[cfg(unix)]
    fn write_stub(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A stub `oxibrain` shell script: counts spawns in `counter`, dies
    /// before answering on the first spawn, answers the handshake after.
    #[cfg(unix)]
    fn flaky_stub(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let stub = dir.join("oxibrain");
        let counter = dir.join("spawns");
        write_stub(
            &stub,
            &format!(
                "#!/bin/sh\n\
                 n=$(cat {counter} 2>/dev/null || echo 0)\n\
                 n=$((n+1))\n\
                 echo $n > {counter}\n\
                 if [ \"$n\" -eq 1 ]; then exit 1; fi\n\
                 read line\n\
                 echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"min_compatible\":1,\"max_compatible\":1,\"store_format_version\":1,\"supported_operations\":[],\"server_name\":\"stub\",\"server_version\":\"0.8.0-stub\"}}}}'\n",
                counter = counter.display()
            ),
        );
        (stub, counter)
    }

    /// The installer hook runs exactly once when `may_install` is set:
    /// `ensure_binary` resolves the (already present) stub without any
    /// network access, and the single retry completes the handshake —
    /// exactly two spawns in total.
    #[cfg(unix)]
    #[tokio::test]
    async fn handshake_retries_once_after_installer_hook() {
        let tmp = tempfile::tempdir().unwrap();
        let (stub, counter) = flaky_stub(tmp.path());
        let section = crate::config::BrainSection {
            binary_path: stub.display().to_string(),
            ..Default::default()
        };
        let installer = Arc::new(BrainInstaller::from_brain_section(tmp.path(), &section));
        let ep = oxibrain_client::LocalProcessEndpoint::new(stub, tmp.path().join("brain"));
        let hs = handshake_brain_with(&ep, true, Some(installer))
            .await
            .unwrap();
        assert_eq!(hs.state, DaemonState::Compatible);
        assert_eq!(hs.protocol_version, Some(1));
        assert_eq!(hs.daemon_build.as_deref(), Some("stub 0.8.0-stub"));
        assert_eq!(std::fs::read_to_string(&counter).unwrap().trim(), "2");
    }

    /// Without `may_install` there is no hook and no retry: one spawn,
    /// then `Unavailable`.
    #[cfg(unix)]
    #[tokio::test]
    async fn handshake_does_not_retry_without_may_install() {
        let tmp = tempfile::tempdir().unwrap();
        let (stub, counter) = flaky_stub(tmp.path());
        let section = crate::config::BrainSection {
            binary_path: stub.display().to_string(),
            ..Default::default()
        };
        let installer = Arc::new(BrainInstaller::from_brain_section(tmp.path(), &section));
        let ep = oxibrain_client::LocalProcessEndpoint::new(stub, tmp.path().join("brain"));
        let hs = handshake_brain_with(&ep, false, Some(installer))
            .await
            .unwrap();
        assert_eq!(hs.state, DaemonState::Unavailable);
        assert_eq!(std::fs::read_to_string(&counter).unwrap().trim(), "1");
    }

    /// A typed protocol rejection classifies `Incompatible` and is never
    /// retried — installing a binary cannot fix a protocol mismatch.
    #[cfg(unix)]
    #[tokio::test]
    async fn handshake_incompatible_on_typed_protocol_rejection() {
        let tmp = tempfile::tempdir().unwrap();
        let stub = tmp.path().join("oxibrain");
        let counter = tmp.path().join("spawns");
        write_stub(
            &stub,
            &format!(
                "#!/bin/sh\n\
                 echo 1 > {counter}\n\
                 read line\n\
                 echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{{\"code\":-32000,\"message\":\"incompatible protocol\",\"data\":{{\"kind\":\"incompatible_protocol\",\"requested\":1,\"min_compatible\":9,\"max_compatible\":9}}}}}}'\n",
                counter = counter.display()
            ),
        );
        let section = crate::config::BrainSection {
            binary_path: stub.display().to_string(),
            ..Default::default()
        };
        let installer = Arc::new(BrainInstaller::from_brain_section(tmp.path(), &section));
        let ep = oxibrain_client::LocalProcessEndpoint::new(stub, tmp.path().join("brain"));
        let hs = handshake_brain_with(&ep, true, Some(installer))
            .await
            .unwrap();
        assert_eq!(hs.state, DaemonState::Incompatible);
        assert_eq!(hs.protocol_version, None);
        assert_eq!(std::fs::read_to_string(&counter).unwrap().trim(), "1");
    }
}
