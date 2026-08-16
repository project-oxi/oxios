# Context Compression Implementation Plan
> **Status**: Shipped — v1.31.x era (chat compression, LobeHub port)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** LLM-generated conversation summaries with streaming Summary/History tabs in the CompressedGroup, ported from LobeHub's compressContext pipeline.

**Architecture:** Backend `CompressionService` streams an LLM summary via `provider.stream()` → `KernelEvent::CompressionDelta` → existing WS pipeline → frontend store. Summary persists in `session.metadata["compression"]`. Frontend extends CompressedGroup with tabbed Summary (streaming markdown) / History (original messages) panel.

**Tech Stack:** Rust (oxios-kernel, axum), TypeScript (React 18, Zustand, virtua, Tailwind), i18n (en/ko).

## Global Constraints

- oxi-sdk model calls via `EngineHandle` / `provider.stream()` — never reimplement.
- New strings need keys in BOTH `web/src/i18n/locales/en.json` and `ko.json`.
- CI gates: `cargo fmt && clippy -D warnings && cargo test --workspace` (Rust), `bunx tsc --noEmit && bunx biome check src && bun run vitest run && bun run build` (web, from `web/`).
- Concurrent sessions: other agents may edit `web/src/stores/chat.ts`, `web/src/routes/chat.tsx`. Always `git pull` before starting; stage only your own files.
- Design spec: `docs/designs/2026-07-28-context-compression-design.md`.

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `crates/oxios-kernel/src/event_bus.rs` | +3 KernelEvent variants (CompressionDelta/Done/Failed) | Modify |
| `crates/oxios-kernel/src/compression.rs` | CompressionService: trigger check, prompt build, LLM stream, persist | Create |
| `crates/oxios-kernel/src/kernel_handle/compression_api.rs` | CompressionApi facade | Create |
| `crates/oxios-kernel/src/kernel_handle/mod.rs` | +compression_api module, +compression field on KernelHandle | Modify |
| `crates/oxios-kernel/src/lib.rs` | +mod compression, re-export CompressionApi | Modify |
| `src/kernel.rs` | Wire CompressionApi into KernelHandle assembly | Modify |
| `src/api/routes/events.rs` | +handle_session_compress, +compression field in GET response | Modify |
| `src/api/routes/mod.rs` | +route registration | Modify |
| `src/api/routes/chat.rs` | kernel_event_to_ws_chunk +3 arms, auto-trigger in persist_session | Modify |
| `web/src/types/index.ts` | +CompressionInfo, +StreamChunk types | Modify |
| `web/src/stores/chat.ts` | +compression state, +WS handlers, +loadSession extraction | Modify |
| `web/src/lib/compressed-summary.ts` | buildCompressedDigest (pure statistical fallback) | Create |
| `web/src/lib/compressed-summary.test.ts` | Digest tests | Create |
| `web/src/components/chat/compressed-group.tsx` | Tabbed panel (Summary/History) | Modify |
| `web/src/lib/chat-rows.ts` | collapse-bar row gains foldedMessages + compression | Modify |
| `web/src/routes/chat.tsx` | Pass new props, auto-trigger | Modify |
| `web/src/i18n/locales/en.json` | +compression keys | Modify |
| `web/src/i18n/locales/ko.json` | +compression keys | Modify |

---

## Task 1: KernelEvent compression variants

**Files:**
- Modify: `crates/oxios-kernel/src/event_bus.rs`

**Interfaces:**
- Produces: `KernelEvent::CompressionDelta { session_id: String, delta: String }`, `KernelEvent::CompressionDone { session_id: String }`, `KernelEvent::CompressionFailed { session_id: String, error: String }`

- [ ] **Step 1: Add variants to KernelEvent enum**

In `crates/oxios-kernel/src/event_bus.rs`, find the last variant before the closing `}` of `pub enum KernelEvent` (around line 400). Add:

```rust
    /// A chunk of the compression summary being streamed.
    CompressionDelta {
        /// The session being compressed.
        session_id: String,
        /// Incremental summary text.
        delta: String,
    },
    /// Compression completed successfully.
    CompressionDone {
        /// The session that was compressed.
        session_id: String,
    },
    /// Compression failed.
    CompressionFailed {
        /// The session that failed compression.
        session_id: String,
        /// Error description.
        error: String,
    },
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p oxios-kernel 2>&1 | head -30`
Expected: compiles (new variants are additive; `#[non_exhaustive]` not on this enum but existing match arms use `_` catch-alls).

