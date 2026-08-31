//! Context compression — LLM-generated session summaries (LobeHub port).
//!
//! When a session exceeds a message threshold, older exchanges are summarized
//! by an LLM and stored in `session.metadata["compression"]`. The summary
//! streams via `KernelEvent::CompressionDelta` for real-time UI rendering.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use parking_lot::Mutex;
use serde_json::json;

use crate::config::OxiosConfig;
use crate::engine::EngineHandle;
use crate::event_bus::{EventBus, KernelEvent};
use crate::state_store::{Session, StateStore};

/// Messages above this count trigger compression.
const COLLAPSE_THRESHOLD: usize = 40;
/// Recent messages never compressed (kept as raw context). Public so the
/// WS `/compact` handler can reject invocations with too little history
/// instead of silently no-oping.
pub const VISIBLE_TAIL: usize = 20;

/// System prompt for the compression LLM call (ported from LobeHub).
const COMPRESSION_SYSTEM_PROMPT: &str = r#"You are a conversation context compressor. Your task is to create a structured summary that preserves essential information while significantly reducing token count.

## Output Format

Structure your summary using these sections (omit empty sections):

### Context
Brief background and conversation setup (1-2 sentences max)

### Key Information
- Critical facts, data, specifications mentioned
- Technical details, configurations, parameters
- Names, identifiers, file paths, URLs

### Decisions & Conclusions
- Decisions made during the conversation
- Agreed-upon solutions or approaches
- Final conclusions reached

### Action Items
- Tasks assigned or planned
- Next steps discussed
- Pending items requiring follow-up

### Code & Technical
```
Preserve essential code snippets, commands, or technical syntax
```

## Rules

### MUST
- Output in the SAME LANGUAGE as the conversation
- Preserve ALL technical terms, code identifiers, file paths, and proper nouns exactly
- Maintain factual accuracy - never invent or assume information
- Keep code snippets that are essential for context

### SHOULD
- Achieve 60-80% compression ratio (summary should be 20-40% of original length)
- Use bullet points for clarity and scannability
- Preserve chronological order for sequential events
- Consolidate repeated information into single entries

### MAY
- Omit greetings, pleasantries, and filler content
- Combine related points into concise statements
- Abbreviate obvious context when meaning is preserved

## Important Notes

- The summary will be injected into a new conversation as context
- Recipient should be able to continue the conversation seamlessly
- Prioritize information that affects future responses"#;

/// LLM-generated session summary service.
pub struct CompressionService {
    state_store: Arc<StateStore>,
    engine_handle: Arc<EngineHandle>,
    config: OxiosConfig,
    event_bus: EventBus,
    /// Sessions currently being compressed (prevents concurrent runs).
    active: Mutex<HashSet<String>>,
}

impl CompressionService {
    /// Create a new CompressionService.
    pub fn new(
        state_store: Arc<StateStore>,
        engine_handle: Arc<EngineHandle>,
        config: OxiosConfig,
        event_bus: EventBus,
    ) -> Self {
        Self {
            state_store,
            engine_handle,
            config,
            event_bus,
            active: Mutex::new(HashSet::new()),
        }
    }

    /// Whether a session should be compressed (delegates to free function).
    pub fn should_compress(&self, session: &Session) -> bool {
        session_needs_compression(session)
    }

