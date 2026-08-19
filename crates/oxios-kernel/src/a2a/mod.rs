//! A2A (Agent-to-Agent) protocol for horizontal agent communication.
//!
//! A2A is Google's protocol for horizontal agent↔agent communication.
//! Unlike MCP which is vertical (agent→tool), A2A enables agents to
//! discover each other, delegate tasks, and share results.

pub mod circuit_breaker;

pub use circuit_breaker::{A2ACircuitBreaker, CircuitState};

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::event_bus::{EventBus, KernelEvent};
use crate::types::{AgentId, AgentStatus};

/// A2A Message types for inter-agent communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum A2AMessage {
    /// Task delegation: "Here, do X"
    TaskDelegation {
        /// Unique task identifier.
        task_id: Uuid,
        /// Human-readable description of the task.
        description: String,
        /// Structured task payload.
        payload: serde_json::Value,
        /// Priority level.
        priority: TaskPriority,
    },
    /// Status update: "I'm working on X, status: Y%"
    StatusUpdate {
        /// Associated task identifier.
        task_id: Uuid,
        /// Progress percentage (0-100).
        progress: u8,
        /// Status message.
        message: String,
    },
    /// Result sharing: "Here's the result of X"
    ResultSharing {
        /// Associated task identifier.
        task_id: Uuid,
        /// Result data.
        result: serde_json::Value,
        /// Human-readable summary.
        summary: String,
    },
    /// Capability query: "Who can do X?"
    CapabilityQuery {
        /// Query description.
        query: String,
        /// Required capabilities.
        required_capabilities: Vec<String>,
    },
    /// Handshake: "Hello, I can do Y"
    Handshake {
        /// Agent identifier.
        agent_id: AgentId,
        /// Agent name.
        name: String,
        /// Agent capabilities.
        capabilities: Vec<String>,
    },
}

impl A2AMessage {
    /// Returns the message type name for logging/debugging.
    pub fn type_name(&self) -> &'static str {
        match self {
            A2AMessage::TaskDelegation { .. } => "task_delegation",
            A2AMessage::StatusUpdate { .. } => "status_update",
            A2AMessage::ResultSharing { .. } => "result_sharing",
            A2AMessage::CapabilityQuery { .. } => "capability_query",
            A2AMessage::Handshake { .. } => "handshake",
        }
    }
}

/// Priority level for delegated tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TaskPriority {
    /// Low priority, best-effort.
    Low,
    /// Normal priority.
    #[default]
    Normal,
    /// High priority, should be handled soon.
    High,
    /// Critical, immediate attention required.
    Critical,
}

/// Specification for a delegated task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Unique task identifier.
    pub task_id: Uuid,
    /// Human-readable description of the task.
    pub description: String,
    /// Structured task payload.
    pub payload: serde_json::Value,
    /// Priority level.
    pub priority: TaskPriority,
    /// Deadline for task completion, if any.
    pub deadline: Option<DateTime<Utc>>,
}

impl TaskSpec {
    /// Creates a new task specification.
    pub fn new(description: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            task_id: Uuid::new_v4(),
            description: description.into(),
            payload,
            priority: TaskPriority::default(),
            deadline: None,
        }
    }

    /// Sets the priority.
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the deadline.
    pub fn with_deadline(mut self, deadline: DateTime<Utc>) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

/// A request sent by one agent to another via A2A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ARequest {
    /// Unique request identifier.
    pub request_id: Uuid,
    /// Sending agent's ID.
    pub from: AgentId,
    /// Receiving agent's ID.
    pub to: AgentId,
    /// The message being sent.
    pub message: A2AMessage,
    /// Timestamp when the request was created.
    pub timestamp: DateTime<Utc>,
}

impl A2ARequest {
    /// Creates a new A2A request.
    pub fn new(from: AgentId, to: AgentId, message: A2AMessage) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            from,
            to,
            message,
            timestamp: Utc::now(),
        }
    }
}

/// A response from a target agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AResponse {
    /// Unique response identifier.
    pub response_id: Uuid,
    /// ID of the request this responds to.
    pub request_id: Uuid,
    /// Responding agent's ID.
    pub from: AgentId,
    /// Original requesting agent's ID.
    pub to: AgentId,
    /// Whether the request was accepted.
    pub accepted: bool,
    /// Response payload (result, error, etc.).
    pub payload: serde_json::Value,
    /// Timestamp when the response was created.
    pub timestamp: DateTime<Utc>,
}

