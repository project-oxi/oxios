//! CLI channel implementation.
//!
//! Implements the [`Channel`] trait from `oxios-gateway` so the CLI
//! can plug into the gateway like any other channel.
//!
//! Uses `mpsc` channels to bridge:
//! - **Incoming**: User typed input → mpsc → Gateway → Kernel
//! - **Outgoing**: Kernel → Gateway → mpsc → stdout

use anyhow::Result;
use async_trait::async_trait;
use oxios_gateway::GatewayInbox;
use oxios_gateway::channel::Channel;
use oxios_gateway::format::ChannelFormatter;
use oxios_gateway::message::{IncomingMessage, OutgoingMessage};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc, watch};

use crate::channels::cli::format::CliFormatter;
use crate::channels::cli::session::Session;

/// The CLI channel adapter.
///
/// Bridges the interactive readline loop with the gateway's channel
/// interface using mpsc channels for message passing.
pub struct CliChannel {
    /// Receiver for incoming messages (user input from readline).
    /// `Option` so `start()` can take ownership via `take()`.
    incoming_rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// Sender for injecting incoming messages.
    incoming_tx: mpsc::Sender<IncomingMessage>,
    /// Current session metadata.
    session: Arc<std::sync::Mutex<Session>>,
    /// CLI response formatter.
    formatter: CliFormatter,
    /// Shared flag indicating whether a request is currently being processed.
    /// Set to `true` by the interactive loop on send, `false` by `send()` on response.
    processing: Arc<AtomicBool>,
}

impl CliChannel {
    /// Creates a new CLI channel with the given buffer size.
    pub fn new(buffer: usize) -> Self {
        let (incoming_tx, incoming_rx) = mpsc::channel(buffer);
        let session = Arc::new(std::sync::Mutex::new(Session::new(None)));
        let processing = Arc::new(AtomicBool::new(false));

        Self {
            incoming_rx: Mutex::new(Some(incoming_rx)),
            incoming_tx,
            session,
            formatter: CliFormatter,
            processing,
        }
    }

    /// Returns a sender that can be used to inject incoming messages.
    #[allow(dead_code)]
    pub fn sender(&self) -> mpsc::Sender<IncomingMessage> {
        self.incoming_tx.clone()
    }

    /// Returns a handle for injecting messages from outside the channel.
    pub fn handle(&self) -> CliChannelHandle {
        CliChannelHandle {
            incoming_tx: self.incoming_tx.clone(),
            session: self.session.clone(),
            processing: self.processing.clone(),
        }
    }
    /// Returns a clone of the shared processing flag.
    #[allow(dead_code)]
    pub fn processing_flag(&self) -> Arc<AtomicBool> {
        self.processing.clone()
    }
}

#[async_trait]
impl Channel for CliChannel {
    fn name(&self) -> &str {
        "cli"
    }

    async fn start(
        &self,
        tx: mpsc::Sender<GatewayInbox>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let internal_rx = self.incoming_rx.lock().await.take();
        let Some(mut internal_rx) = internal_rx else {
            anyhow::bail!("CLI channel already started (no receiver)");
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
            tracing::info!(channel = %channel_name, "CLI channel stopped");
        });

        Ok(handle)
    }

    async fn send(&self, msg: OutgoingMessage) -> Result<()> {
        let output = match &msg.meta {
            Some(meta) if meta.error.is_some() => self.formatter.format_error(&msg),
            Some(_) => self.formatter.format_success(&msg),
            None => msg.content.clone(),
        };
        println!("{output}");
        self.processing.store(false, Ordering::Relaxed);
        Ok(())
    }
}

impl std::fmt::Debug for CliChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliChannel").finish()
    }
}

/// Handle to the CLI channel, used to inject messages from the readline loop.
#[derive(Clone)]
pub struct CliChannelHandle {
    /// Sender for injecting incoming messages into the gateway pipeline.
    pub incoming_tx: mpsc::Sender<IncomingMessage>,
    /// Shared session reference.
    session: Arc<std::sync::Mutex<Session>>,
    /// Shared processing flag (set `true` on send, `false` on response).
    processing: Arc<AtomicBool>,
}

