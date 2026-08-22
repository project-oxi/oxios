use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query};
use axum::extract::{
    State, WebSocketUpgrade,
    ws::{Message, WebSocket},
};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt as FuturesStreamExt};
use serde::{Deserialize, Serialize};

use oxios_gateway::message::IncomingMessage;

use crate::api::bridge::BridgeSendError;
use crate::api::error::AppError;
use crate::api::server::AppState;

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

/// Request body for the chat endpoint.
#[derive(Debug, Deserialize)]
pub(crate) struct ChatRequest {
    /// The user's message content.
    #[serde(alias = "message")]
    content: String,
    /// Optional user identifier (defaults to "default").
    #[serde(default = "default_user")]
    user_id: String,
    /// Optional session ID for multi-turn conversations.
    #[serde(default)]
    session_id: String,
    /// Optional space ID for context partitioning.
    #[serde(default)]
    project_id: String,
    /// RFC-025: comma-separated Mount IDs to bind (primary first).
    #[serde(default)]
    mount_ids: String,
    /// Optional model override. Populates the gateway `model_override`
    /// metadata so the orchestrator forwards it as `ExecEnv::model_override`.
    #[serde(default)]
    model: Option<String>,
    /// Per-message sampling temperature (0.0–2.0). Inserted into gateway
    /// metadata and threaded through `MsgCtx` → `ExecEnv::model_params`.
    #[serde(default)]
    temperature: Option<f64>,
    /// Per-message max output tokens. Inserted into gateway metadata and
    /// threaded through `MsgCtx` → `ExecEnv::model_params`. Optional;
    /// when `None`, the agent runtime falls back to its 8192 default.
    #[serde(default)]
    max_tokens: Option<u32>,
    /// Session-scoped persona override. Validated against the persona
    /// registry, then inserted into gateway metadata so the orchestrator
    /// forwards it as `ExecEnv::persona_id`. Empty → inherit the global
    /// active persona.
    #[serde(default)]
    persona_id: String,
}

pub(crate) fn default_user() -> String {
    "default".into()
}