impl A2AResponse {
    /// Creates a success response.
    pub fn success(
        request_id: Uuid,
        from: AgentId,
        to: AgentId,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            response_id: Uuid::new_v4(),
            request_id,
            from,
            to,
            accepted: true,
            payload,
            timestamp: Utc::now(),
        }
    }

    /// Creates an error response.
    pub fn error(request_id: Uuid, from: AgentId, to: AgentId, error: impl Into<String>) -> Self {
        Self {
            response_id: Uuid::new_v4(),
            request_id,
            from,
            to,
            accepted: false,
            payload: serde_json::json!({ "error": error.into() }),
            timestamp: Utc::now(),
        }
    }
}

/// A pending message waiting for an agent to receive it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMessage {
    /// The request that created this message.
    pub request: A2ARequest,
    /// Timestamp when the message was queued.
    pub queued_at: DateTime<Utc>,
}

impl PendingMessage {
    fn new(request: A2ARequest) -> Self {
        Self {
            request,
            queued_at: Utc::now(),
        }
    }
}

/// A card describing an agent's capabilities for discovery.
///
/// Each agent publishes an AgentCard to the registry, making its
/// capabilities discoverable by other agents via A2A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// Unique identifier for this agent.
    pub agent_id: AgentId,
    /// Human-readable name of the agent.
    pub name: String,
    /// Description of what the agent does.
    pub description: String,
    /// List of capabilities (e.g., ["code-review", "refactor"]).
    pub capabilities: Vec<String>,
    /// List of skills (e.g., ["rust", "python"]).
    pub skills: Vec<String>,
    /// Endpoint for communication (e.g., "local", "remote://...").
    pub endpoint: String,
    /// Current status of the agent.
    pub status: AgentStatus,
}

impl AgentCard {
    /// Creates a new agent card.
    pub fn new(agent_id: AgentId, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            agent_id,
            name: name.into(),
            description: description.into(),
            capabilities: Vec::new(),
            skills: Vec::new(),
            endpoint: "local".into(),
            status: AgentStatus::Starting,
        }
    }

    /// Adds a capability.
    pub fn with_capability(mut self, capability: impl Into<String>) -> Self {
        self.capabilities.push(capability.into());
        self
    }

    /// Adds a skill.
    pub fn with_skill(mut self, skill: impl Into<String>) -> Self {
        self.skills.push(skill.into());
        self
    }

    /// Sets the endpoint.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Sets the initial status.
    pub fn with_status(mut self, status: AgentStatus) -> Self {
        self.status = status;
        self
    }

    /// Returns true if this agent has the given capability.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// Returns true if this agent has the given skill.
    pub fn has_skill(&self, skill: &str) -> bool {
        self.skills.iter().any(|s| s == skill)
    }
}

/// Global registry of available agents and their capability cards.
///
/// The registry enables agents to discover each other by capability,
/// supporting the A2A "handshake" pattern where agents query "who can do X?".
#[derive(Clone)]
pub struct AgentCardRegistry {
    /// Map of agent ID to their card.
    cards: Arc<RwLock<HashMap<AgentId, AgentCard>>>,
    /// Event bus for publishing registry changes.
    event_bus: EventBus,
}

