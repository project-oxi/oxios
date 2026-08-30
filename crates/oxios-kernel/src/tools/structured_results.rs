//! Tool→runtime side channel for structured tool payloads.
//!
//! The SDK strips `AgentToolResult::metadata` at the agent-loop event boundary
//! (`AgentEvent::ToolExecutionEnd` carries only the text `ToolResult`), so a
//! tool that wants to hand the Web UI something richer than its text summary
//! has nowhere to put it. This bus is that place: a tool inserts under its
//! `tool_call_id` right before returning, and the agent-runtime completion
//! callback takes the entry when publishing
//! [`crate::event_bus::KernelEvent::ToolExecutionFinished`], from where it
//! rides the WS `tool_end.results` field to the tool renderers.
//!
//! Consumers:
//!
//! - `web_search` / `get_search_results` — result arrays (title/url/snippet)
//!   rendered as search cards.
//! - `todo` — the phase list after the ops applied, rendered as a live
//!   checklist.
//! - `issue` — the affected issue after the action, rendered as an issue card.
//!
//! Was `search_provider::SearchResultBus`; generalized when `todo` became the
//! second consumer, rather than growing a second bus per tool family.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::Value;

/// Tool→runtime side channel for structured payloads, keyed by `tool_call_id`.
///
/// Lifecycle: a tool inserts right before returning; the agent-runtime
/// completion callback takes (and removes) the entry when publishing
/// `ToolExecutionFinished`. Unclaimed entries (e.g. a runtime crash between
/// the two points) are dropped on the next insert of the same id; payloads are
/// bounded by each producing tool.
#[derive(Default)]
pub struct StructuredResultBus {
    entries: Mutex<HashMap<String, Value>>,
}

impl std::fmt::Debug for StructuredResultBus {
    /// Reports the pending-entry count, never the payloads — tool results can
    /// carry user content and this type is embedded in `Debug` tool structs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructuredResultBus")
            .field("pending", &self.entries.lock().len())
            .finish()
    }
}

impl StructuredResultBus {
    /// Create an empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store the structured payload for a tool call.
    pub fn insert(&self, tool_call_id: impl Into<String>, payload: Value) {
        self.entries.lock().insert(tool_call_id.into(), payload);
    }

    /// Take (remove) the payload for a tool call, if present.
    pub fn take(&self, tool_call_id: &str) -> Option<Value> {
        self.entries.lock().remove(tool_call_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn take_removes_the_entry() {
        let bus = StructuredResultBus::new();
        bus.insert("call-1", json!({"a": 1}));
        assert_eq!(bus.take("call-1"), Some(json!({"a": 1})));
        assert_eq!(bus.take("call-1"), None);
    }

    #[test]
    fn entries_are_keyed_independently() {
        let bus = StructuredResultBus::new();
        bus.insert("call-1", json!("first"));
        bus.insert("call-2", json!("second"));
        assert_eq!(bus.take("call-2"), Some(json!("second")));
        assert_eq!(bus.take("call-1"), Some(json!("first")));
    }

    #[test]
    fn reinsert_replaces_an_unclaimed_entry() {
        let bus = StructuredResultBus::new();
        bus.insert("call-1", json!("stale"));
        bus.insert("call-1", json!("fresh"));
        assert_eq!(bus.take("call-1"), Some(json!("fresh")));
    }
}
