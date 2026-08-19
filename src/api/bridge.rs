//! Web bridge implementation.
//!
//! Implements the [`Channel`] trait for the web interface, allowing
//! the gateway to route messages to and from the HTTP API.
//!
//! Uses mpsc channels to bridge:
//! - **Incoming**: HTTP POST /api/chat → mpsc → Gateway → Kernel
//! - **Outgoing**: Kernel → Gateway → mpsc → WebSocket/SSE clients

use anyhow::Result;
use async_trait::async_trait;
use oxios_gateway::GatewayInbox;
use oxios_gateway::channel::Channel;
use oxios_gateway::message::{IncomingMessage, OutgoingMessage};
use oxios_gateway::{ReliabilityLayer, ReplayResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot, watch};

/// Typed error for the HTTP→gateway request/response bridge.
///
/// F14: lets the HTTP layer classify a timeout (→ 504) from other
/// failures (→ 500) by variant instead of by grepping the error message
/// string. Replaces the previous `e.to_string().contains("timeout")`
/// heuristic in `handle_chat`.
#[derive(Debug)]
pub enum BridgeSendError {
    /// The incoming channel could not enqueue the message (gateway gone).
    SendFailed(String),
    /// The gateway dropped the response channel without replying.
    ChannelDropped,
    /// The gateway did not reply within the configured deadline.
    Timeout,
}

impl std::fmt::Display for BridgeSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeSendError::SendFailed(msg) => {
                write!(f, "incoming channel send failed: {msg}")
            }
            BridgeSendError::ChannelDropped => write!(f, "gateway response channel dropped"),
            BridgeSendError::Timeout => write!(f, "gateway response timeout"),
        }
    }
}

impl std::error::Error for BridgeSendError {}

/// The web bridge adapter.
///
/// Bridges the axum HTTP server with the gateway's channel interface
/// using mpsc channels for message passing.
pub struct WebBridge {
    /// Receiver for incoming messages from the HTTP layer.
    /// `Option` so `start()` can take ownership via `take()`.
    incoming_rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Sender to pass to the HTTP layer for injecting messages.
    incoming_tx: mpsc::Sender<IncomingMessage>,
    /// Broadcaster for outgoing messages to WebSocket/SSE clients.
    outgoing_tx: broadcast::Sender<OutgoingMessage>,
    /// Correlation map for HTTP request-response matching.
    responses: Arc<RwLock<HashMap<uuid::Uuid, oneshot::Sender<OutgoingMessage>>>>,
    /// RFC-024 SP2: per-bridge reliability layer (independent of the
    /// gateway's global one) so WS resume replays go through the same
    /// broadcast channel that live messages use.
    reliability: Arc<ReliabilityLayer>,
}

impl WebBridge {
    /// Creates a new web bridge with a bounded message buffer and its own
    /// reliability layer (for WS resume/replay).
    pub fn new(buffer: usize, reliability: Arc<ReliabilityLayer>) -> Self {
        let (incoming_tx, incoming_rx) = mpsc::channel(buffer);
        let (outgoing_tx, _) = broadcast::channel(buffer);
        Self {
            incoming_rx: Mutex::new(Some(incoming_rx)),
            incoming_tx,
            outgoing_tx,
            responses: Arc::new(RwLock::new(HashMap::new())),
            reliability,
        }
    }

    /// Returns a sender that can be used by HTTP handlers to inject messages.
    pub fn sender(&self) -> mpsc::Sender<IncomingMessage> {
        self.incoming_tx.clone()
    }

    /// Returns a receiver for outgoing messages (used by WebSocket/SSE handlers).
    #[allow(dead_code)]
    pub fn subscribe_outgoing(&self) -> broadcast::Receiver<OutgoingMessage> {
        self.outgoing_tx.subscribe()
    }

    /// Send a message directly (for use in tests or direct API responses).
    #[allow(dead_code)]
    pub fn broadcast_outgoing(&self, msg: OutgoingMessage) -> Result<()> {
        let _ = self.outgoing_tx.send(msg);
        Ok(())
    }