impl AgentCardRegistry {
    /// Creates a new empty registry.
    pub fn new(event_bus: EventBus) -> Self {
        Self {
            cards: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    /// Registers an agent's card in the registry.
    pub async fn register_agent(&self, card: AgentCard) -> Result<()> {
        let agent_id = card.agent_id;
        let mut cards = self.cards.write().await;
        cards.insert(agent_id, card.clone());
        drop(cards);

        self.event_bus.publish(KernelEvent::AgentCreated {
            id: agent_id,
            name: card.name.clone(),
            session_id: None,
        })?;

        tracing::info!(agent_id = %agent_id, name = %card.name, "Agent registered in A2A registry");
        Ok(())
    }

    /// Unregisters an agent from the registry.
    pub async fn unregister_agent(&self, agent_id: AgentId) -> Result<()> {
        let mut cards = self.cards.write().await;
        if let Some(card) = cards.remove(&agent_id) {
            tracing::info!(agent_id = %agent_id, name = %card.name, "Agent unregistered from A2A registry");
            drop(cards);

            self.event_bus.publish(KernelEvent::AgentStopped {
                id: agent_id,
                success: false,
                session_id: None,
            })?;
        }
        Ok(())
    }

    /// Finds all agents that have the given capability.
    pub async fn find_agents_by_capability(&self, capability: &str) -> Result<Vec<AgentCard>> {
        let cards = self.cards.read().await;
        let matches: Vec<AgentCard> = cards
            .values()
            .filter(|card| card.has_capability(capability))
            .cloned()
            .collect();
        Ok(matches)
    }

    /// Finds all agents that have the given skill.
    pub async fn find_agents_by_skill(&self, skill: &str) -> Result<Vec<AgentCard>> {
        let cards = self.cards.read().await;
        let matches: Vec<AgentCard> = cards
            .values()
            .filter(|card| card.has_skill(skill))
            .cloned()
            .collect();
        Ok(matches)
    }

    /// Finds an agent by its ID.
    pub async fn get_agent(&self, agent_id: AgentId) -> Option<AgentCard> {
        let cards = self.cards.read().await;
        cards.get(&agent_id).cloned()
    }

    /// Returns all registered agents.
    pub async fn list_agents(&self) -> Vec<AgentCard> {
        let cards = self.cards.read().await;
        cards.values().cloned().collect()
    }

    /// Returns the count of registered agents.
    pub async fn agent_count(&self) -> usize {
        let cards = self.cards.read().await;
        cards.len()
    }

    /// Updates an agent's status.
    pub async fn update_status(&self, agent_id: AgentId, status: AgentStatus) -> Result<()> {
        let mut cards = self.cards.write().await;
        if let Some(card) = cards.get_mut(&agent_id) {
            card.status = status;
        }
        Ok(())
    }
}

impl std::fmt::Debug for AgentCardRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCardRegistry").finish()
    }
}

/// Per-agent message queue with notification.
///
/// Each agent gets its own queue backed by `tokio::sync::Notify`
/// so consumers can `.await` new messages without polling.
struct AgentQueue {
    /// Buffered pending messages (behind a sync mutex for cheap push/drain).
    messages: parking_lot::Mutex<Vec<PendingMessage>>,
    /// Notifier signalled when a new message is pushed.
    notify: tokio::sync::Notify,
}

impl AgentQueue {
    fn new() -> Self {
        Self {
            messages: parking_lot::Mutex::new(Vec::new()),
            notify: tokio::sync::Notify::new(),
        }
    }
}

/// Callback type invoked when a TaskDelegation message is received.
///
/// The dispatcher calls this with (from, to, task) and expects the
/// handler to execute the work and return the result.
pub type DelegationHandler = Arc<
    dyn Fn(
            AgentId,
            AgentId,
            TaskSpec,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send>>
        + Send
        + Sync,
>;

/// A single entry in the A2A message log.
///
/// Records every message that passes through the protocol for
/// observability and debugging. The log is append-only and bounded
/// to [`A2AProtocol::MAX_LOG_ENTRIES`] entries (oldest are pruned).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AMessageLogEntry {
    /// Sending agent's ID.
    pub from: AgentId,
    /// Receiving agent's ID.
    pub to: AgentId,
    /// Message type name (e.g. "task_delegation", "handshake").
    pub message_type: String,
    /// When this message was logged.
    pub timestamp: DateTime<Utc>,
    /// Short human-readable content summary.
    pub content: String,
}

/// A node in the A2A communication topology.
///
/// Represents a single agent, derived from the agent card registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyNode {
    /// Stable identifier (the agent name, used by the frontend as a node id).
    pub id: String,
    /// Display label.
    pub label: String,
    /// Lowercased status (e.g. "running", "idle", "stopped", "starting").
    pub status: String,
    /// Agent capabilities (e.g. ["code-review"]).
    pub capabilities: Vec<String>,
    /// Agent skills (e.g. ["rust", "python"]).
    pub skills: Vec<String>,
    /// ISO-8601 timestamp of the last observed message involving this
    /// agent, or `None` if no recent activity.
    pub last_seen: Option<String>,
}

