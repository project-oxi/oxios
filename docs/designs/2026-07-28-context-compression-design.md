# Context Compression — Design Spec

> **Date**: 2026-07-28
> **Status**: Implemented (2026-07-31)
> **Scope**: Backend LLM summarization + frontend Summary/History tabs in CompressedGroup
> **Reference**: LobeHub `compressContext` pipeline, ported to Oxios architecture

---

## 1. Goal

Long conversations get an LLM-generated summary. The Web UI's folded message group (CompressedGroup) shows two tabs: **Summary** (streaming markdown) and **History** (original messages). The summary persists across reloads.

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│ Backend (oxios-kernel)                                          │
│                                                                 │
│  Trigger ──► CompressionService ──► provider.stream()           │
│    │              │                       │                     │
│    │              │                  TextDelta chunks            │
│    │              │                       │                     │
│    │              ▼                       ▼                     │
│    │     session.metadata          KernelEvent::                │
│    │     ["compression"]           CompressionDelta             │
│    │              │                       │                     │
│    │              │                       ▼                     │
│    │              │               EventBus broadcast            │
│    │              │                       │                     │
└────┼──────────────┼───────────────────────┼─────────────────────┘
     │              │                       │
     │              ▼                       ▼
     │     GET /api/sessions/:id     WS chunk: compression_delta
     │     (includes compression)    WS chunk: compression_done
     │              │               WS chunk: compression_failed
     │              ▼                       │
└────┼──────────────────────────────────────┼─────────────────────┘
     │              │                       │
     ▼              ▼                       ▼
┌─────────────────────────────────────────────────────────────────┐
│ Frontend (web/)                                                 │
│                                                                 │
│  loadSession ──► store.compression ──► CompressedGroup          │
│  WS handler  ──► store.compression      ├─ Summary tab          │
│                  (streaming append)     └─ History tab          │
└─────────────────────────────────────────────────────────────────┘
```

## 3. Backend

### 3.1 CompressionService

**New file**: `crates/oxios-kernel/src/compression.rs`

```rust
pub struct CompressionService {
    state_store: Arc<StateStore>,
    engine_handle: EngineHandle,
    config: Arc<RwLock<OxiosConfig>>,
    event_bus: EventBus,
    /// Per-session lock to prevent concurrent compression runs.
    active: Mutex<HashSet<String>>,
}
```

**Core method**:

```rust
impl CompressionService {
    /// Compress a session's old messages into an LLM summary.
    /// Streams deltas via EventBus. Persists result to session metadata.
    pub async fn compress(&self, session_id: &str) -> Result<()>;

    /// Check if a session should be compressed (exchange_count >= threshold
    /// and no existing summary covering those messages).
    pub fn should_compress(&self, session: &Session) -> bool;

    /// Trigger compression in the background (tokio::spawn).
    /// No-op if already running for this session.
    pub fn spawn_compress(&self, session_id: String);
}
```

**Compression logic** (`compress`):

1. Load session from StateStore.
2. Determine compression range: `user_messages[0..len - VISIBLE_TAIL]` (VISIBLE_TAIL = 20, matching frontend). Skip if fewer than `COLLAPSE_THRESHOLD` (40) exchanges.
3. Check existing summary: if `metadata["compression"]["compressed_before_index"] >= range_end`, skip (already covered).
4. Set `metadata["compression"]["status"] = "generating"`, save.
5. Build prompt (see §3.2). If an existing summary exists (`compressed_before_index > 0`), prepend it as `<existing_summary>` context before the new exchanges — the LLM merges old summary + new messages into one updated summary. Only messages `[compressed_before_index..range_end]` are formatted as new exchanges.
6. Resolve model: `config.system_agents.model_for_task("history_compress")` → fallback to engine default. (Pattern: `EngineApi::generate_follow_up`.)
7. Stream via `provider.stream(&model, &ctx, None)`:
   - On `TextDelta`: emit `KernelEvent::CompressionDelta { session_id, delta }`.
   - On `Done`: finalize.
   - On `Error`: set status = "failed", emit `KernelEvent::CompressionFailed { session_id, error }`.
8. On success: `update_session_with` → set `metadata["compression"]` = final JSON (see §3.3).
9. Remove session from `active` set.

**Trigger points**:

- **Automatic**: In `persist_session` (chat.rs WS handler), after saving, check `should_compress` → `spawn_compress`. Throttled: only if exchange_count crossed the threshold since last check.
- **Manual**: `POST /api/sessions/:id/compress` → `spawn_compress`, return 202.

### 3.2 Prompt (ported from LobeHub)

System prompt — verbatim from `lobehub/packages/prompts/src/prompts/compressContext/index.ts`:

```
You are a conversation context compressor. Your task is to create a structured
summary that preserves essential information while significantly reducing token count.

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
- Prioritize information that affects future responses
```

User prompt:

```
<chat_history>
{formatted exchanges}
</chat_history>

