//! Concrete browser engine — wraps `oxibrowser-core` directly.
//!
//! No trait abstraction, no dyn dispatch. There is exactly one backend
//! (`oxibrowser-core` 0.21), so concrete types are simpler and faster.
//! 0.21 adds Tab::print_to_pdf + WebAssembly support (wasmi ↔ boa bridge)
//! and the BrowserEvent::PdfExported lifecycle event.
//!
//! `OxiosBrowser` manages a background event-drain task that routes
//! `BrowserEvent`s to per-tab callbacks via `TabCallbackRegistry`.
//! `OxiosTab` delegates every method to the underlying `oxibrowser_core::Tab`.

use std::sync::Arc;

use oxicode_sdk::{BrowseProgress, BrowseProgressCallback, ProgressCallback};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::callback::TabCallbackRegistry;
use super::config::BrowseConfig;
use super::helpers;
use super::types::{BrowseWaitCondition, BrowserError, Observation, browse_result_to_page_content};

// ── Event conversion helpers ────────────────────────────────────────────

/// Extract the `tab_id` from any `BrowserEvent` variant.
fn extract_event_tab_id(event: &oxibrowser_core::BrowserEvent) -> uuid::Uuid {
    use oxibrowser_core::BrowserEvent::*;
    match event {
        NavigationStarted { tab_id, .. }
        | WaitingForSelector { tab_id, .. }
        | DocumentReady { tab_id, .. }
        | ScreenshotCaptured { tab_id, .. }
        | PdfExported { tab_id, .. } => *tab_id,
        _ => uuid::Uuid::nil(),
    }
}

/// Convert an `oxibrowser_core::BrowserEvent` into a `BrowseProgress`.
fn browse_progress_from_event(event: &oxibrowser_core::BrowserEvent) -> Option<BrowseProgress> {
    use oxibrowser_core::BrowserEvent::*;
    match event {
        NavigationStarted { url, .. } => {
            Some(BrowseProgress::NavigationStarted { url: url.clone() })
        }
        WaitingForSelector {
            selector,
            timeout_ms,
            ..
        } => Some(BrowseProgress::WaitingForSelector {
            selector: selector.clone(),
            timeout_ms: *timeout_ms,
        }),
        DocumentReady {
            final_url,
            title,
            status,
            total_bytes,
            total_duration,
            ..
        } => Some(BrowseProgress::DocumentReady {
            url: final_url.clone(),
            title: title.clone(),
            status: *status,
            bytes: *total_bytes,
            duration_ms: total_duration.as_millis() as u64,
        }),
        ScreenshotCaptured {
            bytes,
            viewport_width,
            duration,
            ..
        } => Some(BrowseProgress::ScreenshotCaptured {
            bytes: *bytes,
            width: *viewport_width,
            duration_ms: duration.as_millis() as u64,
        }),
        PdfExported {
            bytes,
            viewport_width,
            duration,
            ..
        } => Some(BrowseProgress::PdfExported {
            bytes: *bytes,
            width: *viewport_width,
            duration_ms: duration.as_millis() as u64,
        }),
        _ => None,
    }
}

// ── OxiosBrowser ────────────────────────────────────────────────────────

/// Browser engine powered by `oxibrowser-core` 0.21.
///
/// Spins a background task that drains the browser's event stream and invokes
/// whatever callback is registered in the `TabCallbackRegistry` for the
/// event's `tab_id`. The task exits gracefully when the browser is dropped.
pub struct OxiosBrowser {
    browser: oxibrowser_core::Browser,
    config: BrowseConfig,
    progress: Arc<TabCallbackRegistry>,
    event_task: Mutex<Option<JoinHandle<()>>>,
}