If there are non-exhaustive match errors, add `_ => {}` arms where needed.

- [ ] **Step 3: Commit**

```bash
git add crates/oxios-kernel/src/event_bus.rs
git commit -m "feat(kernel): add KernelEvent compression variants"
```

---

## Task 2: CompressionService

**Files:**
- Create: `crates/oxios-kernel/src/compression.rs`
- Modify: `crates/oxios-kernel/src/lib.rs`

**Interfaces:**
- Consumes: `StateStore` (load_session, update_session_with), `EngineHandle` (get → resolve_model / create_provider / default_model_id), `EventBus` (publish), `OxiosConfig.system_agents.model_for_task("history_compress")`.
- Produces: `CompressionService` with `should_compress(&Session) -> bool`, `spawn_compress(session_id: String)`, `compress(&str) -> Result<()>`.

- [ ] **Step 1: Create compression.rs**

Create `crates/oxios-kernel/src/compression.rs`:

```rust
//! Context compression — LLM-generated session summaries (LobeHub port).
//!
//! When a session exceeds a message threshold, older exchanges are summarized
//! by an LLM and stored in `session.metadata["compression"]`. The summary
//! streams via `KernelEvent::CompressionDelta` for real-time UI rendering.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::{Mutex, RwLock};
use serde_json::json;

use crate::config::OxiosConfig;
use crate::engine::EngineHandle;
use crate::event_bus::{EventBus, KernelEvent};
use crate::state_store::{Session, StateStore};

/// Messages above this count trigger compression.
const COLLAPSE_THRESHOLD: usize = 40;
/// Recent messages never compressed (kept as raw context).
const VISIBLE_TAIL: usize = 20;

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
    engine_handle: EngineHandle,
    config: Arc<RwLock<OxiosConfig>>,
    event_bus: EventBus,
    /// Sessions currently being compressed (prevents concurrent runs).
    active: Mutex<HashSet<String>>,
}

impl CompressionService {
    /// Create a new CompressionService.
    pub fn new(
        state_store: Arc<StateStore>,
        engine_handle: EngineHandle,
        config: Arc<RwLock<OxiosConfig>>,
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

    /// Whether a session should be compressed: enough exchanges and no
    /// existing summary covering the compressible range.
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
            let cfg = self.config.read();
            match cfg.system_agents.model_for_task("history_compress") {
                Some(id) => self.engine_handle.resolve(&id),
                None => self.engine_handle.resolve_default(),
            }
        };
        let resolved = resolved.context("resolve compression model")?;

        // Build context and stream.
        let mut ctx = oxi_sdk::Context::new();
        ctx.set_system_prompt(COMPRESSION_SYSTEM_PROMPT);
        ctx.add_message(oxi_sdk::Message::User(oxi_sdk::UserMessage::new(user_prompt)));

        let stream = resolved
            .provider
            .stream(&resolved.model, &ctx, None)
            .await
            .context("start compression stream")?;

        use futures::StreamExt;
        let mut summary = String::new();
        let mut pinned = std::pin::pin!(stream);
        while let Some(event) = pinned.next().await {
            match event {
                oxi_sdk::ProviderEvent::TextDelta { delta, .. } => {
                    summary.push_str(&delta);
                    let _ = self.event_bus.publish(KernelEvent::CompressionDelta {
                        session_id: session_id.to_string(),
                        delta,
                    });
                }
                oxi_sdk::ProviderEvent::Done { .. } => break,
                oxi_sdk::ProviderEvent::Error { error, .. } => {
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
                        "model": resolved.model.id,
                    }),
                );
            })
            .await
            .context("persist compression summary")?;

        let _ = self.event_bus.publish(KernelEvent::CompressionDone {
            session_id: session_id.to_string(),
        });

        tracing::info!(
            session_id = %session_id,
            messages = range_end - start_index,
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
    if let Some(comp) = session.metadata.get("compression") {
        if comp.get("status").and_then(|s| s.as_str()) == Some("done") {
            let covered = comp
                .get("compressed_before_index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            if covered >= range_end {
                return false;
            }
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
    use crate::state_store::{AgentResponse, Session, UserMessage};

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
    fn should_compress_below_threshold() {
        let session = make_session(39);
        assert!(!session_needs_compression(&session));
    }

    #[test]
    fn should_compress_at_threshold() {
        let session = make_session(40);
        assert!(session_needs_compression(&session));
    }

    #[test]
    fn should_not_compress_when_already_covered() {
        let mut session = make_session(45);
        // range_end = 45 - 20 = 25. Mark as covered.
        session.metadata.insert(
            "compression".to_string(),
            json!({ "status": "done", "compressed_before_index": 25 }),
        );
        assert!(!session_needs_compression(&session));
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
```

