//! Surface: kernel-connected control interface.
//!
//! A Surface is a component that has direct access to the kernel
//! for management, monitoring, and configuration operations.
//! Unlike channels (which only pass messages through the gateway),
//! surfaces can read system state, modify configuration, manage agents,
//! and expose rich control-plane APIs.
//!
//! The web dashboard is the primary surface. Future surfaces could
//! include desktop apps, IDE plugins, or mobile control apps.
//!
//! # Why a separate trait?
//!
//! Channels implement [`ChannelPlugin`](crate::plugin::ChannelPlugin)
//! and receive only configuration — they are pure message relays.
//! Surfaces receive [`Arc<KernelHandle>`] because they need to inspect
//! and control the kernel directly.
//!
//! ```text
//! Channel (CLI, Telegram):
//!   user ↔ message ↔ gateway ↔ orchestrator
//!
//! Surface (Web, Desktop):
//!   user ↔ HTTP/UI ↔ KernelHandle  (control plane)
//!   user ↔ message ↔ gateway       (chat via optional channel)
//! ```

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::ActiveWebDist;
use crate::Channel;

/// Context provided to surfaces during initialization.
pub struct SurfaceContext {
    /// Full kernel subsystem handle.
    pub kernel: Arc<oxios_kernel::KernelHandle>,
    /// Hot-reloadable configuration.
    pub config: Arc<parking_lot::RwLock<oxios_kernel::OxiosConfig>>,
    /// Gateway handle — lets surfaces drive runtime channel operations
    /// (register/unregister/status) alongside kernel control.
    pub gateway: Arc<crate::Gateway>,
    /// Path to the config file.
    pub config_path: PathBuf,
    /// Pre-resolved web UI dist directory, published through an atomic
    /// pointer so updates can swap it without a 404 window (RFC-024 SP3).
    ///
    /// Surfaces should serve from `web_dist.path()` on every request. When
    /// the inner path is `None`, surfaces fall back to embedded assets.
    pub web_dist: ActiveWebDist,
    /// Shared cancellation token for the process lifecycle (RFC-030 A5).
    ///
    /// The supervisor owns the root token. Each surface instance receives a
    /// clone (or a child token on restart). Surfaces that run a server
    /// (e.g. the web dashboard's axum loop) should wire their graceful-shutdown
    /// path to `shutdown.cancelled()` rather than listening to ctrl_c
    /// themselves, so there is a single shutdown signal source.
    pub shutdown: CancellationToken,
}

/// Handle returned by a surface after initialization.
pub struct SurfaceHandle {
    /// Optional channel to register with the gateway.
    ///
    /// Surfaces that also handle message traffic (like the web dashboard's
    /// chat feature) return a channel here. Pure control surfaces return `None`.
    pub channel: Option<Box<dyn Channel>>,
    /// Background task handles.
    pub tasks: Vec<JoinHandle<()>>,
}

/// A kernel-connected control surface.
///
/// Implementors receive direct access to the kernel handle
/// for management and monitoring operations. They may optionally
/// also register a channel with the gateway for message routing.
#[async_trait]
pub trait Surface: Send + Sync {
    /// Unique name for this surface (e.g., "web").
    fn name(&self) -> &str;

    /// Initialize and start the surface.
    ///
    /// Returns a handle with an optional channel for gateway registration
    /// and background task handles for lifecycle management.
    async fn start(&self, ctx: SurfaceContext) -> Result<SurfaceHandle>;
}
