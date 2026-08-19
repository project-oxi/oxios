use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, Sse};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt as TokioStreamExt;
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

use crate::api::error::AppError;
use crate::api::routes::{PageParams, paginate};
use crate::api::server::AppState;

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Session summary for listing (lightweight version without full history).
#[derive(Debug, Serialize, Clone)]
pub(crate) struct SessionListItem {
    id: String,
    user_id: String,
    project_id: Option<String>,
    /// RFC-035: parent session ID for thread sub-conversations.
    parent_session_id: Option<String>,
    message_count: usize,
    title: Option<String>,
    created_at: String,
    updated_at: String,
}

/// RFC-025: Body for moving a session to a Project (drag-to-reparent).
#[derive(Debug, Deserialize)]
pub(crate) struct MoveSessionBody {
    /// Target Project ID, or null to unassign (move to "unfiled").
    pub project_id: Option<String>,
}

/// GET /api/sessions — List recent sessions (paginated).
pub(crate) async fn handle_sessions_list(
    state: State<Arc<AppState>>,
    Query(params): Query<PageParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.kernel.state.list_sessions().await {
        Ok(sessions) => {
            let items: Vec<SessionListItem> = sessions
                .into_iter()
                .map(|s| SessionListItem {
                    id: s.id,
                    user_id: s.user_id,
                    project_id: s.project_id,
                    parent_session_id: s.parent_session_id,
                    message_count: s.message_count,
                    title: s.title,
                    created_at: s.created_at.to_rfc3339(),
                    updated_at: s.updated_at.to_rfc3339(),
                })
                .collect();
            Ok(Json(paginate(&items, &params)))
        }
        Err(e) => {
            tracing::warn!(error = %e, "list_sessions failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// GET /api/sessions/:id — Get session with full message history.
pub(crate) async fn handle_session_get(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    use oxios_kernel::state_store::SessionId;
    let session_id = SessionId(id);
    match state.kernel.state.load_session(&session_id).await {
        Ok(Some(session)) => Ok(Json(serde_json::json!({
           "id": session.id.0,
           "user_id": session.user_id,
           // RFC-025: top-level field first, legacy metadata fallbacks.
           "project_id": session.project_id.clone()
               .or_else(|| session.metadata.get("project_id").and_then(|v| v.as_str()).map(String::from))
               .or_else(|| session.metadata.get("project_ids").and_then(|v| v.as_str()).map(String::from)),
           "parent_session_id": session.parent_session_id,
           "user_messages": session.user_messages,
           "agent_responses": session.agent_responses,
           "active_persona_id": session.active_persona_id,
           "created_at": session.created_at.to_rfc3339(),
           "updated_at": session.updated_at.to_rfc3339(),
           "metadata": session.metadata,
           // RFC-015: trajectory for chat transparency replay.
           "trajectory_steps": session.trajectory_steps,
           // P4 (§7 persistence): reasoning text for the ThinkingPanel.
           "reasoning_records": session.reasoning_records,
           // Context compression: LLM-generated summary of older messages.
           "compression": session.metadata.get("compression"),
        }))),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// DELETE /api/sessions/:id — Delete a session.
pub(crate) async fn handle_session_delete(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    use oxios_kernel::state_store::SessionId;
    let session_id = SessionId(id);
    match state.kernel.state.delete_session(&session_id).await {
        Ok(true) => Ok(Json(serde_json::json!({
            "status": "deleted",
            "id": session_id.0,
        }))),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// RFC-035: List child sessions (threads) of the given parent.
/// GET /api/sessions/:id/threads
pub(crate) async fn handle_session_threads(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.kernel.state.list_child_sessions(&id).await {
        Ok(threads) => {
            let items: Vec<SessionListItem> = threads
                .into_iter()
                .map(|s| SessionListItem {
                    id: s.id,
                    user_id: s.user_id,
                    project_id: s.project_id,
                    parent_session_id: s.parent_session_id,
                    message_count: s.message_count,
                    title: s.title,
                    created_at: s.created_at.to_rfc3339(),
                    updated_at: s.updated_at.to_rfc3339(),
                })
                .collect();
            Ok(Json(serde_json::json!({ "threads": items })))
        }
        Err(e) => {
            tracing::warn!(error = %e, "list_child_sessions failed");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// RFC-035: Create a new thread under the given parent session.
/// POST /api/sessions/:id/threads → { thread_id, session_id }
pub(crate) async fn handle_session_create_thread(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    match state.kernel.state.create_thread(&id, "default").await {
        Ok(session) => Ok(Json(serde_json::json!({
            "status": "created",
            "thread_id": session.id.0,
            "session_id": session.id.0,
            "parent_session_id": session.parent_session_id,
        }))),
        Err(e) => Err(AppError::Internal(format!("Failed to create thread: {e}"))),
    }
}

/// PATCH /api/sessions/:id/project — Move a session to a different Project
/// (RFC-025 drag-to-reparent). Body: `{ "project_id": "<uuid>" | null }`.
pub(crate) async fn handle_session_move(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    body: axum::extract::Json<MoveSessionBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    use oxios_kernel::state_store::SessionId;
    let session_id = SessionId(id);
    let project_id = body.project_id.as_deref();
    match state
        .kernel
        .state
        .move_session_to_project(&session_id, project_id)
        .await
    {
        Ok(true) => Ok(Json(serde_json::json!({
            "status": "moved",
            "id": session_id.0,
            "project_id": project_id,
        }))),
        Ok(false) => Err(AppError::NotFound("Session not found".into())),
        Err(e) => Err(AppError::Internal(format!("Failed to move session: {e}"))),
    }
}

/// POST /api/sessions/:id/compress — Trigger LLM compression for a session.
pub(crate) async fn handle_session_compress(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let compression = state
        .kernel
        .compression
        .as_ref()
        .ok_or_else(|| AppError::Internal("compression not available".into()))?;

    let sid = oxios_kernel::state_store::SessionId(id.clone());
    let session = state
        .kernel
        .state
        .load_session(&sid)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("session not found".into()))?;

    if !compression.should_compress(&session) {
        return Ok(Json(serde_json::json!({
            "status": "skipped",
            "reason": "session does not meet compression threshold or is already compressed"
        })));
    }

    compression.spawn_compress(id);
    Ok(Json(serde_json::json!({"status": "started"})))
}

/// POST /api/sessions/prune — Prune sessions based on config.
///
/// Removes sessions that exceed TTL or exceed the maximum count.
pub(crate) async fn handle_sessions_prune(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    use oxios_kernel::state_store::PruneConfig;
    let prune_config = {
        let cfg = state.config.read();
        PruneConfig {
            max_sessions: cfg.session.max_sessions,
            ttl_hours: cfg.session.ttl_hours,
        }
    }; // cfg guard dropped here

    let pruned = state
        .kernel
        .state
        .prune_sessions(&prune_config)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "pruned",
        "count": pruned,
        "config": {
            "max_sessions": prune_config.max_sessions,
            "ttl_hours": prune_config.ttl_hours,
        },
    })))
}

// ---------------------------------------------------------------------------
// Events (SSE)
// ---------------------------------------------------------------------------

/// GET /api/events — SSE stream of KernelEvent.
pub(crate) async fn handle_events(
    state: State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let receiver = state.kernel.infra.subscribe();
    // RFC-024 §11: count SSE connection opens. The server cannot reliably
    // observe the close (client disconnect arrives as TCP RST without an
    // application-level signal), so we expose opens only and let the gauge
    // of "in-flight subscribers" live in the broadcast layer.
    oxios_kernel::metrics::get_metrics()
        .sse_connections_open
        .inc();
    let stream = BroadcastStream::new(receiver);
    let stream = TokioStreamExt::filter_map(stream, |result| {
        match result {
            Ok(event) => {
                // Sanitize events: include type and basic metadata only.
                // Detailed data (full directive content, LLM responses) is excluded.
                let sanitized = sanitize_event(&event);
                // RFC-024 SP2: attach the sanitized event's `id` as the SSE
                // event id so the browser (or fetch client) can set
                // `Last-Event-ID` on reconnect. The server treats the header
                // as advisory — it does not maintain a replay buffer for
                // kernel events (those are stateless and a reconnecting
                // client pulls fresh state via `/api/status`).
                let id = sanitized
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_default();
                let data = serde_json::to_string(&sanitized).unwrap_or_default();
                let mut ev = SseEvent::default();
                if !id.is_empty() {
                    ev = ev.id(id);
                }
                Some(Ok(ev.data(data)))
            }
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                // RFC-024 SP2: lag used to be silently dropped (None). It is
                // now a first-class resync signal so the client knows it
                // has missed events and should pull state via the regular
                // HTTP API before continuing.
                let resync = serde_json::json!({
                    "type": "resync",
                    "lagged": n,
                });
                let data = serde_json::to_string(&resync).unwrap_or_default();
                Some(Ok(SseEvent::default().event("resync").data(data)))
            }
        }
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(30))
            .text("ping"),
    )
}

/// Sanitize a kernel event for SSE broadcast.
/// Returns only the event type and non-sensitive metadata.
pub(crate) fn sanitize_event(event: &oxios_kernel::event_bus::KernelEvent) -> serde_json::Value {
    use oxios_kernel::event_bus::KernelEvent;
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let base = serde_json::json!({
        "id": id,
        "timestamp": now,
    });
    let payload = match event {
        KernelEvent::AgentCreated { id, name, .. } => serde_json::json!({
            "type": "agent_created",
            "agent_id": id.to_string(),
            "name": name,
        }),
        KernelEvent::AgentStarted { id, .. } => serde_json::json!({
            "type": "agent_started",
            "agent_id": id.to_string(),
        }),
        KernelEvent::AgentStopped { id, success, .. } => serde_json::json!({
            "type": "agent_stopped",
            "agent_id": id.to_string(),
            "success": success,
        }),
        KernelEvent::AgentFailed { id, error, .. } => serde_json::json!({
            "type": "agent_failed",
            "agent_id": id.to_string(),
            "error": error,
        }),
        KernelEvent::MessageReceived { from, .. } => serde_json::json!({
            "type": "message_received",
            "from": from.to_string(),
            // content excluded — may contain sensitive data
        }),
        KernelEvent::AgentOutput {
            session_id,
            agent_id,
            ..
        } => serde_json::json!({
            "type": "agent_output",
            "session_id": session_id,
            "agent_id": agent_id.to_string(),
            // content excluded
        }),
        KernelEvent::ApprovalRequested {
            id,
            tool_name,
            reason,
            session_id,
            ..
        } => serde_json::json!({
            "type": "approval_requested",
            "id": id.to_string(),
            "tool_name": tool_name,
            "reason": reason,
            "session_id": session_id,
        }),
        KernelEvent::PathAccessRequested {
            id,
            tool_name,
            path,
            mode,
            agent_name,
            reason,
            session_id,
        } => serde_json::json!({
            "type": "path_access_requested",
            "id": id.to_string(),
            "tool_name": tool_name,
            "path": path,
            "mode": mode,
            "agent_name": agent_name,
            "reason": reason,
            "session_id": session_id,
        }),
        KernelEvent::ApprovalResolved { id, approved } => serde_json::json!({
            "type": "approval_resolved",
            "id": id.to_string(),
            "approved": approved,
        }),
        KernelEvent::AgentGroupCreated {
            group_id,
            agent_count,
        } => serde_json::json!({
            "type": "agent_group_created",
            "group_id": group_id.to_string(),
            "agent_count": agent_count,
        }),
        KernelEvent::AgentGroupMemberCompleted {
            group_id,
            agent_id,
            success,
        } => serde_json::json!({
            "type": "agent_group_member_completed",
            "group_id": group_id.to_string(),
            "agent_id": agent_id.to_string(),
            "success": success,
        }),
        KernelEvent::ProjectCreated {
            project_id,
            name,
            source,
        } => serde_json::json!({
            "type": "project_created",
            "project_id": project_id.to_string(),
            "name": name,
            "source": source,
        }),
        KernelEvent::ProjectActivated { project_id, name } => serde_json::json!({
            "type": "project_activated",
            "project_id": project_id.to_string(),
            "name": name,
        }),
        // ── RFC-015: chat transparency events (forwarded to /api/events too) ──
        KernelEvent::ToolExecutionStarted {
            session_id,
            tool_name,
            tool_call_id,
            tool_args,
            context,
        } => serde_json::json!({
            "type": "tool_started",
            "session_id": session_id,
            "tool_name": tool_name,
            "tool_call_id": tool_call_id,
            "tool_args": tool_args,
            "context": context,
        }),
        KernelEvent::ToolExecutionFinished {
            session_id,
            tool_call_id,
            tool_name,
            duration_ms,
            is_error,
            output_summary,
        } => serde_json::json!({
            "type": "tool_finished",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "duration_ms": duration_ms,
            "is_error": is_error,
            "output_summary": output_summary,
        }),
        KernelEvent::ToolExecutionProgress {
            session_id,
            tool_call_id,
            tool_name,
            progress,
            tab_id,
            context,
        } => {
            let mut obj = serde_json::json!({
                "type": "tool_progress",
                "session_id": session_id,
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "progress": progress,
            });
            if let Some(id) = tab_id {
                obj["tab_id"] = serde_json::json!(id.to_string());
            }
            if let Some(ctx) = context {
                obj["context"] = ctx.clone();
            }
            obj
        }
        KernelEvent::MemoryRecallUsed {
            session_id,
            query,
            count,
            source,
        } => serde_json::json!({
            "type": "memory_recall_used",
            "session_id": session_id,
            "query": query,
            "count": count,
            "source": source,
        }),
        KernelEvent::TokenUsageUpdate {
            session_id,
            input_tokens,
            output_tokens,
        } => serde_json::json!({
            "type": "token_usage_update",
            "session_id": session_id,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        }),
        KernelEvent::ReasoningFragment {
            session_id,
            content,
            source,
        } => serde_json::json!({
            "type": "reasoning_fragment",
            "session_id": session_id,
            "content": content,
            "source": source,
        }),
        KernelEvent::ToolArgsDelta {
            session_id,
            tool_call_id,
            args_delta,
        } => serde_json::json!({
            "type": "tool_call_delta",
            "session_id": session_id,
            "tool_call_id": tool_call_id,
            "args_delta": args_delta,
        }),
        KernelEvent::CompactionTriggered { session_id, source } => serde_json::json!({
            "type": "compaction_triggered",
            "session_id": session_id,
            "source": source,
        }),
        KernelEvent::CalendarEventCreated { uid, title, .. } => serde_json::json!({
            "type": "calendar_event_created",
            "uid": uid,
            "title": title,
        }),
        KernelEvent::CalendarEventUpdated { uid, title } => serde_json::json!({
            "type": "calendar_event_updated",
            "uid": uid,
            "title": title,
        }),
        KernelEvent::CalendarEventDeleted { uid, title } => serde_json::json!({
            "type": "calendar_event_deleted",
            "uid": uid,
            "title": title,
        }),
        KernelEvent::MemoCreated { id } => serde_json::json!({
            "type": "memo_created",
            "id": id,
        }),
        KernelEvent::MemoDeleted { id } => serde_json::json!({
            "type": "memo_deleted",
            "id": id,
        }),
        KernelEvent::EmailSent {
            subject,
            message_id,
            template_name,
        } => serde_json::json!({
            "type": "email_sent",
            "subject": subject,
            "message_id": message_id,
            "template_name": template_name,
        }),
        KernelEvent::KnowledgePersisted {
            session_id,
            message_index,
            path,
            source,
        } => serde_json::json!({
            "type": "knowledge_persisted",
            "session_id": session_id,
            "message_index": message_index,
            "path": path,
            "source": source,
        }),
        KernelEvent::KnowledgeRemoved {
            session_id,
            message_index,
        } => serde_json::json!({
            "type": "knowledge_removed",
            "session_id": session_id,
            "message_index": message_index,
        }),
        KernelEvent::AskUserRequest {
            id,
            question,
            options,
        } => serde_json::json!({
            "type": "ask_user_request",
            "id": id,
            "question": question,
            "options": options,
        }),
        KernelEvent::PersonaCreated {
            id,
            name,
            enabled,
            source,
        } => serde_json::json!({
            "type": "persona_created",
            "id": id,
            "name": name,
            "enabled": enabled,
            "source": source,
        }),
        KernelEvent::PersonaUpdated { id, name, source } => serde_json::json!({
            "type": "persona_updated",
            "id": id,
            "name": name,
            "source": source,
        }),
        KernelEvent::IntegrationInstallStarted {
            job_id,
            integration_id,
            label,
        } => serde_json::json!({
            "type": "integration_install_started",
            "job_id": job_id,
            "integration_id": integration_id,
            "label": label,
        }),
        KernelEvent::IntegrationInstallProgress {
            job_id,
            integration_id,
            line,
        } => serde_json::json!({
            "type": "integration_install_progress",
            "job_id": job_id,
            "integration_id": integration_id,
            "line": line,
        }),
        KernelEvent::IntegrationInstallCompleted {
            job_id,
            integration_id,
            command,
            output,
            exit_code,
        } => serde_json::json!({
            "type": "integration_install_completed",
            "job_id": job_id,
            "integration_id": integration_id,
            "command": command,
            "output": output,
            "exit_code": exit_code,
        }),
        KernelEvent::IntegrationInstallFailed {
            job_id,
            integration_id,
            error,
        } => serde_json::json!({
            "type": "integration_install_failed",
            "job_id": job_id,
            "integration_id": integration_id,
            "error": error,
        }),
        // Compression events are delivered via WS, not SSE.
        KernelEvent::CompressionDelta { .. }
        | KernelEvent::CompressionDone { .. }
        | KernelEvent::CompressionFailed { .. } => serde_json::json!({
            "type": "compression",
        }),
    };
    // Merge payload into base
    if let serde_json::Value::Object(mut map) = base {
        if let serde_json::Value::Object(payload_map) = payload {
            for (k, v) in payload_map {
                map.insert(k, v);
            }
        }
        serde_json::Value::Object(map)
    } else {
        payload
    }
}
// ---------------------------------------------------------------------------

/// Approval request for the API response.
#[derive(Debug, Serialize)]
pub(crate) struct ApprovalResponse {
    id: String,
    subject: String,
    action: String,
    resource: String,
    reason: String,
    created_at: String,
    status: String,
}

/// GET /api/approvals — List all approval requests (pending + history).
pub(crate) async fn handle_approvals_list(
    state: State<Arc<AppState>>,
) -> Json<Vec<ApprovalResponse>> {
    let approvals: Vec<ApprovalResponse> = state
        .kernel
        .security
        .list_approvals()
        .iter()
        .map(|(p, s)| {
            let subject_str = match &p.subject {
                oxios_kernel::access_manager::Subject::User(n) => format!("user:{n}"),
                oxios_kernel::access_manager::Subject::Agent(id) => format!("agent:{id}"),
                oxios_kernel::access_manager::Subject::System => "system".into(),
            };
            let action_str = match &p.action {
                oxios_kernel::access_manager::Action::UseTool(t) => format!("use_tool:{t}"),
                oxios_kernel::access_manager::Action::AccessPath(p) => format!("access_path:{p}"),
                oxios_kernel::access_manager::Action::ManageAgents => "manage_agents".into(),
                oxios_kernel::access_manager::Action::ManagePrograms => "manage_programs".into(),
                oxios_kernel::access_manager::Action::ManageWorkspaces => {
                    "manage_workspaces".into()
                }
                oxios_kernel::access_manager::Action::ManageRBAC => "manage_rbac".into(),
                oxios_kernel::access_manager::Action::ViewAuditLog => "view_audit_log".into(),
                oxios_kernel::access_manager::Action::SystemConfig => "system_config".into(),
            };
            let status_str = match s {
                oxios_kernel::access_manager::ApprovalStatus::Pending => "pending",
                oxios_kernel::access_manager::ApprovalStatus::Approved => "approved",
                oxios_kernel::access_manager::ApprovalStatus::Rejected => "rejected",
                oxios_kernel::access_manager::ApprovalStatus::Expired => "expired",
            };
            ApprovalResponse {
                id: p.id.to_string(),
                subject: subject_str,
                action: action_str,
                resource: p.resource.clone(),
                reason: p.reason.clone(),
                created_at: p.created_at.to_rfc3339(),
                status: status_str.to_string(),
            }
        })
        .collect();
    Json(approvals)
}

/// POST /api/approvals/:id/approve — Approve a pending request.
pub(crate) async fn handle_approval_approve(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return Err(StatusCode::BAD_REQUEST),
    };

    if state.kernel.security.approve(uuid) {
        tracing::info!(approval_id = %uuid, "Approval granted");
        // Publish event so SSE clients update automatically
        let _ =
            state
                .kernel
                .infra
                .publish(oxios_kernel::event_bus::KernelEvent::ApprovalResolved {
                    id: uuid,
                    approved: true,
                });
        Ok(Json(serde_json::json!({
            "status": "approved",
            "id": id,
        })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// POST /api/approvals/:id/reject — Reject a pending request.
pub(crate) async fn handle_approval_reject(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return Err(StatusCode::BAD_REQUEST),
    };

    if state.kernel.security.reject(uuid) {
        tracing::info!(approval_id = %uuid, "Approval rejected");
        // Publish event so SSE clients update automatically
        let _ =
            state
                .kernel
                .infra
                .publish(oxios_kernel::event_bus::KernelEvent::ApprovalResolved {
                    id: uuid,
                    approved: false,
                });
        Ok(Json(serde_json::json!({
            "status": "rejected",
            "id": id,
        })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
