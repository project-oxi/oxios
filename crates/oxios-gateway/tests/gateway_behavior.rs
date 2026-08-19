//! Gateway behavioral integration tests.
//!
//! Exercises `oxios_gateway::Gateway` against a minimal fake `Channel`
//! implementation and a real `oxios_kernel::Orchestrator` built with the
//! kernel's mock orchestration helpers. The orchestrator is never invoked
//! by these tests — they exercise the channel-management API surface
//! (`register`, `channel_names`, `send_to`, `unregister`, `signal_shutdown`)
//! and the per-channel shutdown flow without going through the gateway's
//! dispatch loop.
//!
//! The shared mock orchestration helpers live in the kernel's test
//! `common` module and are imported here via `#[path]` so we use the
//! kernel's canonical fakes instead of duplicating them.

#![allow(dead_code)]
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

#[path = "../../oxios-kernel/tests/common/mod.rs"]
mod common;

use std::sync::Arc;

use async_trait::async_trait;
use oxios_gateway::Gateway;
use oxios_gateway::GatewayInbox;
use oxios_gateway::channel::Channel;
use oxios_gateway::message::{IncomingMessage, OutgoingMessage};
use oxios_kernel::EventBus;
use oxios_kernel::state_store::StateStore;
use tempfile::TempDir;
use tokio::sync::{Mutex, mpsc};

// ── Fake channel ─────────────────────────────────────────────────────

/// Minimal `Channel` impl that records every `send` and observes its
/// shutdown signal. Its `start` returns a task that flips `shutdown_observed`
/// when the gateway signals shutdown — these tests do not exercise the
/// channel's receive loop, only the gateway's view of the channel.
///
/// The dispatch-loop tests additionally use the captured inbox sender
/// (`push`) and the `streaming` capability flag.
#[derive(Clone)]
struct FakeChannel {
    name: String,
    streaming: bool,
    sent: Arc<Mutex<Vec<OutgoingMessage>>>,
    shutdown_observed: Arc<Mutex<bool>>,
    incoming_tx: Arc<Mutex<Option<mpsc::Sender<GatewayInbox>>>>,
}

impl FakeChannel {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            streaming: false,
            sent: Arc::new(Mutex::new(Vec::new())),
            shutdown_observed: Arc::new(Mutex::new(false)),
            incoming_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Declare streaming capability (mirrors `WebBridge`).
    fn with_streaming(mut self) -> Self {
        self.streaming = true;
        self
    }

    /// Push an incoming message into the gateway inbox via the sender
    /// captured by `start`.
    async fn push(&self, msg: IncomingMessage) {
        let tx = self
            .incoming_tx
            .lock()
            .await
            .clone()
            .expect("start() not called");
        tx.send((self.name.clone(), msg))
            .await
            .expect("gateway rx closed");
    }

    /// Wait until at least one non-partial (terminal) message was recorded.
    async fn await_terminal(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if self
                .sent
                .lock()
                .await
                .iter()
                .any(|m| m.partial != Some(true))
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("no terminal message arrived within 5s");
    }
}

#[async_trait]
impl Channel for FakeChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        self.streaming
    }

    async fn start(
        &self,
        incoming_tx: mpsc::Sender<GatewayInbox>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        *self.incoming_tx.lock().await = Some(incoming_tx);
        let observed = self.shutdown_observed.clone();
        Ok(tokio::spawn(async move {
            let _ = shutdown.wait_for(|v| *v).await;
            *observed.lock().await = true;
        }))
    }

    async fn send(&self, msg: OutgoingMessage) -> anyhow::Result<()> {
        self.sent.lock().await.push(msg);
        Ok(())
    }
}

// ── Gated supervisor (dispatch-loop tests) ───────────────────────────

/// Supervisor stub whose `run_with_directive` blocks until released.
///
/// Holds the dispatch task inside `handle_unified` so tests can observe
/// mid-turn state (streaming-sink registration) without racing the mock
/// orchestrator's instant execution.
struct GatedSupervisor {
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
    notify: std::sync::Arc<tokio::sync::Notify>,
}

impl GatedSupervisor {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            release: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            notify: std::sync::Arc::new(tokio::sync::Notify::new()),
        })
    }

    /// Let the gated turn proceed.
    fn release(&self) {
        self.release
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    async fn wait_for_release(&self) {
        // Flag-first loop: no lost wakeup — if `release()` ran before we
        // registered a waiter, the flag check exits immediately.
        while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
            self.notify.notified().await;
        }
    }
}

