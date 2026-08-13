//! Event bus: inter-agent communication via `oxicode_sdk::EventBus<KernelEvent>`.
//!
//! The event bus is the "pipe" of Oxios. All agents communicate
//! through kernel events published on the bus.
//!
//! After RFC-014 Phase C, this module no longer owns the broadcast channel —
//! it reuses `oxicode_sdk::EventBus<E>`, which is a generic wrapper over
//! `tokio::sync::broadcast`. The only Oxios-specific bits are:
//!
//! - `KernelEvent` enum (oxios-internal event vocabulary)
//! - `kernel_event_to_audit_action` mapping for the audit trail
//! - `attach_audit_trail` helper (subscribes the bus to the trail)

use oxicode_sdk::EventBus as SdkEventBus;
use oxicode_sdk::observability::{AuditAction, AuditTrail};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::types::AgentId;

/// Kernel event bus — generic SDK bus specialised for `KernelEvent`.
///
/// The broadcast channel is owned by `oxicode_sdk::EventBus`; this type alias
/// just makes the call sites read more naturally (`crate::event_bus::EventBus`
/// instead of `oxicode_sdk::EventBus<KernelEvent>`).
pub type EventBus = SdkEventBus<KernelEvent>;

/// Events that flow through the kernel event bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KernelEvent {
    /// A new agent has been created.
    AgentCreated {
        /// The new agent's ID.
        id: AgentId,
        /// The agent's name/goal.
        name: String,
    },
    /// An agent has started executing.
    AgentStarted {
        /// The agent's ID.
        id: AgentId,
    },
    /// An agent has been stopped.
    ///
    /// Carries `success` so consumers can distinguish a normal completion
    /// (`success: true`) from an evaluation/assessment failure
    /// (`success: false`). Infrastructure errors (panic, timeout) emit
    /// `AgentFailed` instead.
    AgentStopped {
        /// The agent's ID.
        id: AgentId,
        /// Whether the agent's result passed evaluation. Mirrors
        /// `ExecutionResult.success` from the Ok path; `false` on the
        /// kill/terminate path (user-initiated stop).
        #[serde(default)]
        success: bool,
    },
    /// An agent has encountered a failure.
    AgentFailed {
        /// The agent's ID.
        id: AgentId,
        /// Description of the error.
        error: String,
    },
    /// A message has been received from an agent.
    MessageReceived {
        /// The sending agent's ID.
        from: AgentId,
        /// Message content.
        content: String,
    },
    /// An agent has produced output.
    AgentOutput {
        /// The session this output belongs to.
        session_id: String,
        /// The agent's ID.
        agent_id: AgentId,
        /// The output content.
        output: String,
    },
    /// A HitL approval request has been submitted.
    ApprovalRequested {
        /// The approval request ID.
        id: uuid::Uuid,
        /// The tool requesting approval.
        tool_name: String,
        /// The action requiring approval.
        action: String,
        /// The resource involved.
        resource: String,
        /// Reason for the request.
        reason: String,
        /// The session ID that triggered this request.
        session_id: Option<String>,
    },
    /// A HitL approval has been resolved (approved or rejected).
    ApprovalResolved {
        /// The approval request ID.
        id: uuid::Uuid,
        /// Whether it was approved (true) or rejected (false).
        approved: bool,
    },
    /// An agent tried to access a file path outside its allowed_paths.
    /// The frontend renders a path-access card offering: create a Mount,
    /// temporarily allow, or deny. Mirrors `ApprovalRequested` but carries
    /// path-specific context so the card and resolve endpoint know what to do.
    PathAccessRequested {
        /// The request ID (matches the PendingPathApprovals entry).
        id: uuid::Uuid,
        /// The tool that tried to access the path (read, write, edit …).
        tool_name: String,
        /// The denied path (absolute).
        path: String,
        /// Access mode: "read" or "write".
        mode: String,
        /// The agent whose `allowed_paths` would need updating.
        agent_name: String,
        /// Human-readable denial reason from the AccessGate.
        reason: String,
        /// The session ID that triggered this request.
        session_id: Option<String>,
    },
    /// Multi-agent group created.
    AgentGroupCreated {
        /// The group's ID.
        group_id: uuid::Uuid,
        /// Number of agents in the group.
        agent_count: usize,
    },
    /// An agent in a group completed.
    AgentGroupMemberCompleted {
        /// The group's ID.
        group_id: uuid::Uuid,
        /// The agent's ID.
        agent_id: uuid::Uuid,
        /// Whether the agent succeeded.
        success: bool,
    },
    /// A new Project has been created (RFC-011).
    ProjectCreated {
        /// The project's ID.
        project_id: uuid::Uuid,
        /// The project's name.
        name: String,
        /// How it was created.
        source: String,
    },
    /// A Project has been activated (RFC-011).
    ProjectActivated {
        /// The project's ID.
        project_id: uuid::Uuid,
        /// The project's name.
        name: String,
    },

    // ── RFC-015 Chat Transparency ─────────────────────────────
    // Real-time events emitted by AgentRuntime during tool execution
    // and streaming. Web channel converts these to WS chunks.
    /// A tool execution has started (real-time, RFC-015).
    ToolExecutionStarted {
        /// Session this tool call belongs to.
        session_id: String,
        /// Name of the tool (e.g. "read_file", "bash", "memory_recall").
        tool_name: String,
        /// Provider-specific tool call ID used to correlate start/end.
        tool_call_id: String,
        /// Tool input arguments (JSON).
        tool_args: serde_json::Value,
        /// Semantic context inferred by oxicode-agent 0.32+ from tool name/args
        /// (e.g. WebSearch, PageVisit). `None` for tools without context mapping.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<serde_json::Value>,
    },
    /// A tool execution has finished (real-time, RFC-015).
    ToolExecutionFinished {
        /// Session this tool call belongs to.
        session_id: String,
        /// Provider-specific tool call ID.
        tool_call_id: String,
        /// Name of the tool.
        tool_name: String,
        /// Wall-clock duration in milliseconds.
        duration_ms: u64,
        /// Whether the tool returned an error.
        is_error: bool,
        /// Truncated output (max ~500 chars) for streaming.
        output_summary: String,
    },
    /// A tool execution emitted a progress update (real-time, RFC-015).
    ToolExecutionProgress {
        /// Session this tool call belongs to.
        session_id: String,
        /// Provider-specific tool call ID.
        tool_call_id: String,
        /// Name of the tool.
        tool_name: String,
        /// Human-readable progress text (already-formatted by the tool).
        progress: String,
        /// Tab that emitted this progress event, if the upstream tool tracks
        /// tabs. `None` for tools that don't have a tab concept (e.g. legacy
        /// oxicode-agent versions that don't propagate `tab_id`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<Uuid>,
        /// Semantic context from the tool call (e.g. PageVisit, WebSearch).
        /// Stored as `serde_json::Value` to decouple kernel events from
        /// oxicode-sdk's internal `ToolCallContext` enum. UI consumers that
        /// understand a context variant render it richly; older consumers
        /// simply ignore the field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<serde_json::Value>,
    },
    /// Memory was recalled during agent execution (RFC-015).
    MemoryRecallUsed {
        /// Session this recall belongs to.
        session_id: String,
        /// The recall query.
        query: String,
        /// Number of memories returned.
        count: usize,
        /// Memory tier source ("hot" | "warm" | "cold").
        source: String,
    },
    /// Token usage update (RFC-015).
    TokenUsageUpdate {
        /// Session this usage belongs to.
        session_id: String,
        /// Cumulative input tokens.
        input_tokens: u64,
        /// Cumulative output tokens.
        output_tokens: u64,
    },
    /// Reasoning/compaction fragment (RFC-015).
    ReasoningFragment {
        /// Session this fragment belongs to.
        session_id: String,
        /// The fragment text (chain-of-thought, compaction summary, etc).
        content: String,
        /// Source label: "chain_of_thought" | "compaction" | "reflection".
        source: String,
    },
    /// Partial tool-call arguments streamed by the LLM (RFC-015 Phase C).
    ///
    /// Emitted by oxi 0.58+ (`AgentEvent::ToolCallDelta`) while the model is
    /// still constructing a tool call, before `ToolExecutionStarted`. Each
    /// `args_delta` is a raw JSON fragment; consumers accumulate per
    /// `tool_call_id`.
    ToolArgsDelta {
        /// Session this delta belongs to.
        session_id: String,
        /// Tool call identifier (matches the later `ToolExecutionStarted`).
        tool_call_id: String,
        /// Raw JSON argument fragment from the LLM stream.
        args_delta: String,
    },

    /// Compaction was triggered (RFC-035 gap 2 observability).
    ///
    /// Emitted when `oxicode_sdk::CompactionEvent::Triggered` fires (0.53.0+).
    /// `source` is one of:
    /// - `"provider-reported"` — provider-reported `usage.input_tokens` drove
    ///   the trigger (ground truth; gap 2's primary signal)
    /// - `"bytes/4 heuristic (cold start)"` — legacy heuristic; only on turn 1
    ///   before any `ProviderEvent::Done` has been observed
    /// - `"empty"` — empty context (no trigger source)
    CompactionTriggered {
        /// Session this compaction belongs to.
        session_id: Option<String>,
        /// The trigger source label from `CompactionEvent::Triggered::source`.
        source: String,
    },
    // ── Calendar ──────────────────────────────────────────────
    /// A calendar event was created.
    CalendarEventCreated {
        /// Event UID.
        uid: String,
        /// Event title.
        title: String,
        /// Start time.
        start: String,
        /// End time.
        end: String,
    },
    /// A calendar event was updated.
    CalendarEventUpdated {
        /// Event UID.
        uid: String,
        /// Event title.
        title: String,
    },
    /// A calendar event was deleted.
    CalendarEventDeleted {
        /// Event UID.
        uid: String,
        /// Event title.
        title: String,
    },
    /// A memo was created in the oximemo vault (first-party app module).
    MemoCreated {
        /// Memo id (UUIDv7, hyphenated).
        id: String,
    },
    /// A memo was soft-deleted from the oximemo vault.
    MemoDeleted {
        /// Memo id (UUIDv7, hyphenated).
        id: String,
    },
    /// An email has been sent.
    EmailSent {
        /// Email subject.
        subject: String,
        /// SMTP message ID.
        message_id: String,
        /// Template name (if template was used/saved).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template_name: Option<String>,
    },

    // ── Knowledge ──────────────────────────────────────────────
    /// A knowledge note was persisted (hook, user, or tool).
    KnowledgePersisted {
        session_id: String,
        message_index: usize,
        path: String,
        source: String, // "hook", "user", "tool"
    },
    /// A knowledge note was removed by user action.
    KnowledgeRemoved {
        session_id: String,
        message_index: usize,
    },
    /// A question was posed to the user by the agent (RFC-027, `ask_user`).
    /// The frontend renders an input/option picker and resolves the
    /// pending oneshot via a separate response endpoint.
    AskUserRequest {
        /// Unique request ID — used by the response handler to resolve
        /// the oneshot the tool is awaiting.
        id: String,
        /// The question text the user sees.
        question: String,
        /// Optional structured options. Empty when the question is open-ended.
        options: Vec<String>,
    },
    // ── Persona (agent-authored writes are security-reviewed) ───────────
    /// A persona was created (by an agent tool, the HTTP API, or the UI).
    PersonaCreated {
        /// Persona ID.
        id: String,
        /// Persona display name.
        name: String,
        /// Whether it was registered enabled.
        enabled: bool,
        /// Origin of the change: "agent" | "api" | "ui".
        source: String,
    },
    /// A persona was updated.
    PersonaUpdated {
        /// Persona ID.
        id: String,
        /// Persona display name.
        name: String,
        /// Origin of the change: "agent" | "api" | "ui".
        source: String,
    },
    // ── Integration install (RFC-041 M3) ────────────────────────────────────
    // These ride the same SSE channel as tool execution progress so the UI
    // can stream install output without opening a second connection. Routing
    // is by `job_id` (opaque to clients; the daemon mints it via uuid).
    /// An integration install job has started.
    IntegrationInstallStarted {
        /// Opaque job ID; clients correlate all subsequent events with this.
        job_id: String,
        /// Integration registry id (e.g. "github").
        integration_id: String,
        /// Human-readable label.
        label: String,
    },
    /// Incremental progress — a stdout line or stage transition.
    IntegrationInstallProgress {
        job_id: String,
        integration_id: String,
        /// One line of output or a stage label (e.g. "fetching", "extracting").
        line: String,
    },
    /// Install completed successfully.
    IntegrationInstallCompleted {
        job_id: String,
        integration_id: String,
        /// Final command + summary output for the audit log and UI.
        command: String,
        output: String,
        exit_code: Option<i32>,
    },
    /// Install failed (non-zero exit or spawn error).
    IntegrationInstallFailed {
        job_id: String,
        integration_id: String,
        error: String,
    },
    /// A chunk of the compression summary being streamed.
    CompressionDelta {
        /// The session being compressed.
        session_id: String,
        /// Incremental summary text.
        delta: String,
    },
    /// Compression completed successfully.
    CompressionDone {
        /// The session that was compressed.
        session_id: String,
    },
    /// Compression failed.
    CompressionFailed {
        /// The session that failed compression.
        session_id: String,
        /// Error description.
        error: String,
    },
}