impl OxiosBrowser {
    /// Create a new engine with custom config.
    ///
    /// Propagates `BrowseConfig` fields (user_agent, obey_robots, js_timeout_ms)
    /// to the underlying `oxibrowser-core` `BrowserConfig`.
    pub async fn with_config(config: BrowseConfig) -> Result<Self, BrowserError> {
        let mut browser_config = oxibrowser_core::BrowserConfig::headless();

        if let Some(ref ua) = config.user_agent {
            browser_config.user_agent = ua.clone();
        }
        browser_config.obey_robots = config.obey_robots;
        browser_config.js_timeout_ms = config.js_timeout_ms;

        let browser = oxibrowser_core::Browser::new(browser_config)
            .await
            .map_err(|e| BrowserError::Backend(format!("Failed to create browser: {}", e)))?;

        let progress = Arc::new(TabCallbackRegistry::new());
        let mut events_rx = browser.subscribe_events();
        let progress_clone = Arc::clone(&progress);
        let event_task = tokio::spawn(async move {
            loop {
                match events_rx.recv().await {
                    Ok(event) => {
                        let tab_id = extract_event_tab_id(&event);
                        if let Some(bp) = browse_progress_from_event(&event) {
                            progress_clone.invoke_browse(&tab_id, bp);
                        }
                        progress_clone.invoke(&tab_id, event.short_label());
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::debug!(
                            skipped = skipped,
                            "oxibrowser event subscriber lagged; some events were dropped"
                        );
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });

        Ok(Self {
            browser,
            config,
            progress,
            event_task: Mutex::new(Some(event_task)),
        })
    }

    /// Create with default config.
    pub async fn new() -> Result<Self, BrowserError> {
        Self::with_config(BrowseConfig::default()).await
    }

    /// Return the shared config.
    pub fn config(&self) -> &BrowseConfig {
        &self.config
    }

    /// Access the callback registry (for tool registration).
    pub fn callback_registry(&self) -> Arc<TabCallbackRegistry> {
        Arc::clone(&self.progress)
    }

    /// Access the underlying browser (for screenshot engine sharing).
    pub fn browser(&self) -> &oxibrowser_core::Browser {
        &self.browser
    }

    /// Open a new browser tab.
    pub async fn new_tab(&self) -> Result<OxiosTab, BrowserError> {
        let tab = self
            .browser
            .new_tab()
            .await
            .map_err(|e| BrowserError::Backend(format!("Failed to create tab: {}", e)))?;
        let tab_id = tab.tab_id();
        Ok(OxiosTab {
            inner: tab,
            config: self.config.clone(),
            tab_id,
            registry: Arc::clone(&self.progress),
        })
    }

    /// Close all tabs and shut down.
    pub async fn close(&self) -> Result<(), BrowserError> {
        self.browser
            .close()
            .await
            .map_err(|e| BrowserError::Backend(format!("Browser close failed: {}", e)))?;

        if let Some(handle) = self.event_task.lock().await.take() {
            handle.abort();
            let _ = handle.await;
        }
        Ok(())
    }

    /// Returns `true` if the browser is still alive.
    pub fn is_alive(&self) -> bool {
        self.browser.is_open()
    }
}

impl std::fmt::Debug for OxiosBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OxiosBrowser")
            .field("config", &self.config)
            .field("alive", &self.browser.is_open())
            .finish()
    }
}

// ── OxiosTab ────────────────────────────────────────────────────────────

/// A single browser tab backed by `oxibrowser-core`.
///
/// All methods delegate directly to the underlying `oxibrowser_core::Tab`,
/// converting `BrowseResult` → `PageContent` where needed.
pub struct OxiosTab {
    inner: oxibrowser_core::Tab,
    config: BrowseConfig,
    tab_id: uuid::Uuid,
    registry: Arc<TabCallbackRegistry>,
}

impl OxiosTab {
    /// Register a progress callback for this tab.
    pub fn set_progress_callback(&self, cb: ProgressCallback) {
        self.registry.set(self.tab_id, cb);
    }

    /// Remove the progress callback for this tab.
    pub fn clear_progress_callback(&self) {
        self.registry.clear(&self.tab_id);
    }

    /// Register a structured browse progress callback for this tab.
    pub fn set_browse_progress_callback(&self, cb: BrowseProgressCallback) {
        self.registry.set_browse(self.tab_id, cb);
    }

    /// Return this tab's stable ID.
    pub fn tab_id(&self) -> uuid::Uuid {
        self.tab_id
    }

    // ── Page navigation ──────────────────────────────────────────────────

    pub async fn goto(&self, url: &str) -> Result<super::types::PageContent, BrowserError> {
        let page = self
            .inner
            .goto(url)
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;
        Ok(browse_result_to_page_content(page))
    }

    pub async fn content(&self) -> Result<super::types::PageContent, BrowserError> {
        let page = self
            .inner
            .content()
            .await
            .map_err(|e| BrowserError::Backend(e.to_string()))?;
        Ok(browse_result_to_page_content(page))
    }

    pub async fn back(&self) -> Result<super::types::PageContent, BrowserError> {
        let page = self
            .inner
            .back()
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;
        Ok(browse_result_to_page_content(page))
    }

    pub async fn forward(&self) -> Result<super::types::PageContent, BrowserError> {
        let page = self
            .inner
            .forward()
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;
        Ok(browse_result_to_page_content(page))
    }

    pub async fn reload(&self) -> Result<super::types::PageContent, BrowserError> {
        let page = self
            .inner
            .reload()
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;
        Ok(browse_result_to_page_content(page))
    }

