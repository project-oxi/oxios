//! Connection configuration for the oxibrain daemon (RFC-047).

use std::path::PathBuf;

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
