//! Gateway: routes messages between channels and the kernel.
//!
//! The gateway is channel-agnostic. It receives messages from any
//! registered channel via a shared mpsc channel, dispatches them
//! concurrently to the kernel via the orchestrator, and returns
//! responses through the appropriate channel.
//!
//! Architecture:
//! ```text
//! ┌────────────┐  ┌────────────┐  ┌──────────────┐
//! │ Web task   │  │ CLI task   │  │Telegram task │   ← independent receive
//! └─────┬──────┘  └─────┬──────┘  └──────┬───────┘
//!       └───────────────┼────────────────┘
//!                       ▼
//!              ┌────────────────┐
//!              │  Gateway rx    │  ← tokio::select!
//!              │  (mpsc 1024)   │
//!              └───────┬────────┘
//!                      │  tokio::spawn per message
//!               ┌──────┼──────┐
//!               ▼      ▼      ▼
//!            ┌─────┐┌─────┐┌─────┐
//!            │route ││route ││route │  ← Semaphore limits concurrency
//!            └──┬──┘└──┬──┘└──┬──┘
//!               └──────┼──────┘
//!                      ▼
//!               channel.send()
//! ```

use anyhow::Result;
use oxios_ouroboros::FailureClass;
use serde_json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::GatewayInbox;
use crate::channel::Channel;
use crate::error_classify::classify_error;
use crate::message::{ErrorKind, IncomingMessage, OutgoingMessage, ResponseMeta, UserFacingError};
use crate::meta::meta;
use crate::reliability::ReliabilityLayer;

/// Gateway receive buffer size.
///
/// All channels push into this shared buffer. 1024 ≈ 100 concurrent sessions × 10 messages each.
const GATEWAY_BUFFER: usize = 1024;

/// Maximum concurrent orchestrations.
///
/// LLM calls are I/O-bound, so this is higher than CPU core count.
/// 32 = 4 cores × 8× headroom (other tasks run during I/O waits).
const MAX_CONCURRENT_ROUTES: usize = 32;

/// Graceful shutdown timeout per channel task / dispatch task (F20/F27).
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// F21: how many times we retry a `channel.send()` before declaring the
/// message a dead letter.
const SEND_MAX_RETRIES: u32 = 3;

/// F21: base backoff between send retries; multiplied by the attempt number.
const SEND_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Registered channel entry.
struct ChannelEntry {
    /// Channel trait object — for send() calls.
    channel: Arc<dyn Channel>,
    /// Per-channel shutdown signal.
    shutdown_tx: watch::Sender<bool>,
    /// Task handle from start() — for lifetime tracking.
    task: JoinHandle<()>,
}

/// Gateway: routes messages between channels and the kernel.
pub struct Gateway {
    /// Channel registry (register/unregister/send).
    channels: Arc<RwLock<HashMap<String, ChannelEntry>>>,

    /// Unified receive endpoint — all channels push here.
    rx: Mutex<mpsc::Receiver<GatewayInbox>>,

    /// Sender clone — passed to new channels during register().
    tx: mpsc::Sender<GatewayInbox>,

    /// Orchestrator reference for message dispatch.
    orchestrator: Arc<oxios_kernel::Orchestrator>,

    /// Engine API for model switching (action-based routing).
    engine_api: Option<Arc<oxios_kernel::EngineApi>>,

    /// Persona API for persona switching (action-based routing).
    persona_api: Option<Arc<oxios_kernel::PersonaApi>>,
    /// Gateway-wide shutdown signal.
    shutdown: watch::Sender<bool>,
    /// Sender-side mirror of the shutdown flag so `is_shutdown()` can read
    /// it without a receiver. Sender-only `watch::Sender::borrow` is stale;
    /// mirroring keeps the producer-side check truthful for diagnostics.
    shutdown_flag: Arc<std::sync::atomic::AtomicBool>,

    /// Concurrency limiter for route tasks. A permit is acquired in `run()`
    /// *before* spawning each dispatch task (F19), bounding in-flight tasks
    /// to `MAX_CONCURRENT_ROUTES`; the mpsc buffer absorbs the burst.
    concurrency: Arc<Semaphore>,

    /// F20: JoinHandles of in-flight dispatch tasks. Bounded by the
    /// semaphore size since permits are acquired before spawn. Reaped on
    /// each dispatch and drained on shutdown so `run()` can await them.
    in_flight: Arc<Mutex<Vec<JoinHandle<()>>>>,

    /// RFC-024 SP1: delivery reliability layer — assigns a monotonic `seq`
    /// to each outgoing message and keeps a bounded ring buffer for replay.
    /// Cheap to clone; the inner state is `Sync`.
    reliability: Arc<ReliabilityLayer>,
    /// RFC-015 P1: shared with `KernelHandle.streaming_sinks` so the runtime
    /// callback's `TextChunk` lookup finds the gateway's collector sender
    /// for the active session. The kernel assembler wires both sides to the
    /// same `Arc<StreamingSinkRegistry>` at boot.
    streaming_sinks: Arc<oxios_kernel::streaming_sink::StreamingSinkRegistry>,
}

