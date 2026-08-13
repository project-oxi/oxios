//! Screenshot API — CSS-aware web page screenshot capture.
//!
//! Wraps [`oxibrowser_core::Browser`] (0.21) behind a lazily-initialized
//! engine. The browser performs full navigation — HTTP fetch, external
//! stylesheet loading, JS execution (incl. WASM 1.0 via wasmi ↔ boa bridge)
//! — then captures the live DOM through the integrated Blitz rendering
//! pipeline (Stylo CSS + Taffy layout + vello_cpu paint) to produce a
//! pixel-accurate PNG. Tab::print_to_pdf is also available.
//!
//! Independent of the SDK's browsing tools (oxicode-sdk 0.72 dropped its
//! browser re-exports). This engine uses oxibrowser-core directly for
//! CSS-quality screenshots.
//!
//! Only available with the `browser` (formerly `screenshot`) feature.

use std::sync::Arc;

/// Viewport dimensions for screenshot capture.
#[derive(Debug, Clone, Copy)]
pub struct ScreenshotViewport {
    /// Width in CSS pixels.
    pub width: u32,
    /// Height in CSS pixels (used for initial layout; `full_page` overrides).
    pub height: u32,
    /// Device pixel ratio.
    pub scale: f32,
}

impl Default for ScreenshotViewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            scale: 1.0,
        }
    }
}

/// CSS-aware screenshot capture engine.
///
/// Lazily creates an [`oxibrowser_core::Browser`] on first use and reuses
/// it for subsequent captures. Each capture opens a fresh tab, navigates
/// to the URL, screenshots, and closes the tab — so captures are isolated.
pub struct ScreenshotEngine {
    /// Lazily-initialized shared browser instance.
    browser: tokio::sync::OnceCell<Arc<oxibrowser_core::Browser>>,
}

impl Default for ScreenshotEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenshotEngine {
    /// Create a new engine. The browser is created on first [`capture`].
    ///
    /// [`capture`]: Self::capture
    pub fn new() -> Self {
        Self {
            browser: tokio::sync::OnceCell::new(),
        }
    }

    /// Lazily initialize and return the shared browser.
    async fn browser(&self) -> anyhow::Result<&Arc<oxibrowser_core::Browser>> {
        self.browser
            .get_or_try_init(|| async {
                let config = oxibrowser_core::config::BrowserConfig::default();
                let b = oxibrowser_core::Browser::new(config)
                    .await
                    .map_err(|e| anyhow::anyhow!("screenshot browser init failed: {e}"))?;
                Ok(Arc::new(b))
            })
            .await
    }

    /// Navigate to `url` and capture a CSS-rendered PNG screenshot.
    ///
    /// Performs full page load (HTTP fetch + external CSS + JS execution),
    /// then renders the live DOM through the Blitz pipeline. The tab is
    /// always closed — even on error — to prevent session leaks.
    ///
    /// # Arguments
    /// * `url` - Target URL (must be `http` or `https`)
    /// * `viewport` - Capture dimensions (width controls layout; height is
    ///   informational since screenshots are always full-page)
    ///
    /// # Errors
    /// Returns an error if the URL scheme is invalid, the browser cannot
    /// initialize, navigation times out (30s), or the render pipeline errors.
    pub async fn capture(
        &self,
        url: &str,
        viewport: ScreenshotViewport,
    ) -> anyhow::Result<Vec<u8>> {
        // Defense-in-depth: reject non-http(s) schemes. oxibrowser-core's
        // own SSRF filter (CIDR blocking, scheme-aware) handles the rest.
        let parsed =
            url::Url::parse(url).map_err(|e| anyhow::anyhow!("invalid URL '{url}': {e}"))?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => {
                return Err(anyhow::anyhow!(
                    "screenshot URL must be http or https, got '{other}'"
                ));
            }
        }

        let browser = self.browser().await?;

        let tab = browser
            .new_tab()
            .await
            .map_err(|e| anyhow::anyhow!("new_tab failed: {e}"))?;

        // Navigate + screenshot with a 30s timeout. The tab is closed in
        // ALL paths (success, error, timeout) to prevent session leaks.
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tab.goto(url)
                .await
                .map_err(|e| anyhow::anyhow!("navigation to {url} failed: {e}"))?;
            tab.screenshot(viewport.width)
                .await
                .map_err(|e| anyhow::anyhow!("screenshot capture failed: {e}"))
        })
        .await
        .map_err(|_| anyhow::anyhow!("screenshot timed out after 30s for {url}"));

        // Always close the tab — success or failure.
        let _ = tab.close().await;

        result?
    }
}

impl std::fmt::Debug for ScreenshotEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let initialized = self.browser.initialized();
        f.debug_struct("ScreenshotEngine")
            .field("browser_initialized", &initialized)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: capture example.com and verify a valid PNG is produced.
    ///
    /// Exercises the full pipeline: Browser init → navigate → Blitz CSS render
    /// -> PNG encode. Requires network access.
    #[tokio::test]
    async fn capture_example_dot_com_produces_valid_png() {
        let engine = ScreenshotEngine::new();
        let png = engine
            .capture("https://example.com", ScreenshotViewport::default())
            .await
            .expect("screenshot capture should succeed");

        // PNG magic header: 137 80 78 71 13 10 26 10
        assert!(
            png.len() > 100,
            "PNG should be non-trivial: {} bytes",
            png.len()
        );
        assert_eq!(
            &png[..8],
            &[137, 80, 78, 71, 13, 10, 26, 10],
            "valid PNG header"
        );
    }
}
