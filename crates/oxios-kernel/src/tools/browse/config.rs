//! Configuration for browser tools behavior.
//!
//! Copied from the SDK's browse module — all tunable parameters centralized
//! so no values are hardcoded.

use serde::{Deserialize, Serialize};

/// Configuration for browser tools behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowseConfig {
    /// Default `wait_for` timeout in milliseconds.
    #[serde(default = "default_wait_timeout_ms")]
    pub default_wait_timeout_ms: u64,

    /// Default page load timeout in seconds.
    #[serde(default = "default_page_timeout_secs")]
    pub page_timeout_secs: u64,

    /// Screenshot width in pixels.
    #[serde(default = "default_screenshot_width")]
    pub screenshot_width: u32,

    /// Maximum script steps per execution.
    #[serde(default = "default_max_script_steps")]
    pub max_script_steps: usize,

    /// Render cache TTL in seconds (0 = disabled).
    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,

    /// Maximum render cache entries.
    #[serde(default = "default_cache_max_entries")]
    pub cache_max_entries: usize,

    /// Maximum concurrent tabs.
    #[serde(default = "default_max_concurrent_tabs")]
    pub max_concurrent_tabs: usize,

    /// Maximum output size in bytes (truncation threshold).
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,

    /// Maximum idle time (seconds) before a browse session auto-closes.
    /// 0 = no timeout.
    #[serde(default = "default_session_idle_timeout_secs")]
    pub session_idle_timeout_secs: u64,

    /// Custom User-Agent string. `None` uses the browser default.
    #[serde(default)]
    pub user_agent: Option<String>,

    /// Whether to respect robots.txt. Defaults to `true`.
    #[serde(default = "default_obey_robots")]
    pub obey_robots: bool,

    /// JavaScript evaluation timeout in milliseconds.
    #[serde(default = "default_js_timeout_ms")]
    pub js_timeout_ms: u64,
}

impl Default for BrowseConfig {
    fn default() -> Self {
        Self {
            default_wait_timeout_ms: default_wait_timeout_ms(),
            page_timeout_secs: default_page_timeout_secs(),
            screenshot_width: default_screenshot_width(),
            max_script_steps: default_max_script_steps(),
            cache_ttl_secs: default_cache_ttl_secs(),
            cache_max_entries: default_cache_max_entries(),
            max_concurrent_tabs: default_max_concurrent_tabs(),
            max_output_bytes: default_max_output_bytes(),
            session_idle_timeout_secs: default_session_idle_timeout_secs(),
            user_agent: None,
            obey_robots: default_obey_robots(),
            js_timeout_ms: default_js_timeout_ms(),
        }
    }
}

fn default_wait_timeout_ms() -> u64 {
    10_000
}
fn default_page_timeout_secs() -> u64 {
    30
}
fn default_screenshot_width() -> u32 {
    800
}
fn default_max_script_steps() -> usize {
    100
}
fn default_cache_ttl_secs() -> u64 {
    300
}
fn default_cache_max_entries() -> usize {
    50
}
fn default_max_concurrent_tabs() -> usize {
    4
}
fn default_max_output_bytes() -> usize {
    512_000
}
fn default_session_idle_timeout_secs() -> u64 {
    300 // 5 minutes
}
fn default_obey_robots() -> bool {
    true
}
fn default_js_timeout_ms() -> u64 {
    10_000
}
