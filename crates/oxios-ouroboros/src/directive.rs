//! Directive: the task specification for the agent.
//!
//! A Directive is built either from the raw user message (Trivial tasks)
//! or from a crystallize LLM call (Substantial tasks). It carries everything
//! the agent needs to execute and (optionally) what the reviewer checks against.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What the agent should do. Built from either the raw message (Trivial)
/// or a crystallize LLM call (Substantial).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Directive {
    /// The goal the agent aims to achieve.
    pub goal: String,

    /// The user's original message, preserved verbatim for language fidelity
    /// and exact filenames/paths.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub original_request: String,

    /// Constraints the agent must respect during execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,

    /// Verifiable acceptance criteria. Injected into the agent's system prompt
    /// AND checked by the review pass. Empty = no review (Trivial).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,

    /// Optional JSON Schema for structured output validation.
    /// When set, the reviewer checks JSON conformance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

impl Directive {
    /// Create a lightweight directive from a user message verbatim.
    /// Used for Trivial tasks where the message is sufficient as-is.
    pub fn from_message(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self {
            goal: msg.clone(),
            original_request: msg,
            constraints: Vec::new(),
            acceptance_criteria: Vec::new(),
            output_schema: None,
        }
    }

    /// Whether this directive has criteria worth reviewing.
    pub fn needs_review(&self) -> bool {
        !self.acceptance_criteria.is_empty() || self.output_schema.is_some()
    }
}

/// The execution environment. Resolved by the orchestrator independently
/// of the task directive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecEnv {
    /// Rendered workspace context (active Mounts, project instructions, relevant memories).
    pub workspace_context: Option<String>,

    /// Resolved filesystem paths from active Mounts. paths[0] of the primary
    /// Mount is the CWD; every path is added to the agent's allowed_paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mount_paths: Vec<PathBuf>,

    /// Project ID detected by the orchestrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<uuid::Uuid>,

    /// Hint for the capability system (CSpace template name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cspace_hint: Option<String>,

    /// Model override for resilience recovery (RFC-029 P2).
    ///
    /// When `Some`, the agent runtime uses this model instead of the
    /// engine default. Set by the `RecoveryCoordinator` when retrying a
    /// failed directive with a fallback model. `None` for normal runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,

    /// Per-message model params (temperature / max_tokens). When set, the
    /// agent runtime uses these instead of its defaults. Populated by the
    /// gateway from the WS `temperature` / `max_tokens` fields. `None` →
    /// the agent runtime falls back to its built-in defaults (0.7 / 8192).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_params: Option<ModelParams>,
    /// RFC-032: Role hint (optional). When set, the agent runtime consults
    /// `engine.role_routing[role]` to choose a model ID. Precedence:
    /// `model_override` (recovery retry) > `role_routing[role]` > default.
    /// Set by the gateway when the WS client supplied a `role` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,

    /// Session-scoped persona override (optional). When set, the agent
    /// runtime resolves THIS persona (prompt + role) instead of the global
    /// active persona for the turn. `None` → inherit the global default.
    /// Populated by the gateway from the WS / POST `persona_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<String>,

    /// Agent conversation state to restore on retry (RFC-029 P2b).
    ///
    /// When `Some`, the recovery coordinator captured the previous run's
    /// `Agent::export_state()` and injects it here so the new (fallback
    /// model) agent continues from the checkpoint rather than restarting.
    /// `None` on initial runs or when no state was accumulated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_state: Option<serde_json::Value>,

    /// RFC-033: the chat session key the gateway registered its streaming
    /// sink under. The orchestrator sets this to `ctx.session_id` (which is
    /// the WS session id, or the request id for a session's first message).
    /// The agent runtime uses it as `transparency_session` so token/tool/
    /// thinking deltas and RFC-015 events correlate with the live WS sink.
    /// `#[serde(skip)]` — it is transient (per-execution), never persisted.
    #[serde(skip)]
    pub session_id: Option<String>,
}

/// The result of reviewing an execution against a directive's criteria.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    /// Whether the execution passed all criteria.
    pub passed: bool,

    /// 0.0–1.0 confidence score.
    pub score: f64,

    /// Human-readable notes (✓/✗ prefix convention).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,

    /// Specific gaps where criteria were not met — fed into retry context.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<String>,
}