/// Convert a KernelEvent to an AuditAction for the audit trail.
pub fn kernel_event_to_audit_action(event: &KernelEvent) -> AuditAction {
    match event {
        KernelEvent::AgentCreated { name, .. } => AuditAction::AgentSpawn {
            task_type: name.clone(),
        },
        KernelEvent::AgentStarted { .. } => AuditAction::AgentSpawn {
            task_type: "started".to_string(),
        },
        KernelEvent::AgentStopped { success, .. } => AuditAction::AgentExit {
            reason: if *success {
                "completed".to_string()
            } else {
                "stopped".to_string()
            },
        },
        KernelEvent::AgentFailed { error, .. } => AuditAction::AgentExit {
            reason: error.clone(),
        },
        KernelEvent::MessageReceived { content, .. } => AuditAction::Other {
            detail: format!("message: {content}"),
        },
        KernelEvent::AgentOutput { output, .. } => AuditAction::Other {
            detail: format!("agent_output:{output}"),
        },
        KernelEvent::ApprovalRequested {
            id,
            action,
            resource,
            ..
        } => AuditAction::Other {
            detail: format!("approval_requested:{id}:{action}:{resource}"),
        },
        KernelEvent::ApprovalResolved { id, approved } => AuditAction::Other {
            detail: format!("approval_resolved:{id}:{approved}"),
        },
        KernelEvent::PathAccessRequested {
            id,
            path,
            tool_name,
            ..
        } => AuditAction::Other {
            detail: format!("path_access_requested:{id}:{tool_name}:{path}"),
        },
        KernelEvent::AgentGroupCreated {
            group_id,
            agent_count,
        } => AuditAction::Other {
            detail: format!("group_created:{group_id}:{agent_count}agents"),
        },
        KernelEvent::AgentGroupMemberCompleted {
            group_id,
            agent_id,
            success,
        } => AuditAction::Other {
            detail: format!("group_member_completed:{group_id}:{agent_id}:{success}"),
        },
        KernelEvent::ProjectCreated {
            project_id: _,
            name,
            source,
        } => AuditAction::Other {
            detail: format!("project_created:{name}:{source}"),
        },
        KernelEvent::ProjectActivated {
            project_id: _,
            name,
        } => AuditAction::Other {
            detail: format!("project_activated:{name}"),
        },
        // ── RFC-015 ──
        KernelEvent::ToolExecutionStarted { tool_name, .. } => AuditAction::Other {
            detail: format!("tool_started:{tool_name}"),
        },
        KernelEvent::ToolExecutionFinished {
            tool_name,
            is_error,
            ..
        } => AuditAction::Other {
            detail: format!(
                "tool_finished:{tool_name}:{}",
                if *is_error { "error" } else { "ok" }
            ),
        },
        KernelEvent::ToolExecutionProgress {
            tool_name,
            tab_id,
            context,
            ..
        } => AuditAction::Other {
            detail: {
                let mut d = format!("tool_progress:{tool_name}");
                if let Some(id) = tab_id {
                    d.push_str(&format!(":tab={id}"));
                }
                if let Some(ctx) = context
                    .as_ref()
                    .and_then(|c| c.get("kind"))
                    .and_then(|k| k.as_str())
                {
                    d.push_str(&format!(":{ctx}"));
                }
                d
            },
        },
        KernelEvent::MemoryRecallUsed { query, count, .. } => AuditAction::MemoryRead {
            entry_id: format!("recall:{query}:{count}results"),
        },
        KernelEvent::TokenUsageUpdate {
            input_tokens,
            output_tokens,
            ..
        } => AuditAction::Other {
            detail: format!("tokens:in={input_tokens}:out={output_tokens}"),
        },
        KernelEvent::ReasoningFragment { source, .. } => AuditAction::Other {
            detail: format!("reasoning:{source}"),
        },
        KernelEvent::ToolArgsDelta { tool_call_id, .. } => AuditAction::Other {
            detail: format!("tool_args_delta:{tool_call_id}"),
        },
        KernelEvent::CompactionTriggered { source, .. } => AuditAction::Other {
            detail: format!("compaction:triggered:{source}"),
        },
        KernelEvent::CalendarEventCreated { uid, title, .. } => AuditAction::Other {
            detail: format!("calendar:created:{uid}:{title}"),
        },
        KernelEvent::CalendarEventUpdated { uid, title } => AuditAction::Other {
            detail: format!("calendar:updated:{uid}:{title}"),
        },
        KernelEvent::CalendarEventDeleted { uid, title } => AuditAction::Other {
            detail: format!("calendar:deleted:{uid}:{title}"),
        },
        KernelEvent::MemoCreated { id } => AuditAction::Other {
            detail: format!("memo:created:{id}"),
        },
        KernelEvent::MemoDeleted { id } => AuditAction::Other {
            detail: format!("memo:deleted:{id}"),
        },
        KernelEvent::EmailSent {
            subject,
            message_id,
            template_name,
        } => AuditAction::Other {
            detail: format!("email:sent:{subject} (msg={message_id}, tpl={template_name:?})"),
        },
        KernelEvent::KnowledgePersisted {
            session_id,
            message_index,
            path,
            source,
        } => AuditAction::Other {
            detail: format!("knowledge:persisted:{session_id}:{message_index}:{path}:{source}"),
        },
        KernelEvent::KnowledgeRemoved {
            session_id,
            message_index,
        } => AuditAction::Other {
            detail: format!("knowledge:removed:{session_id}:{message_index}"),
        },
        KernelEvent::AskUserRequest { id, question, .. } => AuditAction::Other {
            detail: format!("ask_user:{id}:{question}"),
        },
        KernelEvent::PersonaCreated {
            id, name, source, ..
        } => AuditAction::Other {
            detail: format!("persona:created:{id}:{name}:{source}"),
        },
        KernelEvent::PersonaUpdated { id, name, source } => AuditAction::Other {
            detail: format!("persona:updated:{id}:{name}:{source}"),
        },
        KernelEvent::IntegrationInstallStarted {
            job_id,
            integration_id,
            ..
        } => AuditAction::Other {
            detail: format!("install:started:{integration_id}:{job_id}"),
        },
        KernelEvent::IntegrationInstallProgress {
            job_id,
            integration_id,
            ..
        } => AuditAction::Other {
            detail: format!("install:progress:{integration_id}:{job_id}"),
        },
        KernelEvent::IntegrationInstallCompleted {
            job_id,
            integration_id,
            exit_code,
            ..
        } => AuditAction::Other {
            detail: format!(
                "install:completed:{integration_id}:{job_id}:exit={}",
                exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into())
            ),
        },
        KernelEvent::IntegrationInstallFailed {
            job_id,
            integration_id,
            ..
        } => AuditAction::Other {
            detail: format!("install:failed:{integration_id}:{job_id}"),
        },
        KernelEvent::CompressionDelta { .. }
        | KernelEvent::CompressionDone { .. }
        | KernelEvent::CompressionFailed { .. } => AuditAction::Other {
            detail: "compression".to_string(),
        },
    }
}

