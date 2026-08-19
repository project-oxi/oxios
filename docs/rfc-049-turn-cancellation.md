# RFC-049: Turn Cancellation

> **Status:** Accepted (implemented)
> **Created:** 2026-08-19
> **Depends on:** RFC-015 (chat transparency — WS chunk protocol), RFC-024 (web↔daemon reliability — seq/replay)
> **Design rationale:** `docs/designs/2026-08-19-chat-ui-defect-remediation-design.md`

## Problem

A user who sends a message has no way to stop it. The chat UI's Stop button
killed only the client-side stream; the backend dispatch future and the
supervisor-spawned agent task kept running, burning provider tokens until the
turn completed naturally. Worse, the client-side-only "stop" left the UI in a
generating state with no terminal frame, so the message never settled.

## Design

### D1 — The turn key already exists; do not invent a new one

`gateway.rs` computes `sink_session_key = session_id.clone().unwrap_or_else(|| request_id.clone())`
and registers the streaming sink under that string. The orchestrator mirrors it
into `ExecEnv.session_id`; the agent runtime uses it as the transparency
session. Three components must agree on this identity:

| Component | Key |
|---|---|
| `StreamingSinkRegistry` | `session_id` or `request_id` (first message) |
| `TurnRegistry` | the same string |
| `ExecEnv.session_id` | the same string |

**Rule:** never introduce a second identifier for a turn. A new registry keys
on this same string or it is not part of the turn.

### D2 — Cancellation needs both halves

Aborting only the gateway dispatch future leaves the supervisor-spawned agent
task running (`BasicSupervisor.handles` is keyed by `AgentId`, and the
lifecycle manager drops the `AgentId` after fork). Two halves must be
reachable to actually stop a turn:

1. **The gateway dispatch future** — woken through the turn's `TurnToken` so
   it stops awaiting, unregisters the sink, and emits the terminal frame.
2. **The agent task** — killed by `AgentId` through the supervisor so the
   provider request actually stops.

### The `TurnRegistry` API

`crates/oxios-kernel/src/turn_registry.rs`. Deliberately dependency-free
(`tokio::sync::Notify` + `AtomicBool`; `oxios-kernel` does not depend on
`tokio-util`).

```rust
pub struct TurnRegistry { /* HashMap<String, TurnEntry> */ }

impl TurnRegistry {
    pub fn new() -> Self;

    /// Begin a turn. Replaces any stale entry for the same key so a new turn
    /// never inherits a previous turn's cancel flag.
    pub fn open(&self, key: &str) -> TurnToken;

    /// Bind the forked agent to the turn so cancellation can kill it.
    /// Returns `true` when the entry exists AND was already cancelled at bind
    /// time — the caller MUST kill the just-forked agent, since `cancel` ran
    /// concurrently with `fork_directive` and could not see the agent id.
    pub fn bind_agent(&self, key: &str, agent_id: Uuid) -> bool;

    /// Cancel a turn. Returns the bound agent id so the caller can kill it
    /// (the registry deliberately does not own a supervisor reference).
    pub fn cancel(&self, key: &str) -> Option<Uuid>;

    /// Idempotent — safe to call on every non-cancelled exit path.
    pub fn close(&self, key: &str);
}

pub struct TurnToken { /* Arc<AtomicBool>, Arc<Notify> */ }

impl TurnToken {
    pub fn is_cancelled(&self) -> bool;
    /// Resolves as soon as the turn is cancelled; safe to `select!` on (the
    /// flag is checked before awaiting, so an early cancel is not missed).
    pub async fn cancelled(&self);
}
```

The gateway opens the turn and `select!`s on `turn_token.cancelled()` against
`orchestrator.handle_unified(...)`; the supervisor publishes the `AgentId`
back through `bind_agent`. The chat WS `cancel` arm calls `turns.cancel(&key)`
and, when an agent is bound, `agents.kill(...)`.

### The `cancel` WS frame

A client→server frame on the chat WebSocket:

```json
{ "type": "cancel", "session_id": "optional" }
```

Resolution order (`resolve_cancel_key` in `src/api/routes/chat.rs`):

1. The frame's `session_id` — when present and non-empty.
2. The connection's active session (a write-through mirror kept by the
   forwarder task), so a bare `{"type":"cancel"}` stops the in-flight turn.
3. Neither → the arm no-ops. A missing key or a failed kill must never fail
   the frame.

The terminal frame is NOT sent by the cancel arm — it is produced by the
gateway's cancelled branch so the client sees exactly one terminal frame.

### The gateway-side terminal

On `turn_token.cancelled()` the dispatch task:

1. drops the strong channel sender so the inner collector task drains;
2. unregisters the streaming sink;
3. closes the turn in the registry;
4. emits one terminal `OutgoingMessage` with `ErrorKind::Cancelled` and the
   user-facing copy `"Turn cancelled by user"`, preserving the original
   `request_id` so `chat.rs`'s pending-user-message slot still resolves and
   persists the user's message.

The terminal travels the normal `done`/`error` path, so the client's existing
settle logic runs: the partial answer is kept and marked `cancelled` (not
dropped, not rendered as an error).

### `ErrorKind::Cancelled`

`oxios-gateway/src/message.rs` gains `Cancelled` — a user-initiated stop, not
a fault. `error_classify.rs` never produces it (the gateway emits the terminal
directly); it exists to keep the exhaustive match honest.

### Client rendering split: interrupted vs error

| Terminal | Client render |
|---|---|
| `error` with `kind: "cancelled"` | partial answer kept, `InterruptedNotice` with the cancelled copy, Stop button resets |
| any other `error` | dedicated error message with kind-specific copy + retry suggestion |
| abrupt disconnect (no terminal) | partial answer kept, `InterruptedNotice` `reason="interrupted"` |

Cancellation is a first-class, truthful state — not an error the user did
something wrong to trigger.

## Verification

- `crates/oxios-kernel/src/turn_registry.rs` unit tests (open/bind/cancel,
  cancel-before-bind race, close idempotence).
- `crates/oxios-gateway/tests/cancel_turn.rs` — `ErrorKind::Cancelled`
  serializes to `"cancelled"`; a cancelled token resolves and reports the
  bound agent.
- `src/api/routes/chat.rs` `cancel_tests` — `cancel_frame_marks_turn_and_returns_agent_id`.
- `cargo test --workspace`, `cargo clippy --workspace --all-features -- -D warnings`.