Please compress the above conversation history.
Output ONLY the structured summary following the format specified. No additional commentary or meta-discussion.
```

Exchange formatting:

```
[User]: {content}
[Assistant]: {content}
```

### 3.3 Storage Schema

`session.metadata["compression"]`:

```json
{
  "summary": "### Context\n...",
  "status": "done" | "generating" | "failed",
  "error": null | "error message",
  "compressed_at": "2026-07-28T12:00:00Z",
  "original_count": 45,
  "compressed_before_index": 25,
  "model": "anthropic/claude-sonnet-4-20250514"
}
```

- `compressed_before_index`: messages `[0..compressed_before_index]` are covered by this summary. Messages after that index are "recent" and shown as-is.
- Original messages are NEVER deleted. History tab renders them.

### 3.4 KernelEvent Variants

Add to `event_bus.rs`:

```rust
/// A chunk of the compression summary being streamed.
CompressionDelta {
    session_id: String,
    delta: String,
},
/// Compression completed successfully.
CompressionDone {
    session_id: String,
},
/// Compression failed.
CompressionFailed {
    session_id: String,
    error: String,
},
```

### 3.5 WS Chunk Forwarding

In `kernel_event_to_ws_chunk` (chat.rs), add:

```rust
KernelEvent::CompressionDelta { session_id, delta } => {
    json!({ "type": "compression_delta", "content": delta, "session_id": session_id })
}
KernelEvent::CompressionDone { session_id } => {
    json!({ "type": "compression_done", "session_id": session_id })
}
KernelEvent::CompressionFailed { session_id, error } => {
    json!({ "type": "compression_failed", "error": error, "session_id": session_id })
}
```

Session filtering: same pattern as ToolExecutionStarted (filter by active_session_id).

### 3.6 API Endpoints

**New**: `POST /api/sessions/:id/compress`

- Handler: `handle_session_compress` in `events.rs`
- Calls `compression_service.spawn_compress(id)`
- Returns `202 Accepted` with `{ "status": "started" }` or `{ "status": "already_running" }`
- If session not found: 404. If exchange_count < threshold: 422 with reason.

**Modified**: `GET /api/sessions/:id` response gains a top-level `compression` field extracted from `session.metadata["compression"]` (null if absent). This avoids the frontend digging into the generic metadata map.

### 3.7 KernelHandle Integration

**New file**: `crates/oxios-kernel/src/kernel_handle/compression_api.rs`

```rust
pub struct CompressionApi {
    service: Arc<CompressionService>,
}

impl CompressionApi {
    pub fn new(service: Arc<CompressionService>) -> Self;
    pub fn spawn_compress(&self, session_id: String);
    pub fn should_compress(&self, session: &Session) -> bool;
    pub async fn compress_now(&self, session_id: &str) -> Result<()>;
}
```

Wire in `KernelHandle` (mod.rs) + `src/kernel.rs` assembly.

### 3.8 Config

Already exists: `config.system_agents.history_compress` (`SystemAgentItem` with `model`, `enabled`, `context_limit`, `custom_prompt`). The service reads `model_for_task("history_compress")` and `is_enabled("history_compress")`.

Default: enabled = true (opt-out), model = "" (engine default).

## 4. Frontend

### 4.1 Types

Add to `web/src/types/index.ts`:

```ts
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

Add to `StreamChunk.type` union: `'compression_delta' | 'compression_done' | 'compression_failed'`.

### 4.2 Chat Store

Add to `web/src/stores/chat.ts` state:

```ts
compression: CompressionInfo | null
```

- `loadSession`: extract `compression` from response → store.
- WS handler:
  - `compression_delta` → append to `compression.summary`, set status = "generating".
  - `compression_done` → set status = "done".
  - `compression_failed` → set status = "failed", store error.

### 4.3 CompressedGroup Component

Extend `web/src/components/chat/compressed-group.tsx`:

**New props**:

```ts
interface CompressedGroupProps {
  count: number
  expanded: boolean
  onToggle: () => void
  // New:
  foldedMessages: ChatMessage[]
  compression: CompressionInfo | null
  className?: string
}
```

**Expanded state renders a tabbed panel**:

```
┌─────────────────────────────────────────┐
│ ▾ 23 earlier messages                   │
├─────────────────────────────────────────┤
│ [Summary] [History]                     │
├─────────────────────────────────────────┤
│ (tab content)                           │
└─────────────────────────────────────────┘
```

**Summary tab content by status**:

| Status | Render |
|---|---|
| `done` | Markdown (reuse existing markdown renderer) |
| `generating` | Markdown + shimmer animation on last line |
| `failed` | Error notice + statistical digest fallback |
| `null` (no compression) | Statistical digest (`buildCompressedDigest`) |

**History tab**: Scrollable list of `foldedMessages`, each rendered as a compact message row (role icon + truncated content + timestamp). Click to expand individual message.

**Tab state**: `useState<'summary' | 'history'>('summary')`. Persisted in localStorage per session (LobeHub pattern).

### 4.4 Statistical Digest Fallback

**New file**: `web/src/lib/compressed-summary.ts` (from Round 2 plan)

