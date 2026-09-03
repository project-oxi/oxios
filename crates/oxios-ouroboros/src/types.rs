//! Shared types used across the intent handling pipeline and kernel.

use serde::{Deserialize, Serialize};

/// Record of a single tool call during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Tool name (e.g. "read", "bash", "grep").
    pub tool: String,
    /// Input parameters or invocation summary.
    pub input: String,
    /// Output or result summary.
    pub output: String,
    /// Duration of the tool call in milliseconds.
    pub duration_ms: u64,
    /// Whether the tool call returned an error.
    #[serde(default)]
    pub is_error: bool,
    /// Provider-specific tool call ID for start/end correlation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,
    /// Timestamp when the tool call started (UTC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

/// Result of executing a directive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The output produced by execution.
    pub output: String,
    /// Number of steps completed during execution.
    pub steps_completed: usize,
    /// Whether execution completed successfully.
    pub success: bool,
    /// Tool calls recorded during execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRecord>,
    /// Total input tokens consumed during execution.
    #[serde(default)]
    pub tokens_input: u64,
    /// Total output tokens generated during execution.
    #[serde(default)]
    pub tokens_output: u64,
    /// Model ID used for this execution.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_id: String,
    /// Failure classification (RFC-029). `Some` when the run failed with a
    /// classifiable provider/infra error; `None` on success, cancellation,
    /// abort, or unclassified failure. P2's RecoveryCoordinator reads this
    /// to choose the recovery strategy.
    ///
    /// `#[serde(default, skip_serializing_if)]` keeps existing JSON payloads
    /// (without the field) round-tripping cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<crate::resilience::FailureClass>,
    /// Agent conversation state captured at the point of failure (RFC-029
    /// P2b). `Some` when the agent had accumulated state before a provider
    /// failure — allows the RecoveryCoordinator to restore into a new
    /// agent (with a different model) and continue rather than restarting.
    ///
    /// Serialized from `Agent::export_state()`. `None` on success, or when
    /// the failure occurred before any state was accumulated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_state: Option<serde_json::Value>,

    /// P4 (§7 persistence): full concatenated reasoning text from
    /// `AgentEvent::ThinkingDelta { text }`, capped at ~4 KB at runtime.
    /// Surfaced in the terminal OutgoingMessage metadata so chat.rs can
    /// persist it alongside `tool_calls` and restore on session reopen.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning_text: String,
    /// Block-stream transparency (2026-07-27): positioned reasoning spans,
    /// interleaved with tool calls by `before_step` on reopen. Empty for
    /// pre-block-stream runs (frontend falls back to `reasoning_text`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_segments: Vec<ReasoningSegment>,
    /// Execution recipe selected for this turn (execution-recipe design).
    /// `Some("coding-omp-v1")` for coding-recipe turns; `None` for
    /// `general-v1` (the default composition) so existing payloads stay
    /// round-trip clean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
}

/// One positioned reasoning span within a turn (block-stream transparency).
///
/// `before_step` is the number of tool calls that STARTED in this turn before
/// this span — used to interleave reasoning with tools when rendering a
/// reopened session. A trailing span (after the last tool) has `before_step`
/// equal to the turn's tool count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReasoningSegment {
    #[serde(default)]
    /// Number of tool calls that started in this turn before this span.
    pub before_step: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    /// The reasoning text for this span (may be appended to across deltas).
    pub text: String,
}

/// Single option for a structured interview question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewOptionOutput {
    /// Stable identifier for the answer payload (e.g. "points", "ko").
    pub value: String,
    /// Human-readable label rendered as a chip/button.
    pub label: String,
    /// Optional longer description shown as a tooltip.
    #[serde(default)]
    pub description: String,
}

/// One structured question produced by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterviewQuestionOutput {
    /// Short identifier used as the answer key (e.g. "q1", "q2").
    pub id: String,
    /// The question text (also present in the parallel `questions` array).
    pub text: String,
    /// Question kind — drives the frontend widget selection.
    #[serde(default = "default_question_kind")]
    pub kind: String,
    /// Choice options (empty for free_text / yes_no).
    #[serde(default)]
    pub options: Vec<InterviewOptionOutput>,
}

fn default_question_kind() -> String {
    "free_text".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // ToolCallRecord — JSON shape and skip-if-empty behavior
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_record_serialization_omits_empty_optional_fields() {
        let r = ToolCallRecord {
            tool: "bash".into(),
            input: "ls".into(),
            output: "file.txt".into(),
            duration_ms: 42,
            is_error: false,
            tool_call_id: String::new(), // empty → skip
            timestamp: None,             // None → skip
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("tool_call_id"));
        assert!(!json.contains("timestamp"));
        assert!(json.contains("\"tool\":\"bash\""));
        assert!(json.contains("\"duration_ms\":42"));
    }

    #[test]
    fn tool_call_record_round_trips_with_all_fields() {
        let r = ToolCallRecord {
            tool: "read".into(),
            input: "src/lib.rs".into(),
            output: "fn main(){}".into(),
            duration_ms: 7,
            is_error: false,
            tool_call_id: "call-abc".into(),
            timestamp: Some(chrono::Utc::now()),
        };
        let json = serde_json::to_value(&r).unwrap();
        let back: ToolCallRecord = serde_json::from_value(json).unwrap();
        assert_eq!(back.tool, "read");
        assert_eq!(back.tool_call_id, "call-abc");
        assert!(back.timestamp.is_some());
    }

    // -----------------------------------------------------------------------
    // ExecutionResult — default fields and skip-if-empty
    // -----------------------------------------------------------------------

    #[test]
    fn execution_result_serialization_omits_default_optional_fields() {
        // No tool calls, no failure, no restore_state, no reasoning, no model_id
        // → all should be skipped. token counts stay (they're not Option).
        let r = ExecutionResult {
            output: "ok".into(),
            steps_completed: 3,
            success: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("failure_class"));
        assert!(!json.contains("restore_state"));
        assert!(!json.contains("reasoning_text"));
        assert!(!json.contains("model_id"));
        assert!(json.contains("\"output\":\"ok\""));
        assert!(json.contains("\"steps_completed\":3"));
        assert!(json.contains("\"success\":true"));
    }

    // -----------------------------------------------------------------------
    // InterviewQuestionOutput — default question kind
    // -----------------------------------------------------------------------

    #[test]
    fn interview_question_kind_defaults_to_free_text_when_omitted() {
        let json = r#"{"id":"q1","text":"name?","options":[]}"#;
        let q: InterviewQuestionOutput = serde_json::from_str(json).unwrap();
        assert_eq!(q.kind, "free_text");
    }

    #[test]
    fn interview_question_preserves_explicit_kind() {
        let json = r#"{"id":"q1","text":"pick","kind":"single_select","options":[]}"#;
        let q: InterviewQuestionOutput = serde_json::from_str(json).unwrap();
        assert_eq!(q.kind, "single_select");
    }

    #[test]
    fn interview_option_output_serialization_roundtrips() {
        let opt = InterviewOptionOutput {
            value: "yes".into(),
            label: "Yes, do it".into(),
            description: "proceed with the change".into(),
        };
        let json = serde_json::to_value(&opt).unwrap();
        let back: InterviewOptionOutput = serde_json::from_value(json).unwrap();
        assert_eq!(back.value, "yes");
        assert_eq!(back.label, "Yes, do it");
        assert_eq!(back.description, "proceed with the change");
    }
}
