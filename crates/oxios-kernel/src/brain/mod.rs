//! Brain transport — one process per operation (oxibrain 0.10 agent-first
//! CLI contract).
//!
//! Four modules cooperate:
//! - [`config`] — `BrainConfig` (dir) and `resolved_brain_dir`.
//! - [`installer`] — `BrainInstaller`: locate/install the `oxibrain` binary.
//! - [`oneshot`] — one-shot admin CLI invocations (`admin index`, `admin
//!   extract --pending`).
//! - [`session`] — `BrainSession`: agent ops as one-shot `oxibrain <op>`
//!   subprocesses (stdin payload, response envelope); console-only native
//!   RPCs over a `serve --stdio` child spawned per call and reaped at once.
//!
//! Every operation degrades gracefully: absent binary or dead child
//! returns `None`/empty and the agent turn completes normally
//! (RFC-047 §4 / RFC-049 daemonless degradation contract).

pub mod config;
pub mod installer;
pub mod oneshot;
pub mod session;
pub use config::{BrainConfig, resolved_brain_dir};
pub use installer::{ASSET_TAR, BrainInstaller, LEGACY_LAUNCHD_LABEL, RELEASES_LATEST_URL};
pub use oneshot::{embed_gap, run_cli};
pub use session::{BrainSession, Spawn, StdioChild};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let cfg = BrainConfig::new("/data/brain");
        assert_eq!(cfg.dir, std::path::PathBuf::from("/data/brain"));
    }
}