    // ── Interaction ──────────────────────────────────────────────────────

    pub async fn click(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .click(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn type_(&self, selector: &str, text: &str) -> Result<(), BrowserError> {
        self.inner
            .r#type(selector, text)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn fill(&self, selector: &str, value: &str) -> Result<(), BrowserError> {
        self.inner
            .fill(selector, value)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn press(&self, combo: &str) -> Result<(), BrowserError> {
        self.inner
            .press(combo)
            .await
            .map_err(|e| BrowserError::Evaluation(e.to_string()))
    }

    pub async fn wait_for(&self, selector: &str, timeout_ms: u64) -> Result<(), BrowserError> {
        self.inner
            .wait_for(selector, timeout_ms)
            .await
            .map_err(|e| BrowserError::Timeout(e.to_string()))
    }

    pub async fn wait_for_condition(
        &self,
        cond: &BrowseWaitCondition,
        timeout_ms: u64,
    ) -> Result<(), BrowserError> {
        let mapped = match cond {
            BrowseWaitCondition::Visible(s) => {
                oxibrowser_core::tab::WaitCondition::Visible(s.clone())
            }
            BrowseWaitCondition::NetworkIdle => oxibrowser_core::tab::WaitCondition::NetworkIdle,
            BrowseWaitCondition::DomContentLoaded => {
                oxibrowser_core::tab::WaitCondition::DomContentLoaded
            }
            BrowseWaitCondition::Load => oxibrowser_core::tab::WaitCondition::Load,
        };
        self.inner
            .wait_for_condition(mapped, timeout_ms)
            .await
            .map_err(|e| BrowserError::Timeout(e.to_string()))
    }

    pub async fn observe(&self) -> Result<Observation, BrowserError> {
        let page = self
            .inner
            .content()
            .await
            .map_err(|e| BrowserError::Backend(e.to_string()))?;
        let value = self
            .inner
            .evaluate(helpers::JS_OBSERVE)
            .await
            .map_err(|e| BrowserError::Evaluation(e.to_string()))?;
        Ok(Observation {
            url: page.url,
            title: page.title,
            elements: helpers::parse_observed_elements(value),
        })
    }

    pub async fn query_all(&self, selector: &str) -> Result<Vec<String>, BrowserError> {
        self.inner
            .query_all(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn evaluate(&self, js: &str) -> Result<Value, BrowserError> {
        self.inner
            .evaluate(js)
            .await
            .map_err(|e| BrowserError::Evaluation(e.to_string()))
    }

    pub async fn evaluate_await(&self, js: &str) -> Result<Value, BrowserError> {
        self.inner
            .evaluate_await(js)
            .await
            .map_err(|e| BrowserError::Evaluation(e.to_string()))
    }

    pub async fn screenshot(&self, width: u32) -> Result<Vec<u8>, BrowserError> {
        self.inner
            .screenshot(width)
            .await
            .map_err(|e| BrowserError::Screenshot(e.to_string()))
    }

    pub async fn close(&self) -> Result<(), BrowserError> {
        self.inner
            .close()
            .await
            .map_err(|e| BrowserError::TabClosed(e.to_string()))
    }

    // ── Form interaction ─────────────────────────────────────────────────

    pub async fn select_option(&self, selector: &str, value: &str) -> Result<(), BrowserError> {
        self.inner
            .select_option(selector, value)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn check(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .check(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn uncheck(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .uncheck(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn clear(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .clear_input(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn hover(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .hover(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn double_click(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .double_click(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn right_click(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .right_click(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn scroll(&self, delta_x: f64, delta_y: f64) -> Result<(), BrowserError> {
        self.inner
            .scroll(delta_x, delta_y)
            .await
            .map_err(|e| BrowserError::Evaluation(e.to_string()))
    }

    pub async fn scroll_into_view(&self, selector: &str) -> Result<(), BrowserError> {
        self.inner
            .scroll_into_view(selector, true)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn drag(&self, from_selector: &str, to_selector: &str) -> Result<(), BrowserError> {
        self.inner
            .drag(from_selector, to_selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn upload_file(&self, selector: &str, path: &str) -> Result<(), BrowserError> {
        self.inner
            .upload_file(selector, path)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    pub async fn get_value(&self, selector: &str) -> Result<String, BrowserError> {
        self.inner
            .get_value(selector)
            .await
            .map_err(|e| BrowserError::ElementNotFound(e.to_string()))
    }

    // ── Status ───────────────────────────────────────────────────────────

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Return the config reference.
    pub fn config(&self) -> &BrowseConfig {
        &self.config
    }
}