/// Response body for the chat endpoint.
#[derive(Debug, Serialize)]
pub(crate) struct ChatResponse {
    /// The message ID.
    id: String,
    /// Echo of the user's message.
    echo: String,
    /// The response from the orchestrator.
    reply: String,
    /// Session ID for multi-turn conversations.
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// Space ID for context partitioning.
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    /// Phase reached during orchestration.
    #[serde(skip_serializing_if = "Option::is_none")]
    phase: Option<String>,
    /// RFC-014: Space tag decoration.
    #[serde(skip_serializing_if = "Option::is_none")]
    project_tag: Option<String>,
    /// RFC-025: active Mount IDs (comma-separated), primary first.
    #[serde(skip_serializing_if = "Option::is_none")]
    mount_ids: Option<String>,
    /// RFC-025: Mount decoration tag (e.g. "[🔧 oxios + oxicode-sdk]").
    #[serde(skip_serializing_if = "Option::is_none")]
    mount_tag: Option<String>,
    /// RFC-014: Evaluation passed.
    #[serde(skip_serializing_if = "Option::is_none")]
    evaluation_passed: Option<bool>,
    /// RFC-014: Duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

/// POST /api/chat — Send a message to the kernel via gateway and get response.
pub(crate) async fn handle_chat(
    state: State<Arc<AppState>>,
    Json(body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, AppError> {
    // Validate chat content size (max 64KB for user message)
    const MAX_CHAT_LENGTH: usize = 64 * 1024;
    if body.content.len() > MAX_CHAT_LENGTH {
        return Err(AppError::PayloadTooLarge {
            size: body.content.len(),
            limit: MAX_CHAT_LENGTH,
        });
    }
    tracing::info!(
        content_len = body.content.len(),
        content_preview = %body.content.chars().take(50).collect::<String>(),
        user = %body.user_id,
        "Chat message received"
    );

    // Build the incoming message.
    let mut msg = IncomingMessage::new("web", &body.user_id, &body.content);

    // Include session_id from request if provided (for multi-turn conversations).
    if !body.session_id.is_empty() {
        msg.metadata
            .insert("session_id".to_owned(), body.session_id.clone());
    }

    // RFC-025: Request project_id drives both context (orchestrator) and
    // session grouping (sidebar tree). Capture it for session persistence.
    let request_project_id = if !body.project_id.is_empty() {
        Some(body.project_id.clone())
    } else {
        None
    };
    if let Some(ref pid) = request_project_id {
        msg.metadata.insert("project_ids".to_owned(), pid.clone());
    }
    // RFC-025: include mount_ids from request (multi-path injection).
    if !body.mount_ids.is_empty() {
        msg.metadata
            .insert("mount_ids".to_owned(), body.mount_ids.clone());
    }

    // Per-message model override. Carried via gateway metadata to the
    // orchestrator, which forwards it as ExecEnv::model_override (highest
    // precedence in agent_runtime model resolution). Validate early so an
    // unknown model ID fails immediately instead of after assess/crystallize.
    if let Some(m) = &body.model {
        state
            .kernel
            .engine
            .validate_model(m)
            .map_err(AppError::BadRequest)?;
        msg.metadata.insert("model_override".to_owned(), m.clone());
    }

    // Session-scoped persona override: validate against the registry the
    // same way the WS ingress does — unknown/disabled personas are a
    // 400 rather than a silent global fallback.
    let request_persona_id = if !body.persona_id.is_empty() {
        Some(body.persona_id.clone())
    } else {
        None
    };
    if let Some(pid) = &request_persona_id {
        match state.kernel.persona.get(pid) {
            Some(p) if p.enabled => {}
            Some(_) => {
                return Err(AppError::BadRequest(format!("Persona '{pid}' is disabled")));
            }
            None => {
                return Err(AppError::BadRequest(format!("Persona '{pid}' not found")));
            }
        }
        msg.metadata.insert("persona_id".to_owned(), pid.clone());
    }
    if let Some(t) = body.temperature {
        msg.metadata.insert("temperature".to_owned(), t.to_string());
    }
    if let Some(mt) = body.max_tokens {
        msg.metadata.insert("max_tokens".to_owned(), mt.to_string());
    }

    let msg_id = msg.id.to_string();
    let content_echo = body.content.clone();

    // Send and wait for response from the gateway pipeline.
    tracing::info!("Sending message to gateway...");
    match state.bridge.send_and_wait(msg).await {
        Ok(response) => {
            tracing::info!(reply_len = response.content.len(), "Chat response received");

            // Extract from typed meta (RFC-014) or fall back to metadata HashMap
            let meta = response.meta.as_ref();
            let session_id = meta
                .and_then(|m| m.session_id.clone())
                .or_else(|| response.metadata.get("session_id").cloned());
            let project_id = meta
                .and_then(|m| m.project_id.clone())
                .or_else(|| response.metadata.get("project_ids").cloned());
            let phase = meta
                .map(|m| m.phase.clone())
                .or_else(|| response.metadata.get("phase").cloned());
            let project_tag = meta.and_then(|m| m.project_tag.clone());
            // RFC-025: active Mount IDs + tag (from channel metadata set by the gateway).
            let mount_ids = response.metadata.get("mount_ids").cloned();
            let mount_tag = response.metadata.get("mount_tag").cloned();
            let evaluation_passed = meta.and_then(|m| m.evaluation_passed);
            let duration_ms = meta.and_then(|m| m.duration_ms);

            // RFC-015: parse tool_calls into trajectory step records.
            let trajectory_steps: Vec<oxios_kernel::state_store::TrajectoryStepRecord> = response
                .metadata
                .get("tool_calls")
                .and_then(|v| serde_json::from_str::<Vec<serde_json::Value>>(v).ok())
                .map(|calls| {
                    calls
                        .into_iter()
                        .enumerate()
                        .map(|(i, c)| oxios_kernel::state_store::TrajectoryStepRecord {
                            tool_name: c
                                .get("tool")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            tool_args: c.get("input").cloned().unwrap_or(serde_json::Value::Null),
                            output_summary: c
                                .get("output")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            duration_ms: c.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
                            is_error: false,
                            tool_call_id: format!("legacy-{i}"),
                            timestamp: chrono::Utc::now(),
                        })
                        .collect()
                })
                .unwrap_or_default();

            let session_id_for_save = session_id.clone().unwrap_or_else(|| msg_id.clone());
            let sid = oxios_kernel::state_store::SessionId(session_id_for_save.clone());
            match state.kernel.state.load_session(&sid).await {
                Ok(Some(mut session)) => {
                    session.add_user_message(&content_echo);
                    // Capture existing trajectory length before extending
                    let traj_start = session.trajectory_steps.len();
                    session.extend_trajectory(trajectory_steps);
                    // P4 (§7 persistence): persist reasoning text from
                    // terminal OutgoingMessage metadata alongside trajectory.
                    if let Some(rt) = response.metadata.get("reasoning_text").cloned()
                        && !rt.is_empty()
                    {
                        session.add_reasoning(oxios_kernel::state_store::ReasoningRecord {
                            content: rt,
                            source: "thinking".to_string(),
                            timestamp: chrono::Utc::now(),
                            segments: response
                                .metadata
                                .get("reasoning_segments")
                                .and_then(|s| {
                                    serde_json::from_str::<Vec<oxios_ouroboros::ReasoningSegment>>(
                                        s,
                                    )
                                    .ok()
                                })
                                .unwrap_or_default(),
                        });
                    }
                    let traj_end = session.trajectory_steps.len();
                    session.add_agent_response(oxios_kernel::state_store::AgentResponse {
                        content: response.content.clone(),
                        session_id: Some(sid.0.clone()),
                        phase_reached: phase.clone(),
                        evaluation_passed,
                        timestamp: chrono::Utc::now(),
                        trajectory_range: if traj_end > traj_start {
                            Some(oxios_kernel::state_store::TrajectoryRange {
                                start: traj_start,
                                end: traj_end,
                            })
                        } else {
                            None
                        },
                    });
                    // RFC-025: Set top-level project_id for grouping.
                    // User-requested project_id takes priority; otherwise
                    // keep the existing grouping.
                    if let Some(ref pid) = request_project_id {
                        session.project_id = Some(pid.clone());
                    }
                    // Session persona override outlives the turn: persist it
                    // so GET /api/sessions/:id and the persona-affordance
                    // hook see the session's persona on reopen.
                    if let Some(ref pid) = request_persona_id {
                        session.active_persona_id = Some(pid.clone());
                    }
                    if let Err(e) = state.kernel.state.save_session(&session).await {
                        tracing::warn!(error = %e, "Failed to persist session");
                    }
                }
                Ok(None) => {
                    // Create new session
                    let mut session = oxios_kernel::state_store::Session::new(body.user_id.clone());
                    session.id = oxios_kernel::state_store::SessionId(session_id_for_save);
                    session.add_user_message(&content_echo);
                    // New session: trajectory starts at 0
                    let traj_start = 0usize;
                    session.extend_trajectory(trajectory_steps);
                    // P4 (§7 persistence): persist reasoning text from
                    // terminal OutgoingMessage metadata alongside trajectory.
                    if let Some(rt) = response.metadata.get("reasoning_text").cloned()
                        && !rt.is_empty()
                    {
                        session.add_reasoning(oxios_kernel::state_store::ReasoningRecord {
                            content: rt,
                            source: "thinking".to_string(),
                            timestamp: chrono::Utc::now(),
                            segments: response
                                .metadata
                                .get("reasoning_segments")
                                .and_then(|s| {
                                    serde_json::from_str::<Vec<oxios_ouroboros::ReasoningSegment>>(
                                        s,
                                    )
                                    .ok()
                                })
                                .unwrap_or_default(),
                        });
                    }
                    let traj_end = session.trajectory_steps.len();
                    session.add_agent_response(oxios_kernel::state_store::AgentResponse {
                        content: response.content.clone(),
                        session_id: Some(sid.0.clone()),
                        phase_reached: phase.clone(),
                        evaluation_passed,
                        timestamp: chrono::Utc::now(),
                        trajectory_range: if traj_end > traj_start {
                            Some(oxios_kernel::state_store::TrajectoryRange {
                                start: traj_start,
                                end: traj_end,
                            })
                        } else {
                            None
                        },
                    });
                    // RFC-025: Set top-level project_id for grouping.
                    if let Some(ref pid) = request_project_id {
                        session.project_id = Some(pid.clone());
                    }
                    if let Some(ref pid) = request_persona_id {
                        session.active_persona_id = Some(pid.clone());
                    }
                }
                Err(e) => tracing::warn!(error = %e, "Failed to load/create session"),
            }

            // Auto-prune sessions if configured (throttled to once per hour)
            let cfg = state.config.read();
            if cfg.session.auto_prune && state.kernel.state.should_auto_prune() {
                let prune_config = oxios_kernel::state_store::PruneConfig {
                    max_sessions: cfg.session.max_sessions,
                    ttl_hours: cfg.session.ttl_hours,
                };
                drop(cfg); // release read lock before async
                let kernel = state.kernel.clone();
                tokio::spawn(async move {
                    if let Err(e) = kernel.state.prune_sessions(&prune_config).await {
                        tracing::warn!(error = %e, "Session auto-prune failed");
                    }
                });
            }

            Ok(Json(ChatResponse {
                id: msg_id,
                echo: content_echo,
                reply: response.content,
                session_id: session_id.clone(),
                project_id: project_id.clone(),
                phase,
                project_tag,
                mount_ids,
                mount_tag,
                evaluation_passed,
                duration_ms,
            }))
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to get response from gateway");
            // RFC-024 C1 / F14: distinguish timeout (504) from other
            // failures (500) by error variant, not by grepping the error
            // message. A string-match classification would silently
            // regress to 500 if the message is ever reworded or wrapped.
            match e {
                BridgeSendError::Timeout => Err(AppError::GatewayTimeout(e.to_string())),
                BridgeSendError::SendFailed(_) | BridgeSendError::ChannelDropped => {
                    Err(AppError::Internal("gateway response failed".into()))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// One-shot promote: seed a session from a captured exchange (no inference)
// ---------------------------------------------------------------------------

/// Request body for `POST /api/chat/seed` — persist a captured one-shot
/// exchange (user message + agent response) as a new session, with no
/// gateway/orchestrator call. Used by the QuickAsk "promote to chat" flow.
#[derive(Debug, Deserialize)]
pub(crate) struct SeedRequest {
    pub user_message: String,
    pub agent_response: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub trajectory_steps: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub reasoning_text: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SeedResponse {
    pub session_id: String,
}

/// POST /api/chat/seed — Persist a captured one-shot exchange as a new session.
///
/// Writes the user message + agent response verbatim (plus any captured
/// trajectory/reasoning) directly to the StateStore with no gateway call. This
/// is what makes QuickAsk "promote to chat" instant and drift-free: the user
/// sees the exact answer they read in the dialog, now in a real session.
pub(crate) async fn handle_chat_seed(
    state: State<Arc<AppState>>,
    Json(body): Json<SeedRequest>,
) -> Result<Json<SeedResponse>, AppError> {
    const MAX_SEED_LENGTH: usize = 64 * 1024;
    if body.user_message.len() > MAX_SEED_LENGTH || body.agent_response.len() > MAX_SEED_LENGTH {
        return Err(AppError::PayloadTooLarge {
            size: body.user_message.len().max(body.agent_response.len()),
            limit: MAX_SEED_LENGTH,
        });
    }

    let mut session = oxios_kernel::state_store::Session::new("default".to_string());
    session.add_user_message(&body.user_message);

    // Trajectory steps (tool calls) if the one-shot captured them.
    if let Some(steps_raw) = body.trajectory_steps {
        let steps: Vec<oxios_kernel::state_store::TrajectoryStepRecord> = steps_raw
            .into_iter()
            .enumerate()
            .map(|(i, c)| oxios_kernel::state_store::TrajectoryStepRecord {
                tool_name: c
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                tool_args: c.get("input").cloned().unwrap_or(serde_json::Value::Null),
                output_summary: c
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                duration_ms: c.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
                is_error: false,
                tool_call_id: format!("seed-{i}"),
                timestamp: chrono::Utc::now(),
            })
            .collect();
        session.extend_trajectory(steps);
    }

    // Reasoning text if captured.
    if let Some(rt) = body.reasoning_text
        && !rt.is_empty()
    {
        session.add_reasoning(oxios_kernel::state_store::ReasoningRecord {
            content: rt,
            source: "thinking".to_string(),
            timestamp: chrono::Utc::now(),
            segments: Vec::new(),
        });
    }

    session.add_agent_response(oxios_kernel::state_store::AgentResponse {
        content: body.agent_response,
        session_id: Some(session.id.0.clone()),
        phase_reached: None,
        evaluation_passed: None,
        timestamp: chrono::Utc::now(),
        trajectory_range: None,
    });

    if let Some(ref pid) = body.project_id
        && !pid.is_empty()
    {
        session.project_id = Some(pid.clone());
    }

    let session_id = session.id.0.clone();
    if let Err(e) = state.kernel.state.save_session(&session).await {
        tracing::warn!(error = %e, "Failed to persist seeded session");
        return Err(AppError::Internal(
            "failed to persist seeded session".into(),
        ));
    }

    tracing::info!(session_id = %session_id, "Seeded one-shot session via /api/chat/seed");
    Ok(Json(SeedResponse { session_id }))
}

/// Query parameters for WebSocket connections.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct WsParams {
    /// One-time ticket for authentication (preferred).
    ticket: Option<String>,
    /// Bearer token for authentication (fallback).
    token: Option<String>,
}

/// POST /api/chat/ticket — Generate a one-time WebSocket ticket.
pub(crate) async fn handle_chat_ticket(
    state: State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Auth middleware already validated Bearer token if auth is enabled.
    let ticket = state.kernel.security.generate_ws_ticket();
    Ok(Json(serde_json::json!({ "ticket": ticket })))
}

/// GET /api/chat/stream — WebSocket endpoint for real-time chat streaming.
pub(crate) async fn handle_chat_stream(
    ws: WebSocketUpgrade,
    state: State<Arc<AppState>>,
    Query(params): Query<WsParams>,
) -> impl axum::response::IntoResponse {
    // Authenticate if auth is enabled
    if state.config.read().security.auth_enabled {
        let authenticated = if let Some(ref ticket) = params.ticket {
            state.kernel.security.validate_ws_ticket(ticket)
        } else if let Some(ref token) = params.token {
            state.kernel.security.validate_token(token)
        } else {
            false
        };
        if !authenticated {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    }
    ws.on_upgrade(move |socket| handle_chat_websocket(socket, state.0))
}

/// RFC-015 P2: per-turn live-text dedup state for the WS forwarder.
///
/// When the gateway streams per-delta text partials (channels with
/// `Channel::supports_streaming`), the client already received the full
/// response text as accumulated `token` fragments. The terminal
/// `OutgoingMessage` that follows still carries the complete text (legacy
/// terminal contract), and the frontend *appends* every `token` chunk —
/// re-emitting it would render the response twice. This tracker suppresses
/// the redundant terminal token and resets at each terminal boundary
/// (`done` / `error`) so the next turn starts clean. Non-streaming turns
/// (no partials) keep the terminal token: it is their only text delivery.
#[derive(Debug, Default)]
struct TurnTextStreamTracker {
    /// A non-empty text fragment reached the client this turn.
    text_delivered: bool,
}

impl TurnTextStreamTracker {
    /// Record a forwarded streaming text fragment. Empty fragments don't
    /// count: a turn whose deltas were all empty never delivered text, so
    /// its terminal token must still carry the response.
    fn note_text_partial(&mut self, content: &str) {
        if !content.is_empty() {
            self.text_delivered = true;
        }
    }

    /// Whether the terminal full-text `token` chunk is redundant.
    fn terminal_token_redundant(&self) -> bool {
        self.text_delivered
    }

    /// Turn ended (terminal `done`/`error` emitted) — reset for next turn.
    fn reset(&mut self) {
        self.text_delivered = false;
    }

    /// Reset at the start of a turn. The terminal `done`/`error` reset is
    /// the normal path; this guards the case where a turn ends without one
    /// (an abrupt disconnect, or the RFC-049 cancel path), which would
    /// otherwise suppress the NEXT turn's terminal text on the same
    /// connection.
    fn begin_turn(&mut self) {
        self.text_delivered = false;
    }
}

/// RFC-049: resolve the turn key for a `cancel` frame. The frame's
/// `session_id` wins; an empty value counts as absent (matching the recv
/// task's `incoming_session_id` filtering); otherwise fall back to the
/// connection's active session; neither → `None` (the arm no-ops).
fn resolve_cancel_key(
    frame_session: Option<&str>,
    frame_request: Option<&str>,
    active_session: Option<&str>,
) -> Option<String> {
    frame_session
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| frame_request.filter(|s| !s.is_empty()).map(str::to_string))
        .or_else(|| active_session.filter(|s| !s.is_empty()).map(str::to_string))
}

/// Handles a WebSocket connection for chat streaming.
///
/// Protocol:
/// - **Incoming** (frontend → backend):
///   `{ type: "message", content: "...", session_id?: "...", project_id?: "..." }`
/// - **Outgoing token** (backend → frontend):
///   `{ type: "token", content: "...", session_id?, project_id? }`
/// - **Outgoing done** (backend → frontend):
///   `{ type: "done", session_id?, project_id?, phase?, evaluation_passed? }`
pub(crate) async fn handle_chat_websocket(socket: WebSocket, state: Arc<AppState>) {
    // RFC-024 §11: count WS connection opens. The drop counter (`close` /
    // `keepalive_timeout`) is wired at the join site below where the
    // task's exit reason is observable.
    oxios_kernel::metrics::get_metrics()
        .ws_connections_open
        .inc();

    // Assign a unique connection ID for point-to-point message routing.
    // Prevents cross-tab message leakage in multi-session scenarios.
    let conn_id = uuid::Uuid::new_v4().to_string();
    // Clone for recv_task (send_task gets its own clone below).
    let conn_id_for_recv = conn_id.clone();
    let conn_id_for_send = conn_id.clone();
    let (ws_tx, mut ws_rx) = socket.split();
    // RFC-024 SP2 (B3): the sink is shared between the recv_task (which
    // forwards gateway chunks) and a dedicated keepalive task (which sends
    // periodic pings). An `Arc<Mutex<>>` keeps the change tiny: the lock is
    // held only across the actual `send` call (microseconds), and ping
    // frequency is 20 s so contention with token streams is negligible.
    let ws_tx = std::sync::Arc::new(tokio::sync::Mutex::new(ws_tx));
    // RFC-049: connection-scoped "active session" shared between the
    // forwarder task (which writes it whenever it adopts a session id) and
    // the recv task (which reads it in the `cancel` arm to resolve the turn
    // key when the client's cancel frame carries no session id). The
    // forwarder keeps its own local copy for event filtering; this handle is
    // a write-through mirror.
    let active_session_shared: Arc<tokio::sync::Mutex<Option<String>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    // RFC-024 SP2 (B3): keepalive wiring. The send_task notifies this on
    // every `Message::Pong` it reads, extending the keepalive deadline.
    // The keepalive task (spawned below) sends a `Ping` every 20 s and
    // declares the connection dead if no `Pong` arrives within 60 s of
    // the last activity.
    let pong_signal = std::sync::Arc::new(tokio::sync::Notify::new());
    // Set to true when the keepalive task aborts the connection due to a
    // missing pong. The close site reads it to choose between the
    // `close` and `keepalive_timeout` metric labels.
    let keepalive_timed_out = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Subscribe to outgoing messages from the web channel (not kernel event bus).
    // WebChannel::send() broadcasts OutgoingMessage here; the kernel event bus
    // carries KernelEvents which are a different type entirely.
    let mut outgoing_rx = state.bridge.subscribe();
    // RFC-015: subscribe to kernel event bus for real-time chat transparency
    // events (tool execution, token usage, memory recall, reasoning fragments).
    // Filtered by session_id in the recv loop to avoid leaking other agents' events.
    let mut kernel_event_rx = state.kernel.infra.subscribe();

    // Clone handles for the spawned tasks.
    let incoming_tx = state.bridge.incoming_tx.clone();
    let state_store = state.kernel.state.store().clone();

    // Read session prune config
    let prune_config = {
        let cfg = state.config.read();
        if cfg.session.auto_prune {
            Some(oxios_kernel::state_store::PruneConfig {
                max_sessions: cfg.session.max_sessions,
                ttl_hours: cfg.session.ttl_hours,
            })
        } else {
            None
        }
    };

    // Track in-flight user messages for session persistence.
    // The send_task inserts before forwarding to gateway (keyed by message
    // ID); the recv_task removes the matching entry when the response arrives.
    //
    // RFC-025 Web-M3: this is a HashMap, not a single Option slot. A single
    // slot caused the first message + response pair to be silently dropped
    // when a user sent a second message before the first response arrived
    // (the second insert overwrote the first, so the first response's
    // `pending_id == msg_id` check failed and persistence was skipped).
    //
    // Bounded by the number of in-flight requests: each entry is removed by
    // its matching response. If a response never arrives (e.g. agent killed),
    // the entry stays — this is acceptable since in-flight counts are small.
    let pending_user_msg: Arc<tokio::sync::Mutex<HashMap<uuid::Uuid, PendingMessage>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    let pending_for_send = pending_user_msg.clone();
    let bridge_for_resume = state.bridge.clone();

    // ── Forward gateway responses → WebSocket client ──
    //
    // Each chunk carries session_id + project_id so the frontend can
    // maintain multi-turn context. After the "done" chunk we persist
    // the session to disk (same as the POST handler).
    //
    // RFC-015: also forward real-time kernel events (tool execution, token
    // usage, memory recall, reasoning fragments) as WS chunks so the
    // frontend can show live progress.
    // The connection's three concurrent halves (recv / send / keepalive) run in
    // one JoinSet so teardown can drain each task exactly once. The previous
    // form spawned three JoinHandles and polled them by `&mut` inside a
    // `select!`, then `.await`ed them again to drain — the winning handle was
    // double-polled, panicking "JoinHandle polled after completion". Under the
    // release profile (`panic = "abort"`) that aborts the whole daemon on every
    // WebSocket close. JoinSet removes a task as it yields it, so this is
    // structurally impossible to re-trigger.
    let mut set: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
    set.spawn({
        let ws_tx = ws_tx.clone();
        let state = state.clone();
        let active_session_shared = active_session_shared.clone();
        async move {
            // Track the active session so we only forward events tagged with it.
            // Multi-turn conversations keep the same session_id across messages.
            let mut active_session_id: Option<String> = None;
            // Phase A: track the current assistant message id so KernelEvents
            // (which lack message_id attribution) can be stamped with the
            // current stream's message_id at the WS layer.
            let mut active_message_id: Option<uuid::Uuid> = None;
            // RFC-015 P2: dedups the terminal full-text token against the
            // turn's streamed partials (see TurnTextStreamTracker).
            let mut turn_text = TurnTextStreamTracker::default();

            loop {
                tokio::select! {
                    // Bias toward gateway messages (text streaming + done).
                    biased;
                    msg_result = outgoing_rx.recv() => {
                        let Ok(msg) = msg_result else { break };

                        // Filter by target_conn_id: only process messages addressed
                        // to this connection (or broadcast messages with None).
                        if msg.target_conn_id.as_ref().is_some_and(|id| id != &conn_id_for_recv) {
                            continue;
                        }

                        let msg_id = msg.id;
                        let session_id = msg
                            .meta
                            .as_ref()
                            .and_then(|m| m.session_id.clone())
                            .or_else(|| msg.metadata.get("session_id").cloned());
                        let project_id = msg
                            .meta
                            .as_ref()
                            .and_then(|m| m.project_id.clone())
                            .or_else(|| msg.metadata.get("project_ids").cloned());
                        let phase = msg
                            .meta
                            .as_ref()
                            .map(|m| m.phase.clone())
                            .or_else(|| msg.metadata.get("phase").cloned());
                        let evaluation_passed = msg.meta.as_ref().and_then(|m| m.evaluation_passed);
                        let project_tag = msg.meta.as_ref().and_then(|m| m.project_tag.clone());
                        let duration_ms = msg.meta.as_ref().and_then(|m| m.duration_ms);

                        // Remember the session we are forwarding for. Subsequent
                        // kernel events without a session_id are still forwarded
                        // (some events are system-wide).
                        if session_id.is_some() {
                            active_session_id = session_id.clone();
                            // RFC-049: write through to the shared copy so the
                            // recv task's `cancel` arm can fall back to it.
                            *active_session_shared.lock().await = session_id.clone();
                        }
                        if msg.partial == Some(true) || msg.metadata.contains_key("stream_kind") {
                            active_message_id = Some(msg_id);
                        }

                        // RFC-024 SP2 / C2: a synthetic `type: "resync"` message
                        // (broadcast by the bridge when a resume cursor was
                        // older than the replay buffer) is forwarded as a
                        // resync chunk and *skips* persistence / token / done
                        // emission — the client is expected to pull state via
                        // the regular HTTP API after seeing it.
                        if msg.metadata.get("type").map(|v| v.as_str()) == Some("resync") {
                            let chunk = serde_json::json!({"type": "resync"});
                            if ws_tx
                                .lock()
                                .await
                                .send(Message::Text(chunk.to_string().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                            continue;
                        }

                        // RFC-015 P1: partial streaming deltas (per-delta text
                        // chunks from the runtime). They carry only a fragment
                        // of the response, so we must skip persistence (which
                        // would write the fragment as a "full response") and
                        // skip the terminal `done` chunk (which the gateway
                        // emits separately on the terminal OutgoingMessage).
                        // The token chunk itself still forwards so the
                        // frontend's `flushPendingTokens` can accumulate.
                        // A message is "partial" (skip persist_session + the
                        // terminal `done`) when it is either a streamed
                        // fragment OR a transient stream-control marker.
                        // reasoning.start / reasoning.end carry no terminal
                        // content — without this guard each marker would
                        // persist an empty assistant response AND emit a
                        // premature `done`, deleting the frontend's
                        // StreamProcessor mid-reasoning and leaving the
                        // Thinking block stuck. (`model` is already
                        // short-circuited via `continue` above.)
                        let stream_kind = msg.metadata.get("stream_kind").map(|v| v.as_str());
                        let is_partial = msg.partial == Some(true)
                            || matches!(stream_kind, Some("reasoning.start") | Some("reasoning.end"));
                        // RFC-015 model mark: one-shot announcement emitted at
                        // stream start. Forward as a typed `model` chunk and
                        // `continue` — it carries no content, so it must NOT
                        // hit persist_session (would write an empty response),
                        // the token path (empty token), or the terminal `done`
                        // (would terminate the stream before any text).
                        if msg.metadata.get("stream_kind").map(|v| v.as_str()) == Some("model") {
                            // Reset per-turn text-dedup state: the `model` mark is
                            // the first frame of every turn, so this is the safest
                            // place to guard against a prior turn that ended
                            // without a terminal `done`/`error` (abrupt
                            // disconnect, RFC-049 cancel path).
                            turn_text.begin_turn();
                            let model_id = msg
                                .metadata
                                .get("model")
                                .map(|v| v.as_str())
                                .unwrap_or("");
                            let model_chunk = serde_json::json!({
                                "type": "model",
                                "seq": msg.seq,
                                "message_id": msg_id,
                                "model": model_id,
                                "session_id": session_id,
                                "project_id": project_id,
                            });
                            let json = match serde_json::to_string(&model_chunk) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::error!(error = %e, "Failed to serialize model chunk");
                                    continue;
                                }
                            };
                            if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                            continue;
                        }

                        // ── Persist session to disk FIRST ──
                        // Always persist, even if WS send fails later. This ensures
                        // the exchange is durable even if the connection drops mid-stream.
                        //
                        // RFC-025 Web-M3: check-and-remove happen under a single
                        // lock so there is no TOCTOU window between peeking the
                        // pending slot and taking it. The lock is released before
                        // the async persist_session call.
                        if !is_partial && let Some(ref sid) = session_id {
                            let pm = {
                                let mut guard = pending_user_msg.lock().await;
                                guard.remove(&msg_id)
                            };
                            // lock released here, before async I/O
                            if let Some(pm) = pm {
                                persist_session(
                                    &state_store,
                                    sid,
                                    pm.content.as_str(),
                                    pm.user_id.as_str(),
                                    &msg.content,
                                    project_id.as_deref(),
                                    &msg.metadata,
                                    prune_config.clone(),
                                )
                                .await;
                            }
                        }

                        // ── Forward to WebSocket client ──
                        //
                        // RFC-032: when the gateway attached a structured error
                        // (e.g. budget exceeded, quota exhausted), send an error
                        // chunk instead of a token chunk so the frontend can
                        // display a visible error indicator and stop loading.
                        let has_error = msg.meta.as_ref().and_then(|m| m.error.as_ref()).is_some();

                        if has_error {
                            // Audit F-4: `has_error` already confirmed the error is
                            // Some, but assert it (defensive) instead of unwrapping
                            // — under `panic=abort` an unwrap here would kill the
                            // daemon and drop the rest of the stream.
                            let err = match msg.meta.as_ref().and_then(|m| m.error.as_ref()) {
                                Some(e) => e,
                                None => {
                                    tracing::error!(
                                        "chat forward: has_error=true but error missing — inconsistent state"
                                    );
                                    continue;
                                }
                            };
                            let error_chunk = serde_json::json!({
                                "type": "error",
                                "seq": msg.seq,
                                "message_id": msg_id,
                                "message": err.message,
                                "kind": err.kind,
                                "suggestion": err.suggestion,
                                "session_id": session_id,
                                "project_id": project_id,
                            });
                            let json = match serde_json::to_string(&error_chunk) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::error!(error = %e, "Failed to serialize error chunk");
                                    continue;
                                }
                            };
                            if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                            // Terminal error ends the turn.
                            turn_text.reset();
                        } else {
                            let has_interview = msg.meta.as_ref().and_then(|m| m.interview_questions.as_ref()).is_some();
                            let is_reasoning = msg.metadata.get("stream_kind").map(|v| v.as_str()) == Some("reasoning");
                            let is_reasoning_start = msg.metadata.get("stream_kind").map(|v| v.as_str()) == Some("reasoning.start");
                            let is_reasoning_end = msg.metadata.get("stream_kind").map(|v| v.as_str()) == Some("reasoning.end");

                            if has_interview {
                                let interview_chunk = serde_json::json!({
                                    "type": "interview",
                                    "seq": msg.seq,
                                    "message_id": msg_id,
                                    "session_id": session_id,
                                    "project_id": project_id,
                                    "questions": msg.meta.as_ref().and_then(|m| m.interview_questions.clone()),
                                    "round": msg.meta.as_ref().and_then(|m| m.interview_round),
                                });
                                let json = match serde_json::to_string(&interview_chunk) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::error!(error = %e, "Failed to serialize interview chunk");
                                        continue;
                                    }
                                };
                                if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            } else if is_reasoning {
                                let reasoning_chunk = serde_json::json!({
                                    "type": "reasoning",
                                    "seq": msg.seq,
                                    "message_id": msg_id,
                                    "content": msg.content,
                                    "source": "thinking",
                                    "session_id": session_id,
                                    "project_id": project_id,
                                });
                                let json = match serde_json::to_string(&reasoning_chunk) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::error!(error = %e, "Failed to serialize reasoning chunk");
                                        continue;
                                    }
                                };
                                if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            } else if is_reasoning_start {
                                // Phase B: reasoning lifecycle marker — frontend auto-expands Thinking block.
                                let chunk = serde_json::json!({
                                    "type": "reasoning",
                                    "subtype": "start",
                                    "seq": msg.seq,
                                    "message_id": msg_id,
                                    "session_id": session_id,
                                    "project_id": project_id,
                                });
                                let json = match serde_json::to_string(&chunk) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::error!(error = %e, "Failed to serialize reasoning.start chunk");
                                        continue;
                                    }
                                };
                                if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            } else if is_reasoning_end {
                                // Phase B: reasoning lifecycle marker — frontend auto-collapses Thinking block.
                                let chunk = serde_json::json!({
                                    "type": "reasoning",
                                    "subtype": "end",
                                    "seq": msg.seq,
                                    "message_id": msg_id,
                                    "session_id": session_id,
                                    "project_id": project_id,
                                });
                                let json = match serde_json::to_string(&chunk) {
                                    Ok(j) => j,
                                    Err(e) => {
                                        tracing::error!(error = %e, "Failed to serialize reasoning.end chunk");
                                        continue;
                                    }
                                };
                                if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            } else {
                                // RFC-015 P2: when live text partials already
                                // delivered the response this turn, the terminal
                                // message's full-text `token` chunk would
                                // duplicate them (the frontend appends every
                                // token chunk). Skip it; `done` follows below.
                                let skip_terminal_token =
                                    !is_partial && turn_text.terminal_token_redundant();
                                if !skip_terminal_token {
                                    let token_chunk = serde_json::json!({
                                        "type": "token",
                                        "seq": msg.seq,
                                        "message_id": msg_id,
                                        "content": msg.content,
                                        "session_id": session_id,
                                        "project_id": project_id,
                                    });
                                    let json = match serde_json::to_string(&token_chunk) {
                                        Ok(j) => j,
                                        Err(e) => {
                                            tracing::error!(error = %e, "Failed to serialize outgoing message");
                                            continue;
                                        }
                                    };
                                    if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                        break;
                                    }
                                }
                                if is_partial {
                                    turn_text.note_text_partial(&msg.content);
                                }
                            }
                        }
                        if !is_partial && !has_error {
                            let done_chunk = serde_json::json!({
                                "type": "done",
                                "seq": msg.seq,
                                "message_id": msg_id,
                                "session_id": session_id,
                                "project_id": project_id,
                                "phase": phase,
                                "evaluation_passed": evaluation_passed,
                                "project_tag": project_tag,
                                "duration_ms": duration_ms,
                                // RFC-025: surface detected mount info to the frontend.
                                "mount_tag": msg.metadata.get("mount_tag"),
                                "mount_ids": msg.metadata.get("mount_ids"),
                                // Source: gateway.rs serializes the terminal
                                // orchestration's tool_calls into channel
                                // metadata (same key the non-streaming path
                                // parses at the RFC-015 block above).
                                "tool_calls": msg.metadata.get("tool_calls")
                                    .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
                                    .unwrap_or(serde_json::json!([])),
                            });
                            let done_json = match serde_json::to_string(&done_chunk) {
                                Ok(j) => j,
                                Err(_) => break,
                            };
                            if ws_tx.lock().await.send(Message::Text(done_json.into())).await.is_err() {
                                break; // WS closed — session was already persisted above
                            }
                            // Terminal `done` — turn boundary.
                            turn_text.reset();

                            // Auto-trigger compression for long sessions.
                            if let Some(sid) = &active_session_id
                                && let Some(ref compression) = state.kernel.compression
                                && let Ok(Some(session)) = state.kernel.state.load_session(
                                    &oxios_kernel::state_store::SessionId(sid.clone()),
                                ).await
                                && compression.should_compress(&session)
                            {
                                compression.spawn_compress(sid.clone());
                            }
                        }
                    }
                    event_result = kernel_event_rx.recv() => {
                        // Convert KernelEvent → WS chunk when relevant.
                        let Ok(event) = event_result else {
                            // Lagged or closed — skip and keep waiting.
                            continue;
                        };
                        // Phase A: stamp the current stream's message_id onto every chunk.
                        if let Some(chunk) = kernel_event_to_ws_chunk(&event, &active_session_id, &active_message_id) {
                            let json = match serde_json::to_string(&chunk) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::warn!(error = %e, "Failed to serialize transparency chunk");
                                    continue;
                                }
                            };
                            if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                        // Phase E: when a web_search tool finishes, extract URLs
                        // from output_summary and emit a grounding chunk so the
                        // frontend can render citations as a separate panel.
                        if let Some(grounding) = grounding_from_event(&event, &active_message_id) {
                            let json = match serde_json::to_string(&grounding) {
                                Ok(j) => j,
                                Err(e) => {
                                    tracing::warn!(error = %e, "Failed to serialize grounding chunk");
                                    continue;
                                }
                            };
                            if ws_tx.lock().await.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    // ── Receive from WebSocket client → gateway ──
    //
    // Frontend sends JSON:
    //   `{ type: "message", content: "...", session_id?, project_id? }`
    set.spawn({
        let pong_signal = pong_signal.clone();
        let state = state.clone();
        let ws_tx = ws_tx.clone();
        let active_session_shared = active_session_shared.clone();
        async move {
            while let Some(Ok(msg)) = FuturesStreamExt::next(&mut ws_rx).await {
                match msg {
                    Message::Text(text) => {
                        let parsed: serde_json::Value = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };

                        let msg_type = parsed
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("message");

                        let incoming_session_id = parsed
                            .get("session_id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from);

                        // RFC-049: the client may supply its own request_id for
                        // the session's FIRST message (no session_id yet). The
                        // gateway keys the turn by msg.id in that case, so a
                        // client-known id lets the cancel frame address the turn.
                        let incoming_request_id = parsed
                            .get("request_id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from);

                        let incoming_project_id = parsed
                            .get("project_id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from);
                        // RFC-025: mount_ids (comma-separated, primary first).
                        let incoming_mount_ids = parsed
                            .get("mount_ids")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from);
                        // RFC-032: role hint from the WS client. When set, the
                        // orchestrator resolves the model via engine.role_routing[role].
                        let incoming_role = parsed
                            .get("role")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from);
                        // Per-message model override from the WS client. When set,
                        // the orchestrator carries it into ExecEnv::model_override
                        // (highest precedence over role_routing[role] / default).
                        let incoming_model = parsed
                            .get("model")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from);
                        // Validate model override early — reject unknown IDs
                        // before the orchestrator wastes time on assess/crystallize.
                        if let Some(ref m) = incoming_model
                            && let Err(e) = state.kernel.engine.validate_model(m)
                        {
                            let err_json = serde_json::json!({
                                "type": "error",
                                "message": e
                            });
                            let _ = ws_tx
                                .lock()
                                .await
                                .send(Message::Text(err_json.to_string().into()))
                                .await;
                            continue;
                        }
                        // Session-scoped persona override (chat persona
                        // picker). Validated early like the model override:
                        // unknown or disabled personas are rejected before
                        // the turn starts. Absent → inherit the global
                        // active persona.
                        let incoming_persona_id = parsed
                            .get("persona_id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from);
                        if let Some(pid) = &incoming_persona_id {
                            let invalid = match state.kernel.persona.get(pid) {
                                Some(p) if p.enabled => None,
                                Some(_) => Some("disabled".to_string()),
                                None => Some("unknown".to_string()),
                            };
                            if let Some(reason) = invalid {
                                let err_json = serde_json::json!({
                                    "type": "error",
                                    "message": format!("Persona '{pid}' is {reason}")
                                });
                                let _ = ws_tx
                                    .lock()
                                    .await
                                    .send(Message::Text(err_json.to_string().into()))
                                    .await;
                                continue;
                            }
                        }
                        // One-shot (QuickAsk) requests set `ephemeral: true`.
                        // The recv task skips the pending-message insert so
                        // the send task's persist guard finds no
                        // PendingMessage and never writes a session.
                        let incoming_ephemeral = parsed
                            .get("ephemeral")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        // Per-message model params. Optional. Threaded via
                        // gateway metadata to ExecEnv::model_params.
                        let incoming_temperature =
                            parsed.get("temperature").and_then(|v| v.as_f64());
                        let incoming_max_tokens = parsed
                            .get("max_tokens")
                            .and_then(|v| v.as_u64())
                            .and_then(|v| u32::try_from(v).ok());

                        match msg_type {
                            // RFC-024 SP2 / C2 (replay): client announces its
                            // last-seen seq and asks the server to replay any
                            // messages it missed while disconnected. The bridge
                            // broadcasts the replayed slice (or a synthetic
                            // `type: "resync"` message) which the recv_task
                            // forwards to the client. We do NOT treat this as a
                            // user message, so we `continue` without touching
                            // the gateway.
                            "resume" => {
                                let last_seq: u64 =
                                    parsed.get("last_seq").and_then(|v| v.as_u64()).unwrap_or(0);
                                tracing::debug!(
                                    conn_id = %conn_id_for_send,
                                    last_seq,
                                    "WS resume: replaying since last_seq"
                                );
                                bridge_for_resume.replay_after(last_seq);
                                continue;
                            }
                            // Chat UI redesign: user submits structured interview
                            // answers. Convert to natural language and forward as
                            // a regular message so the Orchestrator's existing
                            // multi-turn interview path handles it without any
                            // special treatment.
                            "interview_response" => {
                                let answers = parsed
                                    .get("answers")
                                    .and_then(|v| v.as_array())
                                    .cloned()
                                    .unwrap_or_default();

                                // Prefer the pre-formatted Q&A text from the frontend
                                // (includes question context so the LLM understands
                                // the answers). Fall back to raw values if absent.
                                let answer_text = parsed
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(String::from)
                                    .unwrap_or_else(|| {
                                        answers
                                            .iter()
                                            .filter_map(|a| {
                                                let value = a.get("value")?.as_str()?;
                                                if value.is_empty() {
                                                    return None;
                                                }
                                                Some(value.to_string())
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    });

                                if answer_text.is_empty() {
                                    continue;
                                }

                                let mut incoming =
                                    IncomingMessage::new("web", "default", answer_text.clone());
                                if let Some(ref sid) = incoming_session_id {
                                    incoming.metadata.insert("session_id".into(), sid.clone());
                                }
                                if let Some(ref vid) = incoming_project_id {
                                    incoming.metadata.insert("project_ids".into(), vid.clone());
                                }
                                if let Some(ref mids) = incoming_mount_ids {
                                    incoming.metadata.insert("mount_ids".into(), mids.clone());
                                }
                                if let Some(ref role) = incoming_role {
                                    incoming.metadata.insert("role".into(), role.clone());
                                }
                                if let Some(m) = &incoming_model {
                                    incoming.metadata.insert("model_override".into(), m.clone());
                                }
                                if let Some(pid) = &incoming_persona_id {
                                    incoming.metadata.insert("persona_id".into(), pid.clone());
                                }
                                if let Some(t) = incoming_temperature {
                                    incoming
                                        .metadata
                                        .insert("temperature".into(), t.to_string());
                                }
                                if let Some(mt) = incoming_max_tokens {
                                    incoming
                                        .metadata
                                        .insert("max_tokens".into(), mt.to_string());
                                }
                                incoming
                                    .metadata
                                    .insert("conn_id".into(), conn_id_for_send.clone());

                                {
                                    let mut pending = pending_for_send.lock().await;
                                    pending.insert(
                                        incoming.id,
                                        PendingMessage {
                                            content: answer_text,
                                            user_id: "default".to_string(),
                                        },
                                    );
                                }

                                if incoming_tx.send(incoming).await.is_err() {
                                    break;
                                }
                            }
                            // RFC-049: user pressed Stop. Resolve the turn key
                            // (frame session id, falling back to the
                            // connection's active session) and cancel the turn,
                            // killing the bound agent so the provider request
                            // actually stops. The client-visible terminal
                            // frame is the gateway's cancelled turn path — no
                            // reply chunk is sent here, and a missing key or a
                            // failed kill must never fail the frame.
                            "cancel" => {
                                let key = resolve_cancel_key(
                                    parsed.get("session_id").and_then(|v| v.as_str()),
                                    parsed.get("request_id").and_then(|v| v.as_str()),
                                    active_session_shared.lock().await.as_deref(),
                                );
                                let Some(key) = key else { continue };
                                if let Some(agent_id) = state.kernel.turns.cancel(&key)
                                    && let Err(e) =
                                        state.kernel.agents.kill(&agent_id.to_string()).await
                                {
                                    tracing::warn!(
                                        agent_id = %agent_id,
                                        error = %e,
                                        "cancel: agent kill failed"
                                    );
                                }
                            }
                            // Default: regular chat message
                            _ => {
                                let content = parsed
                                    .get("content")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                if content.is_empty() {
                                    continue;
                                }

                                let mut incoming =
                                    IncomingMessage::new("web", "default", content.clone());

                                // RFC-049: honour a client-supplied request_id
                                // (first message, no session yet) so the turn
                                // key — `msg.id` when session_id is absent —
                                // is known to the client and cancel can address
                                // it. Only valid UUIDs are accepted; anything
                                // else keeps the server-generated id.
                                if let Some(ref rid) = incoming_request_id
                                    && let Ok(rid) = uuid::Uuid::parse_str(rid)
                                {
                                    incoming.id = rid;
                                }

                                if let Some(ref sid) = incoming_session_id {
                                    incoming.metadata.insert("session_id".into(), sid.clone());
                                }
                                if let Some(ref vid) = incoming_project_id {
                                    incoming.metadata.insert("project_ids".into(), vid.clone());
                                }
                                if let Some(ref mids) = incoming_mount_ids {
                                    incoming.metadata.insert("mount_ids".into(), mids.clone());
                                }
                                if let Some(ref role) = incoming_role {
                                    incoming.metadata.insert("role".into(), role.clone());
                                }
                                if let Some(m) = &incoming_model {
                                    incoming.metadata.insert("model_override".into(), m.clone());
                                }
                                if let Some(pid) = &incoming_persona_id {
                                    incoming.metadata.insert("persona_id".into(), pid.clone());
                                }
                                if let Some(t) = incoming_temperature {
                                    incoming
                                        .metadata
                                        .insert("temperature".into(), t.to_string());
                                }
                                if let Some(mt) = incoming_max_tokens {
                                    incoming
                                        .metadata
                                        .insert("max_tokens".into(), mt.to_string());
                                }
                                incoming
                                    .metadata
                                    .insert("conn_id".into(), conn_id_for_send.clone());
                                if incoming_ephemeral {
                                    incoming.metadata.insert("ephemeral".into(), "true".into());
                                }

                                // Ephemeral (one-shot) requests skip the
                                // pending map: the send task's persist guard
                                // (`if let Some(pm) = pm`) then returns None
                                // and persist_session is never called — no
                                // StateStore write, no sidebar entry.
                                if !incoming_ephemeral {
                                    let mut pending = pending_for_send.lock().await;
                                    pending.insert(
                                        incoming.id,
                                        PendingMessage {
                                            content,
                                            user_id: "default".to_string(),
                                        },
                                    );
                                }
                                // RFC-033: dispatch the regular chat message to
                                // the gateway so it reaches the orchestrator.
                                // Without this the WS default branch only
                                // recorded the message in `pending` and never
                                // forwarded it — regular chat never executed.
                                if incoming_tx.send(incoming).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Message::Close(_) => break,
                    Message::Pong(_) => {
                        // RFC-024 SP2 (B3): the client responded to one of
                        // our pings. Reset the keepalive deadline so the
                        // connection is not killed mid-conversation.
                        pong_signal.notify_one();
                    }
                    _ => {}
                }
            }
        }
    });

    // RFC-024 SP2 (B3): dedicated keepalive task.
    //
    // Sends a protocol Ping every 20 s. The browser answers with an
    // automatic Pong (RFC 6455 §5.5.3) which the recv_task observes as
    // `Message::Pong(_)` and forwards here as a `pong_signal` notification.
    //
    // The 60 s deadline is anchored to `last_pong` — the most recent
    // instant at which we actually heard from the peer. A SENT Ping is
    // NOT proof of liveness: a half-open TCP socket can buffer the
    // write successfully while the peer is gone. Only a RECEIVED Pong
    // is. The deadline therefore advances ONLY in the
    // `pong_signal.notified()` branch. The ticker branch nudges the
    // peer with another Ping but leaves `last_pong` untouched, so a
    // dead socket trips the `last_pong + 60s` future regardless of
    // how many Pings we managed to write into the kernel buffer.
    //
    // The deadline uses `sleep_until(last_pong + 60s)` with
    // `Sleep::reset` rather than a fixed `sleep(60s)`. A fixed sleep
    // inside `select!` is recreated every iteration that any other
    // branch wins, so the 60 s mark becomes unreachable while the
    // ticker is firing — the original implementation had this bug.
    set.spawn({
        let ws_tx = ws_tx.clone();
        let pong_signal = pong_signal.clone();
        let keepalive_timed_out = keepalive_timed_out.clone();
        async move {
            let ping_interval = std::time::Duration::from_secs(20);
            let pong_timeout = std::time::Duration::from_secs(60);

            let mut last_pong = tokio::time::Instant::now();
            let deadline_sleep = tokio::time::sleep_until(last_pong + pong_timeout);
            tokio::pin!(deadline_sleep);

            let mut ticker = tokio::time::interval(ping_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // First tick fires immediately; skip it to give the connection
            // a chance to settle.
            ticker.tick().await;

            loop {
                tokio::select! {
                    biased;
                    // 1. Pong received: extend the deadline by 60 s from now.
                    //    This is the ONLY place `last_pong` advances.
                    _ = pong_signal.notified() => {
                        last_pong = tokio::time::Instant::now();
                        deadline_sleep.as_mut().reset(last_pong + pong_timeout);
                    }
                    // 2. Scheduled tick: nudge the peer with a Ping. We do
                    //    NOT update `last_pong` — a successful send is
                    //    not proof of liveness. If the peer is gone, the
                    //    deadline future will eventually fire and we close.
                    _ = ticker.tick() => {
                        if let Err(e) = ws_tx
                            .lock()
                            .await
                            .send(Message::Ping(Vec::new().into()))
                            .await
                        {
                            tracing::debug!(
                                conn_id = %conn_id,
                                error = %e,
                                "Keepalive ping send failed; closing"
                            );
                            return;
                        }
                    }
                    // 3. Deadline elapsed: peer is dead (no Pong in 60 s).
                    _ = &mut deadline_sleep => {
                        tracing::warn!(
                            conn_id = %conn_id,
                            "WebSocket keepalive timeout (no pong within 60 s)"
                        );
                        keepalive_timed_out.store(true, std::sync::atomic::Ordering::SeqCst);
                        return;
                    }
                }
            }
        }
    });
    // F7: when any side finishes, tear the connection down so the broadcast
    // subscribers and WebSocket half are released promptly. Without this, a
    // client that stops reading but leaves the socket half-open lets a recv or
    // send task block forever on `ws_tx.send().await`, leaking a broadcast
    // subscriber and its buffer per abandoned connection — a trivial memory /
    // subscriber-exhaustion DoS.
    //
    // The first task to finish triggers teardown; `abort_all()` cancels the two
    // survivors, and the drain loop collects each exactly once (each resolves
    // to `Err(JoinError::Cancelled)`). `join_next()` removes a task as it
    // yields it, so no handle is ever polled twice — the double-poll panic that
    // the old `select! { &mut task }` + `task.await` drain caused is now
    // structurally impossible.
    let _ = set.join_next().await;
    set.abort_all();
    while set.join_next().await.is_some() {}
    // RFC-024 §11: pick the close label. The keepalive task sets the
    // flag only when it bailed because the pong deadline elapsed; the
    // peer-driven path (client sent `Message::Close` or the socket
    // was reset) is the default.
    let m = oxios_kernel::metrics::get_metrics();
    if keepalive_timed_out.load(std::sync::atomic::Ordering::SeqCst) {
        m.ws_connections_keepalive_timeout.inc();
    } else {
        m.ws_connections_close.inc();
    }
}

/// User message awaiting a gateway response for session persistence.
struct PendingMessage {
    content: String,
    user_id: String,
}

/// Persist a chat exchange (user message + agent response) to the session store.
///
/// Mirrors the logic in the POST `/api/chat` handler so that WebSocket-based
/// conversations are also durable across tab switches and browser restarts.
#[allow(clippy::too_many_arguments)]
async fn persist_session(
    state_store: &oxios_kernel::state_store::StateStore,
    session_id: &str,
    user_content: &str,
    user_id: &str,
    agent_content: &str,
    project_id: Option<&str>,
    metadata: &std::collections::HashMap<String, String>,
    prune_config: Option<oxios_kernel::state_store::PruneConfig>,
) {
    let sid = oxios_kernel::state_store::SessionId(session_id.to_string());
    // RFC-015: parse tool_calls JSON into trajectory step records.
    let trajectory_steps: Vec<oxios_kernel::state_store::TrajectoryStepRecord> = metadata
        .get("tool_calls")
        .and_then(|v| serde_json::from_str::<Vec<serde_json::Value>>(v).ok())
        .map(|calls| {
            calls
                .into_iter()
                .enumerate()
                .map(|(i, c)| oxios_kernel::state_store::TrajectoryStepRecord {
                    tool_name: c
                        .get("tool")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    tool_args: c.get("input").cloned().unwrap_or(serde_json::Value::Null),
                    output_summary: c
                        .get("output")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    duration_ms: c.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_error: false,
                    tool_call_id: format!("legacy-{i}"),
                    timestamp: chrono::Utc::now(),
                })
                .collect()
        })
        .unwrap_or_default();

    match state_store.load_session(&sid).await {
        Ok(Some(mut session)) => {
            session.add_user_message(user_content);
            // Capture existing trajectory length before extending
            let traj_start = session.trajectory_steps.len();
            session.extend_trajectory(trajectory_steps);
            // P4 (§7 persistence): persist reasoning text from terminal
            // OutgoingMessage metadata alongside trajectory.
            if let Some(rt) = metadata.get("reasoning_text").cloned()
                && !rt.is_empty()
            {
                session.add_reasoning(oxios_kernel::state_store::ReasoningRecord {
                    content: rt,
                    source: "thinking".to_string(),
                    timestamp: chrono::Utc::now(),
                    segments: metadata
                        .get("reasoning_segments")
                        .and_then(|s| {
                            serde_json::from_str::<Vec<oxios_ouroboros::ReasoningSegment>>(s).ok()
                        })
                        .unwrap_or_default(),
                });
            }
            let traj_end = session.trajectory_steps.len();
            session.add_agent_response(oxios_kernel::state_store::AgentResponse {
                content: agent_content.to_string(),
                session_id: Some(sid.0.clone()),
                phase_reached: metadata.get("phase").cloned(),
                evaluation_passed: metadata
                    .get("evaluation_passed")
                    .and_then(|v| v.parse().ok()),
                timestamp: chrono::Utc::now(),
                trajectory_range: if traj_end > traj_start {
                    Some(oxios_kernel::state_store::TrajectoryRange {
                        start: traj_start,
                        end: traj_end,
                    })
                } else {
                    None
                },
            });
            // RFC-025: set top-level project_id field (was metadata key — caused
            // singular/plural mismatch with list_sessions and GET endpoint).
            if let Some(vid) = project_id {
                session.project_id = Some(vid.to_string());
            }
            // Session persona override outlives the turn (chat picker).
            if let Some(pid) = metadata.get("persona_id") {
                session.active_persona_id = Some(pid.clone());
            }
            if let Err(e) = state_store.save_session(&session).await {
                tracing::warn!(error = %e, "WS: failed to persist session");
            }
        }
        Ok(None) => {
            let mut session = oxios_kernel::state_store::Session::new(user_id);
            session.id = oxios_kernel::state_store::SessionId(session_id.to_string());
            session.add_user_message(user_content);
            // New session: trajectory starts at 0
            let traj_start = 0usize;
            session.extend_trajectory(trajectory_steps);
            // P4 (§7 persistence): persist reasoning text from
            // terminal OutgoingMessage metadata alongside trajectory.
            if let Some(rt) = metadata.get("reasoning_text").cloned()
                && !rt.is_empty()
            {
                session.add_reasoning(oxios_kernel::state_store::ReasoningRecord {
                    content: rt,
                    source: "thinking".to_string(),
                    timestamp: chrono::Utc::now(),
                    segments: metadata
                        .get("reasoning_segments")
                        .and_then(|s| {
                            serde_json::from_str::<Vec<oxios_ouroboros::ReasoningSegment>>(s).ok()
                        })
                        .unwrap_or_default(),
                });
            }
            let traj_end = session.trajectory_steps.len();
            session.add_agent_response(oxios_kernel::state_store::AgentResponse {
                content: agent_content.to_string(),
                session_id: Some(sid.0.clone()),
                phase_reached: metadata.get("phase").cloned(),
                evaluation_passed: metadata
                    .get("evaluation_passed")
                    .and_then(|v| v.parse().ok()),
                timestamp: chrono::Utc::now(),
                trajectory_range: if traj_end > traj_start {
                    Some(oxios_kernel::state_store::TrajectoryRange {
                        start: traj_start,
                        end: traj_end,
                    })
                } else {
                    None
                },
            });
            // RFC-025: set top-level project_id field.
            if let Some(vid) = project_id {
                session.project_id = Some(vid.to_string());
            }
            // Session persona override outlives the turn (chat picker).
            if let Some(pid) = metadata.get("persona_id") {
                session.active_persona_id = Some(pid.clone());
            }
            if let Err(e) = state_store.save_session(&session).await {
                tracing::warn!(error = %e, "WS: failed to create session");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "WS: failed to load/create session");
        }
    }

    // Auto-prune in background after session save (throttled)
    if let Some(config) = prune_config {
        // Only prune if at least 1 hour has passed since the last prune.
        // Uses a process-global throttle to avoid spawning on every message.
        static LAST_PRUNE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let last = LAST_PRUNE.load(std::sync::atomic::Ordering::Relaxed);
        if now_secs.saturating_sub(last) >= 3600 {
            LAST_PRUNE.store(now_secs, std::sync::atomic::Ordering::Relaxed);
            let store = state_store.clone();
            tokio::spawn(async move {
                if let Err(e) = store.prune_sessions(&config).await {
                    tracing::warn!(error = %e, "WS: session auto-prune failed");
                }
            });
        }
    }
}

/// Convert a `KernelEvent` into a WebSocket chunk for chat transparency
/// (RFC-015). Returns `None` when the event is not relevant to chat
/// progress (e.g. agent lifecycle events unrelated to the current
/// session) or when it does not belong to the active session.
fn kernel_event_to_ws_chunk(
    event: &oxios_kernel::event_bus::KernelEvent,
    active_session_id: &Option<String>,
    active_message_id: &Option<uuid::Uuid>,
) -> Option<serde_json::Value> {
    use oxios_kernel::event_bus::KernelEvent;

    // Helper: skip events that do not belong to the active session.
    // Events without a session_id (e.g. project lifecycle) are always passed
    // through — they are not high-frequency and can be useful context.
    let event_session_id: Option<&str> = match event {
        KernelEvent::ToolExecutionStarted { session_id, .. } => Some(session_id),
        KernelEvent::ToolExecutionFinished { session_id, .. } => Some(session_id),
        KernelEvent::ToolExecutionProgress { session_id, .. } => Some(session_id),
        KernelEvent::MemoryRecallUsed { session_id, .. } => Some(session_id),
        KernelEvent::TokenUsageUpdate { session_id, .. } => Some(session_id),
        KernelEvent::ReasoningFragment { session_id, .. } => Some(session_id),
        KernelEvent::ToolArgsDelta { session_id, .. } => Some(session_id),
        KernelEvent::CompressionDelta { session_id, .. } => Some(session_id),
        KernelEvent::CompressionDone { session_id } => Some(session_id),
        KernelEvent::CompressionFailed { session_id, .. } => Some(session_id),
        KernelEvent::AgentCreated { session_id, .. } => session_id.as_deref(),
        KernelEvent::AgentStarted { session_id, .. } => session_id.as_deref(),
        KernelEvent::AgentStopped { session_id, .. } => session_id.as_deref(),
        KernelEvent::AgentFailed { session_id, .. } => session_id.as_deref(),
        _ => None,
    };
    if let (Some(eid), Some(active)) = (event_session_id, active_session_id.as_ref())
        && eid != active.as_str()
    {
        return None;
    }

    let chunk = match event {
        // Sub-agent forks inside a chat turn surface as timeline blocks.
        // Background/cron forks carry `session_id: None` and stay off the
        // chat stream.
        KernelEvent::AgentCreated {
            id,
            name,
            session_id: Some(_),
        } => Some(serde_json::json!({
            "type": "agent_start",
            "agent_id": id.to_string(),
            "name": name,
        })),
        KernelEvent::AgentCreated { .. } => None,
        KernelEvent::AgentStopped {
            id,
            success,
            session_id: Some(_),
        } => Some(serde_json::json!({
            "type": "agent_end",
            "agent_id": id.to_string(),
            "success": success,
        })),
        KernelEvent::AgentStopped { .. } => None,
        KernelEvent::AgentFailed {
            id,
            error,
            session_id: Some(_),
        } => Some(serde_json::json!({
            "type": "agent_end",
            "agent_id": id.to_string(),
            "success": false,
            "error": error,
        })),
        KernelEvent::AgentFailed { .. } => None,
        // AgentStarted is intentionally unmapped — AgentCreated already
        // opens the block and a second frame adds nothing.
        KernelEvent::ToolExecutionStarted {
            tool_name,
            tool_call_id,
            tool_args,
            context,
            ..
        } => Some(serde_json::json!({
            "type": "tool_start",
            "tool_name": tool_name,
            "tool_call_id": tool_call_id,
            "tool_args": tool_args,
            "context": context,
        })),
        KernelEvent::ToolExecutionFinished {
            tool_name,
            tool_call_id,
            duration_ms,
            is_error,
            output_summary,
            ..
        } => Some(serde_json::json!({
            "type": "tool_end",
            "tool_name": tool_name,
            "tool_call_id": tool_call_id,
            "duration_ms": duration_ms,
            "is_error": is_error,
            "output_summary": output_summary,
        })),
        KernelEvent::ToolExecutionProgress {
            tool_call_id,
            tool_name,
            progress,
            tab_id,
            context,
            ..
        } => {
            let mut obj = serde_json::json!({
                "type": "tool_progress",
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
            Some(obj)
        }
        KernelEvent::ToolArgsDelta {
            tool_call_id,
            args_delta,
            ..
        } => Some(serde_json::json!({
            "type": "tool_call_delta",
            "tool_call_id": tool_call_id,
            "args_delta": args_delta,
        })),
        KernelEvent::MemoryRecallUsed {
            query,
            count,
            source,
            ..
        } => Some(serde_json::json!({
            "type": "memory",
            "action": "recall",
            "query": query,
            "count": count,
            "source": source,
        })),
        KernelEvent::TokenUsageUpdate {
            input_tokens,
            output_tokens,
            ..
        } => Some(serde_json::json!({
            "type": "usage",
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
        })),
        KernelEvent::ReasoningFragment {
            content, source, ..
        } => Some(serde_json::json!({
            "type": "reasoning",
            "content": content,
            "source": source,
        })),
        KernelEvent::ApprovalRequested {
            id,
            tool_name,
            reason,
            session_id,
            ..
        } => {
            // Filter by active session
            if let (Some(eid), Some(active)) = (session_id.as_ref(), active_session_id.as_ref())
                && eid != active.as_str()
            {
                return None;
            }
            Some(serde_json::json!({
                "type": "tool_approval",
                "id": id.to_string(),
                "tool_name": tool_name,
                "reason": reason,
            }))
        }
        KernelEvent::PathAccessRequested {
            id,
            tool_name,
            path,
            mode,
            reason,
            session_id,
            ..
        } => {
            if let (Some(eid), Some(active)) = (session_id.as_ref(), active_session_id.as_ref())
                && eid != active.as_str()
            {
                return None;
            }
            Some(serde_json::json!({
                "type": "path_access",
                "id": id.to_string(),
                "tool_name": tool_name,
                "path": path,
                "mode": mode,
                "reason": reason,
            }))
        }
        KernelEvent::CompressionDelta { session_id, delta } => Some(serde_json::json!({
            "type": "compression_delta",
            "content": delta,
            "session_id": session_id,
        })),
        KernelEvent::CompressionDone { session_id } => Some(serde_json::json!({
            "type": "compression_done",
            "session_id": session_id,
        })),
        KernelEvent::CompressionFailed { session_id, error } => Some(serde_json::json!({
            "type": "compression_failed",
            "error": error,
            "session_id": session_id,
        })),
        _ => None,
    };
    // Phase A: stamp active stream's message_id on every chunk so frontend
    // can route KernelEvent-sourced chunks to the correct assistant message.
    chunk.map(|mut c| {
        c["message_id"] = serde_json::json!(active_message_id);
        c
    })
}

/// Phase E: extract search citations from a finished web_search tool call and
/// emit a separate `grounding` WS chunk so the frontend can render citations
/// as a dedicated panel (not buried inside the tool_end payload).
///
/// Returns `None` when the event is not a web_search completion or when no
/// URLs are found in the output summary.
fn grounding_from_event(
    event: &oxios_kernel::event_bus::KernelEvent,
    active_message_id: &Option<uuid::Uuid>,
) -> Option<serde_json::Value> {
    use oxios_kernel::event_bus::KernelEvent;

    let KernelEvent::ToolExecutionFinished {
        tool_name,
        output_summary,
        ..
    } = event
    else {
        return None;
    };

    let summary: &str = output_summary;
    let is_search = matches!(
        tool_name.as_str(),
        "web_search" | "get_search_results" | "search" | "web_fetch"
    );
    if !is_search {
        return None;
    }

    let citations = extract_citations(summary);

    if citations.is_empty() {
        return None;
    }

    Some(serde_json::json!({
        "type": "grounding",
        "message_id": active_message_id,
        "citations": citations,
        "tool_name": tool_name,
    }))
}

/// Extract citations from search output. JSON-first, URL-scan fallback.
fn extract_citations(text: &str) -> Vec<serde_json::Value> {
    // Strategy 1: structured JSON (provider-specific formats).
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
        let arr = if parsed.is_array() {
            parsed.as_array().cloned()
        } else {
            parsed.get("results").and_then(|v| v.as_array()).cloned()
        };
        if let Some(items) = arr {
            let citations: Vec<serde_json::Value> = items
                .iter()
                .filter_map(|item| {
                    let url = item.get("url")?.as_str()?.to_string();
                    if !url.starts_with("http") {
                        return None;
                    }
                    let mut c = serde_json::json!({ "url": &url, "favicon": favicon_url(&url) });
                    if let Some(t) = item.get("title").and_then(|v| v.as_str()) {
                        c["title"] = t.into();
                    }
                    if let Some(s) = item.get("snippet").and_then(|v| v.as_str()) {
                        c["snippet"] = s.into();
                    }
                    Some(c)
                })
                .collect();
            if !citations.is_empty() {
                return citations;
            }
        }
    }
    // Strategy 2: URL scan (heuristic, handles plain-text output).
    extract_urls(text)
}

/// Generate favicon URL via Google's favicon service.
fn favicon_url(page_url: &str) -> String {
    let domain = page_url
        .split("//")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    format!("https://www.google.com/s2/favicons?domain={domain}&sz=32")
}

/// Simple URL extractor: scans text for `http` patterns (fallback).
fn extract_urls(text: &str) -> Vec<serde_json::Value> {
    let mut urls = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut start = 0;
    while let Some(pos) = text[start..].find("http") {
        let abs = start + pos;
        let rest = &text[abs..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ')' || c == ']' || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(['.', ',']);
        if url.len() > 10 && seen.insert(url.to_string()) {
            let mut citation = serde_json::json!({ "url": url, "favicon": favicon_url(url) });
            if let Some(t) = extract_link_title(text, abs) {
                citation["title"] = serde_json::json!(t);
            }
            urls.push(citation);
        }
        start = abs + end;
    }
    urls
}

/// Extract title from `[title](url)` markdown pattern.
fn extract_link_title(text: &str, url_pos: usize) -> Option<String> {
    if url_pos < 2 {
        return None;
    }
    let before = &text[..url_pos];
    if !before.ends_with("](") {
        return None;
    }
    let title_end = before.len() - 2;
    let title_start = before[..title_end].rfind('[')?;
    let title = &before[title_start + 1..title_end];
    if title.is_empty() {
        return None;
    }
    Some(title.to_string())
}

// ---------------------------------------------------------------------------
// Tool Approval (RFC-017: runtime capability escalation)
// ---------------------------------------------------------------------------

/// POST /api/chat/tool-approval/{id}/respond — Approve or deny a pending
/// tool approval request. Resolves the oneshot the GatedTool is waiting on.
pub(crate) async fn handle_tool_approval_respond(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ToolApprovalResponseBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let approval_id = uuid::Uuid::parse_str(&id)
        .map_err(|e| AppError::BadRequest(format!("invalid approval id: {e}")))?;

    if body.remember && body.approved {
        let grant_key = state
            .kernel
            .infra
            .pending_tool_approvals()
            .grant_key(approval_id);
        if let Some(key) = grant_key {
            crate::api::routes::security_routes::remember_grant(&state, key).await?;
        }
    }
    let result = if body.approved {
        oxios_kernel::tools::ToolApprovalResult::Approved
    } else {
        oxios_kernel::tools::ToolApprovalResult::Denied
    };

    state
        .kernel
        .infra
        .pending_tool_approvals()
        .resolve(approval_id, result)
        .ok_or_else(|| {
            AppError::NotFound(format!("tool approval {id} not found or already resolved"))
        })?;

    tracing::info!(
        approval_id = %id,
        approved = body.approved,
        "Tool approval resolved"
    );

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ToolApprovalResponseBody {
    pub approved: bool,
    #[serde(default)]
    pub remember: bool,
}

// ---------------------------------------------------------------------------
// path-access (interactive Mount / temp-allow / deny)
// ---------------------------------------------------------------------------

/// POST /api/chat/path-access/{id}/respond — Resolve a pending path-access
/// request. The user chooses: create a Mount ("mount"), temporarily allow
/// the path for this session ("temp"), or deny ("deny").
pub(crate) async fn handle_path_access_respond(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PathAccessResponseBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let request_id = uuid::Uuid::parse_str(&id)
        .map_err(|e| AppError::BadRequest(format!("invalid path-access id: {e}")))?;

    let (agent_name, path) = state
        .kernel
        .infra
        .pending_path_access()
        .path_context(request_id)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "path-access request {id} not found or already resolved"
            ))
        })?;

    let result = match body.action.as_str() {
        "temp" | "mount" => {
            let dir = std::path::Path::new(&path);
            let parent = dir.parent().unwrap_or(dir);
            let pattern = format!("{}/**", parent.to_string_lossy().trim_end_matches('/'));

            {
                let access = state.kernel.exec.access_manager();
                let mut mgr = access.lock();
                if let Some(mut perms) = mgr.get_permissions(&agent_name).cloned()
                    && !perms.allowed_paths.iter().any(|p| p == &pattern)
                {
                    perms.allow_path(&pattern);
                    mgr.set_permissions(perms);
                }
            }

            if body.action == "mount"
                && let Some(mounts) = &state.kernel.mounts
            {
                let name = parent
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "mounted-path".to_string());
                if let Err(e) =
                    mounts.create_mount(name, vec![parent.to_string_lossy().to_string()])
                {
                    tracing::warn!(
                        error = %e,
                        path = %path,
                        "mount creation failed; temp-allow still active"
                    );
                }
            }

            tracing::info!(
                agent = %agent_name,
                path = %path,
                pattern = %pattern,
                action = %body.action,
                "path-access granted"
            );
            oxios_kernel::tools::PathAccessResult::Allowed
        }
        _ => {
            tracing::info!(agent = %agent_name, path = %path, "path-access denied");
            oxios_kernel::tools::PathAccessResult::Denied
        }
    };

    state
        .kernel
        .infra
        .pending_path_access()
        .resolve(request_id, result)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "path-access request {id} not found or already resolved"
            ))
        })?;

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct PathAccessResponseBody {
    /// "temp" | "mount" | "deny"
    pub action: String,
}

// ---------------------------------------------------------------------------
// ask_user (RFC-027 agent-driven clarification)
// ---------------------------------------------------------------------------

/// POST /api/chat/ask-user/{id}/respond — Provide the user's answer to a
/// pending `ask_user` tool invocation. Resolves the oneshot the AskUserTool
/// is awaiting so the agent can resume execution.
#[allow(dead_code)]
pub(crate) async fn handle_ask_user_respond(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<AskUserResponseBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let resolved = state
        .kernel
        .infra
        .pending_ask_user()
        .resolve(&id, body.answer);
    if !resolved {
        return Err(AppError::NotFound(format!(
            "ask_user request {id} not found or already resolved"
        )));
    }

    tracing::info!(request_id = %id, "ask_user: user response received");

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
pub(crate) struct AskUserResponseBody {
    /// The user's answer to the pending question.
    pub answer: String,
}

#[cfg(test)]
mod rfc015_tests {
    use super::*;
    use oxios_kernel::event_bus::KernelEvent;

    /// Every RFC-015 KernelEvent should map to the documented WS chunk type
    /// (tool_start, tool_end, memory, usage, reasoning). This is the wire
    /// contract the frontend `chunkToActivity` depends on, so a regression
    /// here breaks the entire chat transparency UI.
    #[test]
    fn tool_started_emits_tool_start() {
        let event = KernelEvent::ToolExecutionStarted {
            session_id: "s1".into(),
            tool_name: "read_file".into(),
            tool_call_id: "c1".into(),
            tool_args: serde_json::json!({"path": "/x"}),
            context: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None).unwrap();
        assert_eq!(chunk["type"], "tool_start");
        assert_eq!(chunk["tool_name"], "read_file");
        assert_eq!(chunk["tool_call_id"], "c1");
        assert_eq!(chunk["tool_args"]["path"], "/x");
    }

    #[test]
    fn tool_finished_emits_tool_end() {
        let event = KernelEvent::ToolExecutionFinished {
            session_id: "s1".into(),
            tool_call_id: "c1".into(),
            tool_name: "read_file".into(),
            duration_ms: 123,
            is_error: false,
            output_summary: "ok".into(),
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None).unwrap();
        assert_eq!(chunk["type"], "tool_end");
        assert_eq!(chunk["duration_ms"], 123);
        assert_eq!(chunk["is_error"], false);
    }

    /// Real-time tool progress (RFC-015 v0.12) must be forwarded as a
    /// `tool_progress` chunk so the Web UI can show a spinner and the
    /// latest progress text while the tool is still running. When the
    /// upstream event carries a `tab_id`, it must be included in the
    /// chunk so the frontend can badge concurrent tab activity.
    #[test]
    fn tool_progress_emits_tool_progress_chunk() {
        let tab_id = uuid::Uuid::new_v4();
        let event = KernelEvent::ToolExecutionProgress {
            session_id: "s1".into(),
            tool_call_id: "c1".into(),
            tool_name: "browse".into(),
            progress: "loading https://example.com".into(),
            tab_id: Some(tab_id),
            context: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None).unwrap();
        assert_eq!(chunk["type"], "tool_progress");
        assert_eq!(chunk["tool_call_id"], "c1");
        assert_eq!(chunk["tool_name"], "browse");
        assert_eq!(chunk["progress"], "loading https://example.com");
        assert_eq!(chunk["tab_id"], tab_id.to_string());
    }

    /// Progress events must be filtered by session_id, same as start/end.
    #[test]
    fn tool_progress_foreign_session_is_filtered() {
        let event = KernelEvent::ToolExecutionProgress {
            session_id: "other".into(),
            tool_call_id: "c1".into(),
            tool_name: "browse".into(),
            progress: "leak me".into(),
            tab_id: None,
            context: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None);
        assert!(chunk.is_none(), "foreign progress should be filtered");
    }

    /// When `tab_id` is `None` (legacy oxicode-agent versions), the chunk must
    /// omit the `tab_id` key entirely so the frontend treats it as
    /// "no badge" rather than rendering `null`.
    #[test]
    fn tool_progress_chunk_omits_tab_id_when_none() {
        let event = KernelEvent::ToolExecutionProgress {
            session_id: "s1".into(),
            tool_call_id: "c1".into(),
            tool_name: "browse".into(),
            progress: "step 1".into(),
            tab_id: None,
            context: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None).unwrap();
        assert!(
            chunk.get("tab_id").is_none(),
            "tab_id key should be absent when None; got: {chunk}"
        );
    }

    #[test]
    fn memory_recall_emits_memory_chunk() {
        let event = KernelEvent::MemoryRecallUsed {
            session_id: "s1".into(),
            query: "rust errors".into(),
            count: 3,
            source: "warm".into(),
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None).unwrap();
        assert_eq!(chunk["type"], "memory");
        assert_eq!(chunk["action"], "recall");
        assert_eq!(chunk["count"], 3);
    }

    #[test]
    fn token_usage_emits_usage_chunk() {
        let event = KernelEvent::TokenUsageUpdate {
            session_id: "s1".into(),
            input_tokens: 100,
            output_tokens: 50,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None).unwrap();
        assert_eq!(chunk["type"], "usage");
        assert_eq!(chunk["input_tokens"], 100);
        assert_eq!(chunk["output_tokens"], 50);
    }

    #[test]
    fn reasoning_emits_reasoning_chunk() {
        let event = KernelEvent::ReasoningFragment {
            session_id: "s1".into(),
            content: "compaction done".into(),
            source: "compaction".into(),
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None).unwrap();
        assert_eq!(chunk["type"], "reasoning");
        assert_eq!(chunk["content"], "compaction done");
        assert_eq!(chunk["source"], "compaction");
    }

    /// Events tagged with a different session must be dropped — otherwise
    /// unrelated agents' tool calls would leak into the wrong chat.
    #[test]
    fn foreign_session_is_filtered() {
        let event = KernelEvent::ToolExecutionStarted {
            session_id: "other".into(),
            tool_name: "bash".into(),
            tool_call_id: "x".into(),
            tool_args: serde_json::Value::Null,
            context: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &Some("s1".into()), &None);
        assert!(chunk.is_none(), "foreign session should not be forwarded");
    }

    /// When no active session is set (e.g. mid-connect), the filter is
    /// effectively a no-op. Session-scoped events still pass through
    /// because we cannot distinguish "this agent is for me" from "this
    /// agent is for someone else" without a session tag. The first
    /// gateway message will populate `active_session_id`, and from that
    /// point foreign sessions are filtered correctly.
    #[test]
    fn no_active_session_passes_session_scoped_events() {
        let event = KernelEvent::TokenUsageUpdate {
            session_id: "s1".into(),
            input_tokens: 1,
            output_tokens: 1,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &None, &None);
        assert!(
            chunk.is_some(),
            "filter is inactive without an active session"
        );
        assert_eq!(chunk.unwrap()["type"], "usage");
    }

    /// Lifecycle events now carry the owning `session_id` (Task 21): a fork
    /// inside a chat turn surfaces as an `agent_start`/`agent_end` timeline
    /// block on that session's WS stream, while uncorrelated events —
    /// background/cron forks with `session_id: None` or events belonging to
    /// another session — are still dropped.
    #[test]
    fn agent_lifecycle_events_reach_the_owning_session_only() {
        let ev = KernelEvent::AgentCreated {
            id: uuid::Uuid::new_v4(),
            name: "researcher".to_string(),
            session_id: Some("sess-a".to_string()),
        };
        let chunk = kernel_event_to_ws_chunk(&ev, &Some("sess-a".to_string()), &None);
        assert!(chunk.is_some(), "the owning session must see its sub-agent");
        assert_eq!(chunk.unwrap()["type"], "agent_start");
        assert!(
            kernel_event_to_ws_chunk(&ev, &Some("sess-b".to_string()), &None).is_none(),
            "another session must not"
        );
    }

    #[test]
    fn background_agent_lifecycle_events_are_skipped() {
        // `session_id: None` = background/cron fork — never a chat block.
        let event = KernelEvent::AgentCreated {
            id: uuid::Uuid::new_v4(),
            name: "cron".to_string(),
            session_id: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &None, &None);
        assert!(chunk.is_none());
        let event = KernelEvent::AgentStopped {
            id: uuid::Uuid::new_v4(),
            success: true,
            session_id: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &None, &None);
        assert!(chunk.is_none());
        let event = KernelEvent::AgentFailed {
            id: uuid::Uuid::new_v4(),
            error: "boom".to_string(),
            session_id: None,
        };
        let chunk = kernel_event_to_ws_chunk(&event, &None, &None);
        assert!(chunk.is_none());
    }

    #[test]
    fn agent_stop_and_fail_emit_terminal_blocks() {
        let stopped = KernelEvent::AgentStopped {
            id: uuid::Uuid::new_v4(),
            success: true,
            session_id: Some("s1".to_string()),
        };
        let chunk = kernel_event_to_ws_chunk(&stopped, &Some("s1".to_string()), &None).unwrap();
        assert_eq!(chunk["type"], "agent_end");
        assert_eq!(chunk["success"], true);

        let failed = KernelEvent::AgentFailed {
            id: uuid::Uuid::new_v4(),
            error: "timeout".to_string(),
            session_id: Some("s1".to_string()),
        };
        let chunk = kernel_event_to_ws_chunk(&failed, &Some("s1".to_string()), &None).unwrap();
        assert_eq!(chunk["type"], "agent_end");
        assert_eq!(chunk["success"], false);
        assert_eq!(chunk["error"], "timeout");
    }

    /// RFC-015 P2: after non-empty text partials were forwarded, the
    /// terminal full-text `token` chunk must be suppressed or the response
    /// renders twice (the frontend appends every token chunk).
    #[test]
    fn terminal_token_suppressed_after_nonempty_text_partial() {
        let mut t = TurnTextStreamTracker::default();
        t.note_text_partial("Hel");
        t.note_text_partial("");
        t.note_text_partial("lo");
        assert!(t.terminal_token_redundant());
    }

    /// A turn whose deltas were all empty never delivered text — the
    /// terminal token is still needed as the only text carrier.
    #[test]
    fn empty_text_partial_keeps_terminal_token() {
        let mut t = TurnTextStreamTracker::default();
        t.note_text_partial("");
        assert!(!t.terminal_token_redundant());
    }

    /// Non-streaming turns (no partials at all) keep the legacy terminal
    /// token delivery.
    #[test]
    fn fresh_tracker_keeps_terminal_token() {
        let t = TurnTextStreamTracker::default();
        assert!(!t.terminal_token_redundant());
    }

    /// `done`/`error` end the turn: the next turn's terminal token must
    /// not be suppressed by the previous turn's partials.
    #[test]
    fn tracker_resets_at_terminal_boundary() {
        let mut t = TurnTextStreamTracker::default();
        t.note_text_partial("answer");
        t.reset();
        assert!(!t.terminal_token_redundant());
    }

    /// The terminal `done`/`error` reset is the normal path, but a turn may
    /// end without one (abrupt disconnect, or the RFC-049 cancel path).
    /// Resetting at the start of the next turn keeps the next terminal text
    /// from being suppressed on the same connection.
    #[test]
    fn tracker_resets_on_new_turn_not_only_on_terminal() {
        let mut t = TurnTextStreamTracker::default();
        t.note_text_partial("answer");
        assert!(t.terminal_token_redundant());
        // A new turn begins (model chunk) without a preceding done/error —
        // the next turn's terminal text must NOT be suppressed.
        t.begin_turn();
        assert!(!t.terminal_token_redundant());
    }
}

// ---------------------------------------------------------------------------
// RFC-016: Knowledge Save API
// ---------------------------------------------------------------------------

/// GET /api/chat/{session_id}/knowledge-saves
///   → Returns the knowledge save records for a session.
pub(crate) async fn handle_knowledge_saves(
    state: State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let saves: Vec<serde_json::Value> = state
        .kernel
        .state
        .load("knowledge-saves", &session_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(Json(serde_json::json!({ "saves": saves })))
}

/// Request body for saving a message to knowledge.
#[derive(Debug, Deserialize)]
pub(crate) struct SaveToKnowledgeRequest {
    /// Optional path hint for the knowledge note.
    #[serde(default)]
    path: Option<String>,
}

/// POST /api/chat/{session_id}/messages/{message_index}/save-to-knowledge
///   → Saves the message content to the knowledge vault.
pub(crate) async fn handle_save_to_knowledge(
    state: State<Arc<AppState>>,
    Path((session_id, message_index)): Path<(String, usize)>,
    Json(_body): Json<SaveToKnowledgeRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Check if already saved
    let existing: Vec<serde_json::Value> = state
        .kernel
        .state
        .load("knowledge-saves", &session_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    for save in &existing {
        if save.get("message_index").and_then(|v| v.as_u64()) == Some(message_index as u64) {
            let path = save
                .get("knowledge_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            return Ok(Json(serde_json::json!({
                "error": "already_saved",
                "path": path,
            })));
        }
    }

    // Load the session to get the message content
    let session = state
        .kernel
        .state
        .load_session(&oxios_kernel::state_store::SessionId(session_id.clone()))
        .await?;

    let session = match session {
        Some(s) => s,
        None => return Err(AppError::from(anyhow::anyhow!("Session not found"))),
    };

    // Find the agent response at the given index
    let response = match session.agent_responses.get(message_index) {
        Some(r) => r,
        None => {
            return Err(AppError::from(anyhow::anyhow!(
                "Message index out of range"
            )));
        }
    };

    let content = &response.content;
    if content.is_empty() {
        return Err(AppError::from(anyhow::anyhow!("Message content is empty")));
    }

    // Generate path
    let path = _body.path.clone().unwrap_or_else(|| {
        let slug: String = content
            .lines()
            .find(|l| l.starts_with("# ") || l.starts_with("## "))
            .map(|l| l.trim_start_matches('#').trim().to_string())
            .unwrap_or_else(|| "note".to_string());
        let slug: String = slug
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        format!("notes/{slug}-{date}.md")
    });

    // Write to KnowledgeBase with provenance metadata (RFC-022)
    let meta = oxios_markdown::types::NoteMeta {
        author: "agent".to_string(),
        source: oxios_markdown::types::NoteSource::Ui,
        quality: oxios_markdown::types::NoteQuality::Raw,
        needs_review: true,
        session_id: Some(session_id.clone()),
        message_index: Some(message_index),
        saved_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    match state
        .kernel
        .knowledge
        .note_write_with_meta(&path, content, &meta)
    {
        Ok(true) => {}
        Ok(false) => {
            // Path is a user-authored file — force write via plain note_write
            state.kernel.knowledge.note_write(&path, content)?;
        }
        Err(e) => return Err(AppError::from(e)),
    }

    // Record the save
    let record = serde_json::json!({
        "message_index": message_index,
        "knowledge_path": path,
        "saved_at": chrono::Utc::now().to_rfc3339(),
        "source": "user",
    });
    let mut saves = existing;
    saves.push(record);
    state
        .kernel
        .state
        .save("knowledge-saves", &session_id, &saves)
        .await?;

    // Publish event
    let _ = state
        .kernel
        .infra
        .publish(oxios_kernel::event_bus::KernelEvent::KnowledgePersisted {
            session_id: session_id.clone(),
            message_index,
            path: path.clone(),
            source: "user".to_string(),
        });

    Ok(Json(serde_json::json!({ "path": path })))
}

/// DELETE /api/chat/{session_id}/messages/{message_index}/knowledge-save
///   → Removes a knowledge note that was saved from this message.
pub(crate) async fn handle_remove_knowledge_save(
    state: State<Arc<AppState>>,
    Path((session_id, message_index)): Path<(String, usize)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let existing: Vec<serde_json::Value> = state
        .kernel
        .state
        .load("knowledge-saves", &session_id)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let target = existing.iter().find(|save| {
        save.get("message_index").and_then(|v| v.as_u64()) == Some(message_index as u64)
    });

    let target = match target {
        Some(t) => t.clone(),
        None => {
            return Err(AppError::from(anyhow::anyhow!(
                "No save found for this message"
            )));
        }
    };

    let path = target
        .get("knowledge_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Delete from KnowledgeBase
    if !path.is_empty() {
        let _ = state.kernel.knowledge.note_delete(path);
    }

    // Remove the record
    let updated: Vec<serde_json::Value> = existing
        .into_iter()
        .filter(|save| {
            save.get("message_index").and_then(|v| v.as_u64()) != Some(message_index as u64)
        })
        .collect();
    state
        .kernel
        .state
        .save("knowledge-saves", &session_id, &updated)
        .await?;

    // Publish removal event
    let _ = state
        .kernel
        .infra
        .publish(oxios_kernel::event_bus::KernelEvent::KnowledgeRemoved {
            session_id: session_id.clone(),
            message_index,
        });

    Ok(Json(serde_json::json!({ "deleted_path": path })))
}

#[cfg(test)]
mod cancel_tests {
    use super::*;

    /// RFC-049: the gateway-side terminal for a cancelled turn is keyed by
    /// `session_id.unwrap_or(request_id)`; this is the registry-level half of
    /// the WS `cancel` frame: marking the turn cancels its token and returns
    /// the bound agent id so the recv arm can kill the agent.
    #[tokio::test]
    async fn cancel_frame_marks_turn_and_returns_agent_id() {
        let turns = oxios_kernel::turn_registry::TurnRegistry::new();
        let token = turns.open("sess-cancel");
        let agent = uuid::Uuid::new_v4();
        turns.bind_agent("sess-cancel", agent);

        let parsed: serde_json::Value =
            serde_json::from_str(r#"{"type":"cancel","session_id":"sess-cancel"}"#).unwrap();
        let key = parsed
            .get("session_id")
            .and_then(|v| v.as_str())
            .expect("cancel frame carries the turn key");

        assert_eq!(turns.cancel(key), Some(agent));
        assert!(token.is_cancelled());
    }

    /// RFC-049: the cancel arm resolves the turn key with the frame's
    /// session_id winning over request_id and the connection's active session.
    #[test]
    fn resolve_cancel_key_frame_wins_over_shared() {
        assert_eq!(
            resolve_cancel_key(Some("frame-sess"), Some("frame-rid"), Some("active-sess")),
            Some("frame-sess".to_string())
        );
        // An empty frame value is treated as absent (matches the recv
        // task's `incoming_session_id` filtering).
        assert_eq!(
            resolve_cancel_key(Some(""), Some("frame-rid"), Some("active-sess")),
            Some("frame-rid".to_string())
        );
    }

    /// RFC-049: with no frame session_id, the frame request_id (first-message
    /// cancel) wins over the connection's active session.
    #[test]
    fn resolve_cancel_key_request_id_falls_back_to_active_session() {
        assert_eq!(
            resolve_cancel_key(None, Some("frame-rid"), Some("active-sess")),
            Some("frame-rid".to_string())
        );
        assert_eq!(
            resolve_cancel_key(None, None, Some("active-sess")),
            Some("active-sess".to_string())
        );
    }

    /// RFC-049: no key present → None, so the arm no-ops.
    #[test]
    fn resolve_cancel_key_none_when_all_absent() {
        assert_eq!(resolve_cancel_key(None, None, None), None);
        assert_eq!(resolve_cancel_key(Some(""), None, None), None);
    }
}