- [ ] **Step 2: Register module in lib.rs**

In `crates/oxios-kernel/src/lib.rs`, add near the other `pub mod` declarations:

```rust
pub mod compression;
```

And add to the re-exports section:

```rust
pub use compression::CompressionService;
```

- [ ] **Step 3: Verify compilation and tests**

Run: `cargo test -p oxios-kernel compression 2>&1 | tail -20`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/oxios-kernel/src/compression.rs crates/oxios-kernel/src/lib.rs
git commit -m "feat(kernel): CompressionService with LLM streaming summary"
```

---

## Task 3: CompressionApi + KernelHandle wiring

**Files:**
- Create: `crates/oxios-kernel/src/kernel_handle/compression_api.rs`
- Modify: `crates/oxios-kernel/src/kernel_handle/mod.rs`
- Modify: `src/kernel.rs`

**Interfaces:**
- Consumes: `Arc<CompressionService>` (from Task 2).
- Produces: `CompressionApi` with `spawn_compress(String)`, `should_compress(&Session) -> bool`, `compress_now(&str) -> Result<()>`.

- [ ] **Step 1: Create compression_api.rs**

Create `crates/oxios-kernel/src/kernel_handle/compression_api.rs`:

```rust
//! CompressionApi — facade for session context compression.

use std::sync::Arc;

use crate::compression::CompressionService;
use crate::state_store::Session;

/// Facade for session context compression (LLM summaries).
#[derive(Clone)]
pub struct CompressionApi {
    service: Arc<CompressionService>,
}

impl CompressionApi {
    /// Create from a shared CompressionService.
    pub fn new(service: Arc<CompressionService>) -> Self {
        Self { service }
    }

    /// Trigger background compression for a session.
    pub fn spawn_compress(&self, session_id: String) {
        self.service.spawn_compress(session_id);
    }

    /// Check if a session needs compression.
    pub fn should_compress(&self, session: &Session) -> bool {
        self.service.should_compress(session)
    }

    /// Run compression synchronously (for testing / manual trigger).
    pub async fn compress_now(&self, session_id: &str) -> anyhow::Result<()> {
        self.service.compress(session_id).await
    }
}
```

- [ ] **Step 2: Register in kernel_handle/mod.rs**

In `crates/oxios-kernel/src/kernel_handle/mod.rs`:

1. Add module declaration (after `pub mod calendar_api;`):
```rust
pub mod compression_api;
```

2. Add re-export (after `pub use calendar_api::CalendarApi;`):
```rust
pub use compression_api::CompressionApi;
```

3. Add field to `KernelHandle` struct (after `pub calendar: Option<CalendarApi>,`):
```rust
    /// Context compression: LLM session summaries.
    pub compression: Option<CompressionApi>,
```

4. In `KernelHandle::new()`, initialize the field as `None` (same pattern as `calendar`).

5. Add a builder method (near `with_token_maxing`):
```rust
    /// Attach the compression API.
    pub fn with_compression(mut self, api: CompressionApi) -> Self {
        self.compression = Some(api);
        self
    }
