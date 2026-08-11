//! Browse data types — oxios-owned, independent of oxicode-sdk.
//!
//! These types are internal to the browse module. The `AgentTool` trait
//! integration uses `oxicode_sdk::{BrowseProgress, BrowseProgressCallback}`
//! (always compiled, tied to the trait), but all browse domain types live here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Errors that can occur during browser operations.
#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("navigation failed: {0}")]
    Navigation(String),
    #[error("element not found: {0}")]
    ElementNotFound(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("evaluation error: {0}")]
    Evaluation(String),
    #[error("screenshot failed: {0}")]
    Screenshot(String),
    #[error("tab closed: {0}")]
    TabClosed(String),
    #[error("browser error: {0}")]
    Backend(String),
    #[error("no active session — call 'open' first")]
    NoActiveSession,
}

/// Shared page content returned by `goto` and `content` methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContent {
    /// Final URL after redirects.
    pub url: String,
    /// Page title.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Rendered page content as markdown.
    pub markdown: String,
    /// Raw HTML body.
    #[serde(default)]
    pub html: String,
}

impl From<BrowserError> for String {
    fn from(e: BrowserError) -> Self {
        e.to_string()
    }
}

impl PageContent {
    /// Create an empty page (for mock / fallback).
    pub fn empty() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            status: 0,
            markdown: String::new(),
            html: String::new(),
        }
    }
}

/// Convert an `oxibrowser_core::BrowseResult` into [`PageContent`].
pub(crate) fn browse_result_to_page_content(page: oxibrowser_core::BrowseResult) -> PageContent {
    PageContent {
        url: page.url,
        title: page.title,
        status: page.status,
        markdown: page.markdown,
        html: page.html,
    }
}

/// A single link on a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkInfo {
    /// Link text.
    pub text: String,
    /// Link URL.
    pub href: String,
}

/// A single element matched by a CSS selector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementInfo {
    /// HTML tag name.
    pub tag: String,
    /// Trimmed text content.
    pub text: String,
    /// All HTML attributes.
    #[serde(default)]
    pub attributes: HashMap<String, String>,
}

/// Structured wait condition (mirrors `oxibrowser_core::tab::WaitCondition`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowseWaitCondition {
    /// A CSS selector matches at least one element in the current DOM.
    Visible(String),
    /// In-flight HTTP request counter has been zero for a quiet window.
    NetworkIdle,
    /// `DOMContentLoaded` boundary crossed.
    DomContentLoaded,
    /// `load` boundary crossed.
    Load,
}

/// One interactive element captured by [`OxiosTab::observe`].
///
/// No coordinates — the boa layout engine only approximates geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedElement {
    /// Stable id within this snapshot, e.g. `"e7"`.
    pub ref_id: String,
    /// ARIA-ish role derived from tag + `role` attr.
    pub role: String,
    /// Accessible name — `aria-label`, else trimmed text content.
    pub name: String,
    /// HTML tag name (lowercase).
    pub tag: String,
    /// CSS selector: `[data-oxios-ref="e7"]`.
    pub selector: String,
    /// Visible (display/visibility/opacity all pass).
    pub visible: bool,
    /// Interactive (not disabled, pointerEvents != none).
    pub interactive: bool,
}

/// The page's interactive surface, returned by [`OxiosTab::observe`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Final URL after redirects.
    pub url: String,
    /// Page `<title>`.
    pub title: String,
    /// Interactive, visible elements in DOM order.
    pub elements: Vec<ObservedElement>,
}