/// An edge in the A2A communication topology.
///
/// Aggregates messages between a pair of agents over a recent
/// time window. The `last_kind` is the type of the most recent
/// message along this edge — useful for color-coding the edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyEdge {
    /// Source agent identifier (matches `TopologyNode.id`).
    pub from: String,
    /// Target agent identifier (matches `TopologyNode.id`).
    pub to: String,
    /// Number of messages between `from` and `to` in the window.
    pub message_count_5m: u32,
    /// Type of the most recent message along this edge.
    pub last_kind: String,
}

/// Response shape for `/api/a2a/topology`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyResponse {
    /// Agents in the topology (nodes).
    pub nodes: Vec<TopologyNode>,
    /// Communication edges aggregated from the recent message log.
    pub edges: Vec<TopologyEdge>,
}

/// A2A Protocol handler for inter-agent communication.
#[derive(Clone)]
pub struct A2AProtocol {
    /// The registry for agent capability discovery.
    registry: AgentCardRegistry,
    /// Per-agent queues with notification support.
    queues: Arc<RwLock<HashMap<AgentId, Arc<AgentQueue>>>>,
    /// Event bus for kernel events.
    event_bus: EventBus,
    /// Optional handler invoked when a TaskDelegation message is received.
    delegation_handler: Arc<RwLock<Option<DelegationHandler>>>,
    /// Append-only message log for observability.
    message_log: Arc<parking_lot::RwLock<Vec<A2AMessageLogEntry>>>,
}

impl A2AProtocol {
    /// Maximum number of log entries retained before pruning.
    pub const MAX_LOG_ENTRIES: usize = 10_000;

    /// Creates a new A2A protocol handler.
    pub fn new(event_bus: EventBus) -> Self {
        let registry = AgentCardRegistry::new(event_bus.clone());
        Self {
            registry,
            queues: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
            delegation_handler: Arc::new(RwLock::new(None)),
            message_log: Arc::new(parking_lot::RwLock::new(Vec::with_capacity(256))),
        }
    }

    /// Register a handler that executes delegated tasks.
    ///
    /// When a `TaskDelegation` message arrives and a handler is set,
    /// the protocol spawns a background task to execute it and sends
    /// the result back as a `ResultSharing` message.
    pub async fn set_delegation_handler(&self, handler: DelegationHandler) {
        let mut h = self.delegation_handler.write().await;
        *h = Some(handler);
    }

    /// Append an entry to the message log, pruning if over capacity.
    fn append_log(&self, entry: A2AMessageLogEntry) {
        let mut log = self.message_log.write();
        log.push(entry);
        if log.len() > Self::MAX_LOG_ENTRIES {
            let excess = log.len() - Self::MAX_LOG_ENTRIES;
            log.drain(..excess);
        }
    }

    /// Returns recent message log entries, most recent last.
    ///
    /// If `limit` is `Some(n)`, returns at most the last `n` entries.
    pub fn get_message_log(&self, limit: Option<usize>) -> Vec<A2AMessageLogEntry> {
        let log = self.message_log.read();
        match limit {
            Some(n) => log
                .iter()
                .rev()
                .take(n)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            None => log.clone(),
        }
    }

    /// Returns message-log entries whose timestamp is within the last
    /// `secs` seconds, most recent last.
    ///
    /// Used by the topology endpoint to derive edges from a sliding
    /// window of recent activity.
    pub fn recent_messages(&self, secs: u64) -> Vec<A2AMessageLogEntry> {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(secs as i64);
        let log = self.message_log.read();
        log.iter()
            .filter(|entry| entry.timestamp >= cutoff)
            .cloned()
            .collect()
    }

    /// Get or create a queue for the given agent.
    async fn get_or_create_queue(&self, agent_id: AgentId) -> Arc<AgentQueue> {
        let mut queues = self.queues.write().await;
        queues
            .entry(agent_id)
            .or_insert_with(|| Arc::new(AgentQueue::new()))
            .clone()
    }

    /// Returns the agent card registry.
    pub fn registry(&self) -> &AgentCardRegistry {
        &self.registry
    }