#[async_trait]
impl oxios_kernel::supervisor::Supervisor for GatedSupervisor {
    async fn exec(&self, _id: oxios_kernel::AgentId) -> anyhow::Result<()> {
        Ok(())
    }
    async fn fork_directive(
        &self,
        _directive: &oxios_ouroboros::Directive,
        _env: &oxios_ouroboros::ExecEnv,
    ) -> anyhow::Result<oxios_kernel::AgentId> {
        Ok(oxios_kernel::AgentId::new_v4())
    }
    async fn run_with_directive(
        &self,
        _id: oxios_kernel::AgentId,
        _directive: &oxios_ouroboros::Directive,
        _env: &oxios_ouroboros::ExecEnv,
    ) -> anyhow::Result<oxios_ouroboros::ExecutionResult> {
        self.wait_for_release().await;
        Ok(oxios_ouroboros::ExecutionResult::default())
    }
    async fn wait(&self, _id: oxios_kernel::AgentId) -> anyhow::Result<oxios_kernel::AgentStatus> {
        Ok(oxios_kernel::AgentStatus::Stopped)
    }
    async fn kill(&self, _id: oxios_kernel::AgentId) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list(&self) -> anyhow::Result<Vec<oxios_kernel::AgentInfo>> {
        Ok(Vec::new())
    }
}

// ── Dispatch-loop test builder ───────────────────────────────────────

/// Build a real `Gateway` whose dispatch loop runs against the mock
/// orchestrator with a gated supervisor and a streaming-sink registry the
/// test also holds.
fn build_gated_gateway(
    registry: Arc<oxios_kernel::streaming_sink::StreamingSinkRegistry>,
) -> (Arc<Gateway>, TempDir, Arc<GatedSupervisor>) {
    let event_bus = EventBus::new(16);
    let temp = TempDir::new().expect("tempdir");
    let store = Arc::new(StateStore::new(temp.path().to_path_buf()).expect("StateStore"));
    let supervisor = GatedSupervisor::new();
    let (orchestrator, _mock) =
        common::build_test_orchestrator(supervisor.clone(), store, event_bus);
    let gateway = Arc::new(Gateway::new(orchestrator).with_streaming_sinks(registry));
    (gateway, temp, supervisor)
}

