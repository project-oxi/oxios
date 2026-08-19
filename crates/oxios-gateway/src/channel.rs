//! Channel trait definition.
//!
//! A channel is a plugin that connects the gateway to a specific
//! interface (Web, CLI, Telegram, etc.).
//!
//! Channels implement [`Channel::start`] to push incoming messages
//! into a shared mpsc channel, and [`Channel::send`] for outgoing
//! responses.

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::GatewayInbox;
use crate::message::OutgoingMessage;

/// A communication channel that plugs into the gateway.
///
/// Each channel runs its own background task (started via [`Channel::start`])
/// and pushes incoming messages into the gateway's shared mpsc channel.
/// The gateway dispatches responses back via [`Channel::send`].
#[async_trait]
pub trait Channel: Send + Sync {
    /// Returns the name of this channel (e.g., "web", "telegram").
    fn name(&self) -> &str;

    /// Start the channel's background receive loop.
    ///
    /// Implementations should spawn an internal `tokio::spawn` task that:
    /// 1. Receives messages from the channel's own source (HTTP, readline, Telegram API).
    /// 2. Pushes them via `tx.send((name, msg)).await`.
    /// 3. Exits gracefully when `shutdown` changes.
    ///
    /// Returns the spawned task's `JoinHandle` so the gateway can track its lifetime.
    async fn start(
        &self,
        tx: mpsc::Sender<GatewayInbox>,
        shutdown: watch::Receiver<bool>,
    ) -> Result<JoinHandle<()>>;

    /// Send a response message through this channel.
    async fn send(&self, msg: OutgoingMessage) -> Result<()>;

    /// Whether this channel consumes live streaming partials.
    ///
    /// The gateway's streaming-sink collector forwards per-token deltas
    /// (`Text` / `ThinkingDelta` fragments and stream-control markers) as
    /// individual [`OutgoingMessage`]s with `partial = Some(true)` or a
    /// `stream_kind` metadata key. Only channels that can *render* those
    /// fragments incrementally (e.g. a WebSocket UI appending to an
    /// in-flight message) should opt in. Non-streaming channels (Telegram,
    /// CLI, RPC bridges) must keep the default `false`: they receive only
    /// the single terminal response per turn, not one delivery per delta.
    ///
    /// Default: `false`.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Live channel-provided status for control-plane introspection.
    ///
    /// Cheap and synchronous (no I/O): return a small JSON object describing
    /// the channel's current identity/runtime state (e.g. the connected bot's
    /// username), or `Value::Null` (the default) when there is nothing to
    /// report. Consumed by `Gateway::channel_status`.
    fn status(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullChannel;

    #[async_trait]
    impl Channel for NullChannel {
        fn name(&self) -> &str {
            "null"
        }

        async fn start(
            &self,
            _tx: mpsc::Sender<GatewayInbox>,
            _shutdown: watch::Receiver<bool>,
        ) -> Result<JoinHandle<()>> {
            Ok(tokio::spawn(async {}))
        }

        async fn send(&self, _msg: OutgoingMessage) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn status_defaults_to_null() {
        let channel = NullChannel;
        assert!(channel.status().is_null());
    }

    #[test]
    fn supports_streaming_defaults_to_false() {
        let channel = NullChannel;
        assert!(!channel.supports_streaming());
    }
}