/// F21: deliver a message through a channel with bounded retries and linear
/// backoff. If all retries fail, the message is logged as a dead letter (with
/// its seq/user/channel for forensic recovery) instead of vanishing silently.
///
/// This protects channels that have no reconnect-replay concept (Telegram,
/// CLI): even without a client-initiated `replay(last_seq)`, transient send
/// failures get a chance to recover rather than being dropped on the floor.
async fn send_with_retry(channel: &Arc<dyn Channel>, msg: OutgoingMessage) -> Result<()> {
    let m = oxios_kernel::metrics::get_metrics();
    for attempt in 1..=SEND_MAX_RETRIES {
        match channel.send(msg.clone()).await {
            Ok(()) => {
                // RFC-024 §11: count delivered messages. Every call site
                // (dispatch, send_to, action handlers) routes through this
                // helper, so a single counter here is enough.
                m.gateway_messages_delivered.inc();
                return Ok(());
            }
            Err(e) => {
                if attempt >= SEND_MAX_RETRIES {
                    tracing::error!(
                        error = %e,
                        seq = ?msg.seq,
                        channel = %msg.channel,
                        user = %msg.user_id,
                        attempts = SEND_MAX_RETRIES,
                        "Failed to send message after retries; dead-lettering"
                    );
                    // RFC-024 §11: dead-lettered after exhausting retries.
                    m.gateway_messages_dropped.inc();
                    return Err(e);
                }
                tracing::warn!(
                    error = %e,
                    attempt,
                    "Channel send failed; retrying after backoff"
                );
                tokio::time::sleep(SEND_RETRY_DELAY * attempt).await;
            }
        }
    }
    // Unreachable: the loop always returns on the last attempt.
    #[allow(unreachable_code)]
    Ok(())
}

