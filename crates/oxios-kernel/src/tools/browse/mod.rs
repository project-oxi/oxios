//! Oxios-owned browse tools — direct `oxibrowser-core` 0.21 dependency.
//!
//! `config` and `types` are always compiled (no oxibrowser-core dependency)
//! so that `BrowseConfig` and the data types remain available for the
//! kernel config system regardless of the `browser` feature.
//!
//! Everything that touches `oxibrowser-core` (engine, helpers, tab_guard,
//! callback, and the 4 agent tools) is behind `#[cfg(feature = "browser")]`.

// ── Always compiled (no oxibrowser-core dependency) ─────────────────────
pub mod config;
pub mod types;

// ── Feature-gated (requires oxibrowser-core) ────────────────────────────
#[cfg(feature = "browser")]
pub mod browse_extract_tool;
#[cfg(feature = "browser")]
pub mod browse_script_tool;
#[cfg(feature = "browser")]
pub mod browse_session_tool;
#[cfg(feature = "browser")]
pub mod browse_tool;
#[cfg(feature = "browser")]
pub mod callback;
#[cfg(feature = "browser")]
pub mod engine;
#[cfg(feature = "browser")]
pub mod helpers;
#[cfg(feature = "browser")]
pub mod tab_guard;

// ── Always-available re-exports ─────────────────────────────────────────
pub use config::BrowseConfig;
pub use types::{
    BrowseWaitCondition, BrowserError, ElementInfo, LinkInfo, Observation, ObservedElement,
    PageContent,
};

// ── Feature-gated re-exports ────────────────────────────────────────────
#[cfg(feature = "browser")]
pub use browse_extract_tool::BrowseExtractTool;
#[cfg(feature = "browser")]
pub use browse_script_tool::BrowseScriptTool;
#[cfg(feature = "browser")]
pub use browse_session_tool::BrowseSessionTool;
#[cfg(feature = "browser")]
pub use browse_tool::BrowseTool;
#[cfg(feature = "browser")]
pub use callback::TabCallbackRegistry;
#[cfg(feature = "browser")]
pub use engine::{OxiosBrowser, OxiosTab};