impl std::fmt::Debug for CliChannelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliChannelHandle").finish()
    }
}
impl CliChannelHandle {
    /// Creates a handle from a CliChannel.
    #[allow(dead_code)]
    pub fn from_channel(channel: &CliChannel) -> Self {
        channel.handle()
    }

    /// Send a user message into the gateway pipeline.
    pub async fn send_user_message(&self, content: String) -> Result<()> {
        let mut msg = IncomingMessage::new("cli", "cli-user", &content);
        {
            let session = self.session.lock().unwrap_or_else(|e| {
                tracing::error!("Mutex poisoned: {e}");
                e.into_inner()
            });
            msg.metadata
                .insert("session_id".to_owned(), session.id.to_string());
        }
        self.incoming_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }

    /// Touch the session (update activity).
    pub fn touch_session(&self) {
        if let Ok(mut session) = self.session.lock() {
            session.touch();
        }
    }

    /// Reset the session (create a new one).
    pub fn reset_session(&self) {
        if let Ok(mut session) = self.session.lock() {
            *session = Session::new(None);
        }
    }

    /// Get the current session ID.
    #[allow(dead_code)]
    pub fn session_id(&self) -> uuid::Uuid {
        self.session.lock().map(|s| s.id).unwrap_or_default()
    }

    /// Mark that a request is being processed.
    pub fn set_processing(&self, value: bool) {
        self.processing.store(value, Ordering::Relaxed);
    }

    /// Check whether a request is currently being processed.
    pub fn is_processing(&self) -> bool {
        self.processing.load(Ordering::Relaxed)
    }

    /// Send a switch_model action to the gateway.
    ///
    /// The gateway detects the `action` metadata and routes to `EngineApi::set_model()`
    /// instead of the orchestrator.
    pub async fn send_switch_model(&self, model_id: &str) -> Result<()> {
        let mut msg = IncomingMessage::new("cli", "cli-user", format!("switch_model: {model_id}"));
        msg.metadata
            .insert("action".to_owned(), "switch_model".to_owned());
        msg.metadata
            .insert("model_id".to_owned(), model_id.to_owned());
        {
            let session = self.session.lock().unwrap_or_else(|e| {
                tracing::error!("Mutex poisoned: {e}");
                e.into_inner()
            });
            msg.metadata
                .insert("session_id".to_owned(), session.id.to_string());
        }
        self.incoming_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }

    /// Send a switch_persona action to the gateway.
    ///
    /// The gateway detects the `action` metadata and routes to `PersonaApi::set_active()`
    /// instead of the orchestrator.
    pub async fn send_switch_persona(&self, persona_id: &str) -> Result<()> {
        let mut msg =
            IncomingMessage::new("cli", "cli-user", format!("switch_persona: {persona_id}"));
        msg.metadata
            .insert("action".to_owned(), "switch_persona".to_owned());
        msg.metadata
            .insert("persona_id".to_owned(), persona_id.to_owned());
        {
            let session = self.session.lock().unwrap_or_else(|e| {
                tracing::error!("Mutex poisoned: {e}");
                e.into_inner()
            });
            msg.metadata
                .insert("session_id".to_owned(), session.id.to_string());
        }
        self.incoming_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }

    /// Send a get_persona action to the gateway.
    ///
    /// The gateway reports the active persona back through the normal
    /// channel send path.
    pub async fn send_get_persona(&self) -> Result<()> {
        let mut msg = IncomingMessage::new("cli", "cli-user", "get_persona");
        msg.metadata
            .insert("action".to_owned(), "get_persona".to_owned());
        {
            let session = self.session.lock().unwrap_or_else(|e| {
                tracing::error!("Mutex poisoned: {e}");
                e.into_inner()
            });
            msg.metadata
                .insert("session_id".to_owned(), session.id.to_string());
        }
        self.incoming_tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }
}
