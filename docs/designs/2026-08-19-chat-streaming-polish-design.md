# Chat Streaming Polish — Design

Date: 2026-08-19
Status: implemented (session live-audit follow-up)
Scope: three user-visible gaps found while auditing the web chat streaming
pipeline against the Claude Code / Codex baseline. The core sequential
block-stream model (reason → tool → reason → answer, live per-token deltas)
was verified working; these are polish items on top of it.

## Item 1 — Transcript tail working pulse

**Problem.** Between `tool_end` and the next reasoning span (the second LLM
round-trip, ~3–5 s), nothing in the *transcript* indicates the agent is still
working. The LiveActivityBar covers the input box only; the block timeline
tail is static (settled tool card, collapsed Thought row).

**Design.** `BlockStream` gains an optional `generating` prop and renders a
`WorkingTail` — three pulsing dots, flat muted tier (quieter than reasoning,
consistent with the existing visual hierarchy: answer > tool card > reasoning)
— when `generating && no trailing block is actively streaming`. A trailing
block already shows its own affordance while active (Thinking sweep, tool
spinner, streaming markdown), so the tail pulse appears exactly in the dead
zones: before the first delta of the turn and between phases.

- `isBlockStreaming(block)` helper — true for `reasoning.status==='streaming'`,
  `text.streaming`, `tool.status==='loading'`.
- `AssistantMessage` passes `message.generating`.
- `role="status"` + `aria-label` from i18n key `chat.working` (ko/en).

## Item 2 — Denied tool calls render as errors

**Problem.** A CSpace/permission denial reached the UI with
`is_error: false`, so `ToolCallCard` rendered the neutral style and the
denial was indistinguishable from a successful call.

**Root cause.** `GatedTool` returned denials as
`Ok(AgentToolResult::error(msg))`. oxicode-agent 0.73's loop only derives
`is_error` from `Err(...)` or an `after_tool_call` hook
(`agent_loop/tool_exec.rs`: `Ok(r) => result = r` never inspects
`r.success`), so a soft-error Ok result flows through as success.

**Design.** All four GatedTool denial/timeout returns (tool gate, path-access
card denial, headless path denial, approval denial) now return
`Err(denial_message)` (`ToolError = String`). The SDK loop converts Err to
`AgentToolResult::error(e)` with `is_error: true` — same denial text to the
LLM, now flagged as an error through `KernelEvent::ToolExecutionFinished` →
WS `tool_end.is_error` → adapter `tool.end.error` → destructive card styling.

**Known limitation (documented, not fixed here).** Inner tool soft-errors
(`Ok` results with `success: false` from SDK-provided tools) still stream as
`is_error: false`; that semantics lives upstream in oxicode-agent. Our own
gate denials — the case where the flag matters for trust — are fixed.

**Adapter companion fix.** `tool_end.is_error` built a `ChatError` whose
`message` only read `tool_result`; gate denials carry their reason in
`output_summary`, so the expanded card's error paragraph collapsed to
"Tool error". The adapter now falls back to `output_summary`, and the live
card renders the 🔒 reason (verified end-to-end: wire `is_error:true` →
destructive card → expanded body shows the denial text).

## Item 3 — Transient "Waiting for connection" after a turn

**Verdict: benign, no code change.** Instrumented evidence:

- A raw WebSocket held open 75 s receives server Pings at 20/40/60 s and
  stays healthy; `oxios_ws_connections_total{keepalive_timeout}` is 0 across
  every observation window — the server never killed a connection for missing
  pongs.
- The composer's **placeholder** is `waitingForConnection` whenever
  `connected === false` (chat-input.tsx). In accessibility snapshots a
  placeholder reads like a status line, which made normal connect windows
  (page load, post-daemon-restart reconnect) look like a fault. Placeholders
  never appear in `innerText`, which is why DOM probes kept reporting
  "connected".
- Remaining `close` counts all map to page navigations and manual probe
  closes. The one-off incident from the audit window most plausibly was a
  deploy restart (all sockets closed, clients auto-reconnect) or a hidden-tab
  timer freeze — a mechanism `use-global-events.ts` already recovers from.

## Live findings during verification (follow-ups, out of scope)

1. **Deploy gotcha — `~/.oxios/web/dist` precedence.** A manually placed
   dist dir is served INSTEAD of the binary's embedded assets (RFC-024 C3
   escape hatch). This machine had a stale copy from the previous session, so
   three binary deploys served yesterday's frontend while appearing
   successful. Deployment procedure for dev: refresh `~/.oxios/web/dist`
   from `web/dist` (or remove it to fall back to embedded), then restart.
2. **Turn-done-while-agent-alive frame leakage.** Observed once: a multi-tool
   turn's orchestrator emitted `done` (27.8 s) while its agent was still
   blocked on a path-access card (120 s timeout). The agent's subsequent
   `tool_start`/`path_access`/`tool_end` frames streamed into the NEXT
   turn's window, that turn's message stuck at `generating:true` forever,
   and phantom approval cards appeared for a turn the user believed ended.
   Needs an orchestrator-level fix (kill/wait agents before `done`, or tag
   frames by turn and drop late ones).
3. **Tail pulse live behavior.** With glm-5-turbo, no >250 ms window exists
   where the message is mounted, generating, and no block is streaming: the
   assistant message materializes only at the first chunk (the pre-chunk gap
   belongs to LiveActivityBar by the no-empty-bubble design), and inter-span
   deltas arrive back-to-back. The tail is a unit-verified safety net that
   will surface on models/runtimes with real second-round-trip latency.

## Verification

- `cargo test -p oxios-kernel` — pass (incl. new
  `cspace_denial_returns_err_with_denial_text`)
- `cargo clippy -p oxios-kernel --all-targets` — clean
- `bun run test` in `web/` — 460 pass (incl. BlockStream tail ×3, adapter
  denial-reason ×2)
- Browser (fresh bundle, cache disabled): denial turn → wire
  `is_error:true` + destructive card + 🔒 reason in the expanded card's
  error paragraph; healthy turns unaffected; no duplicate text renders.
