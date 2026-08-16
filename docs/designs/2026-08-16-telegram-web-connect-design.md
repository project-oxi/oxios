# Telegram Instant Connect from Web UI — Design

Date: 2026-08-16
Status: Approved (user pre-approved autonomous execution)

## Problem

The Web UI can *edit* Telegram settings but cannot *connect* Telegram:

1. `TelegramPlugin::setup` reads the bot token **only** from the env var named by
   `channels.telegram.bot_token_env`. The token stored via Web UI → Settings →
   Secrets (`PUT /api/secrets/telegram_bot_token` → auth store) is never read,
   so the channel can never start from a Web-UI-entered token.
2. Channel activation is boot-only (`activate_channels` in `src/main.rs`). There
   is no runtime start/stop API, even though the Gateway already has
   `register`/`unregister`/`channel_names`.
3. `channels.enabled` has no Web UI editor, so the user cannot even reach the
   state where a restart would connect the bot.

RFC-028 §"텔레그램 토큰 정정" specified the secret-store priority for the token
but it was never implemented in the plugin.

## Goal

From the Web UI alone: enter a Telegram bot token → click Connect → channel is
running immediately (no daemon restart). Disconnect equally instant. Connection
status visible at a glance.

## Non-Goals

- Webhooks (long polling stays).
- Runtime control of the CLI channel (interactive; connect/disconnect is
  rejected for non-`telegram` names).
- Persisting bot profile info across daemon restarts (bot username is shown at
  connect time and while the channel object lives; not written to disk).
- Changing the generic settings restart-badge system.

## Decisions

### D1 — Token resolution: one function for display and use

`TelegramPlugin::setup` resolves the token with
`CredentialStore::resolve_secret("telegram_bot_token", &cfg.bot_token_env)` —
priority: env var → oxios auth store (`~/.oxios` via `OXICODE_HOME`) → shared
oxicode-cli store (`~/.oxicode/auth.json`).

RFC-028 wrote "store → env" priority; we keep **env first** deliberately:
it is the kernel-wide credential convention (containers/K8s override), and it
makes the secrets UI's displayed source (same function underneath) exactly what
the plugin consumes. Intent of RFC-028 (store works) is preserved; letter is
superseded by consistency.

The boot-time config validation warning (`OxiosConfig::validate`) switches from
env-only to the same `resolve_secret`, so it no longer false-positives when the
token lives in the store.

Error message when no token is found names both fixes (Web UI secret store,
env var).

### D2 — Fail-fast token validation via `getMe`

`TelegramChannel::validate_token()` calls `getMe` once (per-request 5 s
timeout) and returns:

| Outcome | Meaning | Plugin behavior |
|---|---|---|
| `Valid(username)` | 200 + `ok:true` | proceed; username recorded |
| `Unreachable` | connect error / timeout / 5xx | `warn!` + proceed (boot resilience preserved — the polling loop retries) |
| `Rejected(description)` | Telegram 401/404 (definitive) | `Err` → setup fails; connect endpoint returns 400 with Telegram's description |

This gives the Connect button real feedback ("Unauthorized") instead of a
silently failing polling loop.

### D3 — `channels.telegram.api_base` config field

New optional field (default `https://api.telegram.org`). Wires the existing
dead `with_api_base` builder, enables self-hosted Bot API servers, and lets
tests run against a local mock server instead of api.telegram.org. Added to
`RESTART_REQUIRED_FIELDS` and the Web UI telegram section (text field).

### D4 — Runtime channel control API (`/api/channels`)

New `src/api/routes/channels_routes.rs`:

- `GET /api/channels` → `{ "channels": [ { name, available, enabled, running, token_source? } ] }`
  - `available`: compiled into the binary (from the plugin registry)
  - `enabled`: `config.channels.enabled` contains name
  - `running`: `gateway.channel_names()` contains name
  - `token_source`: `"env" | "auth_store" | null` — telegram only, from
    `resolve_secret`; **never** the token value.
- `POST /api/channels/{name}/connect` (body optional `{"token": "..."}`)
  1. `name` must be `telegram` (else 400 — the CLI channel is interactive).
  2. If `token` present and non-empty → `CredentialStore::store`.
  3. Resolve token (D1). Missing → 400.
  4. If channel already running → `gateway.unregister` first (reconnect
     semantics: re-setup applies current config + store token without daemon
     restart).
  5. `plugin.setup(ChannelContext{ config: snapshot of state.config, config_path })`
     → Err → 400 with message (invalid token surfaces here).
  6. `gateway.register(bundle.channel)` → Err → 500. Non-empty `bundle.tasks`
     (none today) are logged as unsupervised.
  7. Persist `channels.enabled += "telegram"` (dedup) in `state.config` and
     `config.toml` — **after** successful start, so a rejected token never
     leaves `enabled` set.
  8. `200 { "status": "connected", "info": { "bot_username": ... } | null }`.