```

- [ ] **Step 3: Wire in src/kernel.rs**

In `src/kernel.rs`, find the KernelHandle assembly (around line 235 and line 1418 — there are two assembly sites). After the `with_token_maxing` call, add:

```rust
let kh = {
    let compression_service = Arc::new(oxios_kernel::CompressionService::new(
        state_store.clone(),
        engine_handle.clone(),
        config.clone(),
        event_bus.clone(),
    ));
    kh.with_compression(oxios_kernel::CompressionApi::new(compression_service))
};
```

Adjust variable names to match what's in scope at each assembly site (`self.state_store`, `self.engine_handle`, etc.).

- [ ] **Step 4: Verify compilation**

Run: `cargo check 2>&1 | head -30`
Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add crates/oxios-kernel/src/kernel_handle/compression_api.rs crates/oxios-kernel/src/kernel_handle/mod.rs src/kernel.rs
git commit -m "feat(kernel): CompressionApi facade + KernelHandle wiring"
```

---

## Task 4: API routes + WS forwarding + auto-trigger

**Files:**
- Modify: `src/api/routes/events.rs`
- Modify: `src/api/routes/mod.rs`
- Modify: `src/api/routes/chat.rs`

**Interfaces:**
- Consumes: `state.kernel.compression: Option<CompressionApi>` (from Task 3), `KernelEvent::CompressionDelta/Done/Failed` (from Task 1).
- Produces: `POST /api/sessions/:id/compress` endpoint, `compression` field in `GET /api/sessions/:id` response, WS chunks `compression_delta`/`compression_done`/`compression_failed`.

- [ ] **Step 1: Add compress endpoint to events.rs**

In `src/api/routes/events.rs`, after `handle_session_move`, add:

```rust
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
    Ok(Json(serde_json::json!({ "status": "started" })))
}
```

- [ ] **Step 2: Add compression field to GET /api/sessions/:id response**

In `handle_session_get` (events.rs), add to the `serde_json::json!` response object (after `"reasoning_records"`):

```rust
           "compression": session.metadata.get("compression"),
```

- [ ] **Step 3: Register route in mod.rs**

In `src/api/routes/mod.rs`:

1. Add to the `pub(crate) use events::{...}` block:
```rust
    handle_session_compress,
```

2. Add route (after the `/api/sessions/{id}/project` PATCH route):
```rust
        .route("/api/sessions/{id}/compress", post(handle_session_compress))
```

- [ ] **Step 4: Add WS chunk forwarding in chat.rs**

In `src/api/routes/chat.rs`, find `kernel_event_to_ws_chunk`. In the `event_session_id` match (around line 1578), add:

```rust
        KernelEvent::CompressionDelta { session_id, .. } => Some(session_id),
        KernelEvent::CompressionDone { session_id } => Some(session_id),
        KernelEvent::CompressionFailed { session_id, .. } => Some(session_id),
```

Then in the main match body (after the existing arms), add:

```rust
        KernelEvent::CompressionDelta { session_id, delta } => {
            Some(json!({
                "type": "compression_delta",
                "content": delta,
                "session_id": session_id,
            }))
        }
        KernelEvent::CompressionDone { session_id } => {
            Some(json!({
                "type": "compression_done",
                "session_id": session_id,
            }))
        }
        KernelEvent::CompressionFailed { session_id, error } => {
            Some(json!({
                "type": "compression_failed",
                "error": error,
                "session_id": session_id,
            }))
        }
```

- [ ] **Step 5: Add auto-trigger in persist_session**

In `src/api/routes/chat.rs`, at the end of `persist_session` (after the auto-prune block, around line 1560), add:

```rust
    // Auto-trigger compression for long sessions.
    if let Ok(Some(session)) = state_store.load_session(&sid).await {
        if let Some(ref compression) = kernel_handle.compression {
            if compression.should_compress(&session) {
                compression.spawn_compress(session_id.to_string());
            }
        }
    }
```

Note: `persist_session` needs access to `kernel_handle` — check if it's already in scope or needs to be threaded through. If not available, add the auto-trigger in the WS recv_task's `done` handler instead (where `state` is available).

- [ ] **Step 6: Verify compilation**

Run: `cargo check 2>&1 | head -30`
Expected: compiles clean.

- [ ] **Step 7: Commit**

```bash
git add src/api/routes/events.rs src/api/routes/mod.rs src/api/routes/chat.rs
git commit -m "feat(api): compression endpoint, WS forwarding, auto-trigger"
```