impl Gateway {
    /// Creates a new gateway with the given orchestrator.
    pub fn new(orchestrator: Arc<oxios_kernel::Orchestrator>) -> Self {
        let (tx, rx) = mpsc::channel(GATEWAY_BUFFER);
        let (shutdown, _) = watch::channel(false);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            rx: Mutex::new(rx),
            tx,
            orchestrator,
            engine_api: None,
            persona_api: None,
            shutdown,
            shutdown_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_ROUTES)),
            in_flight: Arc::new(Mutex::new(Vec::new())),
            reliability: Arc::new(ReliabilityLayer::new(Default::default())),
            streaming_sinks: Arc::new(oxios_kernel::streaming_sink::StreamingSinkRegistry::new()),
        }
    }

    /// Creates a new gateway with engine and persona APIs for action-based routing.
    pub fn with_apis(
        orchestrator: Arc<oxios_kernel::Orchestrator>,
        engine_api: Arc<oxios_kernel::EngineApi>,
        persona_api: Arc<oxios_kernel::PersonaApi>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(GATEWAY_BUFFER);
        let (shutdown, _) = watch::channel(false);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            rx: Mutex::new(rx),
            tx,
            orchestrator,
            engine_api: Some(engine_api),
            persona_api: Some(persona_api),
            shutdown,
            shutdown_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_ROUTES)),
            in_flight: Arc::new(Mutex::new(Vec::new())),
            reliability: Arc::new(ReliabilityLayer::new(Default::default())),
            streaming_sinks: Arc::new(oxios_kernel::streaming_sink::StreamingSinkRegistry::new()),
        }
    }

    /// Attaches a streaming sink registry for message-level streaming.
    /// Both sides must point at the same `Arc`.
    pub fn with_streaming_sinks(
        mut self,
        registry: Arc<oxios_kernel::streaming_sink::StreamingSinkRegistry>,
    ) -> Self {
        self.streaming_sinks = registry;
        self
    }

    /// Signal the gateway to stop its event loop.
    pub fn signal_shutdown(&self) {
        self.shutdown_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.shutdown.send(true);
        tracing::info!("Gateway shutdown signal sent");
    }
    /// Check if shutdown has been signalled.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown_flag
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    // ── Channel management ──────────────────────────────────

    /// Registers a channel with the gateway and starts its background receive task.
    pub async fn register(&self, channel: Box<dyn Channel>) -> Result<()> {
        let name = channel.name().to_owned();
        let (ch_shutdown, ch_shutdown_rx) = watch::channel(false);

        // Wrap in Arc for shared access from route tasks.
        let ch_arc: Arc<dyn Channel> = Arc::from(channel);

        // Start the channel's background receive loop.
        let task = ch_arc.start(self.tx.clone(), ch_shutdown_rx).await?;

        self.channels.write().await.insert(
            name.clone(),
            ChannelEntry {
                channel: ch_arc,
                shutdown_tx: ch_shutdown,
                task,
            },
        );

        tracing::info!(channel = %name, "Channel registered and started");
        Ok(())
    }

    /// Unregisters a channel, signalling it to stop and waiting for its task to finish.
    pub async fn unregister(&self, name: &str) -> Result<()> {
        let entry = self.channels.write().await.remove(name);
        if let Some(entry) = entry {
            let _ = entry.shutdown_tx.send(true);
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, entry.task).await;
            tracing::info!(channel = %name, "Channel unregistered");
        }
        Ok(())
    }

    /// Returns the names of all registered channels.
    pub async fn channel_names(&self) -> Vec<String> {
        self.channels.read().await.keys().cloned().collect()
    }

    /// Returns a registered channel's live status
    /// ([`Channel::status`](crate::channel::Channel::status)), or `None`
    /// when no channel with that name is registered.
    pub async fn channel_status(&self, name: &str) -> Option<serde_json::Value> {
        self.channels
            .read()
            .await
            .get(name)
            .map(|e| e.channel.status())
    }

    // ── Event loop ──────────────────────────────────────────

    /// Runs the gateway event loop.
    ///
    /// Receives from the shared mpsc channel (any channel can push).
    /// Each message is dispatched to an independent task for concurrent
    /// processing. A concurrency permit is acquired *before* spawning each
    /// dispatch task (F19), so the number of in-flight tasks is bounded by
    /// `MAX_CONCURRENT_ROUTES`; excess messages wait in the mpsc buffer,
    /// producing natural backpressure instead of unbounded task allocation.
    /// Returns when all senders are dropped or shutdown is signalled.
    pub async fn run(&self) -> Result<()> {
        tracing::info!("Gateway event loop started");
        let mut rx = self.rx.lock().await;
        let mut shutdown = self.shutdown.subscribe();

        let shutdown_requested = loop {
            tokio::select! {
                inbox = rx.recv() => {
                    match inbox {
                        Some((channel_name, msg)) => {
                            // F19: acquire the concurrency permit before spawning
                            // so in-flight tasks are bounded. The mpsc buffer
                            // absorbs bursts; if all permits are held, recv
                            // effectively pauses until one frees up.
                            let permit = match self.concurrency.clone().acquire_owned().await {
                                Ok(p) => p,
                                Err(_) => {
                                    tracing::warn!("Semaphore closed, dropping message");
                                    continue;
                                }
                            };
                            // Shutdown may have been signalled while we waited
                            // for the permit — re-check before dispatching.
                            if *shutdown.borrow() {
                                tracing::info!(
                                    "Shutdown signalled during dispatch; dropping unprocessed message"
                                );
                                break true;
                            }
                            self.dispatch(channel_name, msg, permit);
                        }
                        None => {
                            tracing::info!("All channels disconnected, exiting");
                            break false;
                        }
                    }
                }

                _ = shutdown.changed() => {
                    tracing::info!("Gateway shutting down");
                    break true;
                }
            }
        };

        if shutdown_requested {
            self.finish_shutdown().await?;
        }
        Ok(())
    }

    /// F20/F27: signal all channels to stop and await in-flight dispatch tasks
    /// and channel receive tasks with a bounded timeout. Ensures no message
    /// processing or channel I/O is abandoned mid-flight on shutdown.
    async fn finish_shutdown(&self) -> Result<()> {
        // F20: drain and await in-flight dispatch tasks.
        let dispatch_handles: Vec<JoinHandle<()>> = self.in_flight.lock().await.drain(..).collect();
        for handle in dispatch_handles {
            if tokio::time::timeout(SHUTDOWN_TIMEOUT, handle)
                .await
                .is_err()
            {
                tracing::warn!("Dispatch task did not finish within shutdown timeout; abandoning");
            }
        }

        // F27: signal and await channel receive tasks. Drain the registry so
        // we can own the JoinHandles (they aren't Clone).
        let entries: Vec<(String, ChannelEntry)> = {
            let mut channels = self.channels.write().await;
            channels.drain().collect()
        };
        for (name, entry) in entries {
            let _ = entry.shutdown_tx.send(true);
            match tokio::time::timeout(SHUTDOWN_TIMEOUT, entry.task).await {
                Ok(Ok(())) => tracing::info!(channel = %name, "Channel task completed"),
                Ok(Err(e)) => tracing::warn!(channel = %name, error = %e, "Channel task panicked"),
                Err(_) => tracing::warn!(
                    channel = %name,
                    "Channel task did not finish within shutdown timeout"
                ),
            }
        }

        Ok(())
    }

    /// Dispatch an incoming message to an independent task.
    ///
    /// The concurrency `permit` is acquired by `run()` before calling this,
    /// bounding in-flight tasks (F19). The permit is held for the lifetime of
    /// the spawned task.
    ///
    /// If the message contains an `action` metadata key, it is routed to the
    /// appropriate API handler (the permit is released — actions are cheap,
    /// no LLM call) instead of the orchestrator.
    fn dispatch(&self, channel_name: String, msg: IncomingMessage, permit: OwnedSemaphorePermit) {
        // ── Action-based routing ───────────────────────────────────
        if let Some(action) = msg.metadata.get(meta::ACTION).cloned() {
            match action.as_str() {
                "switch_model" => {
                    drop(permit); // actions don't consume the concurrency permit
                    self.dispatch_switch_model(channel_name, msg);
                    return;
                }
                "switch_persona" => {
                    drop(permit);
                    self.dispatch_switch_persona(channel_name, msg);
                    return;
                }
                _ => {
                    tracing::warn!(action = %action, "Unknown action metadata, forwarding to orchestrator");
                    // Fall through to normal routing
                }
            }
        }

        // ── Normal orchestrator routing ────────────────────────────
        let orchestrator = self.orchestrator.clone();
        let channels = self.channels.clone();
        let reliability = self.reliability.clone();
        let in_flight = self.in_flight.clone();
        let streaming_sinks = self.streaming_sinks.clone();

        // F20: track the handle so shutdown can await it. Reap finished
        // handles to keep the vec bounded.
        let handle = tokio::spawn(async move {
            // _permit is held for the task lifetime — released on drop.
            let _permit = permit;

            tracing::info!(
                channel = %msg.channel,
                user = %msg.user_id,
                content_len = msg.content.len(),
                request_id = %msg.id,
                "Routing incoming message"
            );

            // ── Duration measurement ──
            let start = std::time::Instant::now();

            let session_id = msg.metadata.get(meta::SESSION_ID).cloned();
            let project_ids = msg.metadata.get(meta::PROJECT_IDS).cloned();
            let mount_ids = msg.metadata.get(meta::MOUNT_IDS).cloned();
            let conn_id = msg.metadata.get("conn_id").cloned();
            let request_id = msg.id.to_string();

            // ── Streaming sink collector (P1 chat transparency) ──────────
            // Create the mpsc the runtime callback will push live text
            // deltas into via the registry. Acquire the channel Arc here
            // (one F22 release point) so the collector can deliver partials
            // immediately. Register a Weak in the registry so the runtime
            // callback's lookup finds the sender for this session.
            //
            // Only channels that opted in via `Channel::supports_streaming`
            // get a collector. Non-streaming channels (Telegram, CLI, RPC
            // bridges) would otherwise receive one delivery per token delta
            // plus stream-control markers — on Telegram that means one Bot
            // API message per token. They still get the single terminal
            // response below, and the runtime's sink lookup silently misses
            // (agent_runtime.rs drops deltas when no sink is registered).
            //
            // Sender-drop ordering is load-bearing: when we `drop(sender_arc)`
            // after `handle_unified` returns and `collector.await` drains,
            // all partials reach the WS BEFORE the terminal `done` is built
            // (advisory: collector race). No async gap between sender drop
            // and join — the JoinHandle completes when the receiver's last
            // value is processed.
            let channel_for_collector: Option<Arc<dyn Channel>> = {
                let guard = channels.read().await;
                guard
                    .get(&channel_name)
                    .map(|e| e.channel.clone())
                    .filter(|c| c.supports_streaming())
            };
            let collector = channel_for_collector.map(|channel| {
                let (tx, mut rx) =
                    tokio::sync::mpsc::channel::<oxios_kernel::agent_runtime::StreamDelta>(
                        oxios_kernel::streaming_sink::STREAMING_CHANNEL_CAPACITY,
                    );
                let sender_arc: oxios_kernel::streaming_sink::StreamingSinkSender = Arc::new(tx);
                // RFC-033: register under the same key the agent runtime will
                // look up — the chat session id, or the request id for a
                // session's first message (the WS client sends no session_id
                // until the server returns one). The orchestrator sets
                // ExecEnv.session_id to ctx.session_id (the same value), so
                // token/tool/thinking deltas reach this collector.
                let sink_session_key = session_id.clone().unwrap_or_else(|| request_id.clone());
                streaming_sinks.register(&sink_session_key, &sender_arc);
                let channel = channel.clone();
                let conn_id = conn_id.clone();
                // RFC-033: partial token messages + unregister use the same
                // resolved key as registration, so the runtime's sink lookup
                // and the chat.rs event filter (active_session_id) all agree.
                let session_id_for_collector = sink_session_key;
                let channel_name_for_collector = channel_name.clone();
                let user_id = msg.user_id.clone();
                let streaming_sinks_for_unregister = streaming_sinks.clone();
                let handle = tokio::spawn(async move {
                    // Phase B: track reasoning lifecycle so we can emit
                    // explicit reasoning.start / reasoning.end markers for
                    // the frontend's Thinking block animation.
                    let mut was_reasoning = false;
                    while let Some(delta) = rx.recv().await {
                        match delta {
                            oxios_kernel::agent_runtime::StreamDelta::Text(text) => {
                                // Phase B: emit reasoning.end before the first
                                // text chunk if we were in a reasoning span.
                                if was_reasoning {
                                    was_reasoning = false;
                                    let mut end_msg = OutgoingMessage::with_id(
                                        uuid::Uuid::new_v4(),
                                        channel_name_for_collector.clone(),
                                        user_id.clone(),
                                        String::new(),
                                    );
                                    end_msg.target_conn_id = conn_id.clone();
                                    end_msg.metadata.insert(
                                        "stream_kind".to_string(),
                                        "reasoning.end".to_string(),
                                    );
                                    end_msg.metadata.insert(
                                        meta::SESSION_ID.to_string(),
                                        session_id_for_collector.clone(),
                                    );
                                    let _ = send_with_retry(&channel, end_msg).await;
                                }
                                let mut outgoing = OutgoingMessage::with_id(
                                    uuid::Uuid::new_v4(),
                                    channel_name_for_collector.clone(),
                                    user_id.clone(),
                                    text,
                                );
                                outgoing.target_conn_id = conn_id.clone();
                                outgoing = outgoing.with_partial(true);
                                outgoing.metadata.insert(
                                    meta::SESSION_ID.to_string(),
                                    session_id_for_collector.clone(),
                                );
                                // Skip `reliability.assign_seq` on partials:
                                // `assign_seq` pushes into the replay buffer
                                // (reliability.rs:81-88), and on WS reconnect
                                // the buffer replays every delta. The frontend
                                // would then re-merge all those text prefixes
                                // into the assistant message, producing
                                // duplicated/garbled output. Partials are
                                // ephemeral: FIFO ordering comes from the
                                // mpsc, each carries a unique `Uuid` for the
                                // frontend's seen-id ring, and reconnect
                                // recovery uses the terminal (seq'd, full
                                // text) instead — no data lost (advisory).
                                let _ = send_with_retry(&channel, outgoing).await;
                            }
                            oxios_kernel::agent_runtime::StreamDelta::Model(model_id) => {
                                // One-shot model announcement at the start of a
                                // streaming turn. Forwarded as a `model` chunk
                                // (no partial flag, no terminal) so the chat
                                // store can attach the real model to the
                                // in-flight assistant message immediately,
                                // including after fallback resolution.
                                let mut outgoing = OutgoingMessage::with_id(
                                    uuid::Uuid::new_v4(),
                                    channel_name_for_collector.clone(),
                                    user_id.clone(),
                                    String::new(),
                                );
                                outgoing.target_conn_id = conn_id.clone();
                                outgoing
                                    .metadata
                                    .insert("stream_kind".to_string(), "model".to_string());
                                outgoing.metadata.insert("model".to_string(), model_id);
                                outgoing.metadata.insert(
                                    meta::SESSION_ID.to_string(),
                                    session_id_for_collector.clone(),
                                );
                                let _ = send_with_retry(&channel, outgoing).await;
                            }
                            oxios_kernel::agent_runtime::StreamDelta::Thinking => {
                                // Phase B: emit an explicit reasoning.start marker
                                // so the frontend's Thinking block can auto-expand.
                                // The runtime pairs a `Thinking` marker with EVERY
                                // `ThinkingDelta` (agent_runtime.rs), so gate on the
                                // span transition — otherwise one start marker per
                                // delta floods the WS (the frontend's reopen logic
                                // absorbs them, but the frames are pure waste).
                                if was_reasoning {
                                    continue;
                                }
                                was_reasoning = true;
                                let mut start_msg = OutgoingMessage::with_id(
                                    uuid::Uuid::new_v4(),
                                    channel_name_for_collector.clone(),
                                    user_id.clone(),
                                    String::new(),
                                );
                                start_msg.target_conn_id = conn_id.clone();
                                start_msg.metadata.insert(
                                    "stream_kind".to_string(),
                                    "reasoning.start".to_string(),
                                );
                                start_msg.metadata.insert(
                                    meta::SESSION_ID.to_string(),
                                    session_id_for_collector.clone(),
                                );
                                let _ = send_with_retry(&channel, start_msg).await;
                            }
                            oxios_kernel::agent_runtime::StreamDelta::ThinkingDelta(text) => {
                                // Phase B: ensure was_reasoning is set even if
                                // the Thinking signal was missed (defensive).
                                was_reasoning = true;
                                // Batched reasoning fragment — same partial
                                // delivery rules as Text (partial=true, no
                                // assign_seq, conn_id scoped). Emitted as a
                                // `reasoning` WS chunk (NOT `token`) so the
                                // frontend's existing chunkToActivity routes
                                // it into the activity timeline (reasoning
                                // activity card) instead of the answer text.
                                // The chat store merges consecutive
                                // reasoning chunks into a single activity
                                // (see chunkToActivity merge logic).
                                let mut outgoing = OutgoingMessage::with_id(
                                    uuid::Uuid::new_v4(),
                                    channel_name_for_collector.clone(),
                                    user_id.clone(),
                                    text,
                                );
                                outgoing.target_conn_id = conn_id.clone();
                                outgoing = outgoing.with_partial(true);
                                outgoing
                                    .metadata
                                    .insert("stream_kind".to_string(), "reasoning".to_string());
                                outgoing.metadata.insert(
                                    meta::SESSION_ID.to_string(),
                                    session_id_for_collector.clone(),
                                );
                                let _ = send_with_retry(&channel, outgoing).await;
                            }
                            oxios_kernel::agent_runtime::StreamDelta::ThinkingEnd => {
                                // oxi 0.58+: authoritative reasoning-end signal.
                                // Reset the shared flag so the first-Text
                                // heuristic (kept as a fallback for
                                // reasoning_content models that never emit
                                // ThinkingEnd) does not double-fire.
                                was_reasoning = false;
                                let mut end_msg = OutgoingMessage::with_id(
                                    uuid::Uuid::new_v4(),
                                    channel_name_for_collector.clone(),
                                    user_id.clone(),
                                    String::new(),
                                );
                                end_msg.target_conn_id = conn_id.clone();
                                end_msg
                                    .metadata
                                    .insert("stream_kind".to_string(), "reasoning.end".to_string());
                                end_msg.metadata.insert(
                                    meta::SESSION_ID.to_string(),
                                    session_id_for_collector.clone(),
                                );
                                let _ = send_with_retry(&channel, end_msg).await;
                            }
                            _ => {}
                        }
                    }
                    // Phase B: if the stream ended while still reasoning (no
                    // Text chunk followed), emit a final reasoning.end.
                    if was_reasoning {
                        let mut end_msg = OutgoingMessage::with_id(
                            uuid::Uuid::new_v4(),
                            channel_name_for_collector.clone(),
                            user_id.clone(),
                            String::new(),
                        );
                        end_msg.target_conn_id = conn_id.clone();
                        end_msg
                            .metadata
                            .insert("stream_kind".to_string(), "reasoning.end".to_string());
                        end_msg.metadata.insert(
                            meta::SESSION_ID.to_string(),
                            session_id_for_collector.clone(),
                        );
                        let _ = send_with_retry(&channel, end_msg).await;
                    }
                    streaming_sinks_for_unregister.unregister(&session_id_for_collector);
                });
                // Returned so outer code can drop the strong sender and
                // await the JoinHandle — guarantees all partials reach the
                // WS BEFORE the terminal `done` is built (advisory:
                // collector race).
                (sender_arc, handle)
            });

            let role = msg.metadata.get("role").map(String::as_str);
            let model_override = msg.metadata.get("model_override").map(String::as_str);
            // Parse optional per-message model params from metadata. Strings
            // are used so the WS layer doesn't need a structured payload
            // change — the gateway translates them to typed ModelParams here.
            let temperature = msg
                .metadata
                .get("temperature")
                .and_then(|v| v.parse::<f64>().ok());
            let max_tokens = msg
                .metadata
                .get("max_tokens")
                .and_then(|v| v.parse::<u32>().ok());
            let model_params = if temperature.is_some() || max_tokens.is_some() {
                Some(oxios_ouroboros::ModelParams {
                    temperature,
                    max_tokens,
                })
            } else {
                None
            };
            let result = orchestrator
                .handle_unified(
                    &msg.user_id,
                    &msg.content,
                    session_id.as_deref(),
                    project_ids.as_deref(),
                    mount_ids.as_deref(),
                    role,
                    model_override,
                    model_params,
                    &request_id,
                )
                .await;

            // RFC-015 P1: drop the strong sender and await the collector.
            // This is load-bearing for ordering — when the runtime callback
            // stops pushing (turn done), dropping the gateway's strong sender
            // makes the collector's `rx.recv()` return None. Awaiting the
            // JoinHandle guarantees every partial has been delivered to the
            // channel before the terminal `done` is built below. Without
            // this, the terminal message could arrive before the last text
            // delta (advisory: collector race).
            if let Some((sender, handle)) = collector {
                drop(sender);
                let _ = handle.await;
            }

            let duration_ms = start.elapsed().as_millis() as u64;

            // F22: clone the channel Arc out of the lock and release the
            // guard before the (potentially slow) send, so register/
            // unregister aren't blocked during delivery.
            let channel: Option<Arc<dyn Channel>> = {
                let guard = channels.read().await;
                guard.get(&channel_name).map(|e| e.channel.clone())
            };

            match (result, channel) {
                (Ok(orchestration), Some(channel)) => {
                    tracing::info!(
                        phase = %orchestration.phase_reached,
                        duration_ms = duration_ms,
                        "Orchestration complete"
                    );

                    // Channel-specific metadata (chat_id, message_id, etc.)
                    let mut channel_meta = HashMap::new();
                    if let Some(ref sid) = orchestration.session_id {
                        channel_meta.insert(meta::SESSION_ID.to_owned(), sid.clone());
                    }
                    if let Some(ref pid) = orchestration.primary_project_id {
                        channel_meta.insert(meta::PROJECT_IDS.to_owned(), pid.to_string());
                    }
                    // RFC-025: surface active Mount IDs so the frontend can show
                    // a detection badge and bind them on follow-up turns.
                    if !orchestration.active_mount_ids.is_empty()
                        && let Ok(json) = serde_json::to_string(&orchestration.active_mount_ids)
                    {
                        channel_meta.insert(meta::MOUNT_IDS.to_owned(), json);
                    }
                    if let Some(ref mtag) = orchestration.mount_tag {
                        channel_meta.insert("mount_tag".to_owned(), mtag.clone());
                    }
                    // Serialize tool_calls into metadata so web routes can read them.
                    if !orchestration.tool_calls.is_empty()
                        && let Ok(json) = serde_json::to_string(&orchestration.tool_calls)
                    {
                        channel_meta.insert("tool_calls".to_owned(), json);
                    }
                    // P4 (§7 persistence): surface reasoning_text so chat.rs can
                    // persist it alongside tool_calls and loadSession can
                    // restore the reasoning activity on reopen.
                    if !orchestration.reasoning_text.is_empty() {
                        channel_meta.insert(
                            "reasoning_text".to_owned(),
                            orchestration.reasoning_text.clone(),
                        );
                    }
                    // Block-stream transparency: positioned reasoning spans
                    // so chat.rs can persist them for reopen fidelity.
                    if !orchestration.reasoning_segments.is_empty()
                        && let Ok(json) = serde_json::to_string(&orchestration.reasoning_segments)
                    {
                        channel_meta.insert("reasoning_segments".to_owned(), json);
                    }
                    // Typed orchestration metadata (RFC-014)
                    let response_meta = ResponseMeta {
                        session_id: orchestration.session_id,
                        project_id: orchestration.primary_project_id.map(|u| u.to_string()),
                        project_tag: orchestration.project_tag,
                        phase: orchestration.phase_reached.to_string(),
                        evaluation_passed: orchestration.evaluation_passed,
                        duration_ms: Some(duration_ms),
                        error: orchestration
                            .failure_class
                            .map(|fc| failure_class_to_user_error(fc, &orchestration.response)),
                        // questions — the frontend falls back to markdown.
                        interview_questions: orchestration.interview_questions,
                        interview_round: orchestration.interview_round,
                    };

                    let mut outgoing = OutgoingMessage::success(
                        msg.id,
                        &msg.channel,
                        &msg.user_id,
                        &orchestration.response,
                        channel_meta,
                        response_meta,
                    );
                    outgoing.target_conn_id = conn_id;
                    let outgoing = reliability.assign_seq(outgoing);
                    // F21: retry with backoff before dead-lettering.
                    let _ = send_with_retry(&channel, outgoing).await;
                }
                (Err(e), Some(channel)) => {
                    tracing::error!(error = %e, "Orchestration failed");
                    let user_err = classify_error(&e);

                    // Preserve session_id in error response for conversation continuity
                    let mut outgoing =
                        OutgoingMessage::error(msg.id, &msg.channel, &msg.user_id, user_err);
                    if let Some(sid) = msg.metadata.get(meta::SESSION_ID).cloned() {
                        outgoing.metadata.insert(meta::SESSION_ID.to_string(), sid);
                    }
                    outgoing.target_conn_id = conn_id;

                    let outgoing = reliability.assign_seq(outgoing);
                    let _ = send_with_retry(&channel, outgoing).await;
                }
                (_, None) => {
                    // Channel was unregistered between dispatch and the
                    // completion callback. The HTTP request's oneshot in
                    // `WebBridge::responses` will time out at
                    // `send_and_wait`'s deadline, returning 504 to the
                    // client — so the C1 contract is still satisfied,
                    // just at the deadline rather than immediately.
                    // RFC-024 §A4: surface the request_id and channel for
                    // diagnosis (the previous `warn!` line was too quiet
                    // to be noticed in production).
                    oxios_kernel::metrics::get_metrics()
                        .gateway_messages_dropped
                        .inc();
                    tracing::error!(
                        channel = %channel_name,
                        request_id = %msg.id,
                        "Channel no longer registered; HTTP request will hit \
                         send_and_wait timeout (C1 still satisfied via 504)"
                    );
                }
            }
        });

        // Track + reap in one pass.
        if let Ok(mut in_flight) = in_flight.try_lock() {
            in_flight.retain(|h| !h.is_finished());
            in_flight.push(handle);
        }
        // If try_lock fails (someone else holds it, e.g. shutdown drain),
        // the handle is still tracked by the runtime — worst case we don't
        // await it on shutdown, which is the pre-fix behavior.
    }

    // ── Public utilities ────────────────────────────────────

    /// Sends a message through the named channel.
    ///
    /// F22: releases the channels read lock before the send.
    /// Returns Ok(()) even when the channel isn't registered (warns and
    /// drops the message) — existing contract for fire-and-forget callers.
    pub async fn send_to(&self, channel_name: &str, msg: OutgoingMessage) -> Result<()> {
        let channel = {
            let channels = self.channels.read().await;
            channels.get(channel_name).map(|e| e.channel.clone())
        };
        let Some(channel) = channel else {
            // Unknown channel: log and succeed (existing contract). The
            // message is dropped — callers that need delivery guarantees
            // must check channel existence first. Returning Err here broke
            // fire-and-forget callers of optional channels.
            tracing::warn!(
                channel = channel_name,
                "No such channel registered; dropping outgoing message"
            );
            return Ok(());
        };
        let msg = self.reliability.assign_seq(msg);
        send_with_retry(&channel, msg).await
    }

    // ── Action dispatch handlers ───────────────────────────

    /// Handle a `switch_model` action by calling EngineApi::set_model().
    fn dispatch_switch_model(&self, channel_name: String, msg: IncomingMessage) {
        let engine_api = self.engine_api.clone();
        let channels = self.channels.clone();
        let reliability = self.reliability.clone();
        let conn_id = msg.metadata.get("conn_id").cloned();

        tokio::spawn(async move {
            let model_id = msg
                .metadata
                .get(meta::MODEL_ID)
                .cloned()
                .unwrap_or_default();

            tracing::info!(
                channel = %msg.channel,
                model_id = %model_id,
                request_id = %msg.id,
                "Routing switch_model action"
            );
            // F22: clone the channel Arc out of the lock so the read guard
            // is released before the (potentially slow) send.
            let channel: Option<Arc<dyn Channel>> = {
                let guard = channels.read().await;
                guard.get(&channel_name).map(|e| e.channel.clone())
            };

            match (engine_api, channel) {
                (Some(api), Some(channel)) => match api.set_model(&model_id) {
                    Ok(()) => {
                        let response = format!("✅ Switched model to {model_id}.");
                        let mut outgoing = OutgoingMessage::success(
                            msg.id,
                            &msg.channel,
                            &msg.user_id,
                            &response,
                            HashMap::new(),
                            ResponseMeta {
                                session_id: None,
                                project_id: None,
                                project_tag: None,
                                phase: "action".to_string(),
                                evaluation_passed: Some(true),
                                duration_ms: None,
                                error: None,
                                interview_questions: None,
                                interview_round: None,
                            },
                        );
                        outgoing.target_conn_id = conn_id;
                        let outgoing = reliability.assign_seq(outgoing);
                        let _ = send_with_retry(&channel, outgoing).await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "switch_model failed");
                        let user_err = UserFacingError {
                            message: format!("❌ Failed to switch model: {e}"),
                            kind: ErrorKind::Internal,
                            suggestion: Some(
                                "Check that the model ID is correct (e.g. anthropic/claude-sonnet-4)."
                                    .to_string(),
                            ),
                        };
                        let mut outgoing =
                            OutgoingMessage::error(msg.id, &msg.channel, &msg.user_id, user_err);
                        outgoing.target_conn_id = conn_id;
                        let outgoing = reliability.assign_seq(outgoing);
                        let _ = send_with_retry(&channel, outgoing).await;
                    }
                },
                (None, _) => {
                    tracing::warn!("switch_model action received but no EngineApi configured");
                }
                (_, None) => {
                    tracing::warn!(channel = %channel_name, "Channel no longer registered");
                }
            }
        });
    }

    /// Handle a `switch_persona` action by calling PersonaApi::set_active().
    fn dispatch_switch_persona(&self, channel_name: String, msg: IncomingMessage) {
        let persona_api = self.persona_api.clone();
        let channels = self.channels.clone();
        let reliability = self.reliability.clone();
        let conn_id = msg.metadata.get("conn_id").cloned();

        tokio::spawn(async move {
            let persona_id = msg
                .metadata
                .get(meta::PERSONA_ID)
                .cloned()
                .unwrap_or_default();

            tracing::info!(
                channel = %msg.channel,
                persona_id = %persona_id,
                request_id = %msg.id,
                "Routing switch_persona action"
            );
            // F22: clone the channel Arc out of the lock so the read guard
            // is released before the (potentially slow) send.
            let channel: Option<Arc<dyn Channel>> = {
                let guard = channels.read().await;
                guard.get(&channel_name).map(|e| e.channel.clone())
            };

            match (persona_api, channel) {
                (Some(api), Some(channel)) => match api.set_active(&persona_id).await {
                    Ok(_) => {
                        let response = format!("✅ Switched persona to '{persona_id}'.");
                        let mut outgoing = OutgoingMessage::success(
                            msg.id,
                            &msg.channel,
                            &msg.user_id,
                            &response,
                            HashMap::new(),
                            ResponseMeta {
                                session_id: None,
                                project_id: None,
                                project_tag: None,
                                phase: "action".to_string(),
                                evaluation_passed: Some(true),
                                duration_ms: None,
                                error: None,
                                interview_questions: None,
                                interview_round: None,
                            },
                        );
                        outgoing.target_conn_id = conn_id;
                        let outgoing = reliability.assign_seq(outgoing);
                        let _ = send_with_retry(&channel, outgoing).await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "switch_persona failed");
                        let user_err = UserFacingError {
                                message: format!("❌ Failed to switch persona: {e}"),
                                kind: ErrorKind::Internal,
                                suggestion: Some("Check that the persona ID is correct (type .help to see available commands).".to_string()),
                            };
                        let mut outgoing =
                            OutgoingMessage::error(msg.id, &msg.channel, &msg.user_id, user_err);
                        outgoing.target_conn_id = conn_id;
                        let outgoing = reliability.assign_seq(outgoing);
                        let _ = send_with_retry(&channel, outgoing).await;
                    }
                },
                (None, _) => {
                    tracing::warn!("switch_persona action received but no PersonaApi configured");
                }
                (_, None) => {
                    tracing::warn!(channel = %channel_name, "Channel no longer registered");
                }
            }
        });
    }
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway").finish()
    }
}

/// Convert an `oxios_ouroboros::FailureClass` to a `UserFacingError` for
/// the frontend. The `response` text is already populated by the orchestrator
/// with a user-friendly message (RFC-032).
fn failure_class_to_user_error(fc: FailureClass, response: &str) -> UserFacingError {
    let kind = match fc {
        FailureClass::BudgetExceeded | FailureClass::QuotaExhausted => ErrorKind::ProviderError,
        FailureClass::AuthFailure => ErrorKind::ApiKeyMissing,
        FailureClass::ModelUnavailable => ErrorKind::ProviderError,
        FailureClass::ContextOverflow => ErrorKind::ExecutionFailed,
        FailureClass::Transient => ErrorKind::ProviderError,
        FailureClass::Unknown => ErrorKind::Internal,
    };
    UserFacingError {
        message: response.to_string(),
        kind,
        suggestion: None,
    }
}