    /// Execute a delegated task through the registered handler (blocking).
    ///
    /// Also enqueues the delegation message and publishes events for
    /// audit trail purposes, then calls the handler directly and waits.
    ///
    /// Returns `None` if no handler is registered.
    pub async fn execute_delegation(
        &self,
        from: AgentId,
        to: AgentId,
        task: TaskSpec,
    ) -> Option<Result<serde_json::Value>> {
        let handler = self.delegation_handler.read().await;
        let handler_ref = handler.as_ref()?;

        // Publish audit event.
        let _ = self.event_bus.publish(KernelEvent::MessageReceived {
            from,
            content: format!("[task_delegation] {:?}", task.task_id),
        });

        // Log for observability.
        self.append_log(A2AMessageLogEntry {
            from,
            to,
            message_type: "task_delegation".to_string(),
            timestamp: Utc::now(),
            content: task.description.clone(),
        });

        tracing::info!(
            from = %from,
            to = %to,
            task_id = %task.task_id,
            "A2A execute_delegation: starting"
        );

        let result = handler_ref(from, to, task).await;

        tracing::info!(
            from = %from,
            to = %to,
            success = result.is_ok(),
            "A2A execute_delegation: completed"
        );

        Some(result)
    }

    /// Sends a message from one agent to another.
    pub async fn send_message(
        &self,
        from: AgentId,
        to: AgentId,
        message: A2AMessage,
    ) -> Result<Uuid> {
        let msg_type = message.type_name();
        let request = A2ARequest::new(from, to, message.clone());
        let request_id = request.request_id;

        // Log the message for observability.
        let content_summary = match &request.message {
            A2AMessage::TaskDelegation { description, .. } => description.clone(),
            A2AMessage::StatusUpdate { message, .. } => message.clone(),
            A2AMessage::ResultSharing { summary, .. } => summary.clone(),
            A2AMessage::CapabilityQuery { query, .. } => query.clone(),
            A2AMessage::Handshake { name, .. } => format!("handshake from {name}"),
        };
        self.append_log(A2AMessageLogEntry {
            from,
            to,
            message_type: msg_type.to_string(),
            timestamp: Utc::now(),
            content: content_summary,
        });

        // Push to the target agent's queue and notify.
        let queue = self.get_or_create_queue(to).await;
        queue
            .messages
            .lock()
            .push(PendingMessage::new(request.clone()));
        queue.notify.notify_one();

        // The event bus is an auxiliary observability/coordination channel;
        // `append_log` above already records the message durably. If publish
        // fails we must NOT return Err — the message has already been pushed
        // to the recipient's queue, so propagating would cause the caller to
        // retry and produce a duplicate delivery.
        if let Err(e) = self.event_bus.publish(KernelEvent::MessageReceived {
            from,
            content: format!("[{msg_type}] {request_id:?}"),
        }) {
            tracing::warn!(
                error = %e,
                from = %from,
                to = %to,
                request_id = %request_id,
                "a2a: failed to publish MessageReceived event (message was still delivered)"
            );
        }

        tracing::debug!(
            from = %from,
            to = %to,
            request_id = %request_id,
            msg_type,
            "A2A message sent"
        );

        Ok(request_id)
    }

    /// Delegates a task from one agent to another.
    pub async fn delegate_task(&self, from: AgentId, to: AgentId, task: TaskSpec) -> Result<Uuid> {
        let message = A2AMessage::TaskDelegation {
            task_id: task.task_id,
            description: task.description.clone(),
            payload: task.payload.clone(),
            priority: task.priority,
        };

        self.send_message(from, to, message).await
    }

    /// Sends a status update from one agent to another.
    pub async fn send_status_update(
        &self,
        from: AgentId,
        to: AgentId,
        task_id: Uuid,
        progress: u8,
        message: String,
    ) -> Result<Uuid> {
        let message = A2AMessage::StatusUpdate {
            task_id,
            progress,
            message,
        };

        self.send_message(from, to, message).await
    }

    /// Shares a result from one agent to another.
    pub async fn share_result(
        &self,
        from: AgentId,
        to: AgentId,
        task_id: Uuid,
        result: serde_json::Value,
        summary: String,
    ) -> Result<Uuid> {
        let message = A2AMessage::ResultSharing {
            task_id,
            result,
            summary,
        };

        self.send_message(from, to, message).await
    }