- `POST /api/channels/{name}/disconnect`
  1. `gateway.unregister` (idempotent).
  2. Persist `channels.enabled -= name`.
  3. `200 { "status": "disconnected" }`.

Routes sit behind the existing API middleware/auth exactly like sibling routes.

`Channel` trait (oxios-gateway) gains `fn status(&self) -> serde_json::Value`
(default `Null`) and `Gateway::channel_status(name)` exposes it;
`TelegramChannel` reports `{ "bot_username": ... }`. This is how the connect
response returns the bot identity without telegram-specific types in the
gateway.

### D5 — Gateway handle plumbing

- `SurfaceContext` (oxios-gateway) gains `pub gateway: Arc<Gateway>`, filled in
  `activate_surfaces` from `kernel.gateway()`.
- `AppState` gains `pub gateway: Arc<Gateway>`; `WebSurface::start` threads it
  from the context. (KernelHandle cannot hold the gateway: oxios-gateway
  depends on oxios-kernel, not vice versa.)

### D6 — Web UI: connection card in the Telegram section

`ChannelsSection` renders a connection card above the field rows (telegram
section only):

- Status line: badge (Connected / Not connected) + token source badge.
- When no token is stored (or user opens the input): password-style token
  input + **Connect** button (one call: `POST connect {token}`).
- When a token exists: single **Connect** button; while connected also
  **Disconnect**.
- Saving config fields then pressing Connect/Reconnect applies them without a
  daemon restart (reconnect re-reads config) — noted in the section
  description copy.
- `api_base` text field added to the telegram section fields.
- react-query (`['channels']`) + `api` client + sonner toasts, following
  `secrets-section.tsx` patterns. i18n strings for ko/en.

### D7 — Restart-classification stays honest

`channels.*` fields remain restart-required in `RESTART_REQUIRED_FIELDS`
(saving config alone still needs restart **or** reconnect). `channels.telegram.api_base`
is added to that list. UI copy explains reconnect as the instant path.

## Edge Cases

- **Reconnect while running** — unregister-then-setup-then-register; the old
  polling task is stopped by `unregister`'s shutdown signal before the new one
  starts, so no double polling.
- **Connect fails after unregister (reconnect path)** — old channel stopped,
  error returned; status shows not-running; config `enabled` unchanged.
- **Concurrent connects** — both unregister first; last register wins in the
  gateway map; the loser's task was already signalled to stop. Acceptable.
- **Disconnect when not running** — idempotent 200; still persists
  `enabled=false`.
- **Token in env var AND store** — env wins (D1); secrets UI badge and plugin
  agree because they share `resolve_secret`.
- **Boot with bad stored token** — boot `getMe` validation rejects definitively
  → `activate_channels` logs the error and continues (existing behavior for
  setup failures; daemon health unaffected).
- **Auth** — endpoints inherit the router's auth middleware; with
  `auth_enabled=false` + non-loopback bind the existing F2 warning already
  covers the exposure class.

## Testing

Backend (cargo, per-task TDD):

- `resolve_bot_token` helper: env wins; missing → actionable error. Store-path
  test isolates `OXICODE_HOME` to a temp dir.
- `validate_token` against a local fake Telegram server (tokio `TcpListener`,
  canned JSON): 200 → `Valid(username)`; 401 → `Rejected`; refused connection →
  `Unreachable`.
- Plugin setup end-to-end against the fake server via `channels.telegram.api_base`.
- Gateway `register` → `channel_status` → `unregister` roundtrip (dummy channel).
- Pure helpers in channels_routes (`upsert_enabled`, `remove_enabled`,
  token-source classification).
- Config validation warning no longer fires when the token is store-sourced.

Frontend: `bun run build`; browser-drive the real UI (alt-home daemon) for
visual verification.

E2E smoke (isolated `OXIOS_HOME`, alternate port, fake Telegram server as
`api_base`): store token → connect → 200 + running; invalid token (real
api.telegram.org) → 400 Unauthorized; disconnect → stopped + `enabled` removed
from `config.toml`. The user's real daemon is never touched.

## Compat

- Config: `api_base` has a serde default — old configs parse unchanged.
- API: purely additive routes.
- No changes to boot activation flow semantics beyond token source + fail-fast
  validation.