    /// Deliver a response to the registered handler, if any.
    /// Also broadcasts for WebSocket/SSE clients.
    #[allow(dead_code)]
    pub async fn deliver_response(&self, msg: OutgoingMessage) -> Result<()> {
        let msg_id = msg.id;

        // Try to deliver to a registered HTTP handler first.
        {
            let mut responses = self.responses.write().await;
            if let Some(sender) = responses.remove(&msg_id) {
                let _ = sender.send(msg.clone());
            }
        }

        // Always broadcast for WebSocket/SSE clients.
        let _ = self.outgoing_tx.send(msg);

        tracing::debug!(msg_id = %msg_id, "Delivering response");
        Ok(())
    }
}

#[async_trait]
impl Channel for WebBridge {
    fn name(&self) -> &str {
        "web"
    }

    /// The WS/SSE broadcast path renders per-delta partials incrementally
    /// (chat.rs merges `partial`/`stream_kind` messages into the in-flight
    /// assistant message), so the gateway's streaming collector must run
    /// for this channel.
    fn supports_streaming(&self) -> bool {
        true
    }

    async fn start(
        &self,
        tx: mpsc::Sender<GatewayInbox>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let internal_rx = self.incoming_rx.lock().await.take();
        let Some(mut internal_rx) = internal_rx else {
            anyhow::bail!("Web bridge already started (no receiver)");
        };
        let channel_name = self.name().to_owned();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = internal_rx.recv() => {
                        match msg {
                            Some(msg) => {
                                if tx.send((channel_name.clone(), msg)).await.is_err() {
                                    break; // Gateway receiver closed
                                }
                            }
                            None => break,
                        }
                    }
                    _ = shutdown.changed() => break,
                }
            }
            tracing::info!(channel = %channel_name, "Web bridge stopped");
        });

        Ok(handle)
    }

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        // Route the response back to the waiting HTTP handler via correlation map.
        // The OutgoingMessage.id matches the original IncomingMessage.id,
        // which is the key registered by send_and_wait().
        {
            let mut responses = self.responses.write().await;
            if let Some(sender) = responses.remove(&msg.id) {
                let _ = sender.send(msg.clone());
                tracing::debug!(msg_id = %msg.id, "Correlated response to HTTP handler");
            }
        }

        // Always broadcast for WebSocket/SSE clients.
        let _ = self.outgoing_tx.send(msg);
        Ok(())
    }
}

impl std::fmt::Debug for WebBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebBridge").finish()
    }
}

/// Shared handle to the web bridge, used by route handlers.
#[derive(Debug, Clone)]
pub struct WebBridgeHandle {
    /// Sender for injecting incoming messages into the gateway pipeline.
    pub incoming_tx: mpsc::Sender<IncomingMessage>,
    /// Broadcast sender for pushing outgoing messages to WebSocket/SSE.
    pub outgoing_tx: broadcast::Sender<OutgoingMessage>,
    /// Correlation map for HTTP request-response matching.
    responses: Arc<RwLock<HashMap<uuid::Uuid, oneshot::Sender<OutgoingMessage>>>>,
    /// RFC-024 SP2: per-bridge reliability layer shared with [`WebBridge`].
    reliability: Arc<ReliabilityLayer>,
    /// RFC-024 SP1: ceiling on `send_and_wait`. When the gateway does not
    /// respond within this duration, the request is dropped and the HTTP
    /// layer returns 504 Gateway Timeout. Default 120 s.
    response_timeout: std::time::Duration,
}

impl WebBridgeHandle {
    /// Creates a new handle from a WebBridge.
    pub fn from_bridge(channel: &WebBridge) -> Self {
        Self {
            incoming_tx: channel.sender(),
            outgoing_tx: channel.outgoing_tx.clone(),
            responses: channel.responses.clone(),
            reliability: channel.reliability.clone(),
            response_timeout: std::time::Duration::from_secs(120),
        }
    }