impl Verdict {
    /// Whether the verdict indicates a pass.
    pub fn all_passed(&self) -> bool {
        self.passed
    }
}

/// A single conversation exchange: user message → agent response (or question).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    /// User's message.
    pub user: String,
    /// Agent's response (or clarifying question).
    pub agent: String,
}

/// Message processing context for the intent handler.
#[derive(Debug, Clone)]
pub struct MsgCtx {
    /// Session ID for history lookup.
    pub session_id: String,
    /// Previous exchanges in this session (from the state store).
    /// Provides clarify context and topic-shift detection.
    pub history: Vec<Exchange>,
    /// Comma-separated project IDs.
    pub project_ids: Option<String>,
    /// Comma-separated Mount IDs.
    pub mount_ids: Option<String>,
    /// RFC-032: Role hint (optional). When set, the orchestrator
    /// resolves the model via `engine.role_routing[role]`. Populated
    /// by the gateway from the WS `role` field.
    pub role: Option<String>,
    /// Session-scoped persona override (optional). When set, the orchestrator
    /// carries it into [`ExecEnv::persona_id`] so the agent runtime resolves
    /// this persona (prompt + role) instead of the global active persona.
    /// `None` → inherit the global default. Populated by the gateway from
    /// the WS / POST `persona_id` field.
    pub persona_id: Option<String>,
    /// Per-message model override (optional). When set, the orchestrator
    /// carries it into [`ExecEnv::model_override`] so the agent runtime
    /// uses this model instead of `role_routing[role]` or the engine
    /// default. Populated by the gateway from the WS / POST `model`
    /// field. `None` for normal runs.
    pub model_override: Option<String>,
    /// User identifier.
    /// Per-message model override (optional). When set, the orchestrator
    /// carries it into [`ExecEnv::model_override`] so the agent runtime
    /// uses this model instead of `role_routing[role]` or the engine
    /// default. Populated by the gateway from the WS / POST `model`
    /// field. `None` for normal runs.
    pub user_id: String,

    /// Per-message model params. Carried into ExecEnv::model_params.
    /// Populated by the gateway from the WS payload.
    pub model_params: Option<ModelParams>,
}

/// Per-message model generation params. Wired from WS payload through
/// `MsgCtx` → `ExecEnv` → `AgentRuntimeConfig` → `AgentConfig`. Each
/// field is `None` when the client did not supply a value, so the agent
/// runtime can apply per-field defaults independently.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelParams {
    /// Sampling temperature (0.0–2.0). Typical: 0.0–1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Maximum tokens to generate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// Timestamp of directive creation (used for session metadata, not stored on the directive itself).