/// Poll the sink registry for a session key until it appears or `ms` elapse.
async fn wait_for_sink(
    registry: &oxios_kernel::streaming_sink::StreamingSinkRegistry,
    session_id: &str,
    ms: u64,
) -> Option<oxios_kernel::streaming_sink::StreamingSinkSender> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < deadline {
        if let Some(tx) = registry.lookup(session_id) {
            return Some(tx);
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    None
}

// ── Minimal Supervisor stub ──────────────────────────────────────────

/// Minimal `Supervisor` impl that satisfies `build_test_orchestrator`'s
/// signature without running any agents. Required because `Gateway::new`
/// takes an `Arc<Orchestrator>` even though the orchestrator is only
/// invoked from the dispatch path these tests don't enter.
struct StubSupervisor;

impl StubSupervisor {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

#[async_trait]
impl oxios_kernel::supervisor::Supervisor for StubSupervisor {
    async fn exec(&self, _id: oxios_kernel::AgentId) -> anyhow::Result<()> {
        Ok(())
    }
    async fn fork_directive(
        &self,
        _directive: &oxios_ouroboros::Directive,
        _env: &oxios_ouroboros::ExecEnv,
    ) -> anyhow::Result<oxios_kernel::AgentId> {
        Ok(oxios_kernel::AgentId::new_v4())
    }
    async fn run_with_directive(
        &self,
        _id: oxios_kernel::AgentId,
        _directive: &oxios_ouroboros::Directive,
        _env: &oxios_ouroboros::ExecEnv,
    ) -> anyhow::Result<oxios_ouroboros::ExecutionResult> {
        Ok(oxios_ouroboros::ExecutionResult::default())
    }
    async fn wait(&self, _id: oxios_kernel::AgentId) -> anyhow::Result<oxios_kernel::AgentStatus> {
        Ok(oxios_kernel::AgentStatus::Stopped)
    }
    async fn kill(&self, _id: oxios_kernel::AgentId) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list(&self) -> anyhow::Result<Vec<oxios_kernel::AgentInfo>> {
        Ok(Vec::new())
    }
}

// ── Test gateway builder ─────────────────────────────────────────────

/// Build a real `Gateway` against an `Orchestrator` wired with the stub
/// supervisor and the kernel's `MockIntentEngine`. The orchestrator is
/// never invoked — these tests only exercise the gateway's channel
/// management surface.
fn build_gateway() -> Arc<Gateway> {
    let event_bus = EventBus::new(16);
    let temp = TempDir::new().expect("tempdir");
    let store = Arc::new(StateStore::new(temp.path().to_path_buf()).expect("StateStore"));
    let supervisor = StubSupervisor::new();
    let (orchestrator, _mock) = common::build_test_orchestrator(supervisor, store, event_bus);
    Arc::new(Gateway::new(orchestrator))
}

// ── Tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn register_starts_channel_and_makes_it_visible() {
    let gateway = build_gateway();
    let channel = FakeChannel::new("fake-a");

    gateway
        .register(Box::new(channel) as Box<dyn Channel>)
        .await
        .expect("register should succeed");

    let names = gateway.channel_names().await;
    assert_eq!(names, vec!["fake-a".to_string()]);
}

#[tokio::test]
async fn send_to_routes_through_registered_channel_with_assigned_seq() {
    let gateway = build_gateway();
    let channel = FakeChannel::new("fake-b");

    gateway
        .register(Box::new(channel.clone()) as Box<dyn Channel>)
        .await
        .unwrap();

    let msg = OutgoingMessage::with_id(uuid::Uuid::new_v4(), "fake-b", "u", "hello");
    gateway.send_to("fake-b", msg).await.unwrap();

    let sent = channel.sent.lock().await;
    assert_eq!(sent.len(), 1);
    // F22 + RFC-024: send_to assigns a monotonic seq so the WS layer can
    // replay missed messages by cursor.
    assert!(sent[0].seq.is_some(), "send_to must assign a seq");
    assert_eq!(sent[0].content, "hello");
}

#[tokio::test]
async fn send_to_unknown_channel_is_noop() {
    // Fire-and-forget contract: missing channel logs and returns Ok(())
    // rather than failing the caller. Tests the documented behavior.
    let gateway = build_gateway();
    let msg = OutgoingMessage::with_id(uuid::Uuid::new_v4(), "ghost", "u", "x");
    gateway.send_to("ghost", msg).await.unwrap();
}

#[tokio::test]
async fn unregister_signals_shutdown_and_removes_channel() {
    let gateway = build_gateway();
    let channel = FakeChannel::new("fake-c");

    gateway
        .register(Box::new(channel.clone()) as Box<dyn Channel>)
        .await
        .unwrap();
    assert_eq!(gateway.channel_names().await, vec!["fake-c".to_string()]);

    gateway.unregister("fake-c").await.unwrap();

    assert!(gateway.channel_names().await.is_empty());
    // The shutdown task observes the watch signal within its bounded
    // grace period (F20/F27). Wait briefly so the spawn can complete.
    for _ in 0..50 {
        if *channel.shutdown_observed.lock().await {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        *channel.shutdown_observed.lock().await,
        "unregister must signal the channel's shutdown watch"
    );
}

#[tokio::test]
async fn registering_same_name_twice_replaces_previous() {
    // Documented behavior: register() overwrites the prior entry under
    // the same name. The new channel receives traffic; the old one
    // receives nothing after the swap.
    let gateway = build_gateway();
    let first = FakeChannel::new("dup");
    let second = FakeChannel::new("dup");

    gateway
        .register(Box::new(first.clone()) as Box<dyn Channel>)
        .await
        .unwrap();
    gateway
        .register(Box::new(second.clone()) as Box<dyn Channel>)
        .await
        .unwrap();

    gateway
        .send_to(
            "dup",
            OutgoingMessage::with_id(uuid::Uuid::new_v4(), "dup", "u", "yo"),
        )
        .await
        .unwrap();

    assert!(first.sent.lock().await.is_empty());
    assert_eq!(second.sent.lock().await.len(), 1);
}

#[tokio::test]
async fn unregister_unknown_channel_is_noop() {
    let gateway = build_gateway();
    // Must not panic or error.
    gateway.unregister("never-registered").await.unwrap();
}

#[tokio::test]
async fn signal_shutdown_flips_internal_flag() {
    let gateway = build_gateway();
    assert!(!gateway.is_shutdown());
    gateway.signal_shutdown();
    assert!(gateway.is_shutdown());
}

// ── Dispatch loop: streaming collector gating ────────────────────────

/// Regression (Telegram token-spam): a channel that does not opt into
/// streaming must never receive per-delta partial messages. The gateway's
/// streaming-sink collector must not even register a sink for its turns —
/// otherwise every `StreamDelta` becomes one channel delivery (on
/// Telegram: one Bot API message per token, plus empty stream markers).
#[tokio::test]
async fn non_streaming_channel_receives_no_streaming_partials() {
    let registry = Arc::new(oxios_kernel::streaming_sink::StreamingSinkRegistry::new());
    let (gateway, _temp, supervisor) = build_gated_gateway(registry.clone());
    let fake = FakeChannel::new("tg");
    gateway.register(Box::new(fake.clone())).await.unwrap();

    let run_handle = {
        let g = gateway.clone();
        tokio::spawn(async move { g.run().await })
    };

    let sid = "dispatch-nostream".to_string();
    let mut msg = IncomingMessage::new("tg", "user", "hello");
    msg.metadata.insert("session_id".to_string(), sid.clone());
    fake.push(msg).await;

    // While the turn is held inside the gated supervisor, check whether the
    // gateway registered a streaming sink for this session.
    let gateway_sink = wait_for_sink(&registry, &sid, 500).await;
    let sink_was_registered = gateway_sink.is_some();
    // Simulate the agent runtime emitting a text delta — exactly what
    // agent_runtime.rs does when a sink lookup succeeds.
    if let Some(tx) = gateway_sink.clone() {
        let _ = tx.try_send(oxios_kernel::agent_runtime::StreamDelta::Text(
            "token-fragment".to_string(),
        ));
    }
    drop(gateway_sink);

    supervisor.release();
    fake.await_terminal().await;

    assert!(
        !sink_was_registered,
        "collector must not register a sink for non-streaming channels"
    );

    let sent = fake.sent.lock().await.clone();
    assert_eq!(sent.len(), 1, "terminal response only, got {sent:?}");
    for m in &sent {
        assert_ne!(m.partial, Some(true), "no partial deltas: {m:?}");
        assert!(
            !m.metadata.contains_key("stream_kind"),
            "no stream markers: {m:?}"
        );
    }

    gateway.signal_shutdown();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), run_handle).await;
}

