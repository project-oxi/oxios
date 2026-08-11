//! Oxios-owned browse tools — direct `oxibrowser-core` 0.20 dependency.
//!
//! This module replaces the previous `oxicode-sdk` browser port dependency.
//! All types are oxios-owned; only the `AgentTool` trait and its associated
//! types (`BrowseProgress`, `ProgressCallback`) come from `oxicode_sdk`.

pub mod browse_extract_tool;
pub mod browse_script_tool;
pub mod browse_session_tool;
pub mod browse_tool;
pub mod callback;
pub mod config;
pub mod engine;
pub mod helpers;
pub mod tab_guard;
pub mod types;

// Re-exports for convenience
pub use browse_extract_tool::BrowseExtractTool;
pub use browse_script_tool::BrowseScriptTool;
pub use browse_session_tool::BrowseSessionTool;
pub use browse_tool::BrowseTool;
pub use callback::TabCallbackRegistry;
pub use config::BrowseConfig;
pub use engine::{OxiosBrowser, OxiosTab};
pub use types::{
    BrowserError, BrowseWaitCondition, ElementInfo, LinkInfo, Observation, ObservedElement,
    PageContent,
};
