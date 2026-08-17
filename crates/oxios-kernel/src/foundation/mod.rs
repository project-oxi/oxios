//! Oxi Foundation (RFC-048).
//!
//! A versioned filesystem contract at `~/.oxi/foundation/v1` that owns
//! non-secret profile metadata, OS Keychain credential locators, and an
//! immutable shared package lock. Foundation is **not** a provider proxy
//! and never shells out to an external worker — Oxios's executor stays the
//! in-process `oxicode_sdk::Oxicode`.
//!
//! Submodules:
//! - [`bootstrap`]: idempotent first-run setup (Brain discovery, profile load,
//!   shared directory creation).
//! - [`profile`]: schema-versioned profile registry parser with strict
//!   non-secret and Keychain-locator validation.
//! - [`packages`]: read-only shared package registry importer with
//!   digest/source/trust verification.
//! - [`migrate`]: explicit, user-invoked credential migration from legacy
//!   auth files into Keychain-backed profile locators.
//!
//! All submodules share the [`paths`] constants — there is exactly one
//! `~/.oxi/foundation/v1` root, exactly one `profiles.json`, exactly one
//! `packages.lock`.

#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod bootstrap;
pub mod migrate;
pub mod packages;
pub mod profile;
pub mod resolver;

/// Current schema version for the Foundation directory.
///
/// Bump on any breaking change to `profiles.json`, `packages.lock`, or the
/// directory layout. The bootstrap step refuses to write a registry with a
/// newer version than it understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Relative path of the profile registry inside the Foundation directory.
pub const PROFILES_FILE: &str = "profiles.json";

/// Relative path of the immutable shared package lock.
pub const PACKAGES_LOCK: &str = "packages.lock";

/// Directory holding the package archives referenced by the lock
/// (`<versioned>/packages/<id>.zip`).
pub const PACKAGES_DIR: &str = "packages";

/// Default directory name under the user home.
pub const FOUNDATION_DIR: &str = ".oxi/foundation";

/// Versioned root inside the Foundation directory.
pub const VERSIONED_DIR: &str = "v1";

/// Default Brain daemon socket directory (`~/.oxi/brain`).
pub const BRAIN_DIR: &str = ".oxi/brain";

/// Default Brain daemon socket filename.
pub const BRAIN_SOCKET: &str = "oxibrain.sock";

/// Resolve the Foundation directory for the given home.
///
/// `~/.oxi/foundation` by default. Test code can override via `home`.
pub fn foundation_root(home: &Path) -> PathBuf {
    home.join(FOUNDATION_DIR)
}

/// Resolve the versioned Foundation directory (`~/.oxi/foundation/v1`).
pub fn versioned_root(home: &Path) -> PathBuf {
    foundation_root(home).join(VERSIONED_DIR)
}

/// Resolve the package archive directory (`~/.oxi/foundation/v1/packages`).
pub fn packages_dir(home: &Path) -> PathBuf {
    versioned_root(home).join(PACKAGES_DIR)
}

/// Default Brain socket path used when no explicit override is provided.
pub fn default_brain_socket(home: &Path) -> PathBuf {
    home.join(BRAIN_DIR).join(BRAIN_SOCKET)
}

/// State of an external daemon (Brain) used by the bootstrap handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    /// Daemon responded to the version handshake and is compatible.
    Compatible,
    /// Daemon socket is not present / no process is listening.
    Unavailable,
    /// Daemon responded but its version is outside the supported range.
    Incompatible,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_match_rfc_spec() {
        let home = Path::new("/Users/example");
        assert_eq!(
            foundation_root(home),
            PathBuf::from("/Users/example/.oxi/foundation")
        );
        assert_eq!(
            versioned_root(home),
            PathBuf::from("/Users/example/.oxi/foundation/v1")
        );
        assert_eq!(
            default_brain_socket(home),
            PathBuf::from("/Users/example/.oxi/brain/oxibrain.sock")
        );
    }

    #[test]
    fn schema_version_is_v1() {
        assert_eq!(SCHEMA_VERSION, 1);
    }
}
