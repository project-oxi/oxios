//! Surface activation helpers.
//!
//! Re-exports the Surface trait from oxios-gateway and provides
//! the activation function used by the binary's `cmd_serve()`.

pub use oxios_gateway::{Surface, SurfaceContext};

use anyhow::Result;
use oxios_gateway::ActiveWebDist;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::kernel::Kernel;

/// Build the list of available surfaces.
#[allow(clippy::vec_init_then_push)]
pub fn build_surfaces() -> Vec<Box<dyn Surface>> {
    let mut surfaces: Vec<Box<dyn Surface>> = Vec::new();
    #[cfg(feature = "web")]
    surfaces.push(Box::new(crate::api::WebSurface::new()));
    #[cfg(feature = "remote")]
    surfaces.push(Box::new(crate::remote::RemoteRpcSurface::new()));
    surfaces
}

/// One activated surface and its background task handles.
pub struct ActivatedSurface {
    /// Surface name (e.g. "web").
    pub name: String,
    /// Background task handles spawned by the surface.
    pub tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// Activate all enabled surfaces.
///
/// Surfaces receive full kernel access. If a surface also returns a channel,
/// it is registered with the gateway for message routing. The shared `shutdown`
/// token (RFC-030 A5) is threaded into each surface's context so it wires its
/// graceful-shutdown path to a single signal source owned by the supervisor.
pub async fn activate_surfaces(
    kernel: &Kernel,
    config_path: &Path,
    web_dist: ActiveWebDist,
    shutdown: CancellationToken,
) -> Result<Vec<ActivatedSurface>> {
    let surfaces = build_surfaces();
    let config = kernel.config();
    let mut activated = Vec::new();

    // Read surface names from config — surfaces are listed separately from channels.
    let surface_names: Vec<String> = config
        .surfaces
        .as_ref()
        .map(|s| s.enabled.clone())
        .unwrap_or_else(|| {
            // Default: enable web if the feature is compiled in.
            #[cfg(feature = "web")]
            {
                vec!["web".to_string()]
            }
            #[cfg(not(feature = "web"))]
            {
                vec![]
            }
        });

    let surface_map: std::collections::HashMap<&str, &dyn Surface> =
        surfaces.iter().map(|s| (s.name(), s.as_ref())).collect();

    for name in &surface_names {
        match surface_map.get(name.as_str()) {
            Some(surface) => {
                let ctx = SurfaceContext {
                    kernel: kernel.handle(),
                    gateway: kernel.gateway(),
                    config: Arc::new(parking_lot::RwLock::new(config.clone())),
                    config_path: config_path.to_path_buf(),
                    web_dist: web_dist.clone(),
                    shutdown: shutdown.clone(),
                };
                match surface.start(ctx).await {
                    Ok(handle) => {
                        tracing::info!(surface = %name, "Surface activated");
                        if let Some(channel) = handle.channel
                            && let Err(e) = kernel.register_channel(channel).await
                        {
                            tracing::error!(surface = %name, error = %e, "Failed to register surface channel");
                        }
                        activated.push(ActivatedSurface {
                            name: name.clone(),
                            tasks: handle.tasks,
                        });
                    }
                    Err(e) => {
                        tracing::error!(surface = %name, error = %e, "Failed to activate surface")
                    }
                }
            }
            None => tracing::warn!(
                surface = %name,
                "Surface '{}' not available (not compiled in). Available: {}",
                name,
                surface_map.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        }
    }

    Ok(activated)
}
