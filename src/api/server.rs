//! Web application state for the dashboard channel.
//!
//! `AppState` is the shared state accessible to all route handlers.
//! The server lifecycle is managed by [`WebPlugin`](crate::api::plugin::WebPlugin).

use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::api::bridge::WebBridgeHandle;
use crate::api::error::AppError;
use crate::api::middleware::RateLimiter;
use oxios_gateway::ActiveWebDist;
use oxios_kernel::{KernelHandle, OxiosConfig, config};

/// Shared application state for the web dashboard.
///
/// All subsystems are accessible through `state.kernel.xxx()`.
#[derive(Clone)]
pub struct AppState {
    /// Base URL for API responses.
    pub base_url: String,
    /// Handle to the kernel subsystem (provides access to all kernel components).
    pub kernel: Arc<KernelHandle>,
    /// Handle to the web channel for message passing.
    pub bridge: WebBridgeHandle,
    /// Loaded configuration (hot-reloadable via RwLock).
    pub config: Arc<RwLock<OxiosConfig>>,
    /// Path to the config file (for persistence on PUT /api/config).
    pub config_path: PathBuf,
    /// Server start time (for uptime calculation).
    pub start_time: Instant,
    /// Rate limiter for API endpoints.
    #[allow(dead_code)]
    pub rate_limiter: RateLimiter,
    /// Atomic handle to the active web-dist directory (RFC-024 SP3).
    /// Serves from `web_dist.path()` on every request; updates swap the
    /// pointer atomically so no request ever sees a half-populated dist.
    pub web_dist: ActiveWebDist,
    /// RFC-024 SP4: subsystem readiness gate. The readiness middleware
    /// RFC-024 SP4: subsystem readiness gate. The readiness middleware
    /// returns 503 (with `Retry-After`) when the gate is not yet open.
    pub readiness: std::sync::Arc<oxios_kernel::ReadinessGate>,
    /// Task store (RFC-043) — SQLite-backed task lifecycle management.
    pub task_store: std::sync::Arc<tokio::sync::Mutex<oxios_kernel::task::TaskStore>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("base_url", &self.base_url)
            .field("kernel", &"...")
            .field("channel", &"...")
            .field("config", &"...")
            .field("config_path", &self.config_path)
            .finish()
    }
}

impl AppState {
    /// Reload config from disk and update in-memory state.
    #[allow(dead_code)]
    pub async fn reload_config(&self) -> Result<(), AppError> {
        let config = config::load_config(&self.config_path)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        *self.config.write() = config;

        tracing::info!("Config hot-reloaded from {}", self.config_path.display());
        Ok(())
    }
}
