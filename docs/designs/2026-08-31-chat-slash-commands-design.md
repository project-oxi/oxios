# Chat Slash Commands — Real Execution (2026-08-31)

## Problem

The Web chat slash menu (`web/src/components/chat/chat-input.tsx` `SLASH_COMMANDS`) is
text-insertion only. Nine commands (`/compact`, `/new-topic`, `/clear`, `/search`, `/web`,
`/skill`, `/persona`, `/save`, `/export`) insert a literal `/cmd ` string into the editor and
send it to the model as plain text. No layer — WS handler, gateway, orchestrator, kernel —
parses these prefixes. Only `/clear` does anything (clears the editor locally). Claude
Desktop / Codex treat slash commands as first-class actions; ours are decorative.

## Design

Commands split into three execution classes. Every kept command maps onto machinery that
already exists; no new kernel subsystems.

### Class A — Client-local (no WS traffic)

| Command | Action |
|---|---|
| `/clear` | Reset the conversation (`useChatStore.newSession()`). Claude Code parity. |
| `/model [id]` | Set the session model override (`activeModelId`). No arg → usage hint. |
| `/export` | Fetch `GET /api/sessions/:id`, build Markdown, download as a blob. |
| `/help` | Push a notice row listing all commands. |
| `/search <query>` | Client-side validated (no pre-check needed), then a normal turn with a `turn_command` annotation (Class C). |
| `/skill <name> [args]` | Client validates `name` against `GET /api/skills`, then a normal turn annotated (Class C). Unknown name → local error notice, no send. |

### Class B — Server-executed (new WS frame, no LLM turn)

New client→server frame:

```json
{ "type": "command", "command": "compact", "args": "", "session_id": "..." }
```

New server→client response (sent on the same connection, `ws_tx` directly):

```json
{ "type": "command_result", "command": "compact", "status": "ok" | "error",
  "message": "...", "data": {} }
```

| Command | Server action |
|---|---|
| `/compact` | Load session → missing ⇒ error; `exchange_count <= VISIBLE_TAIL` (20) ⇒ ok + "not enough history"; else `CompressionService::spawn_compress`. Progress already streams via `compression_delta/done/failed` chunks (RFC-015 path). |
| `/persona [id]` | No arg ⇒ `command_result` data lists enabled personas. With id ⇒ validate via `kernel.persona.get` + `enabled`, then persist `session.active_persona_id` (same write the turn path performs). |
| `/skill` (no arg) | `command_result` data lists installed skills (name + description). |

The frame is parsed in the WS recv task in `src/api/routes/chat.rs` (`match msg_type` new
`"command"` arm) — same layer that already handles `resume` / `cancel` / `interview_response`.
Command frames never create gateway turns, never touch `isStreaming`, and are never persisted
as chat messages.

### Class C — Turn-annotated (LLM turn with an explicit directive)

`/search` and `/skill <name>` run a normal agent turn whose frame carries an optional
`turn_command` field. The client sends the cleaned user text (command prefix stripped) plus:

```json
{ "type": "message", "content": "<query>",
  "turn_command": { "kind": "web_search", "query": "..." } }
{ "type": "message", "content": "<args>",
  "turn_command": { "kind": "skill", "name": "pdf", "args": "..." } }
```

Plumbing mirrors the existing `persona_id` / `brain_space` precedent field-for-field:

```
WS frame.turn_command (JSON string in gateway metadata)
  → gateway.rs parses to oxios_ouroboros::TurnCommand
  → Orchestrator::handle_unified(+1 param) → MsgCtx.turn_command
  → resolve_exec_env → ExecEnv.turn_command
  → execute_directive_with_session → execute_inner(+1 param)
  → system_prompt injection:
      web_search: "## Turn Command: Web Search … you MUST use the web_search tool"
      skill:      "## Invoked Skill: <name>" + full SKILL.md body
                  (loaded via kernel_handle.extensions.get_skill_content)
```

`TurnCommand` (serde-tagged enum) lives in `oxios-ouroboros/src/directive.rs` next to
`MsgCtx`/`ExecEnv` — no kernel types involved.

## Frontend

- `web/src/lib/slash-commands.ts` (new) — single registry: `{ id, label, icon, argHint,
  kind: 'client' | 'server' | 'turn' }`. The input menu, the parser, and `/help` all read it.
- `chat-input.tsx` — menu upgrade: keyboard navigation (↑/↓ + Enter/Tab select, Esc closes),
  arg hint line, arg-less commands execute on selection instead of inserting text, copilot
  variant no longer opens the menu.
- `stores/chat.ts` — `sendSlashCommand(raw)` action executes Class A/B; `sendMessage` gains
  the Class C parse helper usage; new `command_result` chunk case appends a notice row.
- Notice rows: `ChatMessage { role: 'system', metadata: { notice: true } }`; `MessageView`
  `case 'system'` renders a compact centered pill (not an assistant bubble).
- `/export` — `web/src/lib/export-transcript.ts` (new): session fetch → Markdown → download.

## Command set changes

Removed from the menu: `/new-topic` (alias of `/clear`), `/web` (alias of `/search`),
`/save` (sessions auto-persist after every turn; `/export` covers the offload case).
Added: `/help`, `/model`.

## Non-goals

- CLI/Telegram channel parity (commands parse at the WS layer only; gateway-level parsing
  would benefit them but changes every channel's contract — deferred).
- Slash-command completion of persona/skill names inside the menu (menu shows usage hints;
  lists arrive via `/persona`, `/skill` no-arg).
- Copilot (cmd+J) one-shot dialog stays command-free.

## Verification

- Rust: unit tests for the WS command parser and the system-prompt injection fn; serde
  roundtrip for `TurnCommand`.
- Web: vitest for the command parser/registry; existing suites must stay green.
- E2E smoke: run `oxios`, open chat, execute `/help`, `/compact` (short session → skip
  notice), `/persona` (list), `/model gpt-x`, `/export` (download), `/search` turn annotation.
