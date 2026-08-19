# Chat Completion Gating + Stale Deployment Fix — Design

Date: 2026-08-20
Status: implemented (autonomous run; user waived approval gates)

## Problem

User reported three chat-UI defects. Investigation (3 parallel scouts + direct
code reads + live-daemon bundle probe) reclassified them:

1. **"지식에 저장 / 이모티콘 등록 버튼이 응답 완료 후에 생겨야 한다"**
   Real code gap at HEAD. `MessageReactionsBar`, `KnowledgeSaveIndicator`
   and the hover `MessageActionBar` render as soon as the assistant bubble
   mounts (first content chunk) — while the turn is still streaming.
   `FollowUpChips` already gates on `!generating`
   (`follow-up-chips.tsx:46`); the other affordances do not.
2. **"추론/도구 과정이 스텝 완료 후에야 보인다"**
   Not reproducible at HEAD. Per-token reasoning/text deltas and live
   tool-start/progress/end events flow kernel → gateway (partial WS frames)
   → StreamProcessor synchronously (`agent_runtime.rs` `run_tokio_stream`,
   `gateway.rs` collector, `chat.ts` rAF flush). Root cause of the symptom:
   the running daemon is a stale test binary.
3. **"백틱 1개 인라인 코드가 코드블록처럼 렌더링된다"**
   Fixed at HEAD by `80fae79f2` (`rehypeMarkInlineCode`), with regression
   test `markdown-code-block.test.tsx`. Same root cause: stale binary.

### Root cause of 2 and 3: stale deployment

- Running daemon: `/Users/won/bin/oxios` → symlink →
  `oxios-1.42.0-livestream6`, built **2026-08-19 11:09**.
- Served SPA bundle contains `text-[0.85em]` (old `InlineCode`) but **zero**
  occurrences of `dataInlineCode` — it predates `80fae79f2` (11:52) and the
  chat-UI remediation `c58490049` (20:08).
- SPA is compile-time embedded (`embedded_web.rs`,
  `include_dir!("$CARGO_MANIFEST_DIR/web/dist")`) — rebuilding the binary is
  the only delivery path.

## Design

### 1. Post-completion action gating (web)

`AssistantMessage.tsx` — one rule, mirroring `FollowUpChips`:
**while `message.generating` is true, no post-turn affordance renders.**

- `actions`: pass `<MessageActionBar>` only when `!generating`
  (copy/regenerate/delete mid-stream are premature or hazardous).
- `KnowledgeSaveIndicator`: gate on `!generating` (keep existing
  `sessionId != null && assistantIndex != null` conditions).
- `MessageReactionsRow`: gate on `!generating`.
- `ChatMetadata` (model name row) stays visible during streaming —
  informational, not an affordance.

Error/abort paths already clear `generating` via `finalizeStreamingMessage`
(`stores/chat.ts`), so retry/delete remain reachable on failed turns.

### 2. Emoji button — decision

The "이모티콘 등록" trigger opens a client-side reaction picker:
`localStorage`-backed reactions (`oxios:message-reactions`) plus an in-store
👍/👎 rating. No backend endpoint, single-user scope. Kept as-is (gated on
completion) — removing a shipped feature was not requested; the user only
questioned its purpose. Removal is a one-line follow-up if unwanted.

### 3. Deployment fix (no code change)

1. `web`: frozen install → typecheck → tests → Biome → `bun run build`
   (fresh `web/dist`).
2. `cargo build --release` — embeds the fresh SPA.
3. Install as `/Users/won/bin/oxios-1.43.0`; repoint the `/Users/won/bin/oxios`
   symlink (old `livestream6` binary preserved for rollback).
4. Restart daemon (SIGTERM → relaunch `--foreground --config
   ~/.oxios/config.toml`, detached), wait for port 4200.
5. Verify served bundle now contains `dataInlineCode`.

### 4. Verification

Browser-driven against the rebuilt daemon on :4200 (`auth_enabled = false`):

- Live streaming: reasoning/tool events appear during the turn, not after.
- Button timing: no action row / reactions / save button while generating;
  all present after `done`.
- Inline code: `` `span` `` renders inside a `<p>` (inline pill), fenced
   block still renders as `CodeBlock`.
- Unit test pins the gating contract (TDD).

## Non-goals

- No new streaming protocol work — HEAD pipeline is already live per-token.
- No persistence for reactions/rating.
- No Rust code changes.

## Outcome (post-verification)

- **Deployed**: `/Users/won/bin/oxios-1.43.0` (symlink repointed from the
  stale `oxios-1.42.0-livestream6`), daemon restarted, serving bundle
  `scroll-area-AQGS3yjw.js` containing `dataInlineCode`.
- **Extra fix found during E2E**: `ensureLastAssistant` created the streaming
  placeholder without `generating: true`, so the post-completion action row
  flashed on the empty bubble during the agent_start → first-content-chunk
  gap (seconds of LLM latency). Placeholder now mounts generating; covered by
  `assistant-completion-gating.test.tsx`.
- **Verified in browser** (WS frame capture + DOM probes):
  - Live streaming: reasoning deltas and token deltas arrive per-token with
    real timestamps (agent_start +16 ms after send; reasoning deltas spread
    over the turn; tokens streamed as generated). No step-level batching.
  - Inline code: `` `원패스` `` renders as `<p><code class="px-1.5 …">`;
    fenced blocks render `<pre><code class="language-python">`.
  - Buttons: absent while generating (including the placeholder window),
    present after `done`.
  - Contract pin added: `stores-turn-preserve.test.ts` replays the full
    sub-agent turn frame sequence and asserts content survives `done`.
- **Follow-up (documented, not fixed)**: refresh/reconnect racing. A page
  reload during the post-turn reconnect window can lose the next send (the
  message spawns its agent server-side but the dying socket never renders
  the stream) and the client promises a `resync` chunk handler in comments
  (`chat.ts` ~L875, L930) that does not exist in `KNOWN_CHUNK_TYPES`.
  Symptom class matches the historical "must refresh to see the response"
  reports. Reproduction required an artificial reload mid-reconnect; a
  healthy connection renders every turn correctly.
