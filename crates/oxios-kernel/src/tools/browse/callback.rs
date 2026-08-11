//! Callback management for browser tools.
//!
//! `TabCallbackRegistry` routes per-tab events from the background drain task
//! to the correct tool callback. `BrowseCallbacks` eliminates per-tool
//! boilerplate for storing progress/browse callbacks.

use parking_lot::Mutex;
use std::collections::HashMap;

use oxicode_sdk::{BrowseProgress, BrowseProgressCallback, ProgressCallback};

// ── TabCallbackRegistry ─────────────────────────────────────────────────

/// Per-`tab_id` callback entry.
#[derive(Default)]
struct TabCallbacks {
    progress: Option<ProgressCallback>,
    browse: Option<BrowseProgressCallback>,
}

/// Per-`tab_id` callback registry for browser event routing.
///
/// Each tool invocation opens its own tab and registers a callback keyed by
/// the tab's `tab_id`. The engine's background event-drain task extracts
/// `tab_id` from each `BrowserEvent` and routes it to the correct callback.
pub struct TabCallbackRegistry {
    entries: Mutex<HashMap<uuid::Uuid, TabCallbacks>>,
}

impl Default for TabCallbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TabCallbackRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Register a string progress callback for `tab_id`.
    pub fn set(&self, tab_id: uuid::Uuid, cb: ProgressCallback) {
        self.entries.lock().entry(tab_id).or_default().progress = Some(cb);
    }

    /// Register a structured browse progress callback for `tab_id`.
    pub fn set_browse(&self, tab_id: uuid::Uuid, cb: BrowseProgressCallback) {
        self.entries.lock().entry(tab_id).or_default().browse = Some(cb);
    }

    /// Remove all callbacks for `tab_id`.
    pub fn clear(&self, tab_id: &uuid::Uuid) {
        self.entries.lock().remove(tab_id);
    }

    /// Invoke the string progress callback for `tab_id`, if registered.
    pub fn invoke(&self, tab_id: &uuid::Uuid, msg: String) {
        if let Some(entry) = self.entries.lock().get(tab_id)
            && let Some(ref cb) = entry.progress
        {
            cb(msg);
        }
    }

    /// Invoke the browse progress callback for `tab_id`, if registered.
    pub fn invoke_browse(&self, tab_id: &uuid::Uuid, progress: BrowseProgress) {
        if let Some(entry) = self.entries.lock().get(tab_id)
            && let Some(ref cb) = entry.browse
        {
            cb(progress);
        }
    }

    /// Whether a string callback is registered for `tab_id`.
    pub fn is_set(&self, tab_id: &uuid::Uuid) -> bool {
        self.entries.lock().contains_key(tab_id)
    }

    /// Number of registered tabs.
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    /// Returns `true` if no tabs have registered callbacks.
    pub fn is_empty(&self) -> bool {
        self.entries.lock().is_empty()
    }
}

// ── BrowseCallbacks ─────────────────────────────────────────────────────

/// Shared callback state for browser tools.
///
/// The agent loop calls `store_progress` / `store_browse` before `execute`;
/// the tool's `execute` calls `register_on_registry` to wire callbacks to
/// the actual tab via the engine's `TabCallbackRegistry`.
pub(crate) struct BrowseCallbacks {
    progress: Mutex<Option<ProgressCallback>>,
    browse: Mutex<Option<BrowseProgressCallback>>,
}

impl BrowseCallbacks {
    /// Create with no callbacks.
    pub fn new() -> Self {
        Self {
            progress: Mutex::new(None),
            browse: Mutex::new(None),
        }
    }

    /// Store a string progress callback (from `on_progress`).
    pub fn store_progress(&self, cb: ProgressCallback) {
        *self.progress.lock() = Some(cb);
    }

    /// Store a structured browse progress callback (from `on_browse_progress`).
    pub fn store_browse(&self, cb: BrowseProgressCallback) {
        *self.browse.lock() = Some(cb);
    }

    /// Register both pending callbacks on the engine's `TabCallbackRegistry`.
    pub fn register_on_registry(&self, tab_id: uuid::Uuid, registry: &TabCallbackRegistry) {
        if let Some(cb) = self.progress.lock().take() {
            registry.set(tab_id, cb);
        }
        if let Some(bcb) = self.browse.lock().take() {
            registry.set_browse(tab_id, bcb);
        }
    }

    /// Register browse callback on registry only, if pending.
    pub fn register_browse_on_registry(
        &self,
        tab_id: uuid::Uuid,
        registry: &TabCallbackRegistry,
    ) {
        if let Some(bcb) = self.browse.lock().take() {
            registry.set_browse(tab_id, bcb);
        }
    }

    /// Register progress callback on registry only, if pending.
    pub fn register_progress_on_registry(
        &self,
        tab_id: uuid::Uuid,
        registry: &TabCallbackRegistry,
    ) {
        if let Some(cb) = self.progress.lock().take() {
            registry.set(tab_id, cb);
        }
    }

    /// Take the pending progress callback without registering.
    pub fn take_progress(&self) -> Option<ProgressCallback> {
        self.progress.lock().take()
    }

    /// Register callbacks directly on a tab.
    pub fn register_on_tab(&self, tab: &super::engine::OxiosTab) {
        if let Some(cb) = self.progress.lock().take() {
            tab.set_progress_callback(cb);
        }
        if let Some(bcb) = self.browse.lock().take() {
            tab.set_browse_progress_callback(bcb);
        }
    }
}

impl Default for BrowseCallbacks {
    fn default() -> Self {
        Self::new()
    }
}
