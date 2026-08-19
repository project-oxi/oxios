# Chat UI Defect Remediation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every defect found in the 2026-08-19 four-axis audit of the web chat surface (streaming pipeline, rendering/artifacts, agent transparency, shell/input) so the chat matches the Claude Code / Claude Desktop bar end-to-end.

**Architecture:** Three layers change. (1) **Kernel/gateway** gains a turn registry so an in-flight chat turn is addressable by the same key the streaming sink already uses (`session_id`, or `request_id` for a session's first message) — that unlocks real server-side cancellation and kills the token burn. (2) **Wire contract** is made truthful: `ErrorKind` serialization matches the client union, a `cancelled` kind is added, agent lifecycle events gain session correlation, and the never-emitted `phase` streaming chunk is deleted rather than faked. (3) **Frontend** gets a turn watchdog, loss-free socket close, session-load states, streaming-safe markdown, artifact-panel dialog semantics, selector-scoped store subscriptions, and an i18n/token sweep.

**Tech Stack:** Rust 2024 (tokio, serde, axum), React 19 + TanStack Router + zustand + virtua, react-markdown 10 / rehype, vitest + @testing-library/react, biome, i18next.

## Global Constraints

- Rust edition 2024, MSRV 1.96. Target `aarch64-apple-darwin`.
- `anyhow` in binaries, `thiserror` in libraries. `#![warn(missing_docs)]` holds on public crates — every new public item gets a doc comment.
- **No new third-party crates.** `oxios-kernel` does **not** depend on `tokio-util`; do not add it. Use `tokio::sync::Notify` + `AtomicBool`.
- Kernel stays monolithic (ARCHITECTURE.md §10). New kernel modules are files under `crates/oxios-kernel/src/`, registered in `lib.rs`.
- **Structural/tool output is English** (WS error strings, CLI, log lines). **Web UI is bilingual** — every user-visible string goes through `react-i18next` with a key in BOTH `web/src/i18n/locales/en.json` and `ko.json`.
- Design tokens: no `dark:` variants and no hardcoded hex/named colors in components (DESIGN.md §4). Use the OKLCH token layer in `web/src/index.css`.
- Gates before each commit touching Rust: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-features -- -D warnings`, `cargo test -p <crate>`. Touching web: `bun run typecheck`, `bunx biome check <files>`, `bun run vitest run`.
- Conventional commits, scopes: `kernel`, `gateway`, `web`, `docs`.
- Do **not** reformat pre-existing drift in files you touch. There are known lint failures in `src/stores/quick-ask.ts`, `src/routes/cron-jobs.tsx`, `src/components/chat/messages/AssistantMessage.tsx`, `src/__tests__/hooks/use-knowledge-saves.test.tsx`, `src/components/chat/messages/components/BlockStream.test.tsx` — leave them alone unless your task edits that file, in which case fix only your own hunks.

---

## Already Landed (2026-08-19, do not redo)

Three defects from the audit were fixed and verified during the audit session. They are recorded here so a reader of this plan does not re-open them.

| Defect | Fix | Verification |
|---|---|---|
| `extractText()` flattened rehype-highlight `<span>` children to `''` — every language-tagged code block rendered mangled (`fn main() { println!("hi"); }` → ` () { (); }`) and Copy copied the corruption | `CodeBlock` split into `code: string` (copy payload from `hastToText(node)`) + `children: ReactNode` (highlighted nodes). Syntax highlighting now actually renders — it was being discarded entirely | `web/src/__tests__/components/chat/markdown-code-block.test.tsx` |
| react-markdown 10 dropped the `inline` prop (zero occurrences in the published lib), so `if (inline)` was unreachable and **every** inline `` `code` `` rendered as a full CodeBlock card, emitting `<div>` inside `<p>` (React hydration error) | `rehypeMarkInlineCode` plugin marks any `<code>` whose parent is not `<pre>`; the component branches on hast structure | same file, inline case |
| WS `grounding` chunk had no case in `adaptChunk` → fell to `default: {events: []}`. `message.search` was never populated; `SearchGrounding` was dead code and every web-search citation was lost | `case 'grounding'` in `adapter.ts`; `StreamProcessor` accumulates + dedupes citations by url across multiple searches in one turn; `materialize()` carries `search`; `chat.ts` routes the chunk through the processor | `web/src/__tests__/lib/stream-grounding.test.ts` |

---

## Design Decisions (read before implementing)

**D1 — The turn key already exists; do not invent a new one.**
`gateway.rs:488` computes `let sink_session_key = session_id.clone().unwrap_or_else(|| request_id.clone());` and registers the streaming sink under it at `:489`; the comment at `:492-495` already states the rule ("partial token messages + unregister use the same key … and the chat.rs event filter (`active_session_id`) all agree"). The orchestrator mirrors it into `ExecEnv.session_id`, and the agent runtime uses it as `transparency_session`. Every new registry keys on this same string. Never introduce a second identity.

**D2 — Cancellation needs both halves.**
Aborting only the gateway dispatch future leaves the supervisor-spawned agent task running and still burning provider tokens (`BasicSupervisor.handles` is keyed by `AgentId`, and `AgentLifecycleManager::execute_directive` currently drops the `AgentId` at `agent_lifecycle.rs:128`). So cancellation must (a) wake the gateway task so it stops awaiting and unregisters the sink, and (b) kill the agent by `AgentId`. Task 1 stores the binding; Task 2 uses it.

**D3 — There is no multi-phase lifecycle to visualize.**
Post-RFC-027 the Ouroboros phase is a plain `String` on `OrchestrationResult.phase_reached`, and the only literal ever assigned is `"execute"` (`orchestrator.rs:602`). There is no phase enum in `oxios-ouroboros`. Building a "step N of M" indicator would be inventing UI for state that does not exist. **Decision: delete the dead `phase` streaming path** (`adapter.ts` case, `StreamProcessor` case, `KNOWN_CHUNK_TYPES` entry) and keep `phase`/`evaluation_passed` purely as `done` metadata, which `chat-metadata.tsx` already renders. Truthful contract over fake progress.

**D4 — Code-block line numbers: declined.**
The audit listed them as missing vs "the Claude bar". Claude Desktop does not render them either, they break copy-paste in most implementations, and with rehype-highlight's flat span stream they require re-splitting the highlighted tree per line on every streaming frame. Long-block collapse and focus-visible copy (Task 13) deliver the actual ergonomic win. This is a deliberate non-goal.

**D5 — The partial-replay gap is intentional; document it.**
Partial deltas skip `assign_seq` (`gateway.rs:514-530`) so they are absent from the 512-entry replay buffer. A mid-stream reconnect therefore recovers the terminal full-text message rather than resuming token flow. That is correct — the terminal carries complete text, and buffering partials would multiply the buffer's memory by the token count. No code change; Task 25 documents the boundary.

**D6 — SSE `/api/events` and the chat WS legitimately overlap.**
Both carry `KernelEvent`s. SSE is the global dashboard feed; WS is per-turn chat transparency. Consumers dedupe by context. No code change; Task 25 documents it.

---

## File Structure

**Created**

| Path | Responsibility |
|---|---|
| `crates/oxios-kernel/src/turn_registry.rs` | Turn-key → cancellation flag + bound `AgentId`. Mirrors `streaming_sink.rs` in shape and lifetime. |
| `web/src/lib/markdown/heal-streaming.ts` | Pure function: close unterminated GFM tables / inline emphasis in a partial markdown buffer. |
| `web/src/lib/relative-time.ts` | Single i18n-backed relative-time formatter, replacing two hardcoded English copies. |
| `web/src/components/chat/session-skeleton.tsx` | Shimmer rows shown while `loadSession` is in flight. |
| `web/src/hooks/use-focus-trap.ts` | Focus trap + Escape handling for the portal panel. |

**Modified (primary)**

| Path | Change |
|---|---|
| `crates/oxios-kernel/src/lib.rs` | Register `turn_registry` module. |
| `crates/oxios-kernel/src/kernel_handle/mod.rs` | `pub turns: Arc<TurnRegistry>` + builder, mirroring `streaming_sinks`. |
| `crates/oxios-kernel/src/agent_lifecycle.rs` | Bind `AgentId` to the turn key after fork; unbind on return. |
| `crates/oxios-kernel/src/event_bus.rs` | `session_id: Option<String>` on the four agent lifecycle variants. |
| `crates/oxios-kernel/src/supervisor.rs` | Thread `env.session_id` into the lifecycle event publishes. |
| `crates/oxios-gateway/src/message.rs` | `ErrorKind` snake_case serde + `Cancelled` variant. |
| `crates/oxios-gateway/src/gateway.rs` | Turn registry open/close; `select!` on cancellation in the dispatch task. |
| `src/api/routes/chat.rs` | WS `cancel` arm; `cancelled` terminal chunk; agent lifecycle → WS chunks; `TurnTextStreamTracker` reset. |
| `web/src/stores/chat.ts` | `cancelTurn`, watchdog, `onclose` flush, `isLoadingSession`/`sessionLoadError`, widened error kinds, selector exports. |
| `web/src/routes/chat.tsx` | Real cancel, granular selectors, scroll-churn fix, skeleton, error/retry. |
| `web/src/components/chat/markdown-message.tsx` | Streaming heal, light/dark highlight, block collapse, focus-visible copy, prose token fix. |
| `web/src/components/portal/portal-panel.tsx` | Dialog semantics + focus trap. |
| `web/src/stores/portal.ts` | Collision-free `artifactKey`, artifact version history. |

---

## Phase 1 — Turn Control

The "Stop does nothing / spinner forever / truncated answer looks clean" cluster. Highest user impact.

### Task 1: Kernel turn registry + AgentId binding

**Files:**
- Create: `crates/oxios-kernel/src/turn_registry.rs`
- Modify: `crates/oxios-kernel/src/lib.rs` (module registration)
- Modify: `crates/oxios-kernel/src/kernel_handle/mod.rs:152` (field) and `:304-310` (builder)
- Modify: `crates/oxios-kernel/src/agent_lifecycle.rs:128`
- Test: inline `#[cfg(test)] mod tests` in `turn_registry.rs`

**Interfaces:**
- Produces: `oxios_kernel::turn_registry::{TurnRegistry, TurnToken}`; `KernelHandle.turns: Arc<TurnRegistry>`; `KernelHandle::with_turns(Arc<TurnRegistry>) -> Self`.
- `TurnRegistry::open(&self, key: &str) -> TurnToken`
- `TurnRegistry::bind_agent(&self, key: &str, agent_id: uuid::Uuid)`
- `TurnRegistry::cancel(&self, key: &str) -> Option<uuid::Uuid>` — marks cancelled, wakes waiters, returns the bound agent id if any
- `TurnRegistry::close(&self, key: &str)`
- `TurnToken::cancelled(&self) -> impl Future<Output = ()>`, `TurnToken::is_cancelled(&self) -> bool`
- Consumed by: Task 2 (gateway), Task 3 (chat.rs WS arm).

- [ ] **Step 1: Write the failing test**

Create `crates/oxios-kernel/src/turn_registry.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_returns_bound_agent_and_wakes_token() {
        let reg = TurnRegistry::new();
        let token = reg.open("sess-1");
        let agent = uuid::Uuid::new_v4();
        reg.bind_agent("sess-1", agent);

        assert!(!token.is_cancelled());
        assert_eq!(reg.cancel("sess-1"), Some(agent));
        assert!(token.is_cancelled());
        // The waiter resolves immediately once cancelled.
        tokio::time::timeout(std::time::Duration::from_millis(50), token.cancelled())
            .await
            .expect("cancelled() must resolve after cancel()");
    }

    #[tokio::test]
    async fn cancel_unknown_key_is_none_and_close_is_idempotent() {
        let reg = TurnRegistry::new();
        assert_eq!(reg.cancel("nope"), None);
        reg.open("sess-2");
        reg.close("sess-2");
        reg.close("sess-2");
        assert_eq!(reg.cancel("sess-2"), None);
    }

    #[tokio::test]
    async fn reopening_a_key_clears_the_previous_cancel_flag() {
        let reg = TurnRegistry::new();
        let first = reg.open("sess-3");
        reg.cancel("sess-3");
        assert!(first.is_cancelled());
        reg.close("sess-3");

        let second = reg.open("sess-3");
        assert!(!second.is_cancelled(), "a new turn must not inherit the old cancel");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p oxios-kernel turn_registry`
Expected: FAIL — `cannot find type TurnRegistry in this scope` (module not yet declared, types absent).

- [ ] **Step 3: Write the implementation**

Prepend to `crates/oxios-kernel/src/turn_registry.rs`:

```rust
//! Turn registry — makes an in-flight chat turn addressable for cancellation.
//!
//! Keyed by the SAME string the streaming sink uses: the chat session id, or
//! the request id for a session's first message (`gateway.rs` computes
//! `session_id.unwrap_or(request_id)`; the orchestrator mirrors it into
//! `ExecEnv.session_id`). Never introduce a second identity for a turn.
//!
//! Two halves must be reachable to actually stop a turn:
//!   * the gateway dispatch future, woken through [`TurnToken::cancelled`], and
//!   * the supervisor-spawned agent task, killed by the [`bind_agent`] binding.
//! Cancelling only the former leaves the provider request running.
//!
//! Deliberately dependency-free (`Notify` + `AtomicBool`): `oxios-kernel` does
//! not depend on `tokio-util`, and adding it for one token is not warranted.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tokio::sync::Notify;

/// Shared cancellation state for one turn.
#[derive(Debug)]
struct TurnEntry {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
    agent_id: Option<uuid::Uuid>,
}

/// Handle held by the dispatch task for the lifetime of one turn.
#[derive(Debug, Clone)]
pub struct TurnToken {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl TurnToken {
    /// Whether this turn has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Resolves as soon as the turn is cancelled. Safe to `select!` on: the
    /// flag is checked before awaiting, so a cancel that lands before the
    /// waiter is registered is not missed.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let waiter = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            waiter.await;
        }
    }
}

/// Registry of cancellable in-flight turns, keyed by turn key.
#[derive(Debug, Default)]
pub struct TurnRegistry {
    inner: Mutex<HashMap<String, TurnEntry>>,
}

impl TurnRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a turn. Replaces any stale entry for the same key so a new turn
    /// never inherits a previous turn's cancel flag.
    pub fn open(&self, key: &str) -> TurnToken {
        let cancelled = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        self.inner.lock().insert(
            key.to_string(),
            TurnEntry {
                cancelled: cancelled.clone(),
                notify: notify.clone(),
                agent_id: None,
            },
        );
        TurnToken { cancelled, notify }
    }

    /// Bind the forked agent to the turn so cancellation can kill it.
    /// No-op when the turn already ended.
    pub fn bind_agent(&self, key: &str, agent_id: uuid::Uuid) {
        if let Some(entry) = self.inner.lock().get_mut(key) {
            entry.agent_id = Some(agent_id);
        }
    }

    /// Cancel a turn. Returns the bound agent id so the caller can kill it
    /// (the registry deliberately does not own a supervisor reference).
    /// `None` when no turn is in flight for this key.
    pub fn cancel(&self, key: &str) -> Option<uuid::Uuid> {
        let guard = self.inner.lock();
        let entry = guard.get(key)?;
        entry.cancelled.store(true, Ordering::Release);
        entry.notify.notify_waiters();
        entry.agent_id
    }

    /// End a turn. Idempotent.
    pub fn close(&self, key: &str) {
        self.inner.lock().remove(key);
    }
}
```

Register the module in `crates/oxios-kernel/src/lib.rs` next to the existing `pub mod streaming_sink;` line:

```rust
pub mod turn_registry;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p oxios-kernel turn_registry`
Expected: PASS (3 tests).

- [ ] **Step 5: Expose on KernelHandle**

In `crates/oxios-kernel/src/kernel_handle/mod.rs`, directly below the `streaming_sinks` field (`:152`):

```rust
    /// RFC-049: in-flight turns, addressable for cancellation. Shares its key
    /// space with `streaming_sinks`.
    pub turns: Arc<crate::turn_registry::TurnRegistry>,
```

Add the builder below `with_streaming_sinks` (`:304-310`):

```rust
    /// Attach the shared turn registry (the gateway holds the same `Arc`).
    pub fn with_turns(mut self, registry: Arc<crate::turn_registry::TurnRegistry>) -> Self {
        self.turns = registry;
        self
    }
```

Add `turns: Arc::new(crate::turn_registry::TurnRegistry::new())` to every `KernelHandle` construction site the compiler flags.

- [ ] **Step 6: Bind the AgentId at fork**

In `crates/oxios-kernel/src/agent_lifecycle.rs`, replace lines 128-129:

```rust
        // 1. Fork
        let agent_id = self.supervisor.fork_directive(directive, env).await?;
        // RFC-049: bind the fork to its turn so a WS `cancel` can kill the
        // agent, not just drop the gateway future. `env.session_id` is the
        // gateway's turn key (orchestrator.rs sets it from ctx.session_id).
        if let Some(key) = env.session_id.as_deref() {
            self.turns.bind_agent(key, agent_id);
        }
```

`AgentLifecycleManager` needs the registry. Add the field and constructor parameter:

```rust
    /// RFC-049: shared turn registry, used to bind forks to their turn key.
    turns: Arc<crate::turn_registry::TurnRegistry>,
```

Thread it from the kernel assembler (`src/kernel.rs`) using the same `Arc` handed to `KernelHandle::with_turns` and `Gateway::with_turns`.

- [ ] **Step 7: Verify the crate builds and tests pass**

Run: `cargo clippy -p oxios-kernel --all-targets -- -D warnings && cargo test -p oxios-kernel`
Expected: clean, all tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/oxios-kernel/src/turn_registry.rs crates/oxios-kernel/src/lib.rs \
        crates/oxios-kernel/src/kernel_handle/mod.rs crates/oxios-kernel/src/agent_lifecycle.rs \
        src/kernel.rs
git commit -m "feat(kernel): add turn registry binding forks to their turn key"
```

---

### Task 2: Gateway honours cancellation

**Files:**
- Modify: `crates/oxios-gateway/src/gateway.rs` (struct field beside `streaming_sinks`, `dispatch`, the spawned task, sink register `:488-489`)
- Modify: `crates/oxios-gateway/src/message.rs` (`ErrorKind`)
- Test: `crates/oxios-gateway/tests/` (new integration test file `cancel_turn.rs`)

**Interfaces:**
- Consumes: `TurnRegistry`, `TurnToken` from Task 1.
- Produces: `Gateway::with_turns(Arc<TurnRegistry>) -> Self`; `ErrorKind::Cancelled`; a terminal `OutgoingMessage` whose `meta.error.kind == ErrorKind::Cancelled` when a turn is cancelled.

- [ ] **Step 1: Add the ErrorKind variant and snake_case serde**

In `crates/oxios-gateway/src/message.rs`, replace the `ErrorKind` enum:

```rust
/// Classified failure reason attached to a terminal response.
///
/// `snake_case` on the wire: the web client narrows on these exact strings
/// (`web/src/stores/chat.ts`). Renaming a variant is a wire-breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The agent ran but produced a failure.
    ExecutionFailed,
    /// No credential configured for the selected provider.
    ApiKeyMissing,
    /// Provider rejected or rate-limited the request.
    ProviderError,
    /// The turn exceeded its time budget.
    Timeout,
    /// A tool or path was denied by the access manager.
    PermissionDenied,
    /// Input failed validation before dispatch.
    ValidationError,
    /// The user cancelled the turn (RFC-049). Not a fault.
    Cancelled,
    /// Unclassified internal failure.
    Internal,
}
```

- [ ] **Step 2: Write the failing test**

Create `crates/oxios-gateway/tests/cancel_turn.rs`:

```rust
//! RFC-049: cancelling an in-flight turn must wake the dispatch task and
//! surface a terminal `Cancelled` error rather than letting the turn run on.

