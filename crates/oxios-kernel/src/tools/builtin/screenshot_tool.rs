//! Screenshot tool — CSS-aware web page screenshot capture.
//!
//! Exposes [`ScreenshotEngine`] to the agent tool-calling loop. The agent
//! provides a URL and viewport; the tool navigates, renders the live DOM
//! through the Blitz CSS pipeline, saves the PNG, and returns the path.
//!
//! Only available with the `screenshot` feature.

use std::sync::Arc;

use async_trait::async_trait;
use oxicode_agent::{AgentTool, AgentToolResult, ToolContext};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::kernel_handle::{ScreenshotEngine, ScreenshotViewport};

/// Agent tool for capturing CSS-rendered screenshots of web pages.
pub struct ScreenshotTool {
    engine: Arc<ScreenshotEngine>,
}

impl ScreenshotTool {
    /// Create from a shared [`ScreenshotEngine`].
    pub fn new(engine: Arc<ScreenshotEngine>) -> Self {
        Self { engine }
    }

    /// Create from a [`KernelHandle`], instantiating a fresh engine.
    pub fn from_kernel(_kernel: &crate::KernelHandle) -> Self {
        Self::new(Arc::new(ScreenshotEngine::new()))
    }
}

#[async_trait]
impl AgentTool for ScreenshotTool {
    fn name(&self) -> &str {
        "browse_screenshot"
    }

    fn label(&self) -> &str {
        "Screenshot"
    }

    fn description(&self) -> &str {
        "Capture a CSS-rendered screenshot of a web page. The page is fully \
         loaded (stylesheets, JavaScript) and rendered through a real CSS \
         layout engine to produce a pixel-accurate PNG. Returns the path to \
         the saved image file."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to screenshot"
                },
                "width": {
                    "type": "number",
                    "description": "Viewport width in pixels (default 1280)",
                    "default": 1280
                },
                "height": {
                    "type": "number",
                    "description": "Viewport height in pixels (default 800)",
                    "default": 800
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        params: Value,
        _signal: Option<oneshot::Receiver<()>>,
        _ctx: &ToolContext,
    ) -> Result<AgentToolResult, String> {
        let url = params["url"]
            .as_str()
            .ok_or("missing required parameter: url")?;

        let width = params["width"].as_u64().unwrap_or(1280) as u32;
        let height = params["height"].as_u64().unwrap_or(800) as u32;

        let viewport = ScreenshotViewport {
            width: width.clamp(320, 4096),
            height: height.clamp(240, 4096),
            scale: 1.0,
        };

        tracing::info!(url, width = viewport.width, "capturing screenshot");

        let png = self
            .engine
            .capture(url, viewport)
            .await
            .map_err(|e| format!("Screenshot failed: {e}"))?;

        // Save to cache dir so the agent can reference the file.
        let cache_dir = dirs::cache_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("oxios")
            .join("screenshots");

        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| format!("cannot create screenshot dir: {e}"))?;

        // Short hash of URL + size for a unique filename.
        let mut hasher = blake3::Hasher::new();
        hasher.update(url.as_bytes());
        hasher.update(&png.len().to_le_bytes());
        let hash = hasher.finalize().to_hex();
        let filename = format!("{}.png", &hash[..16]);
        let filepath = cache_dir.join(&filename);

        std::fs::write(&filepath, &png)
            .map_err(|e| format!("cannot write screenshot file: {e}"))?;

        tracing::info!(
            path = %filepath.display(),
            bytes = png.len(),
            "screenshot saved"
        );

        Ok(AgentToolResult::success(format!(
            "Screenshot saved to {} ({} bytes, {}x{} viewport)",
            filepath.display(),
            png.len(),
            viewport.width,
            viewport.height
        ))
        .with_metadata(serde_json::json!({
            "path": filepath.display().to_string(),
            "bytes": png.len(),
            "url": url,
        })))
    }
}
