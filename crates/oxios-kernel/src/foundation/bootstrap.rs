//! Idempotent Foundation bootstrap (RFC-048 §2).
//!
//! The first-run path is:
//!   1. Ensure `~/.oxi/foundation/v1` exists.
//!   2. Verify or install a compatible `oxibrain` daemon.
//!   3. Handshake with the daemon and classify it as
//!      [`DaemonState::Compatible`] / [`DaemonState::Unavailable`] /
//!      [`DaemonState::Incompatible`].
//!   4. Report the result. Never write the lockfile from a turn — the lock
//!      file is owned by Foundation imports, not by agent execution.

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

use super::{DaemonState, default_brain_socket, versioned_root};

/// Optional hook invoked once when the first connect attempt fails, before
/// the retry. Lets the bootstrap step delegate to a host-specific daemon
/// starter (e.g. the `BrainSupervisor`) without coupling the Foundation
/// module to the kernel's install machinery.
pub type Starter = Arc<dyn Fn(&Path) -> BoxFuture<'static, ()> + Send + Sync>;

/// Minimum protocol version accepted from the oxibrain daemon.
pub const MIN_BRAIN_PROTOCOL_VERSION: u32 = 1;
/// Maximum protocol version accepted from the oxibrain daemon.
pub const MAX_BRAIN_PROTOCOL_VERSION: u32 = 2;

/// Brain handshake result. Carries the daemon's reported protocol version
/// so a future bump can give actionable diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainHandshake {
    pub state: DaemonState,
    pub socket_path: PathBuf,
    pub protocol_version: Option<u32>,
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

/// Configuration for a single bootstrap run. The `socket_path` mirrors
/// `BrainSection::socket_path`; when empty, the Foundation default is used.
#[derive(Clone)]
pub struct BootstrapConfig {
    pub home: PathBuf,
    pub socket_path: Option<PathBuf>,
    /// When `true`, the bootstrap step is allowed to (idempotently) start
    /// a missing compatible daemon. `false` (default) reports
    /// `Unavailable` and lets the user run `foundation bootstrap`
    /// explicitly.
    pub may_start_daemon: bool,
    /// Optional hook invoked exactly once when the first connect attempt
    /// fails, before the retry. Hosts that want a self-managed daemon
    /// wire this to their supervisor. `None` (default) leaves the
    /// handshake in its two-attempt read-only shape.
    pub starter: Option<Starter>,
}

impl std::fmt::Debug for BootstrapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapConfig")
            .field("home", &self.home)
            .field("socket_path", &self.socket_path)
            .field("may_start_daemon", &self.may_start_daemon)
            .field("starter", &self.starter.as_ref().map(|_| "Some(<starter>)"))
            .finish()
    }
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            home: dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")),
            socket_path: None,
            may_start_daemon: false,
            starter: None,
        }
    }
}

/// Run the idempotent bootstrap. Never panics; reports every step.
pub async fn bootstrap(cfg: &BootstrapConfig) -> Result<BootstrapReport> {
    let foundation_dir = versioned_root(&cfg.home);
    ensure_directory(&foundation_dir)
        .with_context(|| format!("create foundation directory {}", foundation_dir.display()))?;
    let socket = cfg
        .socket_path
        .clone()
        .unwrap_or_else(|| default_brain_socket(&cfg.home));

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

    let brain = handshake_brain_with(&socket, cfg.may_start_daemon, cfg.starter.clone()).await?;
    match brain.state {
        DaemonState::Compatible => info!(
            socket = %socket.display(),
            "brain daemon handshake ok"
        ),
        DaemonState::Unavailable => warn!(
            socket = %socket.display(),
            "brain daemon unavailable — degraded mode is expected (RFC-047)"
        ),
        DaemonState::Incompatible => warn!(
            socket = %socket.display(),
            "brain daemon protocol incompatible — install a compatible release"
        ),
    }

    Ok(BootstrapReport {
        foundation_dir,
        profiles_loaded: 0, // populated by the resolver; bootstrap only writes the dir.
        brain,
        idempotent: !fresh,
    })
}

/// Perform the Brain handshake and classify the daemon.
pub async fn handshake_brain(socket: &Path, may_start: bool) -> Result<BrainHandshake> {
    handshake_brain_with(socket, may_start, None).await
}