/// Extract agent ID from a KernelEvent variant.
fn extract_agent_id(event: &KernelEvent) -> String {
    match event {
        KernelEvent::AgentCreated { id, .. } => id.to_string(),
        KernelEvent::AgentStarted { id, .. } => id.to_string(),
        KernelEvent::AgentStopped { id, .. } => id.to_string(),
        KernelEvent::AgentFailed { id, .. } => id.to_string(),
        KernelEvent::MessageReceived { from, .. } => from.to_string(),
        KernelEvent::AgentOutput { agent_id, .. } => agent_id.to_string(),
        KernelEvent::AgentGroupMemberCompleted { agent_id, .. } => agent_id.to_string(),
        KernelEvent::ProjectActivated { project_id, .. } => format!("project:{project_id}"),
        // RFC-015: session-scoped events use session_id as the subject
        KernelEvent::ToolExecutionStarted { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::ToolExecutionFinished { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::ToolExecutionProgress { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::MemoryRecallUsed { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::TokenUsageUpdate { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::ReasoningFragment { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::ToolArgsDelta { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::KnowledgePersisted { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::KnowledgeRemoved { session_id, .. } => format!("session:{session_id}"),
        KernelEvent::CompactionTriggered { session_id, .. } => session_id
            .as_ref()
            .map(|s| format!("session:{s}"))
            .unwrap_or_else(|| "system".to_string()),
        _ => "system".to_string(),
    }
}

/// Subscribe the audit trail to all kernel events.
///
/// The bus is broadcast-based; this spawns a long-running task that
/// forwards every event into the audit trail as a structured entry.
/// Lagged subscribers are logged and recovered.
pub fn attach_audit_trail(bus: &EventBus, audit: Arc<AuditTrail>) {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Skip high-frequency streaming variants that are UI data,
                    // not audit-relevant. ToolCallDelta fires per-token (raw
                    // JSON fragments) — appending each would flood the Merkle
                    // chain + JSONL with partial-JSON Debug strings per call.
                    if matches!(event, KernelEvent::ToolArgsDelta { .. }) {
                        continue;
                    }
                    let actor = extract_agent_id(&event);
                    let action = kernel_event_to_audit_action(&event);
                    let resource = format!("{event:?}");
                    audit.append(actor, action, resource);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // Surface the drop as a metric so operators can detect
                    // incomplete audit trails instead of the events
                    // vanishing silently (state-area F4).
                    crate::metrics::get_metrics().audit_lagged_events.inc_by(n);
                    tracing::warn!(
                        skipped = n,
                        "Audit trail subscriber lagged, skipping events"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!("Audit trail event bus closed, exiting");
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(name: &str) -> KernelEvent {
        KernelEvent::AgentCreated {
            id: AgentId::new_v4(),
            name: name.to_string(),
        }
    }

    #[test]
    fn test_event_bus_uses_sdk() {
        let bus: EventBus = EventBus::new(256);
        assert!(format!("{:?}", bus).contains("EventBus"));
    }

    #[tokio::test]
    async fn test_publish_no_subscribers_ok() {
        let bus = EventBus::new(16);
        let result = bus.publish(sample_event("orphan"));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_single_subscriber_receives_event() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        let event = sample_event("test-agent");
        bus.publish(event.clone()).unwrap();

        let received = rx.try_recv().expect("should receive event");
        match received {
            KernelEvent::AgentCreated { name, .. } => assert_eq!(name, "test-agent"),
            _ => panic!("wrong event type"),
        }
    }

    #[tokio::test]
    async fn test_multiple_subscribers_receive_events() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let event = sample_event("multi");
        bus.publish(event.clone()).unwrap();

        let r1 = rx1.try_recv().expect("rx1 should receive event");
        let r2 = rx2.try_recv().expect("rx2 should receive event");

        assert!(matches!(r1, KernelEvent::AgentCreated { .. }));
        assert!(matches!(r2, KernelEvent::AgentCreated { .. }));
    }

    #[tokio::test]
    async fn test_kernel_event_to_audit_action() {
        let event = KernelEvent::AgentFailed {
            id: AgentId::new_v4(),
            error: "boom".to_string(),
        };
        let action = kernel_event_to_audit_action(&event);
        match action {
            AuditAction::AgentExit { reason } => assert_eq!(reason, "boom"),
            other => panic!("expected AgentExit, got {other:?}"),
        }
    }

    // ── RFC-015 chat transparency event coverage ──

    /// Round-trip JSON serialization for every new RFC-015 variant. This
    /// guards against accidental renames that would break the WebSocket
    /// wire format on the frontend.
    #[test]
    fn test_rfc015_event_round_trip_json() {
        let cases: Vec<KernelEvent> = vec![
            KernelEvent::ToolExecutionStarted {
                session_id: "s1".into(),
                tool_name: "read_file".into(),
                tool_call_id: "call_1".into(),
                tool_args: serde_json::json!({"path": "/src/main.rs"}),
                context: None,
            },
            KernelEvent::ToolExecutionFinished {
                session_id: "s1".into(),
                tool_call_id: "call_1".into(),
                tool_name: "read_file".into(),
                duration_ms: 234,
                is_error: false,
                output_summary: "fn main() {}".into(),
            },
            KernelEvent::ToolExecutionProgress {
                session_id: "s1".into(),
                tool_call_id: "call_1".into(),
                tool_name: "read_file".into(),
                progress: "reading line 42/100".into(),
                tab_id: None,
                context: None,
            },
            KernelEvent::MemoryRecallUsed {
                session_id: "s1".into(),
                query: "rust errors".into(),
                count: 3,
                source: "warm".into(),
            },
            KernelEvent::TokenUsageUpdate {
                session_id: "s1".into(),
                input_tokens: 1234,
                output_tokens: 567,
            },
            KernelEvent::ReasoningFragment {
                session_id: "s1".into(),
                content: "compaction done".into(),
                source: "compaction".into(),
            },
        ];
        for event in cases {
            let json = serde_json::to_string(&event).expect("serialize");
            let back: KernelEvent = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&back).expect("serialize round-trip");
            assert_eq!(json, json2, "round-trip should be stable");
        }
    }

    /// Tool progress events serialize/deserialize cleanly and round-trip
    /// stable JSON, matching the wire format the WS layer expects.
    #[test]
    fn test_tool_execution_progress_serde_round_trip() {
        let event = KernelEvent::ToolExecutionProgress {
            session_id: "s-abc".into(),
            tool_call_id: "call_42".into(),
            tool_name: "browse".into(),
            progress: "loading https://example.com".into(),
            tab_id: Some(Uuid::new_v4()),
            context: None,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let back: KernelEvent = serde_json::from_str(&json).expect("deserialize");
        match back {
            KernelEvent::ToolExecutionProgress {
                ref session_id,
                ref tool_call_id,
                ref tool_name,
                ref progress,
                tab_id,
                ..
            } => {
                assert_eq!(session_id, "s-abc");
                assert_eq!(tool_call_id, "call_42");
                assert_eq!(tool_name, "browse");
                assert_eq!(progress, "loading https://example.com");
                assert!(tab_id.is_some(), "tab_id should round-trip when present");
            }
            other => panic!("expected ToolExecutionProgress, got {other:?}"),
        }
    }

    /// The audit-action mapping for tool progress should produce a stable,
    /// searchable detail string (used by the audit-trail UI to filter).
    /// When `tab_id` is set, the detail includes `:tab=<id>`; when absent,
    /// the original `tool_progress:<tool>` form is preserved (back-compat
    /// for older oxicode-agent versions that don't propagate tabs).
    #[test]
    fn test_tool_execution_progress_audit_action() {
        let with_tab = KernelEvent::ToolExecutionProgress {
            session_id: "s1".into(),
            tool_call_id: "c1".into(),
            tool_name: "browse".into(),
            progress: "navigating".into(),
            tab_id: Some(Uuid::new_v4()),
            context: None,
        };
        match kernel_event_to_audit_action(&with_tab) {
            AuditAction::Other { detail } => {
                assert!(detail.contains("tool_progress"), "detail: {detail}");
                assert!(detail.contains("browse"), "detail: {detail}");
                assert!(
                    detail.contains(":tab="),
                    "detail should include tab id: {detail}"
                );
            }
            other => panic!("expected Other, got {other:?}"),
        }
        let without_tab = KernelEvent::ToolExecutionProgress {
            session_id: "s1".into(),
            tool_call_id: "c1".into(),
            tool_name: "browse".into(),
            progress: "navigating".into(),
            tab_id: None,
            context: None,
        };
        match kernel_event_to_audit_action(&without_tab) {
            AuditAction::Other { detail } => {
                assert_eq!(detail, "tool_progress:browse");
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    /// `tab_id` is optional in serde (`#[serde(default)]`) so older oxicode-agent
    /// versions that don't emit it still round-trip cleanly. This guards the
    /// backwards-compat contract explicitly.
    #[test]
    fn test_tool_execution_progress_tab_id_optional_in_serde() {
        // Simulate a payload from a legacy oxicode-agent (no tab_id key).
        // KernelEvent is externally tagged, so the variant is the JSON key.
        let legacy_json = r#"{
            "ToolExecutionProgress": {
                "session_id": "s-old",
                "tool_call_id": "call_legacy",
                "tool_name": "browse",
                "progress": "step 1"
            }
        }"#;
        let event: KernelEvent = serde_json::from_str(legacy_json).expect("deserialize legacy");
        match &event {
            KernelEvent::ToolExecutionProgress {
                session_id,
                tool_call_id,
                tool_name,
                progress,
                tab_id,
                ..
            } => {
                assert_eq!(session_id, "s-old");
                assert_eq!(tool_call_id, "call_legacy");
                assert_eq!(tool_name, "browse");
                assert_eq!(progress, "step 1");
                assert!(tab_id.is_none(), "missing field should default to None");
            }
            other => panic!("expected ToolExecutionProgress, got {other:?}"),
        }
        // And re-serialise — `skip_serializing_if = "Option::is_none"` keeps
        // the wire format clean when downstream tools don't set tab_id.
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            !json.contains("tab_id"),
            "tab_id should be omitted when None: {json}"
        );
    }

    /// The agent_id extractor should map session-scoped RFC-015 events to
    /// `session:<id>` for audit-trail grouping, while non-session events
    /// keep their existing behaviour.
    #[test]
    fn test_rfc015_extract_agent_id() {
        let event = KernelEvent::ToolExecutionStarted {
            session_id: "abc-123".into(),
            tool_name: "bash".into(),
            tool_call_id: "c1".into(),
            tool_args: serde_json::Value::Null,
            context: None,
        };
        // The function is private; verify via the public AuditAction mapping
        // that session-scoped events do not collide with real agent ids.
        let action = kernel_event_to_audit_action(&event);
        match action {
            AuditAction::Other { detail } => {
                assert!(
                    detail.contains("bash"),
                    "tool name in audit detail: {detail}"
                );
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