    /// Spawn compression in the background. No-op if already running.
    pub fn spawn_compress(self: &Arc<Self>, session_id: String) {
        {
            let mut active = self.active.lock();
            if !active.insert(session_id.clone()) {
                return; // already running
            }
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let result = this.compress(&session_id).await;
            this.active.lock().remove(&session_id);
            if let Err(e) = result {
                tracing::warn!(session_id = %session_id, error = %e, "Compression failed");
            }
        });
    }

    /// Run compression for a session: build prompt, stream LLM, persist.
    pub async fn compress(&self, session_id: &str) -> Result<()> {
        let sid = crate::state_store::SessionId(session_id.to_string());
        let session = self
            .state_store
            .load_session(&sid)
            .await
            .context("load session")?
            .context("session not found")?;

        let count = session.exchange_count();
        let range_end = count.saturating_sub(VISIBLE_TAIL);
        if range_end == 0 {
            return Ok(());
        }

        // Determine incremental start.
        let existing_summary = session
            .metadata
            .get("compression")
            .filter(|c| c.get("status").and_then(|s| s.as_str()) == Some("done"))
            .and_then(|c| c.get("summary").and_then(|s| s.as_str()).map(String::from));
        let start_index = session
            .metadata
            .get("compression")
            .and_then(|c| c.get("compressed_before_index").and_then(|v| v.as_u64()))
            .unwrap_or(0) as usize;

        // Set generating status.
        self.state_store
            .update_session_with(&sid, |s| {
                s.metadata.insert(
                    "compression".to_string(),
                    json!({
                        "status": "generating",
                        "summary": existing_summary.as_deref().unwrap_or(""),
                        "compressed_before_index": start_index,
                    }),
                );
                Ok(())
            })
            .await
            .context("set generating status")?;

        // Build user prompt.
        let user_prompt = build_compression_prompt(
            &session,
            start_index,
            range_end,
            existing_summary.as_deref(),
        );

        // Resolve model.
        let resolved = {
            let model_id = self.config.system_agents.model_for_task("history_compress");
            match model_id {
                Some(id) => self.engine_handle.resolve(&id),
                None => self.engine_handle.resolve_default(),
            }
        }
        .context("resolve compression model")?;

        // Build context and stream.
        let mut ctx = oxicode_sdk::Context::new();
        ctx.set_system_prompt(COMPRESSION_SYSTEM_PROMPT);
        ctx.add_message(oxicode_sdk::Message::User(oxicode_sdk::UserMessage::new(
            user_prompt,
        )));

        let stream = resolved
            .provider
            .stream(&resolved.model, &ctx, None)
            .await
            .context("start compression stream")?;

        let mut summary = String::new();
        let mut pinned = std::pin::pin!(stream);
        while let Some(event) = pinned.next().await {
            match event {
                oxicode_sdk::ProviderEvent::TextDelta { delta, .. } => {
                    summary.push_str(&delta);
                    let _ = self.event_bus.publish(KernelEvent::CompressionDelta {
                        session_id: session_id.to_string(),
                        delta,
                    });
                }
                oxicode_sdk::ProviderEvent::Done { .. } => break,
                oxicode_sdk::ProviderEvent::Error { error, .. } => {
                    let err_msg = format!("{error:?}");
                    self.state_store
                        .update_session_with(&sid, |s| {
                            s.metadata.insert(
                                "compression".to_string(),
                                json!({
                                    "status": "failed",
                                    "error": err_msg,
                                    "compressed_before_index": start_index,
                                }),
                            );
                            Ok(())
                        })
                        .await
                        .ok();
                    let _ = self.event_bus.publish(KernelEvent::CompressionFailed {
                        session_id: session_id.to_string(),
                        error: err_msg,
                    });
                    anyhow::bail!("compression stream error");
                }
                _ => {}
            }
        }

        // Persist final summary.
        self.state_store
            .update_session_with(&sid, |s| {
                s.metadata.insert(
                    "compression".to_string(),
                    json!({
                        "status": "done",
                        "summary": summary,
                        "compressed_at": chrono::Utc::now().to_rfc3339(),
                        "original_count": count,
                        "compressed_before_index": range_end,
                        "model": resolved.model_id,
                    }),
                );
                Ok(())
            })
            .await
            .context("persist compression summary")?;

        let _ = self.event_bus.publish(KernelEvent::CompressionDone {
            session_id: session_id.to_string(),
        });

        tracing::info!(
            session_id = %session_id,
            new_messages = range_end.saturating_sub(start_index),
            summary_len = summary.len(),
            "Session compressed"
        );
        Ok(())
    }
}

