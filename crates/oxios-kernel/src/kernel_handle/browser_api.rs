//! Browser API — headless browser engine facade (RFC-046: browser independence).
//!
//! Wraps the oxios-owned [`OxiosBrowser`] (concrete, direct `oxibrowser-core`
//! 0.20 dependency). The engine is lazily initialized on first use and shared
//! across every agent run that holds the same [`KernelHandle`].
//!
//! Only available with the `browser` feature. Without it, [`BrowserApi::try_engine`]
//! always returns `None` and no browse tools are registered.
//!
//! [`OxiosBrowser`]: crate::tools::browse::OxiosBrowser

use std::sync::Arc;

use crate::config::OxiosConfig;
use crate::tools::browse::{BrowseConfig, OxiosBrowser};

/// Headless browser facade.
///
/// Holds the [`BrowseConfig`] and a lazily-initialized engine cell.
pub struct BrowserApi {
    /// Lazily-initialized shared engine. `None` until first `engine().await`.
    #[cfg(feature = "browser")]
    engine: tokio::sync::OnceCell<Arc<OxiosBrowser>>,
    /// Engine configuration (propagated to the backend on init).
    config: BrowseConfig,
}

impl std::fmt::Debug for BrowserApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserApi")
            .field("config", &self.config)
            .finish()
    }
}

impl BrowserApi {
    /// Create a new browser facade with the given configuration.
    pub fn new(config: BrowseConfig) -> Self {
        Self {
            #[cfg(feature = "browser")]
            engine: tokio::sync::OnceCell::new(),
            config,
        }
    }

    /// Build a [`BrowserApi`] from the kernel config, honoring `[browser].enabled`.
    ///
    /// Returns `None` when browser integration is disabled.
    pub fn from_config(config: &OxiosConfig) -> Option<Self> {
        if config.browser.enabled {
            Some(Self::new(config.browser.engine.clone()))
        } else {
            None
        }
    }

    /// Lazily initialize and return the shared browser engine.
    ///
    /// The underlying `OxiosBrowser` (wrapping `oxibrowser-core`) is created
    /// exactly once; subsequent calls return the cached handle.
    ///
    /// Only available with the `browser` feature.
    #[cfg(feature = "browser")]
    pub async fn engine(&self) -> anyhow::Result<Arc<OxiosBrowser>> {
        self.engine
            .get_or_try_init(|| async {
                let backend = OxiosBrowser::with_config(self.config.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("browser engine init failed: {e}"))?;
                Ok(Arc::new(backend))
            })
            .await
            .map(Arc::clone)
    }

    /// Synchronous accessor — returns the engine only if already initialized.
    ///
    /// Used by the (synchronous) tool registration path, which relies on the
    /// agent runtime having awaited [`engine`](Self::engine) first.
    #[cfg(feature = "browser")]
    pub fn try_engine(&self) -> Option<Arc<OxiosBrowser>> {
        self.engine.get().cloned()
    }

    /// Without `browser`, no engine is ever available.
    #[cfg(not(feature = "browser"))]
    pub fn try_engine(&self) -> Option<Arc<OxiosBrowser>> {
        None
    }
}
