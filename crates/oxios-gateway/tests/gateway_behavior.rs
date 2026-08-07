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
use oxios_gateway::channel::Channel;
use oxios_gateway::message::{IncomingMessage, OutgoingMessage};
use oxios_kernel::EventBus;
use oxios_kernel::state_store::StateStore;
use tempfile::TempDir;
use tokio::sync::Mutex;

// ── Fake channel ─────────────────────────────────────────────────────

/// Minimal `Channel` impl that records every `send` and observes its
/// shutdown signal. Its `start` returns a task that flips `shutdown_observed`
/// when the gateway signals shutdown — these tests do not exercise the
/// channel's receive loop, only the gateway's view of the channel.
#[derive(Clone)]
struct FakeChannel {
    name: String,
    sent: Arc<Mutex<Vec<OutgoingMessage>>>,
    shutdown_observed: Arc<Mutex<bool>>,
}

impl FakeChannel {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            sent: Arc::new(Mutex::new(Vec::new())),
            shutdown_observed: Arc::new(Mutex::new(false)),
        }
    }
}

#[async_trait]
impl Channel for FakeChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(
        &self,
        _incoming_tx: tokio::sync::mpsc::Sender<(String, IncomingMessage)>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
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