    /// Queries the registry for agents that can perform a capability.
    pub async fn query_capabilities(&self, capability: &str) -> Result<Vec<AgentCard>> {
        self.registry.find_agents_by_capability(capability).await
    }

    /// Initiates a handshake with another agent.
    pub async fn send_handshake(&self, from: AgentId, to: AgentId) -> Result<Uuid> {
        let card = self.registry.get_agent(from).await;

        let (name, capabilities) = if let Some(card) = card {
            (card.name, card.capabilities.clone())
        } else {
            ("unknown".into(), Vec::new())
        };

        let message = A2AMessage::Handshake {
            agent_id: from,
            name,
            capabilities,
        };

        self.send_message(from, to, message).await
    }

    /// Receives all pending messages for an agent, draining the queue.
    pub async fn receive_messages(&self, agent_id: AgentId) -> Vec<A2ARequest> {
        let queues = self.queues.read().await;
        if let Some(queue) = queues.get(&agent_id) {
            let drained: Vec<PendingMessage> = queue.messages.lock().drain(..).collect();
            drained.into_iter().map(|m| m.request).collect()
        } else {
            Vec::new()
        }
    }

    /// Returns the number of pending messages for an agent.
    pub async fn pending_count(&self, agent_id: AgentId) -> usize {
        let queues = self.queues.read().await;
        queues
            .get(&agent_id)
            .map(|q| q.messages.lock().len())
            .unwrap_or(0)
    }

    /// Returns true if the agent has any pending messages.
    pub async fn has_messages(&self, agent_id: AgentId) -> bool {
        self.pending_count(agent_id).await > 0
    }

    /// Deliver all pending messages to an agent.
    ///
    /// Unlike `receive_messages` (which drains the queue silently),
    /// this method does NOT re-publish `MessageReceived` events since
    /// they were already published when the messages were originally sent.
    pub async fn deliver_pending_messages(&self, agent_id: AgentId) -> Result<Vec<A2ARequest>> {
        Ok(self.receive_messages(agent_id).await)
    }

    /// Send a message and wait for a response within a timeout.
    ///
    /// Uses `tokio::select!` with `Notify` instead of polling.
    /// Matches `ResultSharing` messages by checking if `task_id` equals the
    /// **delegated task's ID** (not the envelope request_id). This works because
    /// `delegate_task` creates a `TaskDelegation { task_id: task.task_id, ... }`
    /// message, and the handler responds with `ResultSharing { task_id: task.task_id }`.
    pub async fn send_and_wait(
        &self,
        from: AgentId,
        to: AgentId,
        message: A2AMessage,
        timeout: std::time::Duration,
    ) -> Result<A2AResponse> {
        // Extract the task_id from the outgoing message so we can match the response.
        let wait_task_id = match &message {
            A2AMessage::TaskDelegation { task_id, .. } => Some(*task_id),
            _ => None,
        };

        let request_id = self.send_message(from, to, message).await?;
        let queue = self.get_or_create_queue(from).await;
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            // First, check if a matching response is already in the queue.
            {
                let mut msgs = queue.messages.lock();
                let match_idx = msgs.iter().position(|p| {
                    match (&p.request.message, wait_task_id) {
                        // For TaskDelegation: match by the delegated task_id.
                        (A2AMessage::ResultSharing { task_id, .. }, Some(wait_id)) => {
                            *task_id == wait_id
                        }
                        // For non-delegation messages: match by request_id echoed in payload.
                        (A2AMessage::ResultSharing { result, .. }, None) => {
                            result.get("request_id").and_then(|v| v.as_str())
                                == Some(&request_id.to_string())
                        }
                        _ => false,
                    }
                });
                if let Some(idx) = match_idx {
                    let matched = msgs.remove(idx);
                    if let A2AMessage::ResultSharing { result, .. } = matched.request.message {
                        return Ok(A2AResponse::success(request_id, to, from, result));
                    }
                }
            }

            // No match yet — wait for notification or timeout.
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                anyhow::bail!("A2A response timeout after {timeout:?}");
            }