/// Connect to the daemon, optionally call `starter` once on first failure,
/// retry the connect once, then classify via [`finish_handshake`]. This is
/// the testable core of the bootstrap handshake.
///
/// When `may_start` is `false` OR `starter` is `None`, the function is
/// read-only: a single connect attempt is made and any failure is reported
/// as `Unavailable`.
pub async fn handshake_brain_with(
    socket: &Path,
    may_start: bool,
    starter: Option<Starter>,
) -> Result<BrainHandshake> {
    use oxibrain_client::BrainClient;

    let client = match BrainClient::connect(socket).await {
        Ok(c) => c,
        Err(e) => {
            if !(may_start && starter.is_some()) {
                if may_start {
                    warn!(
                        socket = %socket.display(),
                        error = %e,
                        "may_start_daemon=true but no starter is wired — reporting unavailable"
                    );
                }
                return Ok(BrainHandshake {
                    state: DaemonState::Unavailable,
                    socket_path: socket.to_path_buf(),
                    protocol_version: None,
                    daemon_build: None,
                });
            }
            // Invoke the (host-supplied) daemon starter exactly once, then
            // retry the connect. If the retry still fails the daemon is
            // genuinely missing — surface Unavailable.
            let starter = starter.expect("checked is_some above");
            tracing::info!(
                socket = %socket.display(),
                "first connect failed — invoking starter before retry"
            );
            starter(socket).await;
            match BrainClient::connect(socket).await {
                Ok(c) => c,
                Err(e2) => {
                    warn!(
                        socket = %socket.display(),
                        error = %e2,
                        "starter ran but daemon still unreachable — reporting unavailable"
                    );
                    return Ok(BrainHandshake {
                        state: DaemonState::Unavailable,
                        socket_path: socket.to_path_buf(),
                        protocol_version: None,
                        daemon_build: None,
                    });
                }
            }
        }
    };

    finish_handshake(client, socket).await
}

/// Classify an already-connected daemon via a `ping`. `ping` returns Ok on
/// a compatible daemon; on Err we report `Incompatible`. The function
/// emits no warnings — callers wrap into their own context.
async fn finish_handshake(
    mut client: oxibrain_client::BrainClient,
    socket: &Path,
) -> Result<BrainHandshake> {
    // `ping` returns Ok on a compatible daemon. There is no explicit
    // version handshake in oxibrain-client 0.2 yet; we treat any
    // successful handshake as Compatible and reserve Incompatible for a
    // future protocol version mismatch field.
    match client.ping().await {
        Ok(()) => Ok(BrainHandshake {
            state: DaemonState::Compatible,
            socket_path: socket.to_path_buf(),
            protocol_version: Some(1),
            daemon_build: None,
        }),
        Err(_) => Ok(BrainHandshake {
            state: DaemonState::Incompatible,
            socket_path: socket.to_path_buf(),
            protocol_version: None,
            daemon_build: None,
        }),
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

/// Quick non-async readiness probe used by `foundation status`. Does not
/// open a connection — just inspects the socket path and returns
/// [`DaemonState::Unavailable`] when nothing is listening.
pub async fn quick_probe(socket: &Path) -> DaemonState {
    use oxibrain_client::BrainClient;
    match BrainClient::connect(socket).await {
        Ok(mut c) => match c.ping().await {
            Ok(()) => DaemonState::Compatible,
            Err(_) => DaemonState::Incompatible,
        },
        Err(_) => DaemonState::Unavailable,
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

    /// When a connection fails and a `starter` is provided, the starter
    /// must be invoked exactly once and the second connect attempt must
    /// be made (still failing here → `Unavailable`). TDD guard for the
    /// foundation `Starter` hook.
    #[tokio::test]
    async fn starter_is_invoked_on_connect_failure_and_retried_once() {
        let tmp = tempfile::tempdir().unwrap();
        let sock = tmp.path().join("no.sock");
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_hook = Arc::clone(&calls);
        let starter: Starter = Arc::new(move |_path| {
            let calls = Arc::clone(&calls_hook);
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
        });
        let hs = handshake_brain_with(&sock, true, Some(starter))
            .await
            .unwrap();
        assert_eq!(hs.state, DaemonState::Unavailable);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