    /// Override the default `send_and_wait` timeout.
    pub fn with_response_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.response_timeout = timeout;
        self
    }

    /// RFC-024 SP2 / C2 (replay): look up messages newer than `last_seq` in
    /// the per-bridge reliability layer and broadcast them through the
    /// outgoing channel so a WebSocket handler forwarding `outgoing_rx` to
    /// the client sees them as if they had just been delivered.
    ///
    /// If the cursor is older than the buffer's oldest surviving message,
    /// a synthetic `type: "resync"` message is broadcast instead and the
    /// client is expected to pull state via the regular HTTP API.
    pub fn replay_after(&self, last_seq: u64) {
        // RFC-024 §11: count replay outcomes (label=replay|resync).
        let m = oxios_kernel::metrics::get_metrics();
        match self.reliability.replay(last_seq) {
            ReplayResult::Replay(msgs) => {
                m.gateway_replay_replay.inc();
                for m in msgs {
                    let _ = self.outgoing_tx.send(m);
                }
            }
            ReplayResult::Resync => {
                m.gateway_replay_resync.inc();
                let mut meta = HashMap::new();
                meta.insert("type".into(), "resync".into());
                let resync = OutgoingMessage::with_id(uuid::Uuid::new_v4(), "web", "system", "")
                    .with_metadata_only(meta);
                let _ = self.outgoing_tx.send(resync);
            }
        }
    }

    /// Subscribe to outgoing messages.
    pub fn subscribe(&self) -> broadcast::Receiver<OutgoingMessage> {
        self.outgoing_tx.subscribe()
    }

    /// Send an incoming message to the gateway pipeline.
    #[allow(dead_code)]
    pub async fn send_incoming(&self, msg: IncomingMessage) -> Result<()> {
        self.incoming_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Send a message and wait for a response.
    ///
    /// This registers a oneshot receiver for the response and waits for it.
    /// Used by the HTTP chat endpoint to get the orchestrator's response.
    ///
    /// RFC-024 SP1 / C1 (response guarantee): the wait is bounded by
    /// `response_timeout` (default 120 s). On timeout the correlation map
    pub async fn send_and_wait(
        &self,
        msg: IncomingMessage,
    ) -> std::result::Result<OutgoingMessage, BridgeSendError> {
        self.send_and_wait_with_timeout(msg, self.response_timeout)
            .await
    }

    /// Like [`send_and_wait`] but with an explicit timeout. Exposed for
    /// tests and for callers that want a different ceiling (e.g. health
    /// probes with a 1 s deadline).
    pub async fn send_and_wait_with_timeout(
        &self,
        msg: IncomingMessage,
        timeout: std::time::Duration,
    ) -> std::result::Result<OutgoingMessage, BridgeSendError> {
        let (tx, rx) = oneshot::channel::<OutgoingMessage>();
        let msg_id = msg.id;

        // Register the response handler before sending.
        {
            let mut responses = self.responses.write().await;
            responses.insert(msg_id, tx);
        }

        // RFC-024 §11: observe `send_and_wait` duration for every attempt.
        let start = std::time::Instant::now();

        // Send the message.
        if let Err(e) = self.incoming_tx.send(msg).await {
            // Could not even enqueue — drop our correlation entry.
            self.responses.write().await.remove(&msg_id);
            return Err(BridgeSendError::SendFailed(e.to_string()));
        }

        let outcome = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => {
                // Gateway gave up; remove the entry.
                self.responses.write().await.remove(&msg_id);
                Err(BridgeSendError::ChannelDropped)
            }
            Err(_) => {
                // Deadline elapsed; remove the entry to prevent a leak.
                self.responses.write().await.remove(&msg_id);
                Err(BridgeSendError::Timeout)
            }
        };

        // RFC-024 §11: histogram + outcome counter. We always observe the
        // duration (even on success) so the histogram reflects the real
        // latency distribution; the labelled counter separates outcomes.
        let m = oxios_kernel::metrics::get_metrics();
        m.gateway_response_duration
            .observe(start.elapsed().as_secs_f64());
        match &outcome {
            Ok(_) => {} // delivered — counted at the channel layer (gateway.rs)
            Err(BridgeSendError::Timeout) => m.gateway_messages_timed_out.inc(),
            Err(BridgeSendError::ChannelDropped) => m.gateway_messages_dropped.inc(),
            Err(BridgeSendError::SendFailed(_)) => m.gateway_messages_dropped.inc(),
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    //! Boundary tests for the HTTP↔Gateway bridge.
    //!
    //! Exercises `WebBridge` / `WebBridgeHandle` methods that the HTTP
    //! layer relies on: `send_and_wait` request/response correlation via
    //! the oneshot map, `BridgeSendError::Timeout` typed variant, and
    //! `replay_after` delivery (gapless replay) plus resync marker when
    //! the cursor is older than the buffer's oldest surviving message.
    //!
    //! No network or HTTP server involved — the bridge is tested as the
    //! adapter it is, with `Channel::send` / `deliver_response` driven
    //! directly.

    use super::*;
    use oxios_gateway::ReplayConfig;
    use std::time::Duration;
    use tokio::sync::broadcast::error::TryRecvError;
    use uuid::Uuid;

    fn fresh_bridge(buffer: usize) -> WebBridge {
        let reliability = Arc::new(ReliabilityLayer::new(Default::default()));
        WebBridge::new(buffer, reliability)
    }

    #[test]
    fn web_bridge_opts_into_streaming_partials() {
        use oxios_gateway::channel::Channel;
        let bridge = fresh_bridge(4);
        assert!(Channel::supports_streaming(&bridge));
    }

    fn push_seq(reliability: &ReliabilityLayer, content: &str) -> OutgoingMessage {
        let m = OutgoingMessage::with_id(Uuid::new_v4(), "web", "user", content);
        reliability.assign_seq(m)
    }

    // ── Request/response correlation ────────────────────────────────

    #[tokio::test]
    async fn send_and_wait_correlates_response_by_id() {
        // The bridge must route a `Channel::send` whose OutgoingMessage.id
        // matches the registered oneshot back to the awaiting caller, AND
        // broadcast it on outgoing_tx for WS/SSE subscribers.
        let bridge = fresh_bridge(8);
        let handle =
            WebBridgeHandle::from_bridge(&bridge).with_response_timeout(Duration::from_secs(2));

        let mut sub = handle.subscribe();

        let msg_id = Uuid::new_v4();
        let mut incoming = IncomingMessage::new("web", "user", "ping");
        incoming.id = msg_id;
        let wait = tokio::spawn({
            let handle = handle.clone();
            async move { handle.send_and_wait(incoming).await }
        });

        // Wait until the spawned task has registered its oneshot under
        // msg_id, otherwise our reply would miss the correlation map.
        for _ in 0..200 {
            if handle.responses.read().await.contains_key(&msg_id) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            handle.responses.read().await.contains_key(&msg_id),
            "send_and_wait did not register its oneshot in time"
        );

        let reply = OutgoingMessage::with_id(msg_id, "web", "user", "pong");
        bridge.deliver_response(reply).await.unwrap();

        let result = wait
            .await
            .unwrap()
            .expect("correlated response should arrive");
        assert_eq!(result.id, msg_id);
        assert_eq!(result.content, "pong");

        // The same message also reaches subscribers via the broadcast channel.
        let broadcast = sub.recv().await.unwrap();
        assert_eq!(broadcast.content, "pong");
    }

    #[tokio::test]
    async fn send_and_wait_timeout_returns_typed_variant() {
        // RFC-024 SP1: the deadline must produce a typed `Timeout`
        // (not a generic error) so the HTTP layer can map to 504.
        let bridge = fresh_bridge(8);
        let handle =
            WebBridgeHandle::from_bridge(&bridge).with_response_timeout(Duration::from_millis(50));

        let incoming = IncomingMessage::new("web", "user", "hello");
        let msg_id = incoming.id;
        let err = handle.send_and_wait(incoming).await.unwrap_err();
        assert!(
            matches!(err, BridgeSendError::Timeout),
            "expected Timeout variant, got {err:?}"
        );

        // Timeout must remove the correlation entry so the map doesn't
        // leak oneshots across requests.
        assert!(
            !handle.responses.read().await.contains_key(&msg_id),
            "timeout must remove the correlation entry"
        );
    }

    // ── replay_after delivery ───────────────────────────────────────

    #[tokio::test]
    async fn replay_after_broadcasts_messages_with_higher_seq() {
        // RFC-024 §C2: a WS reconnecting with last_seq=k must receive the
        // gapless slice of messages strictly greater than k, in seq order.
        let bridge = fresh_bridge(16);
        let handle = WebBridgeHandle::from_bridge(&bridge);

        let m1 = push_seq(&handle.reliability, "first");
        let m2 = push_seq(&handle.reliability, "second");
        let m3 = push_seq(&handle.reliability, "third");
        let cursor = m1.seq.unwrap();
        assert_eq!(m2.seq.unwrap(), cursor + 1);
        assert_eq!(m3.seq.unwrap(), cursor + 2);

        let mut sub = handle.subscribe();
        handle.replay_after(cursor);

        let seen1 = sub.recv().await.unwrap();
        let seen2 = sub.recv().await.unwrap();
        assert_eq!(seen1.content, "second");
        assert_eq!(seen2.content, "third");
        assert_ne!(
            seen1.metadata.get("type").map(String::as_str),
            Some("resync")
        );
    }

    #[tokio::test]
    async fn replay_after_at_latest_seq_emits_no_messages() {
        // Cursor == latest seq → empty gapless slice, NOT Resync. The WS
        // client is caught up; we must not falsely demand it pull state.
        let bridge = fresh_bridge(16);
        let handle = WebBridgeHandle::from_bridge(&bridge);

        push_seq(&handle.reliability, "a");
        let m2 = push_seq(&handle.reliability, "b");
        let latest = m2.seq.unwrap();
        let mut sub = handle.subscribe();
        handle.replay_after(latest);

        match sub.try_recv() {
            Err(TryRecvError::Empty) => {}
            other => panic!("expected Empty, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_after_when_cursor_older_than_buffer_emits_resync_marker() {
        // RFC-024 §2: cursor older than the buffer's oldest surviving
        // message → Resync. The bridge must broadcast a `type: "resync"`
        // marker so the client knows to pull state via the regular API.
        let reliability = Arc::new(ReliabilityLayer::new(ReplayConfig {
            buffer_size: 2,
            ttl: Duration::from_secs(60),
        }));
        let bridge = WebBridge::new(16, reliability.clone());
        let handle = WebBridgeHandle::from_bridge(&bridge);

        push_seq(&reliability, "evicted-1");
        push_seq(&reliability, "evicted-2");
        push_seq(&reliability, "evicted-3"); // evicts evicted-1

        let mut sub = handle.subscribe();
        handle.replay_after(0);

        let marker = sub.recv().await.unwrap();
        assert_eq!(
            marker.metadata.get("type").map(String::as_str),
            Some("resync"),
            "resync marker must carry the documented metadata"
        );
        assert_eq!(marker.content, "", "resync marker must have empty content");
    }

    #[tokio::test]
    async fn replay_after_uses_per_bridge_reliability_layer() {
        // Two distinct WebBridges must have independent replay buffers —
        // a message pushed through bridge A must not surface on bridge B.
        let reliability_a = Arc::new(ReliabilityLayer::new(Default::default()));
        let reliability_b = Arc::new(ReliabilityLayer::new(Default::default()));
        let bridge_a = WebBridge::new(8, reliability_a.clone());
        let bridge_b = WebBridge::new(8, reliability_b.clone());
        let handle_a = WebBridgeHandle::from_bridge(&bridge_a);
        let handle_b = WebBridgeHandle::from_bridge(&bridge_b);

        push_seq(&reliability_a, "only-on-a");
        push_seq(&reliability_b, "only-on-b");

        let mut sub_a = handle_a.subscribe();
        let mut sub_b = handle_b.subscribe();
        handle_a.replay_after(0);

        // A replays its own message to its own subscribers.
        let replayed = sub_a.recv().await.unwrap();
        assert_eq!(replayed.content, "only-on-a");

        // B's broadcast channel must NOT have received A's replay —
        // per-bridge isolation across both the reliability layer AND
        // the outgoing broadcast sender.
        match sub_b.try_recv() {
            Err(TryRecvError::Empty) => {}
            other => panic!("bridge B must not receive bridge A's replay; got {other:?}"),
        }

        // B's replay on its own layer still surfaces its own message.
        handle_b.replay_after(0);
        let replayed_b = sub_b.recv().await.unwrap();
        assert_eq!(replayed_b.content, "only-on-b");
    }
}
