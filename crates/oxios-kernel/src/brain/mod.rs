//! Brain transport — one process per operation (oxibrain 0.10 agent-first
//! CLI contract).
//!
//! Five modules cooperate:
//! - [`config`] — `BrainConfig` (dir + space) and `resolved_brain_dir`.
//! - [`documents`] — seed the vault root into `documents.toml`.
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
pub mod documents;
pub mod installer;
pub mod oneshot;
pub mod session;
pub use config::{BrainConfig, resolved_brain_dir};
pub use documents::ensure_vault_root;
pub use installer::{ASSET_TAR, BrainInstaller, LEGACY_LAUNCHD_LABEL, RELEASES_LATEST_URL};
pub use oneshot::{embed_gap, run_cli};
pub use session::{BrainSession, Spawn, StdioChild};

/// Resolve the brain space with documented precedence.
///
/// 1. `~/.oxi/config.toml [vault].space` — ecosystem-wide override.
/// 2. `fallback` — the kernel's `BrainConfig::space` default.
///
/// Best-effort: missing/unreadable/malformed ecosystem config returns
/// the fallback. Never blocks the boot path.
pub fn resolve_space(home: &std::path::Path, fallback: &str) -> String {
    let path = home.join(".oxi").join("config.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return fallback.to_string();
    };
    #[derive(serde::Deserialize)]
    struct VaultSection {
        space: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Root {
        vault: Option<VaultSection>,
    }
    match toml::from_str::<Root>(&text) {
        Ok(r) => r
            .vault
            .and_then(|v| v.space)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.to_string()),
        Err(_) => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let cfg = BrainConfig::new("/data/brain", "work");
        assert_eq!(cfg.dir, std::path::PathBuf::from("/data/brain"));
        assert_eq!(cfg.space, "work");
    }

    #[test]
    fn resolve_space_returns_fallback_when_oxi_config_missing() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_oxi_config_unreadable() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        // Directory where a file is expected → read fails with IsADirectory.
        let cfg = oxi_dir.join("config.toml");
        std::fs::create_dir(&cfg).unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_oxi_config_malformed() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "this is not [valid toml").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_space_is_whitespace() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "[vault]\nspace = \"   \"\n").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_fallback_when_vault_table_absent() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "[some_other]\nkey = 1\n").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "personal");
    }

    #[test]
    fn resolve_space_returns_oxi_value_when_valid() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(oxi_dir.join("config.toml"), "[vault]\nspace = \"work\"\n").unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "work");
    }

    #[test]
    fn resolve_space_trims_whitespace_around_valid_value() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let oxi_dir = dir.path().join(".oxi");
        std::fs::create_dir_all(&oxi_dir).unwrap();
        std::fs::write(
            oxi_dir.join("config.toml"),
            "[vault]\nspace = \"  work  \"\n",
        )
        .unwrap();
        assert_eq!(resolve_space(dir.path(), "personal"), "work");
    }
}
