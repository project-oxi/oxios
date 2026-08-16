# RFC-044 — Remote Access, Mobile Companion & Multi-Agent Coding UX

> **Status:** Implemented (v1.37.0) — Phases 1–4
> **Supersedes:** [`2026-07-29-remote-access-architecture-design.md.superseded`](designs/deferred/2026-07-29-remote-access-architecture-design.md.superseded) and [`2026-07-29-managed-relay-architecture.md`](designs/deferred/2026-07-29-managed-relay-architecture.md) (both in `designs/deferred/`).
> **Reference project:** [`orca`](https://github.com/stablyai/orca) — cloned at `/Volumes/MERCURY/PROJECTS/orca` (MIT). Joins `lobehub-analysis/` as a living reference.
> **Scope:** How Oxios is securely reached from any device (native mobile companion + browser), and how multi-purpose agents (coding / Q&A / longform) are expressed as personas with role-specific UI.

---

## 0. TL;DR

Oxios is **local-first** and stays that way — no cloud SaaS, no oxios-operated relay. What this RFC adds:

1. **A `RemoteRpcSurface`** — a new kernel-connected Surface that exposes the daemon over an E2EE WebSocket (Noise_XX), reached directly over **LAN and Tailscale** (no cloud relay). Native mobile clients pair via a **QR code** that bundles a device token + pinned server public key.
2. **A React Native / Expo native companion app** (the orca pattern) — pair → host list → session → chat + optional terminal + agent status. The transport ports orca's proven TS (parallel endpoint race, logical-client migration, hysteresis).
3. **Multi-agent = Persona extension.** Coding / daily-Q&A / novel-writing are personas. A new `capabilities` field drives role-specific **UI capability packs** on one shared chat substrate — not a separate coding app.
4. **Worktree fan-out** for coding — one prompt → N agents in N git worktrees → compare → merge winner (orca's killer feature, mapped onto Oxios's multi-agent + project system).

**Key architectural advantage over orca:** orca's runtime lives inside an Electron desktop shell; the mobile/web clients are companions to *that shell*. Oxios's daemon **is** the runtime. There is no desktop shell to build — the browser is the desktop client, the native companion is the mobile client, both speak to the same Rust daemon. Oxios also needs none of orca's PTY-CLI-wrapping: its native `oxi-sdk` agent emits structured tool-call/diff events directly to the `EventBus`.

---

## 1. Background — Orca reference analysis

Orca is an Electron "AI Orchestrator": it runs third-party CLI agents (Claude Code, Codex, …) side-by-side in parallel git worktrees, with a React Native mobile companion and SSH/headless remote modes. Three parallel scouts analyzed the full 238 MB tree; the distilled lessons follow.

### 1.1 Connection model (the part Oxios borrows most)

Orca fronts **three independent transports** with **one logical RPC client**:

```
                 orca CLOUD (stablyai infra)            REMOTE BOX (SSH worktree)
           director(assignment) + cells(data plane)     node relay.js (standalone daemon)
                │  OAuth join                │                │ SSH exec
  ┌─────────────┴────────────────────────────┴───────┐         │
  │ DESKTOP (Electron main) = the runtime            │         │
  │  OrcaRuntimeRpcServer  ws://0.0.0.0:6768         │         │
  │  - persistent Curve25519 keypair                 │         │
  │  - DeviceRegistry (deviceToken)                  │         │
  │  - DesktopRelayService (cloud relay broker)      │         │
  └──────┬────────────────────────┬──────────────────┘         │
  LAN    │  Tailscale             │   direct (ws/wss)           │
  ws://192.168.x      ws://100.x / *.ts.net                    │
         └────────────┬───────────┘                            │
  ┌───────────────────┴──────────────────────────────────────┐ │
  │ MOBILE / WEB CLIENT — one logical session                 │ │
  │  paths: 'lan' │ 'tailscale' │ 'relay'                     │ │
  │  parallel endpoint race → first authenticated wins         │ │
  └───────────────────────────────────────────────────────────┘┘
```

- **Pairing = a trust-seed bundle.** `orca://pair?code=<base64url JSON {endpoint, deviceToken, publicKeyB64, relay?}>>`. The QR carries everything: token for auth, pinned key for E2EE MITM protection. No passwords.
- **E2EE.** Curve25519 ECDH + XSalsa20-Poly1305, per-session HKDF key schedule, strict-counter framing (v2). Works identically on plain-LAN WiFi and Tailscale — and needs **no TLS certificate management** (encrypts over plain `ws://`).
- **Path racing + hysteresis.** Probe all endpoints in parallel; first E2EE-authenticated wins; require 3 consecutive successes / 30 s before committing; min dwell 60 s; failure cooldown 60 s before relay fallback. Foreground/background policy (suspend relay probes when backgrounded).
- **Headless `serve`.** Binds wide (`0.0.0.0:6768`), advertises a *separate* endpoint (prefers `tailscale ip -4` → hostname), prints a one-line JSON readiness contract. Paired devices reconnect after upgrade **without re-pairing** (token persisted).
- **Tailscale is opportunistic, never a hard dep.** Detected by address shape (`100.64.0.0/10` CGNAT, `*.ts.net` MagicDNS, `fd7a:115c:a1e0::/48`) and `tailscale ip -4`. No SDK.
- **The cloud relay is stablyai-operated infra** (director + cells). This is the one part Oxios **does not adopt** (see §2).

### 1.2 UI/UX

- **"Quiet chrome," monochrome.** The shell recedes to frame the tools (Monaco, xterm, markdown). Token system via `@theme inline` CSS variables.
- **Typed-block transcript.** `NativeChatMessage { role, blocks: [text | tool-call | tool-result | image-ref] }`. Tool runs fold *under* prose. This is the substrate Oxios's `chat.ts` extends.
- **Three clients, one runtime.** Desktop / web / mobile share a runtime; terminal uses xterm + WebGL + `SerializeAddon` scrollback snapshot/replay shared across all clients (reconnect resumes the buffer).
- **Agent cards.** `DashboardAgentRow` — state dot, time-ago, unvisited-bold, child-agent chevron — in a kanban drawer and inline per worktree.

### 1.3 Orchestration (where Oxios diverges)

- Orca spawns **dumb CLI agents as PTY subprocesses** and reads structured status back over a loopback HTTP "agent hook" server (`x-orca-agent-hook-token` auth). Diffs/file-edits are **not** in hooks — they come from the **git layer** (`git diff/status`).
- **Oxios needs none of this.** Its `oxi-sdk` agent runtime emits structured tool-call / diff / status events straight onto the `EventBus` and `agent_log`. The hook protocol and PTY-scraping are pure overhead for Oxios. What Oxios borrows is the *shell*: the runtime graph, status-event semantics, worktree fan-out, and the companion-driven control plane.

### 1.4 Pattern → Oxios (the adoption ledger)

| Orca pattern | Oxios action |
|---|---|
| 1 logical client over swappable transports (`stable-logical-rpc-client.ts`) | Port to the companion transport layer |
| Parallel endpoint race + hysteresis (`mobile-direct-endpoint-probe.ts`, `-hysteresis.ts`) | Adopt verbatim (Tailscale + LAN) |
| Recovery-gated backoff (`mobile-relay-reconnect-controller.ts`) | Adopt |
| QR pairing offer as capability bundle | Adopt (`oxios://pair`) |
| E2EE per-session HKDF + counter framing | Adopt shape; implement as **Noise_XX** (see §6.3) |
| Outbound WS backpressure queue | Adopt |
| Bound-vs-advertised endpoint split for `serve` | Adopt |
| Credential current+grace bundle + resume confirmation | Defer (no relay → simpler: device token is long-lived) |
| SSH relay daemon with socket-bridged grace | **Out of scope** (future: remote-box execution) |
| Typed-block transcript + folded tool runs | Extend `chat.ts` |
| DashboardAgentRow cards | Adopt for agent/worktree views |
| xterm + SerializeAddon scrollback shared across clients | Adopt for the (optional) coding terminal |
| PTY + agent-hook server | **Rejected** — Oxios has native agents |

---

## 2. Relationship to prior Oxios design work

Two prior remote-access designs exist in `docs/designs/deferred/`. This RFC **supersedes both**:

- **`2026-07-29-remote-access-architecture-design.md.superseded`** (Tailscale-Serve-as-default). Its **§2 (current state)** and **§3 (gap analysis)** remain valid and are inherited here (§3). Its core decision — *daemon always binds loopback; external access via a TLS-terminating proxy in front* — is **preserved for the browser/SPA path**. The new element is a **direct E2EE surface for the native companion** that does not depend on `tailscale serve`.
- **`2026-07-29-managed-relay-architecture.md`** (oxios-operated cloud relay + Noise_XX E2E + OAuth broker). **Rejected by explicit decision** in this session: the user chose **Tailscale + LAN direct, no oxios-operated relay**. Rationale: a personal Agent OS should not carry relay-infrastructure ops cost (director + cells, Cloudflare Workers/Durable Objects, cost ceiling, OAuth broker) when Tailscale already gives secure "anywhere" reach for a single user. The managed-relay doc is **kept parked** in `deferred/` as a future option if zero-setup non-tailnet access ever becomes a priority; its Noise_XX crypto choice is **inherited** here (§6.3).

**What changes vs both priors:** neither anticipated a *native mobile companion* nor *QR pairing*. Both were SPA/proxy-centric. This RFC adds the companion + pairing layer and the multi-agent/persona UX on top of the inherited local-first foundation.

---

## 3. Current state (Oxios) — inherited from prior §2, verified

| Aspect | Today |
|---|---|
| Gateway bind | `127.0.0.1:4200` (loopback default); non-loopback bind emits a security warning |
| Auth | `Authorization: Bearer <token>`; tokens SHA-256 hashed in `~/.oxios/api-keys.json`; first-boot auto-issues a `default` key; `POST /api/auth/issue` is **loopback-only** |
| WebSocket / SSE | `?ticket=` (short-lived) / `Authorization: Bearer` query/header |
| TLS | **None** — plain HTTP (acceptable loopback; unsafe remote) |
| CORS | localhost-only by default; SPA uses relative URLs (same-origin) |
| Tailscale | **Zero code references** — configured at OS level only |
| Surfaces | `Surface` trait (`oxios-gateway/src/surface.rs`) doc-comment already names *"mobile control apps"* as a future surface. This RFC implements that. |
| Personas | `Persona { id, name, role, description, system_prompt, enabled, model, personality_traits }` (`persona/mod.rs`), persisted to `~/.oxios/state/personas/index.json` (schema_version 1), per-session `active_persona_id`. Roles today: `developer`, `qa`. |

---

## 4. Goals & non-goals

### Goals

- **G1.** A paired native companion (iOS/Android) reaches the Oxios daemon from anywhere on the same **tailnet or LAN**, over an E2EE channel, within ~30 s of pairing.
- **G2.** The browser remains a first-class desktop client (loopback HTTP, unchanged).
- **G3.** App-layer E2EE is mandatory for the companion surface — no plaintext, no self-signed-cert management, works on plain-LAN WiFi and Tailscale identically.
- **G4.** Coding / daily-Q&A / longform-writing are **personas** with role-specific UI capability packs on one shared chat substrate (OS/program philosophy).
- **G5.** Coding agents support parallel worktree fan-out: one prompt → N agents → compare → merge.
- **G6.** The daemon works fully offline (loopback) with no remote surface enabled. Remote access is **additive**.

### Non-goals (this RFC)

- **N1.** Oxios-operated cloud relay. (Rejected; parked in `deferred/`.)
- **N2.** A Tauri/Electron desktop shell. The browser is the desktop client.
- **N3.** Multi-user tenancy. One user, many devices.
- **N4.** SSH-worktree remote-box execution. Future RFC (borrows orca's `relay.js` daemon pattern).
- **N5.** Replacing the existing loopback Bearer auth. Companion auth is an additive layer.
- **N6.** A file browser in the coding UI. Modern coding agents don't need one — refined chat + diff affordances suffice (user directive).

---

## 5. Architecture

### 5.1 Daemon = runtime (the Oxios advantage)

```
┌─────────────────────────────────────────────────────────────────────┐
│ OXIOS DAEMON (Rust) — single source of truth / runtime               │
│  kernel + orchestrator + AgentRuntime(oxi-sdk) + persona + exec      │
│                                                                       │
│  ┌─────────────────────────┐   ┌──────────────────────────────────┐  │
│  │ WebSurface (EXISTING)   │   │ RemoteRpcSurface (NEW)            │  │
│  │  axum HTTP, loopback    │   │  E2EE WebSocket (Noise_XX)        │  │
│  │  :4200, Bearer auth     │   │  bind: loopback → widen on pair   │  │
│  │  serves embedded SPA    │   │  device keypair + DeviceRegistry  │  │
│  │  (+ tailscale serve     │   │  QR pairing offer                 │  │
│  │   proxy path, unchanged)│   │  RPC method set (§6.5)            │  │
│  └────────────▲────────────┘   └──────▲──────────────▲─────────────┘  │
└───────────────┼───────────────────────┼──────────────┼────────────────┘
        browser │ (desktop client)      │ LAN direct   │ Tailscale direct
   http://127.0.0.1:4200            ws://192.168.x   ws://100.x / *.ts.net
   (or https via tailscale serve)         └──────┬──────┘
                                          ┌───────┴────────────────────┐
                                          │ NATIVE COMPANION (RN/Expo) │
                                          │  QR pair → E2EE → logical  │
                                          │  RPC client, path race     │
                                          └────────────────────────────┘
```

Both surfaces implement the `Surface` trait and receive `Arc<KernelHandle>`. They coexist: a daemon can run the WebSurface alone (today's default), the RemoteRpcSurface alone, or both.

### 5.2 Why two listeners, not one widened bind

The prior design was right to keep the HTTP API on loopback (Bearer tokens, no TLS). The companion needs a **direct** E2EE socket that is not behind `tailscale serve` (which is optional and proxy-only). Keeping them separate:

- preserves the loopback HTTP security model verbatim (no `0.0.0.0` widening of the token-bearing API);
- lets the companion surface **refuse all plaintext** — only Noise-authenticated frames are accepted, so a widened bind leaks nothing;
- allows independent ports, bind policy, and lifecycle (disable remote without touching the web UI).

### 5.3 Surface trait fit

`RemoteRpcSurface` implements `Surface` (`name() = "remote"`, `start(ctx) -> SurfaceHandle`). It owns its axum/tungstenite listener and a background task per companion session. Like the web surface it may optionally return a channel for gateway message routing, but its primary role is the control-plane + streaming RPC.

---

## 6. RemoteRpcSurface contracts (daemon side)

These contracts are normative. Each numbered item is independently testable.

### 6.1 Device identity & key storage

- The daemon generates a persistent **Ed25519 + X25519** keypair at first `--remote` enablement, stored keychain-wrapped at `~/.oxios/state/remote-identity.json` (mode 0600). The X25519 static half is the Noise static key; the Ed25519 fingerprint is the device id. Plaintext bytes live only in memory. (Inherits prior managed-relay Contract 7.1–7.2.)
- A **DeviceRegistry** (`~/.oxios/state/devices.json`) holds paired device tokens: `{ device_id, token_hash, name, scope, paired_at, last_seen }`. Tokens are SHA-256 hashed at rest. A device is *paired* by presenting a one-time offer (§6.2), then issued a long-lived device token.

### 6.2 QR pairing offer

`oxios://pair?code=<base64url(json)>` where the JSON offer is:

```json
{
  "v": 1,
  "endpoint": "ws://100.64.1.20:6768",
  "deviceId": "<hex Ed25519 pubkey fingerprint>",
  "publicKeyB64": "<base64 X25519 static pubkey, PINNED>",
  "endpoints": ["ws://100.64.1.20:6768", "ws://192.168.1.20:6768"],
  "scope": "mobile"
}
```

- `publicKeyB64` is the pinned server static key. The companion verifies the Noise responder's static key byte-equals this pin → MITM protection.
- `endpoints` lists every reachable direct URL (Tailscale first, then LAN IPs) so the companion can race them.
- The offer is short-lived and single-use: presenting it mints a device token and invalidates the offer. (No 10-min relay invite is needed without a relay.)
- `oxios serve --remote --pairing-address <host>` prints the QR + a one-line JSON readiness contract (endpoint, bound vs advertised, deviceId, pairing URL). Pairing-address selection prefers `tailscale ip -4` → OS hostname → explicit override; it **never** changes the bind, only the advertised endpoint (orca's bound-vs-advertised split).

### 6.3 Noise session (E2EE)

- **`Noise_XX_25519_ChaChaPoly_SHA256`** via the [`snow`](https://crates.io/crates/snow) crate. Initiator = companion; responder = daemon. XX gives mutual authentication and a DH-derived transport key. (Inherits the prior managed-relay Contract 7.4; preferred over orca's bespoke ECDH+HKDF because Noise is a standard, audited pattern with mature Rust tooling. orca's QR-pairing + pinning shape is still the UX inspiration.)
- Transport messages are AEAD (ChaCha20-Poly1305) with implicit counters; re-key every 2¹⁶ messages or 1 h. Replay rejected.
- The daemon **refuses non-Noisy frames** on this listener. A plaintext connect is closed immediately.

### 6.4 WebSocket frame format

Binary, single-frame-per-message (orca/prior shape):

```
┌──────────┬─────────────┬─────────────────┐
│  type    │  size (BE)  │  payload        │
│  1 byte  │  4 bytes    │  size bytes     │
└──────────┴─────────────┴─────────────────┘
```

`type`: `0x01` Noise handshake bytes · `0x02` encrypted app frame (Noise transport message) · `0x03/0x04` ping/pong (30 s) · `0x05` close-reason. Max payload 64 KiB; larger app payloads split at the RPC layer and reassemble on the far side. An **outbound backpressure queue** (orca's `ws-outbound-backpressure-queue.ts` pattern) parks frames above an 8 MiB soft cap and force-reconnects past 64 MiB / 4096 frames.

### 6.5 RPC method set (Oxios-native, not orca's)

After Noise decrypt, the app frame is a JSON-RPC envelope over the **same handler chain** the local server uses where possible. Streaming methods use subscriptions (server-pushed frames until unsubscribed), mirroring orca's `terminal.subscribe` model.

| Method | Direction | Purpose |
|---|---|---|
| `status.get` | req→resp | host status, paired devices, protocol version (version-gate like orca's `MOBILE_PROTOCOL_VERSION`) |
| `session.list` / `session.create` | req→resp | list/create sessions; a session carries `active_persona_id` |
| `persona.list` / `persona.activate` | req→resp | enumerate agent types; switch persona (§8) |
| `chat.send` | req→resp | enqueue a user message into a session |
| `chat.subscribe` / `chat.unsubscribe` | sub | streaming transcript (typed-block deltas, tool events, thinking) |
| `agent.status` | sub | agent lifecycle events (working/blocked/waiting/done) |
| `terminal.subscribe` *(capability-gated)* | sub | PTY stream (coding persona only) |
| `worktree.list` / `worktree.create` *(capability-gated)* | req→resp | parallel fan-out targets |
| `events.subscribe` | sub | EventBus fan-out (approvals, task updates) for live control |

The transcript stream reuses the existing block-stream transparency pipeline (RFC-015 / unified streaming). Chat content, tool I/O, file paths, and any API keys are **inside** the Noise session — the wire never sees plaintext.

### 6.6 Endpoint discovery, race, hysteresis (daemon advertises; companion races)

The daemon enumerates its reachable direct URLs at pairing time: Tailscale (`tailscale ip -4`, MagicDNS `*.ts.net`), and non-loopback LAN IPs (skip `100.64.0.0/10` duplicates). These populate the offer's `endpoints[]`. The companion (§7) races them in parallel; the daemon is stateless about which won. Path classification reuses orca's `isTailscaleEndpoint` shape (CGNAT `100.64.0.0/10`, `*.ts.net`, `fd7a:115c:a1e0::/48`).

### 6.7 Security model

- **E2EE mandatory.** No build flag disables Noise. A test asserts relay/snarf-seen bytes are indistinguishable from random.
- **Loopback HTTP unchanged.** The Bearer-token API stays on `127.0.0.1:4200`; remote browsers use `tailscale serve` (TLS proxy) as before.
- **Pairing is the only trust bootstrap.** No OAuth, no accounts — the QR offer is presented physically (on-screen / `serve` stdout), so the user's ability to scan it is the proof of locality.
- **Revocation.** `DELETE /api/devices/{id}` (loopback) drops the device token; the next companion connect fails Noise auth and must re-pair.
- **Audit.** Every companion connect/disconnect, RPC method, and revocation is logged through the existing `AuditTrail` (AccessManager). Metadata only — payload is already inside Noise.

---

## 7. Native companion (React Native + Expo)

### 7.1 Stack & structure

React Native + Expo (Expo Router, file-based nav). The transport layer ports orca's TypeScript nearly verbatim — it is already battle-tested for exactly this problem.

```
companion/
├── app/                      # Expo Router screens
│   ├── _layout.tsx           # root nav
│   ├── index.tsx             # paired hosts list
│   ├── pair-scan.tsx         # QR scan
│   ├── pair-confirm.tsx      # E2EE handshake + endpoint race
│   ├── h/[hostId].tsx        # sessions for a host (per-persona)
│   └── session/[id].tsx      # chat + optional terminal overlay + agent status
└── src/
    ├── transport/            # ← ported from orca/mobile/src/transport
    │   ├── stable-logical-rpc-client.ts
    │   ├── direct-endpoint-probe.ts        # parallel race
    │   ├── endpoint-supervisor.ts          # hysteresis + foreground/background
    │   ├── reconnect-controller.ts         # recovery-gated backoff
    │   ├── e2ee-session.ts                 # Noise_XX client (snow-wasm or lib)
    │   └── connection-health.ts            # verdicts + Tailscale hint
    └── terminal/             # xterm in react-native-webview (capability-gated)
        ├── terminal-webview.tsx
        └── serialize-replay.ts             # scrollback snapshot/replay
```

### 7.2 Transport (ported from orca, adapted)

- **One logical client over swappable physical transports** — `migrateTo()` auth-gates the replacement, replays subscriptions before cutting over, generation-fences callbacks, closes the old session only after replay.
- **Parallel endpoint race** — open one WS per direct URL in parallel, first Noise-authenticated wins, losers closed.
- **Hysteresis** — 3 consecutive direct successes / 30 s observation, 60 s min dwell, 60 s failure cooldown before falling back (with no relay, "fallback" = retry/backoff, not a relay path).
- **Recovery-gated backoff** — full-jitter 250 ms–30 s; states that cannot self-heal on a timer (auth revoked) stop polling and wait for re-pair.
- **Connection-health verdicts** — an unreachable `100.x` / `*.ts.net` endpoint renders *"Can't connect — check Tailscale"* (orca's exact UX).
- **Foreground/background** — suspend probes when backgrounded; reset + immediate probe on foreground (Android Doze / iOS TCP-kill recovery).

### 7.3 Noise in JS/RN

orca uses `tweetnacl` (Curve25519 + XSalsa20-Poly1305). Oxios standardizes on **Noise_XX**; the companion needs a Noise implementation. Options: (a) `noise-lib`/`chirp-noise` JS bindings, (b) a tiny shared Rust `noise-handshake` compiled to a Hermes-compatible Wasm/JSI module. **Decision deferred to Phase 2** — the wire format (§6.3–6.4) is fixed; the client crypto lib is an implementation choice.

### 7.4 Terminal (optional, coding-persona only)

xterm.js hosted in `react-native-webview`, behind a `SerializeAddon` scrollback snapshot/replay shared with the daemon side, with write coalescing. Reconnect resumes the buffer. Gated behind the persona's `terminal` capability — Q&A and writing personas never show it.

---

## 8. Multi-agent via Persona + UI capability packs

### 8.1 The model

A "program" (agent type) **is** a persona. The user's OS/program intuition maps directly onto the existing `Persona` system. One persona is active per session; switching persona = switching agent type.

### 8.2 Persona schema extension

Add one field to `Persona` (`persona/mod.rs`), backward-compatible:

```rust
/// UI capability flags this persona enables on the chat substrate.
/// Drives role-specific affordances (terminal, diff-viewer, approval-cards,
/// worktree-fanout, longform-editor, outline, web-search, ...).
#[serde(default)]
pub capabilities: Vec<String>,
```

Bump `persistence.rs` `schema_version` to 2 (old files load with empty `capabilities`). A persona's full definition is then: `system_prompt` + `model` + `allowed_tools` (via security config) + `capabilities`.

### 8.3 Built-in personas (ship as defaults)

| id | role | capabilities | model lean |
|---|---|---|---|
| `coder` | developer | `terminal`, `diff-viewer`, `approval-cards`, `worktree-fanout`, `exec` | strong reasoning |
| `assistant` | assistant | `web-search`, `quick-facts` | balanced |
| `writer` | writer | `longform-editor`, `outline` | large context |
| `dev` / `qa` | *(existing)* | *(existing developer/QA, kept)* | — |

### 8.4 One chat substrate + role-aware chrome

The transcript renderer inspects the active session's persona `capabilities` and renders extra affordances — **not** a separate app/route. The user's directive (modern coding agents don't need a file browser; refined chat suffices) is honored:

- **Coding** (`coder`): tool calls fold under prose; a tool-call card renders an inline **diff viewer** (syntax-highlighted) for edit/exec tools; a **terminal toggle** appears; **approval prompts** inline; a **"fan out to N agents"** action in the composer.
- **Writing** (`writer`): a **longform editor** panel alongside chat; a **chapter outline** sidebar.
- **Q&A** (`assistant`): clean conversational chat + web-search result cards.

This keeps a single `chat.ts` substrate with capability-driven plugin regions — extensible without forking the chat UX per role.

### 8.5 Worktree fan-out (coding)

orca's headline feature, mapped onto Oxios's existing multi-agent + project system:

1. In a project context, the composer's "fan out" action takes a prompt + count N.
2. The daemon creates N git worktrees under the project and spawns N agents (within `max_agents`), each with the `coder` persona, each in its own worktree.
3. Each agent streams its transcript + status to its own `DashboardAgentRow`-style card (state dot, time-ago).
4. On completion, a **compare** view diffs the N branches; the user merges the winner (or cherry-picks).

This reuses `AgentLifecycleManager` (A2A, scheduling) + the git layer (`git addWorktree` etc., RFC-013). No new orchestration primitive — just a fan-out driver over existing ones.

---

## 9. Implementation phasing

Each phase is independently shippable and verifiable.

### Phase 1 — RemoteRpcSurface (daemon, Rust)
Device keypair + keychain wrap, DeviceRegistry, Noise_XX WS listener (snow), QR offer generation, endpoint enumeration, `--remote --pairing-address` serve mode, the RPC method set stubs. Verified with a CLI/script client (no companion yet). **Exit:** a paired script can `chat.send` + `chat.subscribe` over E2EE from another machine on the tailnet.

### Phase 2 — Native companion MVP (RN/Expo)
Pair → host list → session → chat (typed-block transcript) → agent status. Ported transport (race/hysteresis/backoff). Noise client crypto. Connection-health UX. **Exit:** real coding-from-phone over chat (no terminal yet) from anywhere on the tailnet/LAN.

### Phase 3 — Persona capability packs + coding UX
`capabilities` field + schema bump; built-in personas; coding capability pack (diff viewer, terminal overlay, approval cards); writing pack (longform editor, outline). Refine transcript rendering (folded tool runs). Both web SPA and companion render capability packs. **Exit:** switching persona changes available affordances on both clients.

### Phase 4 — Worktree fan-out + advanced coding
Fan-out driver, per-agent cards, compare/merge view, diff annotation (drop comments on diff lines → feed back to agent). **Exit:** one prompt → N agents → merge winner, end-to-end.

---

## 10. Security model summary

| Boundary | Mechanism |
|---|---|
| Companion ↔ daemon | Noise_XX E2EE (ChaCha20-Poly1305); daemon refuses plaintext |
| Browser ↔ daemon (desktop) | loopback HTTP + Bearer; or `tailscale serve` TLS proxy (unchanged) |
| Pairing trust bootstrap | physical QR presentation (on-screen / `serve` stdout) |
| Device revoke | loopback `DELETE /api/devices/{id}` → next connect fails Noise |
| Audit | companion connect/disconnect/RPC via existing `AuditTrail` |
| Loopback API | never widened; `0.0.0.0` still warned |
| Replay | Noise transport counters |

**What is NOT done:** no oxios-operated relay, no OAuth, no cloud state, no TLS cert management in the daemon, no multi-user.

---

## 11. Open questions

1. **Noise client lib for RN.** tweetnacl-based bespoke (orca style) vs a Noise_XX JS/JSI binding. *Defer to Phase 2 spike; wire format is fixed.*
2. **Companion repo location.** Separate repo (`oxios-companion`) vs monorepo subdir (`companion/`). *Recommendation: monorepo subdir `companion/`, mirroring how `web/` lives in-tree.*
3. **`tailscale_auth` identity-header trust (prior §5.1).** Still a worthwhile *small, local-only* improvement for the browser/`tailscale serve` path, independent of the companion. *Recommendation: bundle into Phase 1 as an optional enhancement; it does not require the companion.*
4. **Terminal PTY on the daemon.** Oxios's `exec` is structured (allowlisted commands), not arbitrary PTY. The `terminal.subscribe` capability either (a) wraps a bounded PTY within exec allowlist rules, or (b) exposes structured tool output only. *Recommendation: (b) first — the "refined chat" form — and add a real PTY only if coders ask; defer to Phase 3 decision.*
5. **Protocol versioning.** Adopt orca's `MOBILE_PROTOCOL_VERSION` / `MIN_COMPATIBLE_DESKTOP_VERSION` exchange on `status.get` so an incompatible companion hard-blocks instead of misbehaving. *Recommendation: yes, Phase 1.*
6. **Worktree fan-out vs `max_agents`.** Fan-out of N must respect the configured agent ceiling; queue or cap N. *Recommendation: cap N at `max_agents` minus reserved, Phase 4.*

---

## 12. Acceptance criteria

- A fresh Mac + paired iPhone reaches the daemon from the phone over the tailnet in < 60 s after `oxios serve --remote`, with chat working end-to-end.
- No byte of `~/.oxios/` content transits the wire in plaintext (test asserts snarf-seen bytes indistinguishable from random across 10k frames).
- `oxios serve` (no `--remote`) works exactly as before; the loopback HTTP API is unchanged.
- A revoked device token is rejected on next connect; re-pair required.
- Switching persona to `coder` enables the diff viewer + terminal toggle on both web and companion; switching to `writer` enables the longform editor.
- Worktree fan-out produces N agents in N worktrees, each streaming status, with a compare/merge view.

---

## Appendix A — Orca → Oxios file-level pointers

| Concept | Orca source | Oxios target |
|---|---|---|
| Logical RPC client | `mobile/src/transport/stable-logical-rpc-client.ts` | `companion/src/transport/` |
| Endpoint race | `mobile/src/transport/mobile-direct-endpoint-probe.ts` | port |
| Hysteresis | `mobile/src/transport/mobile-endpoint-hysteresis.ts` | port |
| Reconnect/backoff | `mobile/src/transport/mobile-relay-reconnect-controller.ts` | port |
| Pairing offer encode | `src/shared/pairing.ts`, `mobile-relay-pairing-offer.ts` | daemon + companion |
| E2EE framing | `src/shared/mobile-e2ee-v2-framing.ts` | daemon (`snow`) + companion |
| WS backpressure | `src/shared/ws-outbound-backpressure-queue.ts` | daemon + companion |
| Bound vs advertised | `src/main/runtime/pairing-endpoint.ts` | daemon serve mode |
| Tailscale detection | `src/shared/remote-runtime-tailscale-hint.ts`, `tailnet-address.ts` | daemon + companion |
| Typed-block transcript | `NativeChatMessage` (renderer) | extend `web/src/stores/chat.ts` |
| Agent status cards | `DashboardAgentRow` | web + companion agent views |
| xterm scrollback | `SerializeAddon` snapshot/replay | companion terminal |
| Worktree fan-out | `orca-runtime.ts createManagedWorktree` | daemon + `AgentLifecycleManager` |

## Appendix B — Explicitly rejected from orca

- **Cloud relay** (director + cells) — user decision; no oxios-operated infra.
- **PTY + agent-hook server** — Oxios has native `oxi-sdk` agents; no CLI-wrapping.
- **SSH relay daemon** (`relay.js`) — future RFC for remote-box execution.
- **Electron desktop host** — Oxios daemon is the host; browser is the desktop client.