/// Pure predicate: does this session need compression? Extracted as a free
/// function so it can be unit-tested without constructing a full service.
pub fn session_needs_compression(session: &Session) -> bool {
    let count = session.exchange_count();
    if count < COLLAPSE_THRESHOLD {
        return false;
    }
    let range_end = count.saturating_sub(VISIBLE_TAIL);
    if let Some(comp) = session.metadata.get("compression")
        && comp.get("status").and_then(|s| s.as_str()) == Some("done")
    {
        let covered = comp
            .get("compressed_before_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        if covered >= range_end {
            return false;
        }
    }
    true
}

/// Build the user prompt for compression, formatting exchanges as
/// `[User]: ...` / `[Assistant]: ...` wrapped in `<chat_history>` XML.
fn build_compression_prompt(
    session: &Session,
    start: usize,
    end: usize,
    existing_summary: Option<&str>,
) -> String {
    let mut prompt = String::new();

    if let Some(prev) = existing_summary {
        prompt.push_str("<existing_summary>\n");
        prompt.push_str(prev);
        prompt.push_str("\n</existing_summary>\n\n");
    }

    prompt.push_str("<chat_history>\n");
    for i in start..end {
        if let Some(um) = session.user_messages.get(i) {
            prompt.push_str(&format!("[User]: {}\n", um.content));
        }
        if let Some(ar) = session.agent_responses.get(i) {
            prompt.push_str(&format!("[Assistant]: {}\n", ar.content));
        }
    }
    prompt.push_str("</chat_history>\n\n");
    prompt.push_str(
        "Please compress the above conversation history.\n\
         Output ONLY the structured summary following the format specified. \
         No additional commentary or meta-discussion.",
    );
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_store::{AgentResponse, UserMessage};

    fn make_session(exchanges: usize) -> Session {
        let mut s = Session::new("test");
        for i in 0..exchanges {
            s.user_messages.push(UserMessage {
                content: format!("question {i}"),
                timestamp: chrono::Utc::now(),
            });
            s.agent_responses.push(AgentResponse {
                content: format!("answer {i}"),
                session_id: None,
                phase_reached: None,
                evaluation_passed: None,
                timestamp: chrono::Utc::now(),
                trajectory_range: None,
            });
        }
        s
    }

    #[test]
    fn needs_compression_below_threshold() {
        let session = make_session(39);
        assert!(!session_needs_compression(&session));
    }

    #[test]
    fn needs_compression_at_threshold() {
        let session = make_session(40);
        assert!(session_needs_compression(&session));
    }

    #[test]
    fn no_compression_when_already_covered() {
        let mut session = make_session(45);
        // range_end = 45 - 20 = 25. Mark as covered.
        session.metadata.insert(
            "compression".to_string(),
            json!({ "status": "done", "compressed_before_index": 25 }),
        );
        assert!(!session_needs_compression(&session));
    }

    #[test]
    fn recompress_when_new_messages_exist() {
        let mut session = make_session(50);
        // Previously compressed only up to 20, but range_end = 50 - 20 = 30.
        // So there are new messages to compress (20..30).
        session.metadata.insert(
            "compression".to_string(),
            json!({ "status": "done", "compressed_before_index": 20 }),
        );
        assert!(session_needs_compression(&session));
    }

    #[test]
    fn build_prompt_includes_existing_summary() {
        let session = make_session(5);
        let prompt = build_compression_prompt(&session, 0, 3, Some("prev summary"));
        assert!(prompt.contains("<existing_summary>"));
        assert!(prompt.contains("prev summary"));
        assert!(prompt.contains("[User]: question 0"));
        assert!(prompt.contains("[Assistant]: answer 2"));
        assert!(!prompt.contains("question 3")); // end is exclusive
    }
}