            tokio::select! {
                _ = queue.notify.notified() => {
                    // A new message arrived — loop to check for a match.
                }
                _ = tokio::time::sleep(remaining) => {
                    anyhow::bail!("A2A response timeout after {timeout:?}");
                }
            }
        }
    }
}

impl std::fmt::Debug for A2AProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("A2AProtocol")
            .field("registry", &self.registry)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_event_bus() -> EventBus {
        EventBus::new(256)
    }

    fn create_test_agent_id() -> AgentId {
        Uuid::new_v4()
    }

    #[tokio::test]
    async fn test_agent_card_creation() {
        let agent_id = create_test_agent_id();
        let card = AgentCard::new(agent_id, "test-agent", "A test agent")
            .with_capability("code-review")
            .with_capability("lint")
            .with_skill("rust")
            .with_endpoint("local");

        assert_eq!(card.agent_id, agent_id);
        assert_eq!(card.name, "test-agent");
        assert!(card.has_capability("code-review"));
        assert!(card.has_capability("lint"));
        assert!(!card.has_capability("refactor"));
        assert!(card.has_skill("rust"));
        assert!(!card.has_skill("python"));
    }

    #[tokio::test]
    async fn test_registry_register_unregister() {
        let bus = create_test_event_bus();
        let registry = AgentCardRegistry::new(bus);

        let agent_id = create_test_agent_id();
        let card = AgentCard::new(agent_id, "register-test", "Test agent").with_capability("test");

        registry.register_agent(card.clone()).await.unwrap();
        assert_eq!(registry.agent_count().await, 1);

        let found = registry.get_agent(agent_id).await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "register-test");

        registry.unregister_agent(agent_id).await.unwrap();
        assert_eq!(registry.agent_count().await, 0);

        let found = registry.get_agent(agent_id).await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_registry_find_by_capability() {
        let bus = create_test_event_bus();
        let registry = AgentCardRegistry::new(bus);

        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        registry
            .register_agent(
                AgentCard::new(id1, "agent-1", "First agent").with_capability("code-review"),
            )
            .await
            .unwrap();

        registry
            .register_agent(
                AgentCard::new(id2, "agent-2", "Second agent")
                    .with_capability("code-review")
                    .with_capability("refactor"),
            )
            .await
            .unwrap();

        let reviewers = registry
            .find_agents_by_capability("code-review")
            .await
            .unwrap();
        assert_eq!(reviewers.len(), 2);
    }

    #[tokio::test]
    async fn test_a2a_protocol_send_receive() {
        let bus = create_test_event_bus();
        let a2a = A2AProtocol::new(bus);

        let from = create_test_agent_id();
        let to = create_test_agent_id();

        let message = A2AMessage::Handshake {
            agent_id: from,
            name: "sender".into(),
            capabilities: vec!["test".into()],
        };

        a2a.send_message(from, to, message).await.unwrap();
        assert_eq!(a2a.pending_count(to).await, 1);

        let messages = a2a.receive_messages(to).await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].from, from);
        assert_eq!(messages[0].to, to);
        assert_eq!(a2a.pending_count(to).await, 0);
    }

    #[tokio::test]
    async fn test_delegate_task() {
        let bus = create_test_event_bus();
        let a2a = A2AProtocol::new(bus);

        let from = create_test_agent_id();
        let to = create_test_agent_id();

        let task = TaskSpec::new("Review PR", serde_json::json!({ "pr": 42 }));

        let request_id = a2a.delegate_task(from, to, task).await.unwrap();
        assert!(request_id != Uuid::nil());

        let messages = a2a.receive_messages(to).await;
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_recent_messages_filters_by_window() {
        let bus = create_test_event_bus();
        let a2a = A2AProtocol::new(bus);

        // Append a recent log entry directly.
        let recent_ts = Utc::now();
        a2a.append_log(A2AMessageLogEntry {
            from: Uuid::new_v4(),
            to: Uuid::new_v4(),
            message_type: "task_delegation".into(),
            timestamp: recent_ts,
            content: "recent".into(),
        });

        // Append an old log entry (10 minutes ago).
        let old_ts = Utc::now() - chrono::Duration::seconds(600);
        a2a.append_log(A2AMessageLogEntry {
            from: Uuid::new_v4(),
            to: Uuid::new_v4(),
            message_type: "handshake".into(),
            timestamp: old_ts,
            content: "old".into(),
        });

        // 5-minute window should include only the recent entry.
        let window = a2a.recent_messages(300);
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].content, "recent");
        assert_eq!(window[0].message_type, "task_delegation");

        // 15-minute window should include both.
        let wider = a2a.recent_messages(900);
        assert_eq!(wider.len(), 2);

        // 1-second window should include only very recent entries.
        let narrow = a2a.recent_messages(1);
        assert_eq!(narrow.len(), 1);
        assert_eq!(narrow[0].content, "recent");
    }

    #[tokio::test]
    async fn test_recent_messages_aggregates_fan_in_fan_out() {
        // Mixed message kinds, multi-agent fan-in / fan-out aggregation.
        let bus = create_test_event_bus();
        let a2a = A2AProtocol::new(bus);

        // Register three agents so the registry has names.
        let orch = Uuid::new_v4();
        let worker_a = Uuid::new_v4();
        let worker_b = Uuid::new_v4();
        for (id, name) in [
            (orch, "orchestrator"),
            (worker_a, "worker-a"),
            (worker_b, "worker-b"),
        ] {
            a2a.registry
                .register_agent(AgentCard::new(id, name, "test").with_status(AgentStatus::Running))
                .await
                .unwrap();
        }

        // orchestrator -> worker-a: 2x TaskDelegation
        for _ in 0..2 {
            a2a.append_log(A2AMessageLogEntry {
                from: orch,
                to: worker_a,
                message_type: "task_delegation".into(),
                timestamp: Utc::now(),
                content: "do work".into(),
            });
        }

        // orchestrator -> worker-b: 1x TaskDelegation, 1x StatusUpdate
        a2a.append_log(A2AMessageLogEntry {
            from: orch,
            to: worker_b,
            message_type: "task_delegation".into(),
            timestamp: Utc::now(),
            content: "do work b".into(),
        });
        a2a.append_log(A2AMessageLogEntry {
            from: worker_b,
            to: orch,
            message_type: "status_update".into(),
            timestamp: Utc::now(),
            content: "50%".into(),
        });

        // worker-a -> orchestrator: 1x ResultSharing (fan-in)
        a2a.append_log(A2AMessageLogEntry {
            from: worker_a,
            to: orch,
            message_type: "result_sharing".into(),
            timestamp: Utc::now(),
            content: "done".into(),
        });

        // Now aggregate: 3 distinct (from,to) pairs, with the expected counts
        // and the most-recent message_type for each edge.
        let entries = a2a.recent_messages(300);
        let mut aggregates: HashMap<(AgentId, AgentId), (u32, String)> = HashMap::new();
        for entry in &entries {
            let agg = aggregates
                .entry((entry.from, entry.to))
                .or_insert((0, String::new()));
            agg.0 = agg.0.saturating_add(1);
            agg.1 = entry.message_type.clone();
        }

        // orchestrator -> worker-a: count=2, last_kind=task_delegation
        let e1 = aggregates.get(&(orch, worker_a)).expect("edge 1 missing");
        assert_eq!(e1.0, 2, "orch->worker_a count");
        assert_eq!(e1.1, "task_delegation", "orch->worker_a last_kind");

        // orchestrator -> worker-b: count=1, last_kind=task_delegation
        let e2 = aggregates.get(&(orch, worker_b)).expect("edge 2 missing");
        assert_eq!(e2.0, 1, "orch->worker_b count");
        assert_eq!(e2.1, "task_delegation", "orch->worker_b last_kind");

        // worker-b -> orchestrator: count=1, last_kind=status_update
        let e3 = aggregates.get(&(worker_b, orch)).expect("edge 3 missing");
        assert_eq!(e3.0, 1, "worker_b->orch count");
        assert_eq!(e3.1, "status_update", "worker_b->orch last_kind");

        // worker-a -> orchestrator: count=1, last_kind=result_sharing (fan-in)
        let e4 = aggregates.get(&(worker_a, orch)).expect("edge 4 missing");
        assert_eq!(e4.0, 1, "worker_a->orch count");
        assert_eq!(e4.1, "result_sharing", "worker_a->orch last_kind");

        // Total of 4 distinct edges.
        assert_eq!(aggregates.len(), 4);
    }
}
