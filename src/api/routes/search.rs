//! Search & Browse API — direct web search and page browsing (no agent loop).
//!
//! Called by the Search & Browse Panel in the frontend. These endpoints
//! delegate to the same oxibrowser engine the agent tools use, but without
//! requiring an active agent session.
//!
//! - `POST /api/search` — web search (DDG/Wiki/Bing)
//! - `GET  /api/screenshot` — render a URL to a CSS-laid-out PNG

use std::sync::Arc;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::api::error::AppError;
use crate::api::server::AppState;

// ── Request / Response types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_engines")]
    pub engines: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_engines() -> String {
    "ddg,wiki".to_string()
}
fn default_limit() -> usize {
    10
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResultItem>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct SearchResultItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub engine: String,
}

#[derive(Debug, Deserialize)]
pub struct BrowseRequest {
    pub url: String,
    #[serde(default = "default_format")]
    #[expect(dead_code)]
    pub format: String,
}

fn default_format() -> String {
    "markdown".to_string()
}

#[derive(Debug, Serialize)]
pub struct BrowseResponse {
    pub url: String,
    pub title: String,
    pub markdown: String,
    pub status: u16,
    pub elapsed_ms: u64,
}

// ── Handlers ─────────────────────────────────────────────────────────────

/// `POST /api/search` — direct web search.
///
/// Calls `oxibrowser::search::dispatch()` — the same function the agent's
/// `web_search` tool uses. No agent loop involved.
pub(crate) async fn handle_search(
    _state: State<Arc<AppState>>,
    Json(body): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, AppError> {
    let start = std::time::Instant::now();

    let output = oxibrowser::search::dispatch(
        &body.query,
        "web",
        &body.engines,
        None, // repo
        None, // token
        body.limit,
        10, // timeout_secs
    )
    .await
    .map_err(|e| AppError::Internal(format!("search failed: {e}")))?;

    let elapsed = start.elapsed().as_millis() as u64;
    let results: Vec<SearchResultItem> = output
        .results
        .into_iter()
        .map(|r| SearchResultItem {
            title: r.title,
            url: r.url,
            snippet: r.snippet,
            engine: output.engine.clone(),
        })
        .collect();

    Ok(Json(SearchResponse {
        results,
        elapsed_ms: elapsed,
    }))
}

/// `POST /api/browse` — browse a URL and return its markdown content.
///
/// Uses the already-wired `BrowserApi` on the kernel handle (pure-Rust
/// oxibrowser-core engine). The `format` field is reserved for future
/// `"text"` or `"html"` variants; currently only `"markdown"` is supported.
pub(crate) async fn handle_browse(
    state: State<Arc<AppState>>,
    Json(body): Json<BrowseRequest>,
) -> Result<Json<BrowseResponse>, AppError> {
    let start = std::time::Instant::now();

    // Get the browser engine (requires `browser` feature)
    let browser = state
        .kernel
        .browser
        .as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("browser engine not available".into()))?;

    let engine = browser
        .engine()
        .await
        .map_err(|e| AppError::Internal(format!("browser init failed: {e}")))?;

    let tab = engine
        .new_tab()
        .await
        .map_err(|e| AppError::Internal(format!("browser tab create failed: {e}")))?;

    let page = tab
        .goto(&body.url)
        .await
        .map_err(|e| AppError::Internal(format!("browse failed: {e}")))?;

    let elapsed = start.elapsed().as_millis() as u64;

    Ok(Json(BrowseResponse {
        url: page.url,
        title: page.title,
        markdown: page.markdown,
        status: page.status,
        elapsed_ms: elapsed,
    }))
}

// ── Screenshot ──────────────────────────────────────────────────────────

/// Query parameters for `GET /api/screenshot`.
#[derive(Debug, Deserialize)]
pub struct ScreenshotQuery {
    /// URL to screenshot.
    pub url: String,
    /// Viewport width in CSS pixels (default 1280).
    pub w: Option<u32>,
    /// Viewport height in CSS pixels (default 800).
    pub h: Option<u32>,
}

/// `GET /api/screenshot?url=...&w=1280&h=800` — render a URL to PNG.
///
/// Performs full navigation (HTTP fetch + external CSS + JS execution),
/// then captures the live DOM through the Blitz CSS rendering pipeline.
/// Returns `image/png` binary body — use directly as `<img src>`.
///
/// Only available with the `screenshot` feature.
#[cfg(feature = "screenshot")]
pub(crate) async fn handle_screenshot(
    _state: State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<ScreenshotQuery>,
) -> Result<axum::response::Response, AppError> {
    use oxios_kernel::ScreenshotViewport;
    use std::sync::LazyLock;

    static ENGINE: LazyLock<std::sync::Arc<oxios_kernel::ScreenshotEngine>> =
        LazyLock::new(|| std::sync::Arc::new(oxios_kernel::ScreenshotEngine::new()));
    let engine = &*ENGINE;

    let viewport = ScreenshotViewport {
        width: q.w.unwrap_or(1280).clamp(320, 4096),
        height: q.h.unwrap_or(800).clamp(240, 4096),
        scale: 1.0,
    };

    let png = engine
        .capture(&q.url, viewport)
        .await
        .map_err(|e| AppError::Internal(format!("screenshot failed: {e}")))?;

    Ok(axum::response::Response::builder()
        .header("content-type", "image/png")
        .header("cache-control", "public, max-age=86400")
        .body(axum::body::Body::from(png))
        .unwrap())
}
