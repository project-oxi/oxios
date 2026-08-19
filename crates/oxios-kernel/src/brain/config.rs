//! Connection configuration for the oxibrain daemon (RFC-047).

use std::path::{Path, PathBuf};

/// How oxios reaches the oxibrain daemon.
#[derive(Debug, Clone)]
pub struct BrainConfig {
    /// Unix-domain socket path the daemon listens on.
    pub socket_path: PathBuf,
    /// Space to operate in (default `"personal"`).
    pub space: String,
}

impl BrainConfig {
    /// Create a new config.
    pub fn new(socket_path: impl Into<PathBuf>, space: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
            space: space.into(),
        }
    }
}

/// Resolve the daemon socket path: explicit config value, else the Oxi
/// Foundation default `~/.oxi/brain/oxibrain.sock`.
pub fn resolved_socket_path(home: &Path, configured: &str) -> PathBuf {
    if configured.is_empty() {
        home.join(".oxi").join("brain").join("oxibrain.sock")
    } else {
        PathBuf::from(configured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_socket_path_defaults_to_oxi_brain_sock() {
        let home = PathBuf::from("/Users/alice");
        assert_eq!(
            resolved_socket_path(&home, ""),
            PathBuf::from("/Users/alice/.oxi/brain/oxibrain.sock")
        );
    }

    #[test]
    fn resolved_socket_path_uses_explicit_value() {
        let home = PathBuf::from("/Users/alice");
        assert_eq!(
            resolved_socket_path(&home, "/var/run/oxibrain.sock"),
            PathBuf::from("/var/run/oxibrain.sock")
        );
    }
}
