//! Brain session configuration (oxibrain 0.8 daemonless).

use std::path::{Path, PathBuf};

/// Resolved brain session parameters.
#[derive(Debug, Clone)]
pub struct BrainConfig {
    /// Brain data directory the session child serves (`--dir`).
    pub dir: PathBuf,
}

impl BrainConfig {
    /// Create a new config.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

/// Resolve the brain data directory: explicit config value, else
/// `$OXI_BRAIN_DIR`, else the Oxi Foundation default `~/.oxi/brain`.
pub fn resolved_brain_dir(home: &Path, configured: &str) -> PathBuf {
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    if let Ok(env) = std::env::var("OXI_BRAIN_DIR")
        && !env.is_empty()
    {
        return PathBuf::from(env);
    }
    home.join(".oxi").join("brain")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_brain_dir_explicit_config_wins() {
        let home = PathBuf::from("/Users/alice");
        assert_eq!(
            resolved_brain_dir(&home, "/data/brain"),
            PathBuf::from("/data/brain")
        );
    }

    #[test]
    fn resolved_brain_dir_env_then_default() {
        let home = PathBuf::from("/Users/alice");
        // No config, no env → default.
        unsafe { std::env::remove_var("OXI_BRAIN_DIR") };
        assert_eq!(
            resolved_brain_dir(&home, ""),
            PathBuf::from("/Users/alice/.oxi/brain")
        );
        // Env honored when config empty.
        unsafe { std::env::set_var("OXI_BRAIN_DIR", "/env/brain") };
        assert_eq!(resolved_brain_dir(&home, ""), PathBuf::from("/env/brain"));
        unsafe { std::env::remove_var("OXI_BRAIN_DIR") };
    }
}