/// Web regression guard: a streaming-capable channel still gets the
/// collector — partial deltas forwarded as partial-flagged messages, then
/// the single terminal.
#[tokio::test]
async fn streaming_channel_receives_partials_and_terminal() {
    let registry = Arc::new(oxios_kernel::streaming_sink::StreamingSinkRegistry::new());
    let (gateway, _temp, supervisor) = build_gated_gateway(registry.clone());
    let fake = FakeChannel::new("web").with_streaming();
    gateway.register(Box::new(fake.clone())).await.unwrap();

    let run_handle = {
        let g = gateway.clone();
        tokio::spawn(async move { g.run().await })
    };

    let sid = "dispatch-stream".to_string();
    let mut msg = IncomingMessage::new("web", "user", "hello");
    msg.metadata.insert("session_id".to_string(), sid.clone());
    fake.push(msg).await;

    let gateway_sink = wait_for_sink(&registry, &sid, 500)
        .await
        .expect("collector must register a sink for streaming channels");
    let _ = gateway_sink.try_send(oxios_kernel::agent_runtime::StreamDelta::Text(
        "token-fragment".to_string(),
    ));
    drop(gateway_sink);

    supervisor.release();
    fake.await_terminal().await;

    let sent = fake.sent.lock().await.clone();
    let partials: Vec<_> = sent.iter().filter(|m| m.partial == Some(true)).collect();
    let terminals: Vec<_> = sent.iter().filter(|m| m.partial != Some(true)).collect();
    assert_eq!(terminals.len(), 1, "exactly one terminal: {sent:?}");
    assert_eq!(partials.len(), 1, "exactly one forwarded delta: {sent:?}");
    assert_eq!(partials[0].content, "token-fragment");
    assert_eq!(
        partials[0].metadata.get("session_id").map(String::as_str),
        Some(sid.as_str())
    );

    gateway.signal_shutdown();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), run_handle).await;
}