---

## Task 5: Frontend types + store + WS handlers

**Files:**
- Modify: `web/src/types/index.ts`
- Modify: `web/src/stores/chat.ts`

**Interfaces:**
- Consumes: WS chunks `compression_delta`/`compression_done`/`compression_failed` (from Task 4), `GET /api/sessions/:id` response with `compression` field.
- Produces: `CompressionInfo` type, `store.compression` state, WS handlers that update it.

- [ ] **Step 1: Add CompressionInfo type**

In `web/src/types/index.ts`, after the `StreamChunk` interface, add:

```ts
/** LLM-generated session compression summary. */
export interface CompressionInfo {
  summary: string
  status: 'done' | 'generating' | 'failed'
  error?: string
  compressed_at?: string
  original_count?: number
  compressed_before_index?: number
  model?: string
}
```

- [ ] **Step 2: Extend StreamChunk type union**

In the `StreamChunk.type` union (around line 361), add:

```ts
    | 'compression_delta'
    | 'compression_done'
    | 'compression_failed'
```

- [ ] **Step 3: Add compression state to chat store**

In `web/src/stores/chat.ts`:

1. Add to the `ChatRuntimeState` interface (or wherever runtime state is defined):
```ts
  /** LLM compression summary for the active session. */
  compression: CompressionInfo | null
```

2. Initialize in the store creator: `compression: null,`

3. In `loadSession`, after `set({ messages, ... })`, extract compression:
```ts
          const compression: CompressionInfo | null = data.compression ?? null
          set({ compression })
```

4. In `newSession`, reset: `compression: null,`

- [ ] **Step 4: Add WS handlers in handleChunk**

In the `handleChunk` switch statement, add cases (before the `default`):

```ts
          case 'compression_delta': {
            if (!chunk.content) break
            set((s) => {
              const prev = s.compression
              return {
                compression: {
                  summary: (prev?.summary ?? '') + chunk.content,
                  status: 'generating',
                  compressed_before_index: prev?.compressed_before_index,
                },
              }
            })
            break
          }
          case 'compression_done': {
            set((s) => ({
              compression: s.compression
                ? { ...s.compression, status: 'done' }
                : null,
            }))
            break
          }
          case 'compression_failed': {
            set((s) => ({
              compression: s.compression
                ? { ...s.compression, status: 'failed', error: chunk.error }
                : { summary: '', status: 'failed', error: chunk.error },
            }))
            break
          }
```

- [ ] **Step 5: Verify TypeScript compiles**

Run: `cd web && bunx tsc --noEmit 2>&1 | head -20`
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
cd web && git add src/types/index.ts src/stores/chat.ts
git commit -m "feat(web): compression state + WS handlers in chat store"
```

---

## Task 6: Statistical digest fallback

**Files:**
- Create: `web/src/lib/compressed-summary.ts`
- Create: `web/src/lib/compressed-summary.test.ts`

**Interfaces:**
- Consumes: `ChatMessage[]`.
- Produces: `buildCompressedDigest(messages) → CompressedDigest`.

- [ ] **Step 1: Write the failing tests**

Create `web/src/lib/compressed-summary.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import type { ChatMessage } from '@/types'
import { buildCompressedDigest } from './compressed-summary'