```ts
export interface CompressedDigest {
  total: number
  userCount: number
  assistantCount: number
  toolCallCount: number
  firstAt?: string
  lastAt?: string
}
export function buildCompressedDigest(messages: ChatMessage[]): CompressedDigest
```

Used when: no compression exists, compression failed, or compression is generating (as a header above the streaming text).

### 4.5 Chat Rows Integration

`web/src/lib/chat-rows.ts`: extend the `collapse-bar` row kind:

```ts
| { kind: 'collapse-bar'; count: number; foldedMessages: ChatMessage[]; compression: CompressionInfo | null }
```

`buildChatRows` passes the folded slice + compression info from the store.

### 4.6 Auto-trigger (frontend)

In `web/src/routes/chat.tsx`, after `loadSession` completes: if `messages.length >= COLLAPSE_THRESHOLD` and `compression === null`, fire `POST /api/sessions/:id/compress`. One-shot per session load (guard with a ref).

### 4.7 i18n

New keys in both `en.json` and `ko.json`:

```json
{
  "chat.compression.summaryTab": "Summary" / "요약",
  "chat.compression.historyTab": "History" / "원본",
  "chat.compression.generating": "Generating summary…" / "요약 생성 중…",
  "chat.compression.failed": "Summary generation failed" / "요약 생성 실패",
  "chat.compression.retry": "Retry" / "다시 시도",
  "chat.compression.messagesCompressed": "{{count}} messages compressed" / "{{count}}개 메시지 압축됨"
}
```

## 5. File Change Summary

| File | Action | Responsibility |
|---|---|---|
| `crates/oxios-kernel/src/compression.rs` | **Create** | CompressionService: trigger, prompt, LLM stream, persist |
| `crates/oxios-kernel/src/event_bus.rs` | Modify | +3 KernelEvent variants |
| `crates/oxios-kernel/src/kernel_handle/compression_api.rs` | **Create** | CompressionApi facade |
| `crates/oxios-kernel/src/kernel_handle/mod.rs` | Modify | +compression field |
| `crates/oxios-kernel/src/lib.rs` | Modify | +mod compression, re-export |
| `src/kernel.rs` | Modify | Wire CompressionApi |
| `src/api/routes/events.rs` | Modify | +handle_session_compress, +compression in GET response |
| `src/api/routes/mod.rs` | Modify | +route registration |
| `src/api/routes/chat.rs` | Modify | kernel_event_to_ws_chunk +3 arms, auto-trigger in persist_session |
| `web/src/types/index.ts` | Modify | +CompressionInfo, +StreamChunk types |
| `web/src/stores/chat.ts` | Modify | +compression state, +WS handlers |
| `web/src/components/chat/compressed-group.tsx` | Modify | Tabbed panel (Summary/History) |
| `web/src/lib/chat-rows.ts` | Modify | collapse-bar row gains foldedMessages + compression |
| `web/src/routes/chat.tsx` | Modify | Pass new props, auto-trigger |
| `web/src/lib/compressed-summary.ts` | **Create** | Statistical digest fallback |
| `web/src/i18n/locales/en.json` | Modify | +compression keys |
| `web/src/i18n/locales/ko.json` | Modify | +compression keys |

## 6. Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Compression unit | Whole session (old messages → one summary) | Oxios sessions are the unit; no sub-grouping needed |
| Trigger | Auto (threshold) + manual (API) | LobeHub parity; auto = fire-and-forget |
| Threshold | 40 exchanges (matches COLLAPSE_THRESHOLD) | Consistent with existing UI fold |
| Visible tail | 20 messages never compressed | Matches VISIBLE_TAIL; recent context always raw |
| Summary storage | `session.metadata["compression"]` | Existing field, no schema migration, JSON-flexible |
| Original preservation | Always keep | History tab needs them; no data loss |
| Summary model | `system_agents.history_compress` config → engine default | Config plumbing already exists |
| Streaming | Yes, via KernelEvent → WS | LobeHub parity; user sees progress |
| Re-compression | Incremental: only compress messages beyond `compressed_before_index` | Avoids re-summarizing already-summarized content |
| Failure handling | status = "failed" + frontend digest fallback | Graceful degradation |

## 7. Acceptance Criteria

1. Session with ≥40 exchanges triggers automatic compression (background, non-blocking).
2. Summary streams to the UI in real-time (visible token-by-token in Summary tab).
3. Summary persists across page reloads (stored in session metadata).
4. History tab shows all original folded messages.
5. Failed compression shows error + statistical digest fallback.
6. Manual `POST /api/sessions/:id/compress` works.
7. Re-compression after new messages only summarizes the delta.
8. No regression in existing chat/virtualization/streaming behavior.
9. `cargo fmt && clippy -D warnings && cargo test --workspace` passes.
10. `bunx tsc --noEmit && bunx biome check src && bun run vitest run && bun run build` passes.

## 8. Out of Scope

- Context replacement in the model loop (Oxios uses memory recall, not message replay).
- Token-based threshold (message count is sufficient; token counting adds complexity).
- Compression cancellation mid-stream (can add later via abort handle).
- Multi-group compression (one summary per session is sufficient for now).
