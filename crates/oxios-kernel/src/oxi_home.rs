//! Canonical Oxi installation paths for oxios-owned state.
//!
//! Two roots, one layout (unified-home contract):
//!
//! * [`oxi_home`] — the shared Oxi ecosystem root (`$OXI_HOME` or
//!   `$HOME/.oxi`): vault, shared config, per-app subtrees.
//! * [`oxios_home`] — the oxios-private subtree under it
//!   (`$OXIOS_HOME` override or `oxi_home()/oxios`).
//!
//! Installs that predate the unified layout kept their state at
//! [`legacy_home_dir`] (`$HOME/.oxios`). Legacy data is READ-ONLY
//! compatibility: per-item reads may fall back to it
//! ([`effective_config_path`]) and the journaled migration
//! ([`crate::home_migrate`]) copies it forward — writes always target
//! the canonical paths. An explicit `$OXIOS_HOME` never merges with
//! the legacy home.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Pure core: prefer the env value, else the derived fallback. Models
/// the `OXI_HOME`/`OXIOS_HOME` "override wins" precedence without
/// touching process env, so tests can pin it hermetically.
fn env_or(env: Option<&OsStr>, fallback: PathBuf) -> PathBuf {
    env.map(PathBuf::from).unwrap_or(fallback)
}

/// Pure core: the unified ecosystem root for a given home directory.
fn oxi_home_from(home: &Path) -> PathBuf {
    home.join(".oxi")
}

/// Pure core: the oxios subtree for a given unified root.
fn oxios_home_from(oxi_root: &Path) -> PathBuf {
    oxi_root.join("oxios")
}

/// Pure core: the legacy pre-unification oxios home for a home
/// directory — surfaced only while it exists on disk.
fn legacy_home_dir_from(home: &Path) -> Option<PathBuf> {
    let legacy = home.join(".oxios");
    legacy.exists().then_some(legacy)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve the shared Oxi home (`OXI_HOME` or `$HOME/.oxi`).
pub fn oxi_home() -> PathBuf {
    env_or(
        std::env::var_os("OXI_HOME").as_deref(),
        oxi_home_from(&home_dir()),
    )
}

/// Resolve the oxios-private subtree. `OXIOS_HOME` remains an explicit
/// override for tests and portable deployments.
pub fn oxios_home() -> PathBuf {
    env_or(
        std::env::var_os("OXIOS_HOME").as_deref(),
        oxios_home_from(&oxi_home()),
    )
}

/// The legacy pre-unification oxios home (`$HOME/.oxios`), present only
/// while it exists on disk. Read-only compatibility: per-item reads may
/// fall back to it and the migration copies from it — nothing ever
/// writes here.
pub fn legacy_home_dir() -> Option<PathBuf> {
    legacy_home_dir_from(&home_dir())
}

/// Canonical-first, legacy read-only fallback for config discovery.
///
/// Returns `canonical` when it exists on disk. Otherwise, when the
/// legacy home carries a `config.toml`, returns that path — callers
/// MUST treat the result as read-only; every write targets the
/// canonical path. When neither exists, `canonical` is returned so
/// fresh-install defaulting is unchanged.
pub fn effective_config_path(canonical: &Path) -> PathBuf {
    effective_config_path_in(canonical, legacy_home_dir().as_deref())
}

/// Injectable core of [`effective_config_path`].
fn effective_config_path_in(canonical: &Path, legacy_home: Option<&Path>) -> PathBuf {
    if canonical.exists() {
        return canonical.to_path_buf();
    }
    if let Some(legacy) = legacy_home {
        let candidate = legacy.join("config.toml");
        if candidate.exists() {
            return candidate;
        }
    }
    canonical.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oxios-oxi-home-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp home");
        dir
    }

    #[test]
    fn oxios_subtree_nests_under_unified_root() {
        let home = Path::new("/tmp/oxi");
        assert_eq!(oxi_home_from(home), Path::new("/tmp/oxi/.oxi"));
        assert_eq!(
            oxios_home_from(&oxi_home_from(home)),
            Path::new("/tmp/oxi/.oxi/oxios")
        );
    }

    #[test]
    fn explicit_env_override_wins_over_derived_default() {
        // env_or models both OXI_HOME and OXIOS_HOME precedence: the
        // override is used verbatim when present, the derived default
        // (`$HOME/.oxi` → `$HOME/.oxi/oxios`) otherwise.
        assert_eq!(
            env_or(Some(OsStr::new("/custom")), PathBuf::from("/derived")),
            PathBuf::from("/custom")
        );
        assert_eq!(
            env_or(None, PathBuf::from("/derived")),
            PathBuf::from("/derived")
        );
    }

    #[test]
    fn legacy_home_probed_only_when_present() {
        let home = tmp("legacy-absent");
        assert_eq!(legacy_home_dir_from(&home), None);

        std::fs::create_dir_all(home.join(".oxios")).expect("mkdir legacy");
        assert_eq!(
            legacy_home_dir_from(&home),
            Some(home.join(".oxios")),
            "legacy home surfaces only once it exists"
        );
    }

    #[test]
    fn effective_config_prefers_canonical_then_legacy_then_canonical() {
        let home = tmp("cfg-fallback");
        let legacy = home.join(".oxios");
        std::fs::create_dir_all(&legacy).expect("mkdir legacy");
        let canonical = home.join(".oxi").join("oxios").join("config.toml");

        // Neither file exists → canonical (fresh-install default).
        assert_eq!(
            effective_config_path_in(&canonical, Some(&legacy)),
            canonical
        );

        // Canonical missing + legacy present → legacy (read-only source).
        std::fs::write(legacy.join("config.toml"), "legacy = true").expect("write legacy config");
        assert_eq!(
            effective_config_path_in(&canonical, Some(&legacy)),
            legacy.join("config.toml")
        );

        // Canonical present → canonical, always.
        std::fs::create_dir_all(canonical.parent().expect("parent")).expect("mkdir canonical");
        std::fs::write(&canonical, "canonical = true").expect("write canonical config");
        assert_eq!(
            effective_config_path_in(&canonical, Some(&legacy)),
            canonical
        );
    }
}