describe('buildCompressedDigest', () => {
  it('counts roles and tool calls across folded messages', () => {
    const messages: ChatMessage[] = [
      { id: 'u1', role: 'user', content: 'q', timestamp: '2026-01-01T00:00:00Z' },
      {
        id: 'a1',
        role: 'assistant',
        content: 'r',
        timestamp: '2026-01-01T00:01:00Z',
        blocks: [
          { type: 'tool', id: 't1', identifier: 'k', apiName: 'grep', arguments: {}, status: 'success' },
          { type: 'tool', id: 't2', identifier: 'k', apiName: 'read', arguments: {}, status: 'success' },
          { type: 'text', id: 'x1', text: 'answer' },
        ],
      },
      { id: 'u2', role: 'user', content: 'q2', timestamp: '2026-01-01T00:02:00Z' },
    ]
    const d = buildCompressedDigest(messages)
    expect(d.total).toBe(3)
    expect(d.userCount).toBe(2)
    expect(d.assistantCount).toBe(1)
    expect(d.toolCallCount).toBe(2)
    expect(d.firstAt).toBe('2026-01-01T00:00:00Z')
    expect(d.lastAt).toBe('2026-01-01T00:02:00Z')
  })

  it('returns zeros for an empty list', () => {
    expect(buildCompressedDigest([])).toMatchObject({
      total: 0,
      userCount: 0,
      assistantCount: 0,
      toolCallCount: 0,
    })
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/lib/compressed-summary.test.ts`
Expected: FAIL — cannot resolve `./compressed-summary`.

- [ ] **Step 3: Implement buildCompressedDigest**

Create `web/src/lib/compressed-summary.ts`:

```ts
// compressed-summary — client-side statistical digest for the collapsed
// message group. Fallback when no LLM summary exists or generation failed.

import type { ChatMessage } from '@/types'

export interface CompressedDigest {
  total: number
  userCount: number
  assistantCount: number
  toolCallCount: number
  firstAt?: string
  lastAt?: string
}

export function buildCompressedDigest(messages: ChatMessage[]): CompressedDigest {
  let userCount = 0
  let assistantCount = 0
  let toolCallCount = 0
  let firstAt: string | undefined
  let lastAt: string | undefined

  for (const m of messages) {
    if (m.role === 'user') userCount++
    else if (m.role === 'assistant') assistantCount++

    if (m.blocks) {
      toolCallCount += m.blocks.filter((b) => b.type === 'tool').length
    }

    const ts = m.timestamp
    if (ts) {
      if (!firstAt || ts < firstAt) firstAt = ts
      if (!lastAt || ts > lastAt) lastAt = ts
    }
  }

  return { total: messages.length, userCount, assistantCount, toolCallCount, firstAt, lastAt }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web && bun run vitest run src/lib/compressed-summary.test.ts`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
cd web && git add src/lib/compressed-summary.ts src/lib/compressed-summary.test.ts
git commit -m "feat(web): statistical digest fallback for compressed group"
```

---

## Task 7: CompressedGroup tabs + chat-rows + route wiring + i18n

**Files:**
- Modify: `web/src/components/chat/compressed-group.tsx`
- Modify: `web/src/lib/chat-rows.ts`
- Modify: `web/src/routes/chat.tsx`
- Modify: `web/src/i18n/locales/en.json`
- Modify: `web/src/i18n/locales/ko.json`

**Interfaces:**
- Consumes: `CompressionInfo` (Task 5), `buildCompressedDigest` (Task 6), `ChatMessage[]` folded slice.
- Produces: Tabbed CompressedGroup with Summary (streaming markdown) and History (original messages).

- [ ] **Step 1: Add i18n keys**

In `web/src/i18n/locales/en.json`, under the `"chat"` object, add:

```json
"compression.summaryTab": "Summary",
"compression.historyTab": "History",
"compression.generating": "Generating summary…",
"compression.failed": "Summary generation failed",
"compression.retry": "Retry",
"compression.messagesCompressed": "{{count}} messages compressed",
"compression.digest.messages": "{{count}} messages",
"compression.digest.userMessages": "{{count}} user",
"compression.digest.assistantMessages": "{{count}} assistant",
"compression.digest.toolCalls": "{{count}} tool calls"
```

In `web/src/i18n/locales/ko.json`, under the `"chat"` object, add:

```json
"compression.summaryTab": "요약",
"compression.historyTab": "원본",
"compression.generating": "요약 생성 중…",
"compression.failed": "요약 생성 실패",
"compression.retry": "다시 시도",
"compression.messagesCompressed": "{{count}}개 메시지 압축됨",
"compression.digest.messages": "{{count}}개 메시지",
"compression.digest.userMessages": "{{count}}개 사용자",
"compression.digest.assistantMessages": "{{count}}개 어시스턴트",
"compression.digest.toolCalls": "{{count}}개 도구 호출"
```

- [ ] **Step 2: Extend chat-rows.ts collapse-bar row**

In `web/src/lib/chat-rows.ts`:

1. Add import at top:
```ts
import type { CompressionInfo } from '@/types'
```

2. Change the `collapse-bar` variant:
```ts
  | { kind: 'collapse-bar'; count: number; foldedMessages: ChatMessage[]; compression: CompressionInfo | null }
```

3. Add to `BuildChatRowsOptions`:
```ts
  compression: CompressionInfo | null
```

4. In `buildChatRows`, update the collapse-bar push:
```ts
    rows.push({
      kind: 'collapse-bar',
      count: collapseCount,
      foldedMessages: messages.slice(0, collapseCount),
      compression: opts.compression,
    })
```

- [ ] **Step 3: Rewrite CompressedGroup with tabs**

Replace `web/src/components/chat/compressed-group.tsx` with:

```tsx
// CompressedGroup — collapsible panel with Summary/History tabs for older
// messages in long conversations (LobeHub CompressedGroup port).

import { ChevronDown, ChevronRight, FileText, History, MessagesSquare } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { buildCompressedDigest } from '@/lib/compressed-summary'
import type { ChatMessage, CompressionInfo } from '@/types'

interface CompressedGroupProps {
  count: number
  expanded: boolean
  onToggle: () => void
  foldedMessages: ChatMessage[]
  compression: CompressionInfo | null
  className?: string
}

type Tab = 'summary' | 'history'

export function CompressedGroup({
  count,
  expanded,
  onToggle,
  foldedMessages,
  compression,
  className,
}: CompressedGroupProps) {
  const { t } = useTranslation()
  const [tab, setTab] = useState<Tab>('summary')

  return (
    <div className={cn('w-full', className)}>
      {/* Toggle bar */}
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-2 rounded-lg border border-dashed bg-muted/30 px-3 py-2 text-xs text-muted-foreground transition-colors hover:bg-muted/60"
      >
        {expanded ? (
          <ChevronDown className="size-3.5 shrink-0" />
        ) : (
          <ChevronRight className="size-3.5 shrink-0" />
        )}
        <MessagesSquare className="size-3.5 shrink-0" />
        <span>
          {expanded ? t('chat.compressedExpanded') : t('chat.compressedCollapsed', { count })}
        </span>
      </button>

      {/* Tabbed panel (only when expanded) */}
      {expanded && (
        <div className="mt-1 rounded-lg border bg-card">
          {/* Tab headers */}
          <div className="flex border-b px-2">
            <TabButton active={tab === 'summary'} onClick={() => setTab('summary')}>
              <FileText className="size-3" />
              {t('chat.compression.summaryTab')}
            </TabButton>
            <TabButton active={tab === 'history'} onClick={() => setTab('history')}>
              <History className="size-3" />
              {t('chat.compression.historyTab')}
            </TabButton>
          </div>

          {/* Tab content */}
          <div className="max-h-80 overflow-y-auto p-3 text-sm">
            {tab === 'summary' ? (
              <SummaryContent compression={compression} messages={foldedMessages} />
            ) : (
              <HistoryContent messages={foldedMessages} />
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex items-center gap-1 border-b-2 px-3 py-1.5 text-xs font-medium transition-colors',
        active
          ? 'border-primary text-foreground'
          : 'border-transparent text-muted-foreground hover:text-foreground',
      )}
    >
      {children}
    </button>
  )
}

function SummaryContent({
  compression,
  messages,
}: {
  compression: CompressionInfo | null
  messages: ChatMessage[]
}) {
  const { t } = useTranslation()

  if (compression?.status === 'generating') {
    return (
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="inline-block size-2 animate-pulse rounded-full bg-primary" />
          {t('chat.compression.generating')}
        </div>
        <div className="whitespace-pre-wrap text-sm">{compression.summary}</div>
      </div>
    )
  }

  if (compression?.status === 'done' && compression.summary) {
    return <div className="prose prose-sm dark:prose-invert max-w-none whitespace-pre-wrap">{compression.summary}</div>
  }

  if (compression?.status === 'failed') {
    return (
      <div className="space-y-2">
        <p className="text-xs text-destructive">{t('chat.compression.failed')}</p>
        <DigestFallback messages={messages} />
      </div>
    )
  }

  // No compression yet — show statistical digest.
  return <DigestFallback messages={messages} />
}

function DigestFallback({ messages }: { messages: ChatMessage[] }) {
  const { t } = useTranslation()
  const d = buildCompressedDigest(messages)
  return (
    <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
      <span>{t('chat.compression.digest.messages', { count: d.total })}</span>
      <span>{t('chat.compression.digest.userMessages', { count: d.userCount })}</span>
      <span>{t('chat.compression.digest.assistantMessages', { count: d.assistantCount })}</span>
      <span>{t('chat.compression.digest.toolCalls', { count: d.toolCallCount })}</span>
    </div>
  )
}

function HistoryContent({ messages }: { messages: ChatMessage[] }) {
  return (
    <div className="space-y-2">
      {messages.map((m) => (
        <div key={m.id} className="flex gap-2 text-xs">
          <span className="shrink-0 font-medium text-muted-foreground">
            {m.role === 'user' ? '👤' : '🤖'}
          </span>
          <span className="line-clamp-2 text-foreground/80">{m.content}</span>
        </div>
      ))}
    </div>
  )
}
```

- [ ] **Step 4: Update routes/chat.tsx**

In `web/src/routes/chat.tsx`:

1. Import `CompressionInfo` if not already available via the store.

2. Get compression from the store (near where `messages` is destructured):
```ts
const compression = useChatStore((s) => s.compression)
```

3. Pass `compression` to `buildChatRows`:
```ts
      buildChatRows({
        messages,
        expanded,
        collapseThreshold: COLLAPSE_THRESHOLD,
        visibleTail: VISIBLE_TAIL,
        compression,
        hasInterview: ...,
        ...
      })
```

4. Update the `collapse-bar` row rendering (around line 298):
```tsx
              if (row.kind === 'collapse-bar') {
                return (
                  <div key="collapse-bar" className="mx-auto max-w-3xl px-4 pt-6">
                    <CompressedGroup
                      count={row.count}
                      expanded={expanded}
                      onToggle={() => setExpanded((v) => !v)}
                      foldedMessages={row.foldedMessages}
                      compression={row.compression}
                    />
                  </div>
                )
              }
```

5. Add auto-trigger (after `loadSession` completes, in a `useEffect` or callback):
```ts
  // Auto-trigger compression for long sessions without a summary.
  const compressTriggered = useRef(false)
  useEffect(() => {
    if (
      activeSessionId &&
      messages.length >= COLLAPSE_THRESHOLD &&
      compression === null &&
      !compressTriggered.current
    ) {
      compressTriggered.current = true
      fetch(`/api/sessions/${encodeURIComponent(activeSessionId)}/compress`, {
        method: 'POST',
        headers: { Authorization: `Bearer ${getToken()}` },
      }).catch(() => {})
    }
  }, [activeSessionId, messages.length, compression])
```

Reset `compressTriggered.current = false` when `activeSessionId` changes.

- [ ] **Step 5: Verify TypeScript + lint + tests**

Run: `cd web && bunx tsc --noEmit && bunx biome check src && bun run vitest run 2>&1 | tail -20`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cd web && git add src/components/chat/compressed-group.tsx src/lib/chat-rows.ts src/routes/chat.tsx src/i18n/locales/en.json src/i18n/locales/ko.json
git commit -m "feat(web): CompressedGroup Summary/History tabs with streaming"
```

---

## Task 8: Final verification

**Files:** None (verification only).

- [ ] **Step 1: Full Rust CI gate**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace 2>&1 | tail -30`
Expected: all pass.

- [ ] **Step 2: Full web CI gate**

Run: `cd web && bunx tsc --noEmit && bunx biome check src && bun run vitest run && bun run build 2>&1 | tail -30`
Expected: all pass.

- [ ] **Step 3: Smoke test**

1. Start the daemon: `cargo run` (or however the dev server starts).
2. Open the web UI, create a session, send 40+ messages (or seed a long session via the API).
3. Verify: collapse bar appears, expanding shows Summary/History tabs.
4. Verify: Summary tab shows streaming text (or digest if no model configured).
5. Verify: reload page → summary persists.
6. Verify: `POST /api/sessions/:id/compress` returns 202.

- [ ] **Step 4: Final commit (if any fixups needed)**

```bash
git add -A && git commit -m "fix: compression smoke test fixups"
```