use oxios_gateway::message::ErrorKind;

#[test]
fn error_kind_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&ErrorKind::ProviderError).unwrap(),
        "\"provider_error\""
    );
    assert_eq!(
        serde_json::to_string(&ErrorKind::Cancelled).unwrap(),
        "\"cancelled\""
    );
    assert_eq!(
        serde_json::to_string(&ErrorKind::ApiKeyMissing).unwrap(),
        "\"api_key_missing\""
    );
}

#[tokio::test]
async fn cancelled_token_resolves_and_reports_agent() {
    let reg = oxios_kernel::turn_registry::TurnRegistry::new();
    let token = reg.open("sess-cancel");
    let agent = uuid::Uuid::new_v4();
    reg.bind_agent("sess-cancel", agent);

    let waiter = tokio::spawn(async move {
        token.cancelled().await;
        true
    });

    assert_eq!(reg.cancel("sess-cancel"), Some(agent));
    assert!(tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("must not hang")
        .unwrap());
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p oxios-gateway --test cancel_turn`
Expected: FAIL — `ErrorKind::Cancelled` unknown until Step 1 is applied, then the snake_case assertions fail against the PascalCase default.

- [ ] **Step 4: Wire the registry into the gateway**

Add the field next to `streaming_sinks` in the `Gateway` struct:

```rust
    /// RFC-049: shared with `KernelHandle.turns` — same `Arc`, same key space
    /// as `streaming_sinks`.
    turns: Arc<oxios_kernel::turn_registry::TurnRegistry>,
```

Add the builder beside `with_streaming_sinks`:

```rust
    /// Attach the shared turn registry.
    pub fn with_turns(
        mut self,
        registry: Arc<oxios_kernel::turn_registry::TurnRegistry>,
    ) -> Self {
        self.turns = registry;
        self
    }
```

In `dispatch`, immediately after `sink_session_key` is computed (`:488`) and before `streaming_sinks.register(...)` (`:489`), open the turn:

```rust
                let turn_token = turns.open(&sink_session_key);
```

Wrap the orchestrator call inside the spawned task so cancellation wins the race:

```rust
                let outcome = tokio::select! {
                    biased;
                    _ = turn_token.cancelled() => None,
                    res = orchestrator.handle_unified(
                        &user_id, &content, session_id.as_deref(), project_ids.as_deref(),
                        mount_ids.as_deref(), role.as_deref(), model_override.as_deref(),
                        model_params, &request_id,
                    ) => Some(res),
                };
```

Replace the existing result handling so `None` emits a terminal cancelled response:

```rust
                let Some(result) = outcome else {
                    streaming_sinks.unregister(&sink_session_key);
                    turns.close(&sink_session_key);
                    let mut cancelled = OutgoingMessage::new(&channel, &user_id, String::new());
                    cancelled.target_conn_id = conn_id.clone();
                    cancelled.meta = Some(ResponseMeta {
                        session_id: session_id.clone(),
                        phase: "cancelled".to_string(),
                        error: Some(UserFacingError {
                            message: "Turn cancelled by user".to_string(),
                            kind: ErrorKind::Cancelled,
                            suggestion: None,
                        }),
                        ..Default::default()
                    });
                    let _ = outgoing_tx.send(cancelled).await;
                    return;
                };
```

Close the turn on the normal path too — add `turns.close(&sink_session_key);` immediately after the existing `streaming_sinks.unregister(...)` call at the end of the task.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p oxios-gateway --test cancel_turn && cargo test -p oxios-gateway`
Expected: PASS.

- [ ] **Step 6: Fix the frontend union in the same wire change**

`ErrorKind` is now snake_case, so the client's narrowing finally matches. In `web/src/stores/chat.ts:1738-1741` replace the narrowing:

```ts
            const rawKind = (chunk as unknown as Record<string, unknown>).kind
            const KNOWN_ERROR_KINDS = [
              'execution_failed',
              'api_key_missing',
              'provider_error',
              'timeout',
              'permission_denied',
              'validation_error',
              'cancelled',
              'internal',
            ] as const
            type ErrorKindValue = (typeof KNOWN_ERROR_KINDS)[number] | 'unknown'
            const errKind: ErrorKindValue = KNOWN_ERROR_KINDS.includes(
              rawKind as (typeof KNOWN_ERROR_KINDS)[number],
            )
              ? (rawKind as ErrorKindValue)
              : 'unknown'
```

Export `KNOWN_ERROR_KINDS` and `ErrorKindValue` from `web/src/types/chat.ts` instead of defining them inline, so `ErrorCard` (Task 6) imports the same list.

- [ ] **Step 7: Run gates**

Run: `cargo clippy --workspace --all-features -- -D warnings && cargo test -p oxios-gateway && cd web && bun run typecheck`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/oxios-gateway/src/gateway.rs crates/oxios-gateway/src/message.rs \
        crates/oxios-gateway/tests/cancel_turn.rs web/src/stores/chat.ts web/src/types/chat.ts
git commit -m "feat(gateway): honour turn cancellation and align ErrorKind wire casing"
```

---

### Task 3: WS `cancel` frame end-to-end

**Files:**
- Modify: `src/api/routes/chat.rs` (recv-loop `match msg_type` at `:1160`)
- Modify: `web/src/stores/chat.ts` (new `cancelTurn` action)
- Modify: `web/src/routes/chat.tsx:270-273` (`handleCancel`)
- Test: `src/api/routes/chat.rs` inline test module; `web/src/__tests__/stores.test.ts`

**Interfaces:**
- Consumes: `TurnRegistry::cancel` (Task 1), `ErrorKind::Cancelled` (Task 2).
- Produces: client → server frame `{ "type": "cancel", "session_id": string | null }`; store action `cancelTurn(): void`.

- [ ] **Step 1: Write the failing frontend test**

Append to `web/src/__tests__/stores.test.ts`:

```ts
describe('cancelTurn', () => {
  it('sends a cancel frame and does not tear down the socket', () => {
    const sent: string[] = []
    const fakeWs = {
      readyState: 1,
      send: (d: string) => sent.push(d),
      close: () => {
        throw new Error('cancelTurn must not close the socket')
      },
    }
    useChatStore.setState({
      _ws: fakeWs as unknown as WebSocket,
      connected: true,
      isStreaming: true,
      activeSessionId: 'sess-9',
    })

    useChatStore.getState().cancelTurn()

    expect(JSON.parse(sent[0]!)).toEqual({ type: 'cancel', session_id: 'sess-9' })
  })

  it('is a no-op when no turn is in flight', () => {
    const sent: string[] = []
    useChatStore.setState({
      _ws: { readyState: 1, send: (d: string) => sent.push(d) } as unknown as WebSocket,
      connected: true,
      isStreaming: false,
    })
    useChatStore.getState().cancelTurn()
    expect(sent).toHaveLength(0)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/__tests__/stores.test.ts -t cancelTurn`
Expected: FAIL — `cancelTurn is not a function`.

- [ ] **Step 3: Implement the store action**

Add to the store's action block in `web/src/stores/chat.ts` (next to `disconnect`):

```ts
      /** RFC-049: ask the server to abort the in-flight turn. Unlike the old
       *  disconnect+reconnect, this actually stops the agent — a reconnect
       *  would replay the terminal message and the "cancelled" answer would
       *  reappear while the provider kept billing. */
      cancelTurn() {
        const { _ws, isStreaming, activeSessionId } = get()
        if (!isStreaming) return
        if (!_ws || _ws.readyState !== WebSocket.OPEN) return
        _ws.send(JSON.stringify({ type: 'cancel', session_id: activeSessionId ?? null }))
      },
```

Declare `cancelTurn: () => void` in the store's TypeScript interface.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web && bun run vitest run src/__tests__/stores.test.ts -t cancelTurn`
Expected: PASS (2 tests).

- [ ] **Step 5: Replace the cosmetic cancel in the route**

In `web/src/routes/chat.tsx` replace `handleCancel` (`:270-273`):

```tsx
  // RFC-049: real cancellation. The previous implementation dropped the
  // socket and reconnected, which left the backend turn running and then
  // replayed its terminal message — the "cancelled" answer came back.
  const handleCancel = () => {
    useChatStore.getState().cancelTurn()
  }
```

- [ ] **Step 6: Write the failing backend test**

Add to the inline test module in `src/api/routes/chat.rs`:

```rust
    #[tokio::test]
    async fn cancel_frame_marks_turn_and_returns_agent_id() {
        let turns = oxios_kernel::turn_registry::TurnRegistry::new();
        let token = turns.open("sess-cancel");
        let agent = uuid::Uuid::new_v4();
        turns.bind_agent("sess-cancel", agent);

        let parsed: serde_json::Value =
            serde_json::from_str(r#"{"type":"cancel","session_id":"sess-cancel"}"#).unwrap();
        let key = parsed
            .get("session_id")
            .and_then(|v| v.as_str())
            .expect("cancel frame carries the turn key");

        assert_eq!(turns.cancel(key), Some(agent));
        assert!(token.is_cancelled());
    }
```

- [ ] **Step 7: Run test to verify it fails**

Run: `cargo test -p oxios cancel_frame_marks_turn`
Expected: FAIL — module path unresolved until Task 1 lands (it has; then this passes trivially and the real work is the arm below).

- [ ] **Step 8: Add the recv-loop arm**

In `src/api/routes/chat.rs`, inside `match msg_type` (`:1160`), before the default `_` arm:

```rust
                            // RFC-049: user pressed Stop. Cancel by turn key
                            // (session id, falling back to the connection's
                            // active session) and kill the bound agent so the
                            // provider request actually stops.
                            "cancel" => {
                                let key = parsed
                                    .get("session_id")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string)
                                    .or_else(|| active_session_id_for_recv.lock().clone());
                                let Some(key) = key else { continue };
                                if let Some(agent_id) = state_for_recv.kernel.turns.cancel(&key) {
                                    if let Err(e) = state_for_recv
                                        .kernel
                                        .agents
                                        .kill(&agent_id.to_string())
                                        .await
                                    {
                                        tracing::warn!(
                                            agent_id = %agent_id,
                                            error = %e,
                                            "cancel: agent kill failed"
                                        );
                                    }
                                }
                            }
```

If `active_session_id_for_recv` does not exist in the recv task's scope, clone the existing `active_session_id` shared handle that the forwarder task already maintains for event filtering, and move the clone into the recv task before the loop.

- [ ] **Step 9: Run the full check**

Run: `cargo clippy --workspace --all-features -- -D warnings && cargo test -p oxios && cd web && bun run vitest run && bun run typecheck`
Expected: clean, all pass.

- [ ] **Step 10: Commit**

```bash
git add src/api/routes/chat.rs web/src/stores/chat.ts web/src/routes/chat.tsx \
        web/src/__tests__/stores.test.ts
git commit -m "feat(web): make the Stop button actually cancel the backend turn"
```

---

### Task 4: Cancelled turns render as interrupted, not as errors

**Files:**
- Modify: `web/src/stores/chat.ts` (`case 'error'` at `:1720-1773`)
- Modify: `web/src/components/chat/messages/AssistantMessage.tsx`
- Create: `web/src/components/chat/interrupted-notice.tsx`
- Modify: `web/src/i18n/locales/en.json`, `web/src/i18n/locales/ko.json`
- Test: `web/src/__tests__/stores.test.ts`

**Interfaces:**
- Consumes: `errKind === 'cancelled'` (Task 2 Step 6).
- Produces: `ChatMessage.metadata.cancelled?: boolean`; `<InterruptedNotice />`.

- [ ] **Step 1: Write the failing test**

```ts
it('a cancelled terminal keeps the partial answer and marks it interrupted', () => {
  useChatStore.setState({
    messages: [
      { id: 'u1', role: 'user', content: 'hi' },
      { id: 'a1', role: 'assistant', content: 'partial ans', generating: true },
    ] as never,
    isStreaming: true,
  })

  useChatStore.getState().handleChunk({
    type: 'error',
    kind: 'cancelled',
    message: 'Turn cancelled by user',
  } as never)

  const msgs = useChatStore.getState().messages
  expect(msgs).toHaveLength(2)
  expect(msgs[1]!.content).toBe('partial ans')
  expect(msgs[1]!.generating).toBe(false)
  expect(msgs[1]!.metadata?.cancelled).toBe(true)
  expect(msgs[1]!.metadata?.isError).toBeFalsy()
  expect(useChatStore.getState().isStreaming).toBe(false)
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/__tests__/stores.test.ts -t interrupted`
Expected: FAIL — a third message (the red error card) is appended and `metadata.cancelled` is undefined.

- [ ] **Step 3: Branch the error arm**

At the top of `case 'error':` in `web/src/stores/chat.ts`, before the existing error-message construction:

```ts
            // A user cancel is a terminal, but not a fault: keep whatever the
            // agent already produced and flag it interrupted. Appending a red
            // ErrorCard here would both lose the partial answer's framing and
            // misreport a deliberate user action as a failure.
            if (rawKind === 'cancelled') {
              flushPendingTokens()
              set((s) => {
                const finalized = finalizeStreamingMessage(s.messages)
                const last = finalized.at(-1)
                const messages =
                  last && last.role === 'assistant'
                    ? finalized.map((m, i) =>
                        i === finalized.length - 1
                          ? { ...m, metadata: { ...m.metadata, cancelled: true } }
                          : m,
                      )
                    : finalized
                return {
                  messages,
                  isStreaming: false,
                  pendingModel: null,
                  activeToolApproval: null,
                  activePathAccess: null,
                }
              })
              get()._drainPendingQueue()
              break
            }
```

Move the `rawKind` / `errKind` computation above this block so it is in scope.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web && bun run vitest run src/__tests__/stores.test.ts -t interrupted`
Expected: PASS.

- [ ] **Step 5: Render the notice**

Create `web/src/components/chat/interrupted-notice.tsx`:

```tsx
// Muted "the user stopped this turn" footer. Deliberately NOT the destructive
// ErrorCard styling: cancelling is a normal action, not a fault.
import { CircleSlash } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export function InterruptedNotice() {
  const { t } = useTranslation()
  return (
    <div className="mt-1 flex items-center gap-1.5 text-2xs text-muted-foreground" role="status">
      <CircleSlash className="h-3 w-3 shrink-0" />
      {t('chat.interrupted')}
    </div>
  )
}
```

In `web/src/components/chat/messages/AssistantMessage.tsx`, beside the existing `{isError && chatError && <ErrorCard .../>}` line (`:92`):

```tsx
        {message.metadata?.cancelled && <InterruptedNotice />}
```

Add to `web/src/i18n/locales/en.json` under `chat`: `"interrupted": "Stopped by you"`.
Add to `web/src/i18n/locales/ko.json` under `chat`: `"interrupted": "사용자가 중단함"`.

- [ ] **Step 6: Run gates**

Run: `cd web && bun run typecheck && bun run vitest run && bunx biome check src/components/chat/interrupted-notice.tsx src/stores/chat.ts`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add web/src/components/chat/interrupted-notice.tsx web/src/stores/chat.ts \
        web/src/components/chat/messages/AssistantMessage.tsx web/src/i18n/locales/*.json \
        web/src/__tests__/stores.test.ts
git commit -m "feat(web): render cancelled turns as interrupted instead of errors"
```

---

### Task 5: Turn watchdog

**Files:**
- Modify: `web/src/stores/chat.ts` (`sendMessage`, terminal arms, `disconnect`)
- Modify: `web/src/i18n/locales/en.json`, `ko.json`
- Test: `web/src/__tests__/stores.test.ts`

**Interfaces:**
- Consumes: `cancelTurn` (Task 3).
- Produces: module constant `TURN_WATCHDOG_MS = 180_000`; internal `_armWatchdog()` / `_clearWatchdog()`.

- [ ] **Step 1: Write the failing test**

```ts
it('a hung turn is finalized by the watchdog instead of spinning forever', () => {
  vi.useFakeTimers()
  const sent: string[] = []
  useChatStore.setState({
    _ws: { readyState: 1, send: (d: string) => sent.push(d) } as unknown as WebSocket,
    connected: true,
    activeSessionId: 'sess-hang',
    messages: [],
  })

  useChatStore.getState().sendMessage('hello')
  expect(useChatStore.getState().isStreaming).toBe(true)

  vi.advanceTimersByTime(TURN_WATCHDOG_MS + 10)

  expect(useChatStore.getState().isStreaming).toBe(false)
  const last = useChatStore.getState().messages.at(-1)!
  expect(last.metadata?.isError).toBe(true)
  expect(last.metadata?.errorKind).toBe('timeout')
  // The server must be told too, or it keeps burning provider tokens.
  expect(sent.some((s) => JSON.parse(s).type === 'cancel')).toBe(true)
  vi.useRealTimers()
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/__tests__/stores.test.ts -t watchdog`
Expected: FAIL — `TURN_WATCHDOG_MS` is not exported; `isStreaming` stays true.

- [ ] **Step 3: Implement**

Module scope in `web/src/stores/chat.ts`, near `_pendingTokens`:

```ts
/** Client-side ceiling for one turn. The POST path has a 120 s
 *  `response_timeout` in `src/api/bridge.rs`; the WS path had none, so a
 *  provider that hangs with the socket still OPEN left the composer disabled
 *  and the spinner running forever. */
export const TURN_WATCHDOG_MS = 180_000
let _watchdogId: number | null = null

function clearWatchdog(): void {
  if (_watchdogId !== null) {
    window.clearTimeout(_watchdogId)
    _watchdogId = null
  }
}

function armWatchdog(): void {
  clearWatchdog()
  _watchdogId = window.setTimeout(() => {
    _watchdogId = null
    const s = useChatStore.getState()
    if (!s.isStreaming) return
    // Tell the server first so the provider request stops.
    s.cancelTurn()
    flushPendingTokens()
    useChatStore.setState((st) => {
      const finalized = finalizeStreamingMessage(st.messages)
      const timeoutMsg: ChatMessage = {
        id: uuid(),
        role: 'assistant',
        content: '',
        timestamp: new Date().toISOString(),
        metadata: { isError: true, errorKind: 'timeout' },
      }
      return {
        messages: [...finalized, timeoutMsg],
        isStreaming: false,
        pendingModel: null,
      }
    })
    useChatStore.getState()._drainPendingQueue()
  }, TURN_WATCHDOG_MS)
}
```

Call `armWatchdog()` in `sendMessage` right after `set({ isStreaming: true, streamStartedAt: Date.now() })` (`:1039-1042`).
Call `clearWatchdog()` at the top of the `done`, `error`, `interview`, `tool_approval`, and `path_access` arms, and in `disconnect()` and `newSession()`.

Add `"timeout"` copy to `ErrorCard`'s kind map in Task 6, and the i18n keys:
`en.json` → `"chat.error.timeout.title": "No response"`, `"chat.error.timeout.hint": "The model stopped responding. The turn was cancelled — try again or switch models."`
`ko.json` → `"chat.error.timeout.title": "응답 없음"`, `"chat.error.timeout.hint": "모델이 응답을 멈췄습니다. 턴을 취소했습니다 — 다시 시도하거나 모델을 바꿔보세요."`

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web && bun run vitest run src/__tests__/stores.test.ts -t watchdog`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/stores/chat.ts web/src/i18n/locales/*.json web/src/__tests__/stores.test.ts
git commit -m "feat(web): add a turn watchdog so a hung provider cannot spin forever"
```

---

### Task 6: Loss-free socket close + kind-aware error copy

**Files:**
- Modify: `web/src/stores/chat.ts` (`ws.onclose` at `:904-940`)
- Modify: `web/src/components/chat/messages/components/ErrorCard.tsx`
- Modify: `web/src/i18n/locales/en.json`, `ko.json`
- Test: `web/src/__tests__/stores.test.ts`, `web/src/__tests__/components/chat/error-card.test.tsx` (new)

**Interfaces:**
- Consumes: `KNOWN_ERROR_KINDS` from `web/src/types/chat.ts` (Task 2 Step 6).
- Produces: `ChatMessage.metadata.interrupted?: boolean` set on abrupt close.

- [ ] **Step 1: Write the failing tests**

```ts
it('flushes buffered tokens and flags the turn interrupted when the socket drops', () => {
  useChatStore.setState({
    messages: [{ id: 'a1', role: 'assistant', content: 'so f', generating: true }] as never,
    isStreaming: true,
  })
  // Buffer a delta that has not hit its rAF flush yet.
  useChatStore.getState().handleChunk({ type: 'token', content: 'ar' } as never)

  triggerSocketClose() // helper in the test file: invokes the stored ws.onclose

  const last = useChatStore.getState().messages.at(-1)!
  expect(last.content).toBe('so far')
  expect(last.generating).toBe(false)
  expect(last.metadata?.interrupted).toBe(true)
})
```

```tsx
it('renders provider_error copy for the backend kind', () => {
  render(<ErrorCard error={{ type: 'stream_error', category: 'provider_error' }} />)
  expect(screen.getByText('Provider error')).toBeTruthy()
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd web && bun run vitest run src/__tests__/stores.test.ts -t interrupted && bun run vitest run src/__tests__/components/chat/error-card.test.tsx`
Expected: FAIL — buffered `ar` is lost; `Something went wrong` renders instead of the provider copy.

- [ ] **Step 3: Fix onclose**

In `web/src/stores/chat.ts`, inside `ws.onclose` immediately after the stale-socket guard and `stopPingTimer()`:

```ts
          // The rAF token buffer is module-scoped and is only drained by a
          // scheduled frame or an explicit flush. An abrupt close delivers no
          // done/error chunk, so without this the tail of the answer is
          // silently dropped and the truncated message looks complete.
          flushPendingTokens()
```

Then replace the `set` with one that flags interruption:

```ts
          set((s) => {
            const wasStreaming = s.isStreaming
            const finalized = finalizeStreamingMessage(s.messages)
            const last = finalized.at(-1)
            const messages =
              wasStreaming && last && last.role === 'assistant'
                ? finalized.map((m, i) =>
                    i === finalized.length - 1
                      ? { ...m, metadata: { ...m.metadata, interrupted: true } }
                      : m,
                  )
                : finalized
            return {
              connected: false,
              isStreaming: false,
              _ws: null,
              _pendingQueue: [],
              messages,
            }
          })
```

Render `InterruptedNotice` (Task 4) for `metadata.interrupted` too — extend the condition in `AssistantMessage.tsx` to `message.metadata?.cancelled || message.metadata?.interrupted`, and give the notice an optional `reason: 'cancelled' | 'interrupted'` prop selecting between `chat.interrupted` and the new key `chat.connectionLost` (`en`: "Connection lost — response may be incomplete", `ko`: "연결이 끊겼습니다 — 응답이 잘렸을 수 있습니다").

- [ ] **Step 4: Rewrite ErrorCard's kind map**

Replace `KIND_COPY` in `web/src/components/chat/messages/components/ErrorCard.tsx` with an i18n-backed lookup over the real backend kinds:

```tsx
const KIND_KEYS: Record<string, string> = {
  execution_failed: 'executionFailed',
  api_key_missing: 'apiKeyMissing',
  provider_error: 'providerError',
  timeout: 'timeout',
  permission_denied: 'permissionDenied',
  validation_error: 'validationError',
  internal: 'internal',
}

export function ErrorCard({ error, onRetry, className }: ErrorCardProps) {
  const { t } = useTranslation()
  const rawKind = (error.category ?? error.type ?? 'unknown') as string
  const key = KIND_KEYS[rawKind] ?? 'unknown'
  const title = t(`chat.error.${key}.title`)
  const hint = t(`chat.error.${key}.hint`, { defaultValue: '' })
  // …existing markup, with {title}, {hint || null}, and t('chat.retry') on the button
```

Add all 8 title/hint pairs to both locale files. English titles: "Execution failed", "API key missing", "Provider error", "No response", "Permission denied", "Invalid request", "Internal error", "Something went wrong". Korean: "실행 실패", "API 키 없음", "프로바이더 오류", "응답 없음", "권한 거부", "잘못된 요청", "내부 오류", "문제가 발생했습니다".

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd web && bun run vitest run`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add web/src/stores/chat.ts web/src/components/chat/messages/components/ErrorCard.tsx \
        web/src/components/chat/interrupted-notice.tsx web/src/components/chat/messages/AssistantMessage.tsx \
        web/src/i18n/locales/*.json web/src/__tests__
git commit -m "fix(web): never drop buffered tokens on socket close; kind-aware error copy"
```

---

## Phase 2 — Honest Contracts

### Task 7: Delete the dead `phase` streaming path

**Files:**
- Modify: `web/src/lib/stream/adapter.ts:202-214`
- Modify: `web/src/lib/stream/StreamProcessor.ts` (`case 'phase'`)
- Modify: `web/src/lib/stream/ChatEvent.ts` (`phase` variant)
- Modify: `web/src/stores/chat.ts` (`KNOWN_CHUNK_TYPES`)
- Modify: `web/src/types/index.ts` (`StreamChunk['type']` union)
- Test: `web/src/__tests__/lib/` (adapter test)

**Interfaces:**
- Produces: nothing new. Removes `ChatEvent` variant `{ kind: 'phase' }`.
- Rationale: see **D3**. `phase` is only ever `"execute"` and is only ever delivered as a field on `done`, which `chat-metadata.tsx` already renders.

- [ ] **Step 1: Write the failing test**

```ts
it('has no phase streaming path — phase is done-metadata only', () => {
  // A standalone phase frame is not part of the contract; the adapter must
  // treat it as unknown rather than manufacturing an event for it.
  expect(adaptChunk({ type: 'phase', phase: 'execute' } as never, { msgId: 'm' }).events).toEqual([])
  expect(KNOWN_CHUNK_TYPES).not.toContain('phase')
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run -t 'phase streaming path'`
Expected: FAIL — the adapter emits a `phase` event and `KNOWN_CHUNK_TYPES` contains `'phase'`.

- [ ] **Step 3: Remove the dead arms**

Delete `case 'phase':` and its body from `adapter.ts` (`:202-214`) and from `StreamProcessor.ts`. Delete the `{ kind: 'phase'; … }` variant from `ChatEvent.ts`. Remove `'phase'` from `KNOWN_CHUNK_TYPES` in `chat.ts` and from the `StreamChunk['type']` union in `types/index.ts`. Keep `phase` and `evaluation_passed` as optional fields on `StreamChunk` (they still arrive on `done`).

Add a comment where the union used to list it:

```ts
  // No standalone 'phase' frame exists: post-RFC-027 the Ouroboros phase is a
  // plain string that is only ever "execute" (orchestrator.rs:602), delivered
  // as a field on `done`. Do not re-add a streaming phase event.
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web && bun run vitest run && bun run typecheck`
Expected: PASS, clean.

- [ ] **Step 5: Commit**

```bash
git add web/src/lib/stream web/src/stores/chat.ts web/src/types/index.ts web/src/__tests__
git commit -m "refactor(web): drop the never-emitted phase streaming path"
```

---

### Task 8: `TurnTextStreamTracker` resets on turn start

**Files:**
- Modify: `src/api/routes/chat.rs:535-560` (the type), `:678` (the `turn_text` binding), and the `model` chunk branch
- Test: inline test module in `src/api/routes/chat.rs` (existing tracker tests at `:2336-2368`)

**Interfaces:**
- Produces: `TurnTextStreamTracker::begin_turn(&mut self)`. The tracker is a plain `struct { text_delivered: bool }` behind a `let mut turn_text` local — **not** atomic, so the new method takes `&mut self` like the existing `note_text_partial`/`reset`.
- Existing methods (do not rename): `note_text_partial(&mut self, content: &str)`, `terminal_token_redundant(&self) -> bool`, `reset(&mut self)`.

- [ ] **Step 1: Write the failing test**

Add beside the existing `tracker_resets_at_terminal_boundary` test (`:2364`):

```rust
    #[test]
    fn tracker_resets_on_new_turn_not_only_on_terminal() {
        let mut t = TurnTextStreamTracker::default();
        t.note_text_partial("answer");
        assert!(t.terminal_token_redundant());
        // A new turn begins (model chunk) without a preceding done/error —
        // the next turn's terminal text must NOT be suppressed.
        t.begin_turn();
        assert!(!t.terminal_token_redundant());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p oxios tracker_resets_on_new_turn`
Expected: FAIL — `no method named begin_turn found for struct TurnTextStreamTracker`.

- [ ] **Step 3: Implement**

Add to `impl TurnTextStreamTracker` (after `reset`, `:559`):

```rust
    /// Reset at the start of a turn. The terminal `done`/`error` reset is the
    /// normal path; this guards the case where a turn ends without one (an
    /// abrupt disconnect, or the RFC-049 cancel path), which would otherwise
    /// suppress the NEXT turn's terminal text on the same connection.
    fn begin_turn(&mut self) {
        self.text_delivered = false;
    }
```

Call `turn_text.begin_turn();` in the `model` chunk branch, which is the first frame of every turn. The binding is `let mut turn_text = TurnTextStreamTracker::default();` at `:678`, inside the forwarder loop's enclosing scope, so it is already mutable and in scope.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p oxios tracker_resets_on_new_turn`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/api/routes/chat.rs
git commit -m "fix(web): reset the turn text tracker on turn start"
```

---

### Task 9: Remove the legacy activity renderer

**Files:**
- Delete: `web/src/components/chat/activity-card.tsx`
- Modify: `web/src/lib/live-activity.ts` (drop `deriveCurrentActivity` if unreferenced after the delete)
- Modify: any importer the compiler flags
- Test: `web/src/__tests__/live-activity.test.ts` (prune only the cases covering deleted exports)

**Interfaces:**
- Removes: `ActivityCard`, `deriveCurrentActivity` (verify with `lsp references` before deleting — if either has a live consumer outside chat, keep it and note the consumer instead).

- [ ] **Step 1: Prove it is dead**

Run `lsp references` on `ActivityCard` and on `deriveCurrentActivity`.
Expected: references only from the component's own file and its tests. If a live consumer exists, STOP and skip this task, recording the consumer in the plan.

- [ ] **Step 2: Delete and prune**

Delete `web/src/components/chat/activity-card.tsx`. Remove the now-unreferenced export from `web/src/lib/live-activity.ts` and the corresponding test cases.

- [ ] **Step 3: Verify**

Run: `cd web && bun run typecheck && bun run vitest run`
Expected: clean, all pass.

- [ ] **Step 4: Commit**

```bash
git rm web/src/components/chat/activity-card.tsx
git add web/src/lib/live-activity.ts web/src/__tests__/live-activity.test.ts
git commit -m "refactor(web): remove the pre-block-stream activity renderer"
```

---

## Phase 3 — Session Loading

### Task 10: Loading skeleton and surfaced load failures

**Files:**
- Create: `web/src/components/chat/session-skeleton.tsx`
- Modify: `web/src/stores/chat.ts:1076-1220` (`loadSession`)
- Modify: `web/src/lib/chat-rows.ts` (new `skeleton` row kind)
- Modify: `web/src/routes/chat.tsx` (render skeleton / load-error banner)
- Modify: `web/src/i18n/locales/en.json`, `ko.json`
- Test: `web/src/__tests__/stores.test.ts`, `web/src/__tests__/lib/chat-rows.test.ts`

**Interfaces:**
- Produces: store fields `isLoadingSession: boolean`, `sessionLoadError: string | null`; action `retryLoadSession(): void`; row kind `{ kind: 'skeleton' }`.

- [ ] **Step 1: Write the failing test**

```ts
it('exposes loading state and surfaces a load failure instead of swallowing it', async () => {
  const fetchMock = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('offline'))
  const p = useChatStore.getState().loadSession('sess-x')
  expect(useChatStore.getState().isLoadingSession).toBe(true)
  await p
  expect(useChatStore.getState().isLoadingSession).toBe(false)
  expect(useChatStore.getState().sessionLoadError).toBeTruthy()
  fetchMock.mockRestore()
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run -t 'load failure'`
Expected: FAIL — neither field exists; the catch at `:1217-1219` swallows silently.

- [ ] **Step 3: Implement store state**

Add `isLoadingSession: false` and `sessionLoadError: null as string | null` to the initial state. In `loadSession`:

```ts
      async loadSession(sessionId: string) {
        set({ isLoadingSession: true, sessionLoadError: null, _lastLoadedSessionId: sessionId })
        try {
          // …existing body unchanged…
        } catch (e) {
          // Was: silently swallowed. A failed history fetch left a permanently
          // blank pane with no way to retry.
          set({ sessionLoadError: e instanceof Error ? e.message : 'load failed' })
        } finally {
          set({ isLoadingSession: false })
        }
      },
      retryLoadSession() {
        const sid = get()._lastLoadedSessionId
        if (sid) void get().loadSession(sid)
      },
```

Also treat a non-OK response as an error: after the fetch, `if (!res.ok) throw new Error(\`HTTP ${res.status}\`)`.

- [ ] **Step 4: Add the skeleton row**

Create `web/src/components/chat/session-skeleton.tsx`:

```tsx
// Shimmer placeholder while a session's history is fetched. Three rows in the
// alternating user/assistant rhythm so the layout does not jump on arrival.
export function SessionSkeleton() {
  return (
    <div className="mx-auto max-w-3xl space-y-4 px-4 py-6" aria-hidden="true">
      {[0, 1, 2].map((i) => (
        <div key={i} className={i % 2 === 0 ? 'flex justify-end' : ''}>
          <div className="w-2/3 space-y-2">
            <div className="h-3 w-1/3 animate-pulse rounded bg-muted" />
            <div className="h-3 w-full animate-pulse rounded bg-muted" />
            <div className="h-3 w-4/5 animate-pulse rounded bg-muted" />
          </div>
        </div>
      ))}
    </div>
  )
}
```

In `buildChatRows`, return `[{ kind: 'skeleton' }]` when a new `isLoadingSession` option is true and `messages` is empty. Render it in `chat.tsx`'s row switch.

- [ ] **Step 5: Add the error banner**

In `chat.tsx`, beside the reconnect banner (`:296-313`):

```tsx
        {sessionLoadError && (
          <div className="flex items-center gap-2 border-b bg-destructive/10 px-4 py-2 text-xs text-destructive">
            <span className="flex-1">{t('chat.sessionLoadFailed')}</span>
            <Button size="sm" variant="ghost" className="h-6 px-2" onClick={retryLoadSession}>
              <RefreshCw className="mr-1 h-3 w-3" />
              {t('chat.retry')}
            </Button>
          </div>
        )}
```

`en.json`: `"chat.sessionLoadFailed": "Could not load this conversation"`. `ko.json`: `"chat.sessionLoadFailed": "대화를 불러오지 못했습니다"`.

- [ ] **Step 6: Run tests and commit**

Run: `cd web && bun run typecheck && bun run vitest run`

```bash
git add web/src/components/chat/session-skeleton.tsx web/src/stores/chat.ts \
        web/src/lib/chat-rows.ts web/src/routes/chat.tsx web/src/i18n/locales/*.json web/src/__tests__
git commit -m "feat(web): session loading skeleton and surfaced load failures"
```

---

## Phase 4 — Rendering Quality

### Task 11: Streaming markdown healing

**Files:**
- Create: `web/src/lib/markdown/heal-streaming.ts`
- Modify: `web/src/components/chat/markdown-message.tsx` (apply when `isStreaming`)
- Test: `web/src/__tests__/lib/heal-streaming.test.ts`

**Interfaces:**
- Produces: `healStreamingMarkdown(src: string): string` — pure, idempotent on complete input.
- Scope: GFM tables missing their delimiter row, and unclosed inline emphasis/code/link. Unterminated fences are already handled correctly by CommonMark (they render as a code block to EOF) — do **not** touch them, the artifact card depends on that behaviour.

- [ ] **Step 1: Write the failing test**

```ts
import { healStreamingMarkdown } from '@/lib/markdown/heal-streaming'

describe('healStreamingMarkdown', () => {
  it('is identity for complete markdown', () => {
    const src = '| a | b |\n| --- | --- |\n| 1 | 2 |\n'
    expect(healStreamingMarkdown(src)).toBe(src)
  })

  it('adds the delimiter row to a header-only table', () => {
    expect(healStreamingMarkdown('| a | b |')).toBe('| a | b |\n| --- | --- |')
  })

  it('closes trailing inline emphasis', () => {
    expect(healStreamingMarkdown('a **bold')).toBe('a **bold**')
    expect(healStreamingMarkdown('a `code')).toBe('a `code`')
  })

  it('leaves an unterminated fence alone', () => {
    const src = '```rust\nfn main() {}'
    expect(healStreamingMarkdown(src)).toBe(src)
  })

  it('does not heal inside a fenced block', () => {
    const src = '```\n| a | b |\n**x'
    expect(healStreamingMarkdown(src)).toBe(src)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/__tests__/lib/heal-streaming.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

```ts
// Streaming markdown healing.
//
// A partially streamed buffer is fed to the parser every frame. CommonMark
// already handles an unterminated fence gracefully (code block to EOF, which
// is exactly what the artifact card needs), but a GFM table without its
// delimiter row renders as raw pipes, and an unclosed `**`/`` ` `` renders
// literally — both flicker distractingly until the closing token arrives.
//
// Idempotent: complete input is returned unchanged.

/** Whether the buffer ends inside an unterminated fenced block. */
function inOpenFence(lines: string[]): boolean {
  let open = false
  for (const line of lines) {
    if (/^\s{0,3}(```|~~~)/.test(line)) open = !open
  }
  return open
}

const DELIMITER_ROW = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/

export function healStreamingMarkdown(src: string): string {
  const lines = src.split('\n')
  if (inOpenFence(lines)) return src

  let out = src

  // 1. Header-only GFM table → synthesize the delimiter row.
  const lastIdx = lines.length - 1
  const last = lines[lastIdx] ?? ''
  const prev = lines[lastIdx - 1] ?? ''
  const isHeaderRow = (l: string) => l.trim().startsWith('|') && l.trim().endsWith('|')
  if (isHeaderRow(last) && !DELIMITER_ROW.test(prev)) {
    const cells = last.trim().slice(1, -1).split('|').length
    out = `${out}\n| ${Array(cells).fill('---').join(' | ')} |`
  }

  // 2. Unclosed inline tokens on the final line, longest marker first so
  //    `**` is not mistaken for two `*`.
  const tail = out.split('\n').at(-1) ?? ''
  for (const marker of ['**', '`', '*', '_']) {
    const count = tail.split(marker).length - 1
    if (count % 2 === 1) out += marker
  }

  return out
}
```

- [ ] **Step 4: Apply it**

In `web/src/components/chat/markdown-message.tsx`, change the child expression:

```tsx
          {preprocessArtifacts(isStreaming ? healStreamingMarkdown(children) : children)}
```

- [ ] **Step 5: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/lib/markdown/heal-streaming.ts web/src/components/chat/markdown-message.tsx \
        web/src/__tests__/lib/heal-streaming.test.ts
git commit -m "feat(web): heal partial tables and inline markers while streaming"
```

---

### Task 12: Theme-aware syntax highlighting

**Files:**
- Modify: `web/src/index.css:3`
- Test: `web/src/__tests__/components/chat/markdown-code-block.test.tsx`

**Interfaces:**
- Produces: highlight token colors driven by the `.dark` class rather than a hardcoded dark theme.

- [ ] **Step 1: Replace the unconditional import**

```css
@import 'highlight.js/styles/github.css';
@import 'highlight.js/styles/github-dark.css' layer(hljs-dark);
```

Then scope the dark layer under the existing single dark trigger. Immediately after the `@custom-variant dark` line, add:

```css
/* highlight.js ships one theme per stylesheet. Load the light theme normally
   and re-apply the dark theme only under `.dark`, so code blocks follow the
   app theme instead of being permanently dark (DESIGN.md: one `.dark`
   trigger, no per-component dark: variants). */
@layer hljs-dark {
  :root:not(.dark) .hljs,
  :root:not(.dark) .hljs * {
    color: revert-layer;
    background: revert-layer;
  }
}
```

If `revert-layer` proves unreliable in the target browser, use the fallback form instead: import only `github.css` globally and add `.dark .hljs { … }` overrides generated from `github-dark.css`, committed as `web/src/styles/hljs-dark.css`.

- [ ] **Step 2: Add the assertion**

```tsx
it('emits highlight classes that the theme layer can style', () => {
  const { container } = render(<MarkdownMessage>{'```rust\nfn main() {}\n```'}</MarkdownMessage>)
  expect(container.querySelector('.hljs-keyword')).toBeTruthy()
})
```

- [ ] **Step 3: Verify visually**

Run the dev server and toggle the theme; a `rust` block must show light-on-white in light mode and the github-dark palette in dark mode.

- [ ] **Step 4: Commit**

```bash
git add web/src/index.css web/src/__tests__/components/chat/markdown-code-block.test.tsx
git commit -m "fix(web): follow the app theme for code syntax highlighting"
```

---

### Task 13: Code block ergonomics

**Files:**
- Modify: `web/src/components/chat/markdown-message.tsx` (`CodeBlock`)
- Modify: `web/src/i18n/locales/en.json`, `ko.json`
- Test: `web/src/__tests__/components/chat/markdown-code-block.test.tsx`

**Interfaces:**
- Produces: collapse-on-overflow behaviour above 24 rendered lines; keyboard-reachable copy button; i18n'd copy/expand labels.
- Non-goal: line numbers (see **D4**).

- [ ] **Step 1: Write the failing test**

```tsx
it('collapses a very long block behind an expand control', () => {
  const body = Array.from({ length: 60 }, (_, i) => `line ${i}`).join('\n')
  const { container } = render(<MarkdownMessage>{`\`\`\`text\n${body}\n\`\`\``}</MarkdownMessage>)
  expect(container.querySelector('[data-collapsed="true"]')).toBeTruthy()
  fireEvent.click(screen.getByRole('button', { name: /expand/i }))
  expect(container.querySelector('[data-collapsed="true"]')).toBeNull()
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run -t 'collapses a very long block'`
Expected: FAIL — no collapse affordance exists.

- [ ] **Step 3: Implement**

In `CodeBlock`, derive the line count from `code` and gate the `<pre>`:

```tsx
  const [expanded, setExpanded] = useState(false)
  const lineCount = code.split('\n').length
  const collapsible = lineCount > COLLAPSE_LINES // COLLAPSE_LINES = 24
  const collapsed = collapsible && !expanded
```

Apply `data-collapsed={collapsed || undefined}` and `className={cn('overflow-x-auto p-3 text-xs leading-relaxed', collapsed && 'max-h-96 overflow-y-hidden')}` to the `<pre>`, and render below it when `collapsible`:

```tsx
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="w-full border-t px-3 py-1.5 text-xs text-muted-foreground hover:text-foreground"
        >
          {expanded ? t('chat.code.collapse') : t('chat.code.expand', { count: lineCount })}
        </button>
```

Change the copy button's class from `opacity-0 group-hover:opacity-100` to `opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100` and replace the literal `Copy`/`Copied` with `t('common.copy')` / `t('common.copied')`.

`en.json`: `"chat.code.expand": "Show all {{count}} lines"`, `"chat.code.collapse": "Collapse"`, `"common.copied": "Copied"`.
`ko.json`: `"chat.code.expand": "{{count}}줄 모두 보기"`, `"chat.code.collapse": "접기"`, `"common.copied": "복사됨"`.

- [ ] **Step 4: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/components/chat/markdown-message.tsx web/src/i18n/locales/*.json web/src/__tests__
git commit -m "feat(web): collapse long code blocks and make copy keyboard-reachable"
```

---

## Phase 5 — Artifact Panel

### Task 14: Dialog semantics and focus management

**Files:**
- Create: `web/src/hooks/use-focus-trap.ts`
- Modify: `web/src/components/portal/portal-panel.tsx`
- Test: `web/src/__tests__/components/portal/portal-panel.test.tsx` (new)

**Interfaces:**
- Produces: `useFocusTrap(ref: RefObject<HTMLElement>, active: boolean, onEscape: () => void): void`.

- [ ] **Step 1: Write the failing test**

```tsx
it('is an accessible dialog that traps focus and closes on Escape', () => {
  usePortalStore.getState().pushView({ type: 'search' })
  render(<PortalPanel />)
  const panel = screen.getByRole('dialog')
  expect(panel.getAttribute('aria-modal')).toBe('true')
  expect(panel.contains(document.activeElement)).toBe(true)
  fireEvent.keyDown(panel, { key: 'Escape' })
  expect(usePortalStore.getState().stack).toHaveLength(0)
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/__tests__/components/portal/portal-panel.test.tsx`
Expected: FAIL — `getByRole('dialog')` finds nothing (the panel is a bare `<div>`).

- [ ] **Step 3: Implement the hook**

```ts
// Focus trap + Escape for panel-style surfaces. Keyboard users currently tab
// straight past the artifact panel and cannot dismiss it without a mouse.
import { type RefObject, useEffect } from 'react'

const FOCUSABLE =
  'a[href],button:not([disabled]),textarea,input,select,[tabindex]:not([tabindex="-1"])'

export function useFocusTrap(
  ref: RefObject<HTMLElement | null>,
  active: boolean,
  onEscape: () => void,
): void {
  useEffect(() => {
    const node = ref.current
    if (!active || !node) return

    const previous = document.activeElement as HTMLElement | null
    const first = node.querySelector<HTMLElement>(FOCUSABLE)
    ;(first ?? node).focus()

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        onEscape()
        return
      }
      if (e.key !== 'Tab') return
      const items = Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE))
      if (items.length === 0) return
      const head = items[0]!
      const tail = items[items.length - 1]!
      if (e.shiftKey && document.activeElement === head) {
        e.preventDefault()
        tail.focus()
      } else if (!e.shiftKey && document.activeElement === tail) {
        e.preventDefault()
        head.focus()
      }
    }

    node.addEventListener('keydown', onKeyDown)
    return () => {
      node.removeEventListener('keydown', onKeyDown)
      previous?.focus?.()
    }
  }, [ref, active, onEscape])
}
```

- [ ] **Step 4: Apply to the panel**

In `portal-panel.tsx`, add a ref on the root element and:

```tsx
    <div
      ref={panelRef}
      role="dialog"
      aria-modal="true"
      aria-label={t('portal.panelLabel')}
      tabIndex={-1}
      className={…}
    >
```

Call `useFocusTrap(panelRef, stack.length > 0, clearStack)`.
`en.json`: `"portal.panelLabel": "Side panel"`. `ko.json`: `"portal.panelLabel": "사이드 패널"`.

- [ ] **Step 5: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/hooks/use-focus-trap.ts web/src/components/portal/portal-panel.tsx \
        web/src/i18n/locales/*.json web/src/__tests__/components/portal
git commit -m "feat(web): give the portal panel dialog semantics and a focus trap"
```

---

### Task 15: Collision-free artifact identity + version history

**Files:**
- Modify: `web/src/stores/portal.ts:104-141`
- Modify: `web/src/components/chat/artifact/artifact-card.tsx`
- Modify: `web/src/components/portal/views/artifact-view.tsx`
- Modify: `web/src/i18n/locales/en.json`, `ko.json`
- Test: `web/src/__tests__/components/chat/artifact.test.ts`

**Interfaces:**
- Produces: `artifactKey(meta: ArtifactMeta): string` now includes `meta.ordinal: number`; `ArtifactView` gains `versions: string[]` and `activeVersion: number`; `pushArtifactVersion(key: string, content: string): void`.
- `ArtifactMeta` gains `ordinal: number` — the artifact's index within its owning message, assigned by `ArtifactCard` from a per-message counter in `ArtifactContext`.

- [ ] **Step 1: Write the failing tests**

```ts
it('does not collide two untitled artifacts of the same type in one message', () => {
  const a = artifactKey({ messageId: 'm1', type: 'html', ordinal: 0 })
  const b = artifactKey({ messageId: 'm1', type: 'html', ordinal: 1 })
  expect(a).not.toBe(b)
})

it('keeps prior artifact versions when the agent rewrites one', () => {
  const meta = { messageId: 'm1', type: 'html', ordinal: 0 }
  usePortalStore.getState().toggleArtifact(meta, '<p>v1</p>')
  usePortalStore.getState().pushArtifactVersion(artifactKey(meta), '<p>v2</p>')
  const view = usePortalStore.getState().stack.at(-1)!
  expect(view.versions).toEqual(['<p>v1</p>', '<p>v2</p>'])
  expect(view.activeVersion).toBe(1)
  expect(view.content).toBe('<p>v2</p>')
})
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd web && bun run vitest run src/__tests__/components/chat/artifact.test.ts`
Expected: FAIL — keys are equal; `pushArtifactVersion` undefined.

- [ ] **Step 3: Implement**

```ts
/** Stable identity key for an artifact within a message. `ordinal`
 *  disambiguates two untitled artifacts of the same type in one message,
 *  which previously collapsed into a single panel entry. */
export function artifactKey(meta: ArtifactMeta): string {
  return `${meta.messageId}::${meta.type}::${meta.ordinal}::${meta.title ?? ''}`
}
```

Add to the artifact view type: `versions: string[]`, `activeVersion: number`. `toggleArtifact` seeds `versions: [content], activeVersion: 0`.

```ts
  /** Append a new revision. Streaming updates mutate the ACTIVE version in
   *  place (`updateArtifactContent`); a completed rewrite pushes a new one so
   *  the user can diff against what the agent replaced. */
  pushArtifactVersion: (key, content) =>
    set((s) => ({
      stack: s.stack.map((v) =>
        v.type === 'artifact' && v.key === key
          ? {
              ...v,
              versions: [...v.versions, content],
              activeVersion: v.versions.length,
              content,
            }
          : v,
      ),
    })),
```

`ArtifactCard` calls `pushArtifactVersion` when its owning message transitions from streaming to settled with content different from the active version; it keeps calling `updateArtifactContent` while streaming.

In `artifact-view.tsx`, render a version switcher when `versions.length > 1`:

```tsx
        {view.versions.length > 1 && (
          <div className="flex items-center gap-1 text-xs text-muted-foreground">
            <button type="button" disabled={view.activeVersion === 0} onClick={() => setVersion(view.activeVersion - 1)}>
              <ChevronLeft className="h-3 w-3" />
            </button>
            <span>{t('artifact.version', { n: view.activeVersion + 1, total: view.versions.length })}</span>
            <button
              type="button"
              disabled={view.activeVersion === view.versions.length - 1}
              onClick={() => setVersion(view.activeVersion + 1)}
            >
              <ChevronRight className="h-3 w-3" />
            </button>
          </div>
        )}
```

`en.json`: `"artifact.version": "v{{n}} of {{total}}"`. `ko.json`: `"artifact.version": "{{total}}개 중 v{{n}}"`.

- [ ] **Step 4: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/stores/portal.ts web/src/components/chat/artifact web/src/components/portal/views/artifact-view.tsx \
        web/src/i18n/locales/*.json web/src/__tests__/components/chat/artifact.test.ts
git commit -m "feat(web): unique artifact keys and version history in the panel"
```

---

## Phase 6 — Performance

### Task 16: Selector-scoped store subscriptions

**Files:**
- Modify: `web/src/routes/chat.tsx:40-68`
- Modify: `web/src/components/chat/messages/useAssistantActions.tsx:29`
- Test: `web/src/__tests__/components/chat/render-count.test.tsx` (new)

**Interfaces:**
- Produces: no API change. `ChatPage` and `useAssistantActions` subscribe via `useShallow` selectors.

- [ ] **Step 1: Write the failing test**

```tsx
it('does not re-render every message bubble when an unrelated store field changes', () => {
  let renders = 0
  function Probe() {
    const { removeMessage } = useAssistantActions('a1')
    renders++
    return <button type="button" onClick={() => removeMessage?.('a1')} />
  }
  render(<Probe />)
  const before = renders
  act(() => {
    useChatStore.setState({ activeMountIds: 'mount-1' })
  })
  expect(renders).toBe(before)
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/__tests__/components/chat/render-count.test.tsx`
Expected: FAIL — the whole-store subscription re-renders on any field change.

- [ ] **Step 3: Implement**

In `useAssistantActions.tsx`:

```tsx
import { useShallow } from 'zustand/react/shallow'

  const { removeMessage, sendMessage } = useChatStore(
    useShallow((s) => ({ removeMessage: s.removeMessage, sendMessage: s.sendMessage })),
  )
  // `messages` was pulled in only to locate the preceding user message for
  // regenerate; read it imperatively so this hook does not re-subscribe on
  // every token.
  const regenerate = useCallback(() => {
    const { messages } = useChatStore.getState()
    // …existing regenerate body, using this local `messages`…
  }, [messageId, removeMessage, sendMessage])
```

In `chat.tsx`, replace the destructured whole-store read with a `useShallow` selector listing exactly the fields the page uses, and read the stable action functions via `useChatStore.getState()` inside handlers where they are only called (not rendered).

- [ ] **Step 4: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/routes/chat.tsx web/src/components/chat/messages/useAssistantActions.tsx web/src/__tests__
git commit -m "perf(web): scope chat store subscriptions to the fields actually used"
```

---

### Task 17: Scroll state churn

**Files:**
- Modify: `web/src/routes/chat.tsx:212-218`
- Test: covered by the existing chat route tests; add an assertion.

- [ ] **Step 1: Implement the guarded update**

```tsx
  const handleVListScroll = (offset: number) => {
    const vl = vListRef.current
    if (!vl) return
    const atBottom = vl.scrollSize - offset - vl.viewportSize < 80
    atBottomRef.current = atBottom
    // Only commit when the boolean actually flips — the raw handler fires per
    // scroll frame and each setState re-rendered the whole page.
    setUserScrolledUp((prev) => (prev === !atBottom ? prev : !atBottom))
  }
```

- [ ] **Step 2: Verify and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/routes/chat.tsx
git commit -m "perf(web): only commit scroll state when the at-bottom flag flips"
```

---

### Task 18: Settled-prefix markdown memoization

**Files:**
- Modify: `web/src/components/chat/messages/components/BlockStream.tsx`
- Modify: `web/src/components/chat/markdown-message.tsx`
- Test: `web/src/__tests__/components/chat/markdown-streaming-perf.test.tsx` (new)

**Interfaces:**
- Produces: `MarkdownMessage` accepts `settledPrefixLength?: number`; a text block renders the settled prefix in a `memo`'d child keyed by its length, and only the tail re-parses per frame.

- [ ] **Step 1: Write the failing test**

```tsx
it('does not re-parse the settled prefix on every streaming frame', () => {
  const parses: string[] = []
  vi.spyOn(console, 'debug').mockImplementation(() => {})
  // MarkdownMessage exposes a test hook: onParse(src) fired per ReactMarkdown render.
  const { rerender } = render(
    <MarkdownMessage isStreaming onParse={(s) => parses.push(s)}>
      {'para one\n\npara two'}
    </MarkdownMessage>,
  )
  rerender(
    <MarkdownMessage isStreaming onParse={(s) => parses.push(s)}>
      {'para one\n\npara two more'}
    </MarkdownMessage>,
  )
  // The settled first paragraph must be parsed once, not twice.
  expect(parses.filter((p) => p.startsWith('para one')).length).toBe(1)
})
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL — the whole buffer is re-parsed each render.

- [ ] **Step 3: Implement**

Split on the last blank-line boundary: everything before it is settled markdown (a completed block-level construct), everything after is the live tail.

```tsx
  const splitAt = isStreaming ? children.lastIndexOf('\n\n') : -1
  const settled = splitAt > 0 ? children.slice(0, splitAt) : ''
  const tail = splitAt > 0 ? children.slice(splitAt) : children
```

Render `<SettledMarkdown key={settled.length} src={settled} />` (a `memo` whose props change only when the prefix grows) followed by the live tail through the existing pipeline. Skip the split entirely when the buffer contains an open fence (reuse `inOpenFence` from Task 11) — splitting inside a fence would break the artifact card.

- [ ] **Step 4: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/components/chat/markdown-message.tsx \
        web/src/components/chat/messages/components/BlockStream.tsx web/src/__tests__
git commit -m "perf(web): re-parse only the live tail of a streaming markdown block"
```

---

## Phase 7 — i18n and Design Tokens

### Task 19: i18n sweep

**Files:**
- Modify: `web/src/routes/chat.tsx:338`
- Modify: `web/src/components/chat/chat-input.tsx:85-158, 707-712, 718, 764`
- Modify: `web/src/components/chat/messages/UserMessage.tsx:38,42`
- Modify: `web/src/components/chat/messages/useAssistantActions.tsx:58,65,72,80`
- Modify: `web/src/components/chat/search-grounding.tsx:45-47,97,154`
- Create: `web/src/lib/relative-time.ts`
- Modify: `web/src/components/chat/empty-chat-state.tsx:66-77`, `web/src/components/chat/AgentFanoutCard.tsx:41-45`
- Modify: `web/src/components/chat/path-access-card.tsx:47,64,67,76` and `web/src/components/chat/approval-mode-selector.tsx:50,54,73` — these bypass i18n a second way, via inline `isKo ? '한국어' : 'English'` ternaries. Replace the ternaries with `t()` keys like every other string; delete the `isKo` locals.
- Modify: `web/src/i18n/locales/en.json`, `ko.json`
- Test: `web/src/__tests__/lib/relative-time.test.ts` (new)

**Interfaces:**
- Produces: `formatRelativeTime(iso: string, t: TFunction): string`.

- [ ] **Step 1: Write the failing test**

```ts
it('formats relative times through i18n', () => {
  const t = ((k: string, o?: Record<string, unknown>) => `${k}:${o?.count ?? ''}`) as never
  const now = Date.now()
  expect(formatRelativeTime(new Date(now - 5_000).toISOString(), t)).toBe('time.justNow:')
  expect(formatRelativeTime(new Date(now - 120_000).toISOString(), t)).toBe('time.minutesAgo:2')
  expect(formatRelativeTime(new Date(now - 7_200_000).toISOString(), t)).toBe('time.hoursAgo:2')
  expect(formatRelativeTime(new Date(now - 172_800_000).toISOString(), t)).toBe('time.daysAgo:2')
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/__tests__/lib/relative-time.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the shared formatter**

```ts
// Single relative-time formatter. Two hardcoded English copies existed
// (empty-chat-state.tsx, AgentFanoutCard.tsx) — both bypassed i18n.
import type { TFunction } from 'i18next'

export function formatRelativeTime(iso: string, t: TFunction): string {
  const deltaMs = Date.now() - new Date(iso).getTime()
  const s = Math.max(0, Math.floor(deltaMs / 1000))
  if (s < 60) return t('time.justNow')
  const m = Math.floor(s / 60)
  if (m < 60) return t('time.minutesAgo', { count: m })
  const h = Math.floor(m / 60)
  if (h < 24) return t('time.hoursAgo', { count: h })
  return t('time.daysAgo', { count: Math.floor(h / 24) })
}
```

- [ ] **Step 4: Replace every hardcoded string**

Exact replacements (add each key to BOTH locale files):

| Site | Literal | Key | en | ko |
|---|---|---|---|---|
| `chat.tsx:338` | `Search` | `common.search` | Search | 검색 |
| `chat-input.tsx:707-712` | `Mount` / `KB` / `Agent` / `Memory` | `mention.mount` / `.knowledge` / `.agent` / `.memory` | Mount / Knowledge / Agent / Memory | 마운트 / 지식 / 에이전트 / 메모리 |
| `chat-input.tsx:718` | `Type to search...` / `No results` | `mention.searchPlaceholder` / `mention.noResults` | Type to search… / No results | 검색어 입력… / 결과 없음 |
| `chat-input.tsx:764` | `Drop files to attach` | `chat.dropToAttach` | Drop files to attach | 파일을 놓아 첨부 |
| `chat-input.tsx:85-158` | slash-command `description` fields | `slash.<command>.description` | keep existing English | 각 명령의 한국어 설명 |
| `UserMessage.tsx:38,42` | `Edit` / `Delete` | `common.edit` / `common.delete` | (exists) | (exists) |
| `useAssistantActions.tsx:58,65,72,80` | `Copy`/`Copied!`/`Regenerate`/`Retry`/`Delete` | `common.copy` / `common.copied` / `chat.regenerate` / `chat.retry` / `common.delete` | (exist) | (exist) |
| `search-grounding.tsx:45-47,97` | `N source(s)` / `Search results` / `N image(s)` | `search.sources` / `search.results` / `search.images` | {{count}} sources / Search results / {{count}} images | 출처 {{count}}개 / 검색 결과 / 이미지 {{count}}개 |

Convert `empty-chat-state.tsx:66-77` and `AgentFanoutCard.tsx:41-45` to call `formatRelativeTime`, deleting both local helpers.
`time.*` keys — en: `just now` / `{{count}}m ago` / `{{count}}h ago` / `{{count}}d ago`; ko: `방금` / `{{count}}분 전` / `{{count}}시간 전` / `{{count}}일 전`.

- [ ] **Step 5: Guard against regression**

Add `web/src/__tests__/i18n-coverage.test.ts`:

```ts
it('en and ko define the same key set', () => {
  const flatten = (o: Record<string, unknown>, p = ''): string[] =>
    Object.entries(o).flatMap(([k, v]) =>
      v && typeof v === 'object' ? flatten(v as Record<string, unknown>, `${p}${k}.`) : [`${p}${k}`],
    )
  expect(flatten(en).sort()).toEqual(flatten(ko).sort())
})
```

- [ ] **Step 6: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/lib/relative-time.ts web/src/components/chat web/src/routes/chat.tsx \
        web/src/i18n/locales/*.json web/src/__tests__
git commit -m "fix(web): route every chat string through i18n and share one relative-time util"
```

---

### Task 20: Design token compliance

**Files:**
- Modify: `web/src/components/chat/markdown-message.tsx:217`, `web/src/components/chat/compressed-group.tsx:132`, `web/src/components/chat/chat-input.tsx:789` (all three carry `dark:prose-invert`)
- Modify: `web/src/components/chat/artifact/renderers/html-renderer.tsx:39` (`bg-white`)
- Modify: `web/src/components/chat/path-access-card.tsx:73`, `web/src/components/chat/tool-approval-card.tsx:78` (`bg-success/90 … text-white`)
- Modify: `web/src/components/chat/search-grounding.tsx:154`, `web/src/components/chat/tool-renders/ImageGeneration.tsx:107` (`bg-black/60 … text-white` image scrims)
- Modify: `web/src/index.css` (two new tokens in Step 3 + the `.prose` token bridge in Step 4 — Steps 3-4)
- Test: `web/src/design-system/__tests__/tokens.test.ts` — this file already exists; add the chat-component case to it rather than creating a second token test.

- [ ] **Step 1: Write the failing test**

```ts
it('chat components use tokens, not raw colors or dark: variants', async () => {
  const files = await glob('src/components/chat/**/*.tsx')
  const offenders: string[] = []
  for (const f of files) {
    const src = await readFile(f, 'utf8')
    if (/\bdark:/.test(src)) offenders.push(`${f}: dark: variant`)
    if (/\b(bg|text|border)-(white|black)\b/.test(src)) offenders.push(`${f}: raw color`)
    if (/#[0-9a-fA-F]{3,8}\b/.test(src)) offenders.push(`${f}: hex literal`)
  }
  expect(offenders).toEqual([])
})
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL listing all eight sites above.

- [ ] **Step 3: Add the two missing tokens**

Two of the violations have no token to migrate to. `--color-success` exists (`index.css:68`) but **`--color-success-foreground` does not**, and there is no scrim token at all. Add both to the `@theme` block beside the existing status tokens:

```css
  /* Foreground for solid `bg-success` surfaces. `--color-success` existed
     without a paired foreground, which is why two buttons hardcoded
     `text-white`. */
  --color-success-foreground: var(--success-foreground);
  /* Image/media scrim. Deliberately theme-INVARIANT: the scrim sits over
     arbitrary user imagery, so it must stay dark with light text in both
     themes. Tokenised rather than hardcoded so the value has one home. */
  --color-scrim: oklch(0% 0 0 / 60%);
  --color-scrim-foreground: oklch(100% 0 0);
```

Define `--success-foreground` in BOTH the `:root` and `.dark` blocks (same value in each — a solid success surface is the same colour in both themes, matching how `--primary-foreground` is handled).

- [ ] **Step 4: Fix each site**

- `markdown-message.tsx:217`, `compressed-group.tsx:132`, `chat-input.tsx:789`: drop `dark:prose-invert` from the class list; instead add a token bridge in `index.css` so the prose plugin reads the OKLCH layer:

```css
/* @tailwindcss/typography ships its own palette. Bind it to the oxi token
   layer so `.dark` alone flips prose, with no per-component dark: variant. */
.prose {
  --tw-prose-body: var(--color-foreground);
  --tw-prose-headings: var(--color-foreground);
  --tw-prose-links: var(--color-primary);
  --tw-prose-bold: var(--color-foreground);
  --tw-prose-code: var(--color-foreground);
  --tw-prose-quotes: var(--color-muted-foreground);
  --tw-prose-hr: var(--color-border);
  --tw-prose-th-borders: var(--color-border);
  --tw-prose-td-borders: var(--color-border);
}
```

- `html-renderer.tsx:39`: `bg-white` → `bg-background`. (The iframe's own document stays author-controlled; this is only the wrapper.)
- `path-access-card.tsx:73` and `tool-approval-card.tsx:78`: `text-white` → `text-success-foreground` (the background is `bg-success/90`, not `bg-primary` — do **not** use `text-primary-foreground` here).
- `search-grounding.tsx:154` and `ImageGeneration.tsx:107`: `bg-black/60 … text-white` → `bg-scrim text-scrim-foreground`.

- [ ] **Step 5: Verify both themes**

Toggle light/dark with a message containing prose, a success-approval card, and a search-result image. Prose must invert, the approval button text must stay legible on the success fill, and the image scrim must stay dark-with-light-text in both themes.

- [ ] **Step 6: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/components/chat web/src/index.css web/src/design-system/__tests__
git commit -m "fix(web): remove dark: variants and raw colors from chat components"
```

---

## Phase 8 — Transparency Completeness

### Task 21: Sub-agent forks in the chat timeline

**Files:**
- Modify: `crates/oxios-kernel/src/event_bus.rs:32-63` (four agent lifecycle variants)
- Modify: `crates/oxios-kernel/src/supervisor.rs` (publish sites)
- Modify: `crates/oxios-kernel/src/agent_lifecycle.rs:207-210`
- Modify: `src/api/routes/chat.rs:1650-1662` (session filter), `:1669+` (chunk mapping), test at `:2320-2329`
- Modify: `web/src/lib/stream/adapter.ts`, `ChatEvent.ts`, `StreamProcessor.ts`, `types/chat.ts`
- Create: `web/src/components/chat/messages/components/SubAgentBlock.tsx`
- Modify: `web/src/components/chat/messages/components/BlockStream.tsx`
- Test: Rust inline test; `web/src/__tests__/lib/stream-subagent.test.ts`

**Interfaces:**
- Produces: `KernelEvent::AgentCreated { id, name, session_id: Option<String> }` (and the same field on `AgentStarted`/`AgentStopped`/`AgentFailed`); WS chunks `{type:"agent_start"|"agent_end", agent_id, name, success?}`; `ChatBlock` variant `{ type: 'subagent'; id; agentId; name; status: 'running'|'done'|'failed' }`.
- **Note:** `src/api/routes/chat.rs:2320-2329` currently asserts lifecycle events are skipped. That assertion encoded "we cannot correlate them to a session". Once the events carry `session_id`, update the test to assert correlated events pass and uncorrelated ones are still dropped — do not simply delete it.

- [ ] **Step 1: Write the failing Rust test**

```rust
    #[test]
    fn agent_lifecycle_events_reach_the_owning_session_only() {
        let ev = oxios_kernel::event_bus::KernelEvent::AgentCreated {
            id: uuid::Uuid::new_v4(),
            name: "researcher".to_string(),
            session_id: Some("sess-a".to_string()),
        };
        assert!(
            kernel_event_to_ws_chunk(&ev, &Some("sess-a".to_string()), &None).is_some(),
            "the owning session must see its sub-agent"
        );
        assert!(
            kernel_event_to_ws_chunk(&ev, &Some("sess-b".to_string()), &None).is_none(),
            "another session must not"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p oxios agent_lifecycle_events_reach`
Expected: FAIL — `AgentCreated` has no `session_id` field.

- [ ] **Step 3: Add the field**

```rust
    /// A new agent has been created.
    AgentCreated {
        /// The new agent's ID.
        id: AgentId,
        /// The agent's name/goal.
        name: String,
        /// Owning chat turn key (`ExecEnv.session_id`), when the fork happened
        /// inside a chat turn. `None` for background/cron forks — those stay
        /// off the chat stream.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
```

Repeat for `AgentStarted`, `AgentStopped`, `AgentFailed`. Fill from `env.session_id` at every publish site in `supervisor.rs` and `agent_lifecycle.rs:207-210`; pass `None` at background sites.

- [ ] **Step 4: Map to WS chunks**

Extend the `event_session_id` match in `kernel_event_to_ws_chunk` with the four variants, and add chunk arms:

```rust
        KernelEvent::AgentCreated { id, name, .. } => Some(serde_json::json!({
            "type": "agent_start",
            "agent_id": id.to_string(),
            "name": name,
        })),
        KernelEvent::AgentStopped { id, success, .. } => Some(serde_json::json!({
            "type": "agent_end",
            "agent_id": id.to_string(),
            "success": success,
        })),
        KernelEvent::AgentFailed { id, error, .. } => Some(serde_json::json!({
            "type": "agent_end",
            "agent_id": id.to_string(),
            "success": false,
            "error": error,
        })),
```

Leave `AgentStarted` unmapped — `AgentCreated` already opens the block and a second frame adds nothing.

- [ ] **Step 5: Render in the block stream**

Add the `subagent` `ChatBlock` variant, the `adapter.ts` cases, the `StreamProcessor` open/close handling (keyed by `agent_id`, same shape as tool blocks), and:

```tsx
// SubAgentBlock — a fork in the agent's own timeline. Same visual tier as a
// tool card (process, not answer), with the child's name and terminal state.
export function SubAgentBlock({ block }: { block: SubAgentBlockData }) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-1.5 text-xs text-muted-foreground" role="status">
      <GitFork className="h-3 w-3 shrink-0" />
      <span className="truncate">{t('chat.subagent', { name: block.name })}</span>
      {block.status === 'running' && <span className="animate-pulse">…</span>}
      {block.status === 'failed' && <span className="text-destructive">✕</span>}
    </div>
  )
}
```

`en.json`: `"chat.subagent": "Sub-agent: {{name}}"`. `ko.json`: `"chat.subagent": "서브 에이전트: {{name}}"`.

- [ ] **Step 6: Update the skip-assertion test**

Rewrite `src/api/routes/chat.rs:2320-2329` to assert that an event with `session_id: None` is still dropped from the chat stream, while a correlated one passes.

- [ ] **Step 7: Run gates and commit**

Run: `cargo clippy --workspace --all-features -- -D warnings && cargo test --workspace && cd web && bun run vitest run && bun run typecheck`

```bash
git add crates/oxios-kernel/src/event_bus.rs crates/oxios-kernel/src/supervisor.rs \
        crates/oxios-kernel/src/agent_lifecycle.rs src/api/routes/chat.rs web/src
git commit -m "feat(kernel): correlate agent lifecycle events to their turn and show forks in chat"
```

---

### Task 22: Branch a conversation and rate an answer

**Files:**
- Modify: `web/src/components/chat/messages/components/message-context-menu.tsx`
- Modify: `web/src/stores/chat.ts` (new `branchFrom`, `rateMessage`)
- Modify: `web/src/components/chat/messages/components/reactions-bar.tsx`
- Modify: `web/src/i18n/locales/en.json`, `ko.json`
- Test: `web/src/__tests__/stores.test.ts`

**Interfaces:**
- Produces: `branchFrom(messageId: string): Promise<void>` — creates a new session seeded with the history up to and including `messageId`; `rateMessage(messageId: string, rating: 1 | -1): void` — stores the rating in `ChatMessage.metadata.rating` and POSTs it alongside the existing reaction endpoint.

- [ ] **Step 1: Write the failing test**

```ts
it('branching copies history up to the chosen message into a new session', async () => {
  useChatStore.setState({
    activeSessionId: 'sess-a',
    messages: [
      { id: 'u1', role: 'user', content: 'one' },
      { id: 'a1', role: 'assistant', content: 'two' },
      { id: 'u2', role: 'user', content: 'three' },
    ] as never,
  })
  await useChatStore.getState().branchFrom('a1')
  const s = useChatStore.getState()
  expect(s.activeSessionId).not.toBe('sess-a')
  expect(s.messages.map((m) => m.id)).toEqual(['u1', 'a1'])
})
```

- [ ] **Step 2: Run test to verify it fails**

Expected: FAIL — `branchFrom is not a function`.

- [ ] **Step 3: Implement**

```ts
      /** Fork the conversation at `messageId` into a fresh session. The trailing
       *  messages stay in the original session — this is a branch, not a trim. */
      async branchFrom(messageId: string) {
        const { messages, activeProjectId } = get()
        const idx = messages.findIndex((m) => m.id === messageId)
        if (idx < 0) return
        const kept = messages.slice(0, idx + 1)
        get().newSession()
        set({ messages: kept, activeProjectId })
      },
      rateMessage(messageId, rating) {
        set((s) => ({
          messages: s.messages.map((m) =>
            m.id === messageId ? { ...m, metadata: { ...m.metadata, rating } } : m,
          ),
        }))
      },
```

Add a `chat.branchHere` menu item and 👍/👎 buttons to the reactions bar, both i18n'd.
`en.json`: `"chat.branchHere": "Branch from here"`, `"chat.rateUp": "Good response"`, `"chat.rateDown": "Bad response"`. `ko.json`: `"대화 분기"`, `"좋은 응답"`, `"아쉬운 응답"`.

- [ ] **Step 4: Run tests and commit**

Run: `cd web && bun run vitest run && bun run typecheck`

```bash
git add web/src/stores/chat.ts web/src/components/chat/messages/components web/src/i18n/locales/*.json web/src/__tests__
git commit -m "feat(web): branch a conversation and rate assistant answers"
```

---

## Phase 9 — Documentation and Final Verification

### Task 23: Document the streaming contract boundaries

**Files:**
- Create: `docs/rfc-049-turn-cancellation.md`
- Modify: `docs/ARCHITECTURE.md` (append a § on the chat streaming contract)
- Modify: `docs/rfc-024-web-daemon-reliability.md` (append the replay-gap decision)
- Modify: `AGENTS.md` (one line under the gotchas list)

- [ ] **Step 1: Write RFC-049**

Cover: the turn key identity rule (D1), the two-halves cancellation requirement (D2), the `TurnRegistry` API, the `cancel` WS frame schema, the `ErrorKind::Cancelled` terminal, and the client's interrupted-vs-error rendering split.

- [ ] **Step 2: Append the contract section to ARCHITECTURE.md**

Document: the full WS chunk table (types, which carry `seq`, which are partials), the D3 phase decision, the D5 replay-gap decision, and the D6 SSE/WS overlap boundary.

- [ ] **Step 3: Add the AGENTS.md gotcha**

```markdown
- **Chat turn identity.** One turn key — `session_id`, or `request_id` for a session's first message — is shared by `StreamingSinkRegistry`, `TurnRegistry`, and `ExecEnv.session_id`. Never introduce a second identifier for a turn.
```

- [ ] **Step 4: Commit**

```bash
git add docs/rfc-049-turn-cancellation.md docs/ARCHITECTURE.md docs/rfc-024-web-daemon-reliability.md AGENTS.md
git commit -m "docs: record the chat streaming contract and turn cancellation design"
```

---

### Task 24: Full gate run and browser verification

**Files:** none (verification only).

- [ ] **Step 1: Rust gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo check --workspace --all-features
cargo nextest run --workspace --no-fail-fast
cargo test --workspace --doc
```

- [ ] **Step 2: Web gates**

```bash
cd web && bun install --frozen-lockfile && bun run typecheck && bun run vitest run && bun run lint && bun run build
```

- [ ] **Step 3: Supply-chain gates**

```bash
cargo audit && cargo deny check
```

- [ ] **Step 4: Browser verification — run every scenario, record the observation**

Deploy first. **`~/.oxios/web/dist` takes precedence over the binary's embedded assets** (RFC-024 C3) — refresh it from `web/dist` or delete it, then restart the daemon. A stale copy has silently served an old frontend across three deploys before.

| # | Scenario | Expected observation |
|---|---|---|
| 1 | Ask for a fenced `rust` block | Code renders complete and syntax-highlighted; Copy yields the exact source |
| 2 | Ask a question whose answer contains inline `` `code` `` | Renders inline inside the paragraph; no card, no React DOM-nesting warning in the console |
| 3 | Ask something that triggers `web_search` | Citation panel lists the results with favicons |
| 4 | Press Stop mid-answer | Streaming halts within ~1 s, partial text retained, muted "중단됨" footer, provider usage stops growing; reconnect does NOT resurrect the answer |
| 5 | Kill the daemon mid-stream | Tail tokens are present, message marked "연결이 끊겼습니다", reconnect banner appears |
| 6 | Send with an exhausted-quota model | Error card shows the quota-specific title and hint, not "Something went wrong" |
| 7 | Switch sessions | Skeleton shows, then history; block the network and confirm the load-failure banner + Retry |
| 8 | Ask for a GFM table | No raw-pipe flicker during streaming |
| 9 | Toggle light/dark | Code blocks follow the theme |
| 10 | Open an artifact, press Tab then Escape | Focus stays inside the panel; Escape closes it |
| 11 | Ask for two untitled HTML artifacts in one message | Two distinct panel entries |
| 12 | Trigger a sub-agent fork | A sub-agent row appears in the chat timeline |

- [ ] **Step 5: Commit any fixes surfaced by verification, then finish**

---

## Self-Review

**Spec coverage.** Every audited defect maps to a task: extractText / inline code / grounding → Already Landed; cosmetic Stop → Tasks 1-4; watchdog → 5; onclose flush → 6; ErrorKind mismatch → 2+6; dead `phase` → 7; tracker reset → 8; dead activity card → 9; session skeleton + silent failure → 10; markdown healing → 11; highlight theme → 12; code-block ergonomics → 13; artifact dialog semantics → 14; artifact key collision + versioning → 15; whole-store subscriptions → 16; scroll churn → 17; per-frame re-parse → 18; i18n + relative-time duplication → 19; token violations → 20; sub-agent visibility → 21; branch + feedback → 22; replay-gap and SSE/WS boundaries → 23 (documented per D5/D6); line numbers → declined per D4.

**Placeholders.** None. Every code step carries the literal content; the two places that permit a judgment call (Task 9's dead-code proof, Task 12's `revert-layer` fallback) state the decision rule and the alternative explicitly.

**Type consistency.** `TurnRegistry`/`TurnToken` names and method signatures are identical in Tasks 1, 2, and 3. `ErrorKind::Cancelled` (Rust) ↔ `'cancelled'` (TS) is fixed by the `rename_all = "snake_case"` attribute in Task 2 and consumed by the same string in Tasks 3, 4, and 6. `artifactKey`'s new `ordinal` field is introduced in Task 15 and used nowhere earlier. `formatRelativeTime(iso, t)` has one signature, used by both call sites in Task 19. `healStreamingMarkdown` and `inOpenFence` are defined in Task 11 and reused by Task 18.

**Ordering constraints.** Task 1 → 2 → 3 → 4 is a hard chain (registry → gateway → WS frame → rendering). Task 5 depends on Task 3 (`cancelTurn`). Task 6's error-copy half depends on Task 2's snake_case wire change. Task 18 depends on Task 11 (`inOpenFence`). Everything else is independent and may run in parallel.