pub type DirectiveTimestamp = DateTime<Utc>;

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Directive::from_message — Trivial task construction
    // -----------------------------------------------------------------------

    #[test]
    fn from_message_uses_message_as_both_goal_and_original_request() {
        let d = Directive::from_message("read the README");
        assert_eq!(d.goal, "read the README");
        assert_eq!(d.original_request, "read the README");
    }

    #[test]
    fn from_message_preserves_exact_punctuation_and_whitespace() {
        // The directive is the verbatim user message — language fidelity
        // matters, so no trimming, lowercasing, or normalization.
        let raw = "  Fix bug #42 (urgent)  ";
        let d = Directive::from_message(raw);
        assert_eq!(d.original_request, raw);
        assert_eq!(d.goal, raw);
    }

    #[test]
    fn from_message_produces_empty_criteria_and_constraints() {
        let d = Directive::from_message("hello");
        assert!(d.constraints.is_empty());
        assert!(d.acceptance_criteria.is_empty());
        assert!(d.output_schema.is_none());
    }

    #[test]
    fn from_message_accepts_string_and_str_via_into() {
        // The signature takes `impl Into<String>` so both &str and String work.
        let from_str = Directive::from_message("from &str");
        let from_string = Directive::from_message(String::from("from String"));
        assert_eq!(from_str.goal, "from &str");
        assert_eq!(from_string.goal, "from String");
    }

    // -----------------------------------------------------------------------
    // Directive::needs_review — review-eligibility contract
    // -----------------------------------------------------------------------

    #[test]
    fn needs_review_false_for_from_message_directive() {
        // from_message is the Trivial path — no criteria, no schema → no review.
        let d = Directive::from_message("hi");
        assert!(!d.needs_review());
    }

    #[test]
    fn needs_review_true_when_acceptance_criteria_present() {
        let mut d = Directive::from_message("do thing");
        d.acceptance_criteria.push("must exit 0".to_string());
        assert!(d.needs_review());
    }

    #[test]
    fn needs_review_true_when_output_schema_present() {
        let mut d = Directive::from_message("do thing");
        d.output_schema = Some(serde_json::json!({"type": "object"}));
        assert!(d.needs_review());
    }

    #[test]
    fn needs_review_true_with_either_criteria_or_schema() {
        // Either signal triggers review; we don't AND them.
        let d_crit = Directive {
            goal: "x".into(),
            original_request: "x".into(),
            constraints: vec![],
            acceptance_criteria: vec!["c".into()],
            output_schema: None,
        };
        let d_schema = Directive {
            goal: "x".into(),
            original_request: "x".into(),
            constraints: vec![],
            acceptance_criteria: vec![],
            output_schema: Some(serde_json::json!({})),
        };
        assert!(d_crit.needs_review());
        assert!(d_schema.needs_review());
    }

    // -----------------------------------------------------------------------
    // Verdict::all_passed — pass-flag accessor
    // -----------------------------------------------------------------------

    #[test]
    fn verdict_all_passed_matches_passed_field() {
        assert!(
            Verdict {
                passed: true,
                score: 1.0,
                notes: vec![],
                gaps: vec![]
            }
            .all_passed()
        );
        assert!(
            !Verdict {
                passed: false,
                score: 0.0,
                notes: vec![],
                gaps: vec!["x".into()]
            }
            .all_passed()
        );
    }

    // -----------------------------------------------------------------------
    // Directive JSON round-trip — serialization invariants
    // -----------------------------------------------------------------------

    #[test]
    fn directive_serialization_roundtrip_preserves_all_fields() {
        let mut d = Directive::from_message("build the thing");
        d.constraints = vec!["no network".to_string(), "single file".to_string()];
        d.acceptance_criteria = vec!["compiles".to_string()];
        d.output_schema = Some(serde_json::json!({"type": "object"}));

        let json = serde_json::to_value(&d).unwrap();
        let back: Directive = serde_json::from_value(json).unwrap();
        assert_eq!(back.goal, d.goal);
        assert_eq!(back.original_request, d.original_request);
        assert_eq!(back.constraints, d.constraints);
        assert_eq!(back.acceptance_criteria, d.acceptance_criteria);
        assert_eq!(back.output_schema, d.output_schema);
    }

    #[test]
    fn directive_serialization_omits_empty_optional_fields() {
        // The struct uses `skip_serializing_if` for empty Vecs/None — JSON
        // stays minimal so the LLM-side prompt doesn't see noise.
        let d = Directive::from_message("hi");
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("\"constraints\""));
        assert!(!json.contains("\"acceptance_criteria\""));
        assert!(!json.contains("\"output_schema\""));
    }

    #[test]
    fn directive_serialization_includes_constraints_when_populated() {
        let mut d = Directive::from_message("hi");
        d.constraints.push("one".to_string());
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"constraints\""));
        assert!(json.contains("\"one\""));
    }

    // -----------------------------------------------------------------------
    // ExecEnv — session_id is transient (#[serde(skip)])
    // -----------------------------------------------------------------------

    #[test]
    fn exec_env_session_id_is_skipped_from_serialization() {
        let env = ExecEnv {
            session_id: Some("sess-1".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&env).unwrap();
        // session_id is per-execution and must not persist.
        assert!(!json.contains("sess-1"));
        assert!(!json.contains("session_id"));
    }

    #[test]
    fn exec_env_round_trip_preserves_non_transient_fields() {
        let env = ExecEnv {
            workspace_context: Some("ctx".to_string()),
            mount_paths: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            cspace_hint: Some("tmpl".to_string()),
            model_override: Some("anthropic/claude-sonnet-4".to_string()),
            role: Some("researcher".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_value(&env).unwrap();
        let back: ExecEnv = serde_json::from_value(json).unwrap();
        assert_eq!(back.workspace_context, Some("ctx".to_string()));
        assert_eq!(
            back.mount_paths,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        assert_eq!(back.cspace_hint, Some("tmpl".to_string()));
        assert_eq!(
            back.model_override,
            Some("anthropic/claude-sonnet-4".to_string())
        );
        assert_eq!(back.role, Some("researcher".to_string()));
    }
}
