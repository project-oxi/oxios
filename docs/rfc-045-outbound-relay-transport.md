# RFC-045 — Outbound Relay Transport for Zero-Setup Remote Access

> **Status:** REJECTED. 2026-08-09. Maintainer decided to stay with the RFC-044 Tailscale/LAN-direct approach. Rationale: for a personal, single-user Agent OS, Tailscale already gives secure "anywhere" reach at zero infra cost; a relay only beats Tailscale when Oxios operates it (the ops cost RFC-044 explicitly rejected) or in device-restriction edge cases. Self-hosted relay is strictly more setup than Tailscale on the very "zero-setup" axis it claims to win. Design + self-review kept as a record so the relay question isn't re-litigated without this analysis. If Oxios is ever distributed to users who can't/won't run Tailscale, revisit.
> **Trigger:** Paseo relay analysis ([`paseo-relay-connection-analysis.md`](../paseo-relay-connection-analysis.md)) — Paseo's outbound-only zero-knowledge relay defeats NAT/firewall without Tailscale, and does so *without* OAuth/accounts.
> **Builds on:** [RFC-044](../rfc-044-remote-access-mobile-multiagent.md) (implemented `RemoteRpcSurface`: Noise_XX + QR pairing + device registry, LAN/Tailscale direct).
> **Un-suspends:** [`managed-relay-architecture.md`](../designs/deferred/2026-07-29-managed-relay-architecture.md) (deferred 2026-07-29) — but **drops its OAuth/Cloudflare/D1 complexity**, replacing it with Paseo's pairing-anchored zero-knowledge model.
> **Scope:** Add a third transport path (relay) to the existing remote companion surface, so a paired companion reaches the daemon from behind any firewall with zero VPN setup.

---

## 0. TL;DR

RFC-044 shipped `RemoteRpcSurface` with Noise_XX E2EE + QR pairing, but only over **direct** LAN/Tailscale paths. It **explicitly rejected** a cloud relay (N1) on the grounds that Tailscale suffices for a single user.

The Paseo analysis changed that calculus: Paseo's relay is **dramatically simpler** than the deferred Oxios managed-relay design (no OAuth, no JWT, no D1, no accounts). It pairs daemon↔companion by a public key carried in a QR fragment, and the relay is a stateless opaque byte-router. This RFC adds exactly that — as an **additive third path** alongside LAN and Tailscale — reusing RFC-044's already-implemented Noise_XX, pairing, and device registry.

**One sentence:** the daemon opens an *outbound* WebSocket to a relay; a paired companion meets it there; the relay forwards Noise-encrypted bytes it cannot read; the companion surface treats the relay socket identically to a direct socket.

---

## 1. Problem

A user wants to control their Oxios agents from a phone on LTE — at a café, on a trip. Today (RFC-044):

- The companion races LAN + Tailscale endpoints. This works **if both devices are on the same tailnet or LAN**.
- Off-network (phone on LTE, Mac at home behind NAT/CGNAT), neither path works without the user installing and joining Tailscale on every device.

The deferred managed-relay doc (2026-07-29) proposed a solution but coupled it to a heavyweight Oxios-operated Cloudflare stack: OAuth broker (GitHub/Google), JWT device tokens, D1 account/device storage, per-user Durable Objects. RFC-044 rejected the whole bundle as disproportionate ops cost.

**The gap is narrower than the deferred doc assumed.** Paseo demonstrates that the *relay* and the *identity infrastructure* are separable: the relay needs no accounts — it routes by a pairing-derived key, and E2EE (Noise) is the entire security boundary. We can add a relay **without** adding any of the OAuth/account machinery.

---

## 2. Goals & non-goals

### Goals

- **G1.** A paired companion reaches the daemon from any network (LTE, hotel WiFi, CGNAT) with zero VPN/client setup, within the existing `oxios serve --remote` flow.
- **G2.** Zero inbound port exposure on the host. The daemon opens only an *outbound* WebSocket to the relay. NAT, CGNAT, ISP firewalling are irrelevant — same property Paseo exploits.
- **G3.** The relay is zero-knowledge: it routes opaque Noise-encrypted bytes and cannot read payload, forge messages, or derive keys. This reuses RFC-044's Noise_XX (already stronger than Paseo's static ECDH — it has forward secrecy).
- **G4.** Reuse RFC-044's implemented primitives: `Responder` (noise.rs), `DeviceIdentity` (identity.rs), `DeviceRegistry` (devices.rs), `PairingOffer` (pairing.rs), `ConnectionCtx`/`OutboundQueue` (transport.rs), RPC dispatch (rpc.rs). No new crypto, no new auth model.
- **G5.** The relay is a **third path**, additive to LAN and Tailscale. The companion still races all available endpoints; relay is one candidate, not a replacement.
- **G6.** The host works fully offline (loopback) with no relay. Relay is opt-in.

### Non-goals (this RFC)

- **N1.** Oxios-operated OAuth / account system. The deferred doc's `auth.oxios.com` broker is **permanently dropped**. Pairing (the QR offer, RFC-044 §6.2) is the sole trust bootstrap — this is the Paseo lesson.
- **N2.** Multi-user tenancy. One daemon, many paired devices — same as RFC-044.
- **N3.** A managed/hosted relay as a hard dependency. The relay is self-hostable (`oxios-relay` binary). An Oxios-operated instance is a deployment decision, not an architecture one.
- **N4.** Changing the existing loopback Bearer auth or the WebSurface.
- **N5.** Replacing Tailscale. It remains the best-effort zero-latency path; relay is the zero-setup fallback.

---

## 3. What already exists (RFC-044, verified against source)

| Component | File | Reused? |
|---|---|---|
| Noise_XX_25519_ChaChaPoly_SHA256 responder | `src/remote/noise.rs` (`Responder`, `Transport`) | **As-is** — relay data sockets run the identical handshake |
| Persistent X25519 static keypair | `src/remote/identity.rs` (`DeviceIdentity`) | **As-is** — `device_id` = SHA256 fingerprint, becomes relay routing key |
| Paired-device registry (hashed tokens, revoke) | `src/remote/devices.rs` (`DeviceRegistry`) | **As-is** — companion still presents a device token post-handshake |
| QR pairing offer `oxios://pair?code=` | `src/remote/pairing.rs` (`PairingOffer`) | **Extended** — add optional `relay_endpoint` field |
| WS transport, `ConnectionCtx`, backpressure queue | `src/remote/transport.rs` | **Generalized** — extract the per-connection handler so it accepts any async byte stream, not just `TcpListener` |
| JSON-RPC 2.0 dispatch + subscriptions | `src/remote/rpc.rs` | **As-is** |
| `RemoteBridge` (Channel for gateway routing) | `src/remote/mod.rs` | **As-is** |
| Endpoint enumeration (Tailscale/LAN) | `src/remote/endpoints.rs` | **Extended** — relay endpoint joins the race |
| `Surface` impl | `src/remote/mod.rs` (`RemoteRpcSurface`) | **Extended** — `start()` launches relay client in addition to/instead of listener |

The decisive reuse: `transport.rs::handle_connection` currently takes a `TcpStream`. Its body does Noise handshake → `ConnectionCtx` → frame loop → RPC dispatch. **None of that is TCP-specific** except the stream type. The relay design's core move is to feed it a relay-backed stream instead.

---

## 4. Architecture

### 4.1 Topology — three paths, one surface

```
                        ┌──────────────────────────┐
                        │  Companion (RN/Expo/Web)  │
                        │  Noise_XX initiator       │
                        │  endpoint race:           │
                        │   relay │ tailscale │ LAN │
                        └──┬──────────┬──────────┬──┘
            relay path     │          │ direct   │ direct
        (zero-setup)       │          │(tailnet) │(LAN)
                           ▼          ▼          ▼
                  ┌──────────────┐   ┌────────────────────────┐
                  │  relay.oxios │   │  OXIOS DAEMON (Rust)    │
                  │  (or self-   │   │  RemoteRpcSurface       │
                  │   hosted)    │   │  ┌──────────────────┐   │
                  │              │   │  │ inbound listener │   │
                  │  zero-       │   │  │ (TCP, RFC-044)   │   │
                  │  knowledge   │   │  └──────────────────┘   │
                  │  byte router │   │  ┌──────────────────┐   │
                  │              │   │  │ relay client     │   │
                  │  keyed by    │   │  │ (NEW, outbound)  │   │
                  │  device_id   │   │  └─────────▲────────┘   │
                  └──────▲───────┘   └───────────┼────────────┘
                         │                      │ outbound WS (443)
                         └──────────────────────┘
```

The daemon opens **one outbound control WebSocket** to the relay. When a companion arrives, the relay signals the daemon over the control socket; the daemon opens a **per-companion data WebSocket** (also outbound), runs the Noise_XX responder on it, and feeds decrypted frames to the same RPC dispatch the inbound listener uses.

### 4.2 Why outbound defeats the firewall

Every consumer NAT and most corporate firewalls permit **outbound** HTTPS/WebSocket (443). They block **inbound** by default. By making the daemon the outbound connector, the relay traversal needs:

- no port forwarding,
- no public IP / DDNS,
- no UPnP,
- no Tailscale.

This is the single property that makes Paseo's remote access "just work," and it is the property RFC-044's direct-only design lacks. CGNAT (carrier-grade NAT, common on mobile and some ISPs) is fatal to inbound but irrelevant to outbound.

---

## 5. Relay protocol (the contract between daemon, relay, companion)

Two WebSocket roles, both connect **outbound to the relay**:

- **daemon** = the Oxios host (holds Noise static key, `device_id`)
- **client** = the companion (holds the pairing offer's pinned public key)

### 5.1 Routing key: `device_id`

The relay routes solely by `device_id` — the daemon's Noise static public key fingerprint (RFC-044 §6.1, `identity.rs::device_id()` = 16-byte hex of SHA256(pubkey)). This is Paseo's `serverId`, renamed to Oxios's existing identity primitive. The relay learns it from the WebSocket URL query string; it never appears inside the encrypted payload.

### 5.2 Connection lifecycle (v2 multiplexed, mirroring Paseo)

```
1. Daemon → Relay:  control WS
   wss://relay.oxios/ws?role=daemon&device_id=<hex>&v=2
   (one long-lived outbound connection; reconnect with backoff on drop)

2. Companion → Relay:  client WS
   wss://relay.oxios/ws?role=client&device_id=<hex>&connection_id=<relay-assigned>&v=2

3. Relay → Daemon (over control WS):  {"type":"connected","connection_id":"<id>"}

4. Daemon → Relay:  data WS
   wss://relay.oxios/ws?role=daemon&device_id=<hex>&connection_id=<id>&v=2
   (one outbound data socket per companion)

5. Both sides run Noise_XX on the data path:
   companion = initiator, daemon = Responder (RFC-044 §6.3, unchanged)
   handshake bytes are relayed opaquely as FrameType::Noise (0x01)

6. Post-handshake: AEAD app frames (FrameType::App 0x02) flow both ways,
   relayed opaquely. The relay cannot decrypt, forge, or meaningfully inspect.
```

**Why control + data split** (not a single merged socket, as Paseo v1):
- The control socket is cheap and long-lived; it only carries `connected`/`disconnected`/`sync` notifications.
- Each companion gets its own data socket + independent Noise session. One stuck companion cannot head-of-line-block another.
- The daemon can run a distinct Noise_XX responder per data socket, isolating keys per companion (matches RFC-044's per-connection `ConnectionCtx`).

### 5.3 Relay-visible messages (control plane, cleartext-over-TLS)

The relay produces/consumes only these JSON control messages over the daemon's control socket:

| Message | Direction | Purpose |
|---|---|---|
| `{"type":"sync","connection_ids":[...]}` | relay→daemon | on (re)connect, current live companions |
| `{"type":"connected","connection_id":"..."}` | relay→daemon | a companion arrived → open data socket |
| `{"type":"disconnected","connection_id":"..."}` | relay→daemon | companion gone → close data socket |

These carry **no user content** — only routing ids. The relay never sees Noise handshake payloads or app frames in a form it can interpret (they are binary WS messages it forwards without parsing).

### 5.4 What the relay forwards vs. produces

- **Forwards opaquely (binary WS messages):** Noise handshake bytes, AEAD app frames, ping/pong. The relay does `client_ws.send(msg)` / `data_ws.send(msg)` without deserializing. This is the zero-knowledge core.
- **Produces (control JSON):** only the routing notifications above.

This mirrors Paseo's `RelayDurableObject.webSocketMessage` exactly: tag by role+connectionId, forward to the peer set, never decode.

### 5.5 Daemon-side keepalive & half-open detection

The control socket can go half-open (NAT drops state, no close event). Paseo solves this with WS-protocol pings every 10 s + a 30 s staleness cutoff. We adopt the same:

- Ping every 10 s over the control WS (tokio-tungstenite supports WS-level ping/pong).
- If no pong within 30 s, terminate and reconnect with backoff (1 s → 30 s cap, ±20 % jitter).
- Reconnect re-runs `sync` so the daemon re-opens data sockets for companions still alive.

---

## 6. Components to add

### 6.1 `src/remote/relay_client.rs` (NEW) — daemon outbound relay client

Owns the control socket + per-companion data sockets. Structured to mirror Paseo's `relay-transport.ts`, in Rust.

```rust
pub struct RelayClient {
    relay_endpoint: String,      // "relay.oxios.sh:443"
    use_tls: bool,
    device_id: String,           // routing key = Noise static key fingerprint
    server_static: Arc<Vec<u8>>, // Noise responder key (identity.rs)
    rpc_handler: RpcFrameHandler,// reused from mod.rs::build_rpc_handler
    shutdown: CancellationToken,
}

impl RelayClient {
    /// Start the outbound control connection. Reconnects with backoff.
    /// On each "connected" notification, opens a data socket and runs
    /// the Noise_XX responder → ConnectionCtx → RPC dispatch, identical
    /// to transport.rs::handle_connection but over a relay WS stream.
    pub async fn run(self) -> Result<()>;
}
```

Internals:
- `connect_control()` — outbound `wss://` to relay, ping/pong keepalive, staleness watchdog, reconnect scheduler.
- `ensure_data_socket(connection_id)` — on `connected`/`sync`, open a per-companion outbound WS, then call a generalized `handle_relay_connection(ws_stream, server_static, rpc_handler)` (see 6.2).
- On `disconnected`, close the matching data socket + drop its Noise session.

### 6.2 Generalize `transport.rs::handle_connection` (MODIFY)

Today `handle_connection` takes `TcpStream`. Extract its core into a generic that accepts **any** async duplex byte stream:

```rust
/// Noise_XX handshake → ConnectionCtx → frame loop → RPC dispatch.
/// Stream-agnostic: works over TcpStream (direct) or a relay WS (relay).
async fn handle_stream<S>(
    stream: S,
    server_static: Arc<Vec<u8>>,
    shutdown: CancellationToken,
    handler: Arc<RpcFrameHandler>,
    audit: Option<Arc<dyn CompanionAudit>>,
    peer_label: PeerLabel,        // "direct:192.168.1.5" | "relay:conn_abc"
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static;
```

The existing `run_listener` becomes a thin wrapper that accepts `TcpStream`s and calls `handle_stream`. The relay client calls `handle_stream` with a relay-WS-backed stream adapter. **Zero behavioral divergence** between direct and relay paths — this is the transport-transparency property the Paseo analysis identified as the design's elegant core.

> The WS-over-relay case needs a small adapter wrapping `tokio_tungstenite::WebSocketStream<MaybeTlsStream<TcpStream>>` into `AsyncRead + AsyncWrite` that already speaks the `[type:1][size:4 BE][payload]` frame format from `noise.rs`. tungstenite gives us `Message::Binary`; the adapter (de)serializes `noise::FrameType` frames. This is ~80 lines.

### 6.3 `PairingOffer` extension (MODIFY `pairing.rs`)

Add an optional relay endpoint so the companion knows a relay path exists:

```rust
pub struct PairingOffer {
    pub v: u32,
    pub endpoint: String,
    pub device_id: String,
    pub public_key_b64: String,
    pub endpoints: Vec<String>,   // LAN + Tailscale direct URLs (existing)
    pub scope: String,
    pub relay_endpoint: Option<String>,  // NEW: "wss://relay.oxios.sh:443"
}
```

When relay is enabled, `endpoints[]` (direct) and `relay_endpoint` (relay) both populate. The companion races all of them; first Noise-authenticated wins (RFC-044 §6.6 race, unchanged). Schema bump `v: 1 → 2` (additive — old clients ignore the new field).

### 6.4 `RemoteRpcSurface::start` wiring (MODIFY `mod.rs`)

Today `start()` binds a `TcpListener` and calls `run_listener`. Extend it:

```rust
async fn start(&self, ctx: SurfaceContext) -> Result<SurfaceHandle> {
    // ... existing direct listener (LAN/Tailscale), gated on config ...

    if config.remote.relay_enabled {
        let relay = RelayClient { /* ... */ };
        handle.spawn(relay.run());  // outbound control + per-companion data
    }

    // Both paths feed the same build_rpc_handler(ctx) → same RPC, same gateway.
}
```

The surface can run: direct-only (today), relay-only, or both. Running both maximizes reach: LAN wins locally, Tailscale wins on-tailnet, relay wins off-network.

### 6.5 Config (MODIFY `config.toml` schema)

```toml
[remote]
enabled = true                  # RFC-044 (existing)
pairing_address = "100.64.1.20" # RFC-044 (existing)
relay_enabled = false           # NEW, default off (additive)
relay_endpoint = "relay.oxios.sh:443"  # NEW
relay_use_tls = true            # NEW
```

CLI: `oxios serve --remote --relay` flips `relay_enabled` (mirrors RFC-044's `apply_remote_overrides`).

---

## 7. Relay server (`oxios-relay`, NEW separate binary)

A small, self-hostable Rust binary. Stateless opaque byte-router keyed by `device_id`. No accounts, no database, no payload storage.

### 7.1 Why Rust, not Cloudflare

The deferred doc assumed Cloudflare Workers + Durable Objects + D1. For Oxios:
- **Rust-native stack.** The daemon, kernel, and all crates are Rust. A Rust relay (`axum` + `tokio-tungstenite`) is consistent, auditable in one language, and needs no Cloudflare account.
- **No account/database needed.** Paseo's insight: the relay routes by a pairing-derived key and forwards encrypted bytes. There is nothing to store. A `HashMap<device_id, Sockets>` in process memory is the entire routing table.
- **Self-hostable by default.** A user can run `oxios-relay` on a $5 VPS. An Oxios-operated instance is a deployment choice, not architecture.

### 7.2 Relay server sketch

```rust
// oxios-relay: stateless WS byte-router
// Routes by ?device_id= query param. Forwards binary frames opaquely.
struct RelayState {
    // device_id → { control: Option<Ws>, clients: HashMap<conn_id, Ws> }
    sessions: DashMap<String, Session>,
}

async fn handle_ws(ws: WebSocket, query: RelayQuery, state: Arc<RelayState>) {
    match (query.role, &query.connection_id) {
        (Daemon, None)    => register_control(ws, query.device_id, state).await,
        (Daemon, Some(id))=> register_data(ws, query.device_id, id, state).await,
        (Client, id)      => register_client(ws, query.device_id, id, state).await,
    }
}
// Each role forwards binary Message frames to the paired peer set, verbatim.
// Control JSON (sync/connected/disconnected) is produced by the relay itself.
```

- **Routing table**: in-memory `DashMap<device_id, Session>`. Lost on restart — acceptable; daemons reconnect and re-sync within seconds.
- **TLS**: terminated by a reverse proxy (Caddy/nginx) or native rustls. The Noise session is independent of TLS (defense in depth, per RFC-044 §6.3 — Noise works on plain `ws://`).
- **No persistence, no logging of payload.** Access logs carry only `device_id`, timestamps, byte counts — metadata only, matching RFC-044 §6.7 audit discipline.

### 7.3 Deployment options

| Option | Who runs it | Notes |
|---|---|---|
| Self-hosted VPS | the user | `oxios-relay` binary behind Caddy. Full control. |
| Oxios-operated | oxios team | `relay.oxios.sh`. Convenience default. Architecture-identical. |
| Cloudflare DO | future | If scale demands; the Rust relay is the reference, a DO port is mechanical (Paseo proved it). |

---

## 8. Trust model & security analysis

### 8.1 Trust boundaries

```
untrusted:   Internet, ISP, transit
               ↓ TLS (relay endpoint)
semi-trusted: relay.oxios  — sees device_id, timing, sizes, IP; CANNOT read/forge/replay
               ↓ opaque Noise frames
trusted:      Oxios daemon — Noise static key, ~/.oxios/*, agent runtime
trusted:      Companion    — pinned daemon pubkey (from QR), ephemeral Noise init key
```

This is **structurally identical to Paseo's** (SECURITY.md), but with a *stronger* E2EE primitive: Noise_XX provides **forward secrecy** and **mutual authentication**, whereas Paseo's static ECDH + NaCl box does not. Oxios inherits this advantage from RFC-044's already-implemented choice.

### 8.2 What the relay cannot do

| Attack | Why it fails |
|---|---|
| Read traffic | All post-handshake frames are Noise AEAD (ChaCha20-Poly1305). Relay sees ciphertext only. |
| Forge commands | Noise provides authenticated encryption; tampered frames fail decrypt → fatal close. |
| Impersonate daemon | Without the daemon's Noise static secret, the relay cannot complete the XX responder role. The companion pins the daemon pubkey from the QR offer (RFC-044 §6.2) → MITM detected. |
| Replay across sessions | Each companion connection derives fresh Noise transport keys (XX DH). Ciphertext from one session fails in another. |
| Switch keys mid-session | The daemon's Noise responder is bound to its static key. A relay attempting to substitute a different peer gets a handshake that doesn't match the companion's pinned key → failure. |

### 8.3 What the relay CAN see (and what we do about it)

- **Metadata:** `device_id`, connection timing, message sizes, source IPs. This is inherent to any relay. Mitigations: size is partially hidden by the 64 KiB frame cap + app-layer chunking; timing is inherent. No padding in v1 (Paseo also omits it); noted as a future option.
- **Online/offline status:** the relay knows when a daemon is connected. This is necessary for routing. Acceptable for a personal relay; the self-hosted option removes even this from a third party.

### 8.4 Known limitation: within-session replay

Snow's Noise transport uses implicit counters — replay within a *live* session is rejected by the counter check. **Cross-session** replay is impossible (fresh keys per XX handshake). This is strictly stronger than Paseo, which explicitly admits it has *no* within-session replay protection (random nonces, no counter tracking — SECURITY.md §"Replay old messages"). Oxios inherits the win from choosing Noise over bespoke ECDH.

### 8.5 Pairing is the trust anchor

The QR offer (`oxios://pair?code=`, RFC-044 §6.2) carries the daemon's pinned Noise static pubkey. Physical presentation (on-screen scan) is the proof of locality. A stolen offer alone cannot connect: the companion still must complete Noise_XX with the daemon's *private* key, which never leaves the host. Revocation = `DeviceRegistry::revoke` (RFC-044 §6.7) → next connect fails auth.

---

## 9. Relationship to prior designs

| Doc | Disposition under this RFC |
|---|---|
| **RFC-044** (current) | **Extended, not replaced.** All its implemented primitives are reused. Relay is an additive path. §6.6 endpoint race gains a relay candidate. |
| **managed-relay-architecture.md** (deferred) | **Un-suspended but radically simplified.** Its Noise_XX choice is inherited (already in RFC-044). Its OAuth broker (sub-spec A), JWT device tokens, D1 account storage, per-user DO routing — **all dropped**, replaced by pairing-anchored `device_id` routing. The Paseo analysis is the empirical proof these are unnecessary. |
| **managed-relay-A-oauth-broker.md** (deferred) | **Permanently dropped.** No OAuth, no accounts. Pairing suffices. |
| **Paseo analysis** | **Reference implementation.** This RFC is the Oxios adaptation of Paseo's relay pattern, upgraded with Noise_XX (forward secrecy) and integrated into Oxios's existing surface. |

---

## 10. Implementation footprint (estimate)

| Change | File(s) | Size |
|---|---|---|
| Relay client (control + data sockets, keepalive, reconnect) | `src/remote/relay_client.rs` (NEW) | M |
| Generalize `handle_connection` → `handle_stream<S>` | `src/remote/transport.rs` (MODIFY) | S |
| WS-stream → AsyncRead/Write adapter (frame codec) | `src/remote/relay_client.rs` | S |
| `PairingOffer.relay_endpoint` + schema v2 | `src/remote/pairing.rs` (MODIFY) | XS |
| `RemoteRpcSurface::start` relay branch + config | `src/remote/mod.rs`, `src/kernel.rs`, `src/cli.rs` (MODIFY) | S |
| Relay server binary | `oxios-relay/` (NEW crate or binary) | M |
| Companion transport: relay path in endpoint race | companion app (RFC-044 §7) | S |

Total: ~2 medium + 4 small. The crypto, auth, RPC, gateway bridge, and device registry are **untouched** — the relay is a transport addition, exactly as Paseo's design intends.

---

## 11. Open questions

1. **Relay server location for v1.** Self-hosted (`oxios-relay` on user VPS) vs Oxios-operated (`relay.oxios.sh`). Recommendation: ship the self-hostable binary first; operate an instance when adoption justifies it. The architecture is identical either way.
2. **Relay protocol v1 vs v2.** This RFC specifies v2 (control + data split) upfront because RFC-044 supports multiple paired devices. v1 (single merged socket) is simpler but caps at one companion per daemon. The cost of v2 is modest (~Paseo's `relay-transport.ts` is 550 lines); we adopt v2.
3. **Relay TLS.** Native rustls vs reverse-proxy termination. Recommendation: reverse proxy (Caddy) for v1 simplicity; the Noise session is independent of TLS.
4. **Frame padding for traffic-analysis resistance.** Deferred. Paseo omits it; the 64 KiB cap bounds the size signal.

---

## 12. Acceptance criteria

- A paired companion on a **different network** (LTE, no Tailscale) completes a Noise_XX handshake through the relay and successfully dispatches `chat.send` → receives a streaming transcript — with the daemon opening **zero inbound ports** (verified: no listener bound beyond loopback).
- A test asserts that the relay process, given a packet capture of a full session, cannot produce any decrypted payload (relay-seen bytes are Noise ciphertext + routing control JSON only).
- A companion that pinned a *wrong* daemon pubkey (simulated MITM) fails the Noise handshake and is rejected — the relay cannot substitute a key.
- `DeviceRegistry::revoke(device_id)` causes the next relay connect to fail at the `auth.verify` RPC gate (RFC-044 §6.1 token check), not at the relay.
- Disconnecting the daemon's network and reconnecting: the control socket re-syncs, live companions' data sockets are re-established, no RPC errors surface to the user.

---

*A self-review (Appendix A) follows, including three corrected gaps and an abstraction fix.*


---

# Appendix A — Self-review (design review pass)

> Reviewer: the author, in a separate critical pass. Goal: find what's wrong, not defend it.

## A.1 Verdict

**The design is sound and minimal.** It adds one new transport path by reusing ~90% of RFC-044's implemented surface, and it correctly drops the deferred managed-relay doc's OAuth/account complexity based on the Paseo empirical proof. Three real gaps were found and are corrected below; one abstraction in §6.2 is wrong and is corrected in §A.4.

## A.2 Strengths

1. **Minimal new crypto surface.** Noise_XX, device identity, registry, RPC, gateway bridge are all reused unchanged. The only new crypto-adjacent code is wiring a relay WS into the existing `Responder`. This is the lowest-risk way to add remote reach.
2. **Stronger E2EE than the reference.** Paseo uses static ECDH + NaCl box (no forward secrecy, no within-session replay protection). Oxios inherits Noise_XX from RFC-044 — forward secrecy + counter-based replay rejection. The relay gains the traversal benefit without inheriting Paseo's crypto weaknesses.
3. **Transport transparency is real, not claimed.** Verified against `transport.rs:243-462`: the entire Noise handshake + frame loop + `ConnectionCtx` + `OutboundQueue` operates on `WebSocketStream<_>` and `Message::Binary`. It is genuinely stream-source-agnostic once the WS layer is established. The relay path produces the same `WebSocketStream` type (via `connect_async` instead of `accept_async`).
4. **The OAuth drop is justified, not lazy.** The deferred managed-relay doc coupled relay to OAuth because it assumed the relay needed to know *who* the user is. Paseo proves routing by a pairing-derived key + E2EE is sufficient: the relay never needs identity, only a routing id. Oxios's `device_id` (Noise pubkey fingerprint) is already that routing id. The simplification removes an entire subsystem (auth Worker + D1 + JWT) with zero security loss.

## A.3 Gaps found (must fix before implementation)

### Gap 1 — No frame buffering during daemon data-socket startup (CORRECTED)

**Problem:** §5.2 specifies that the relay sends `connected` → daemon opens data socket. But the companion may send its Noise initiator message (msg1) *before* the daemon's data socket is open. Without buffering, msg1 is lost and the handshake never starts.

**Paseo's solution:** `bufferFrame`/`flushFrames` in the DO — up to 200 frames buffered per connection_id, flushed when the daemon's data socket opens (`cloudflare-adapter.ts:252-275`).

**Fix (added to §5):** The relay MUST buffer client→daemon binary frames for a connection_id until the daemon's data socket opens, then flush. Cap at 200 frames (matches Paseo) to bound memory. This is a relay-server requirement, documented in §7.2's contract.

### Gap 2 — Duplicate data sockets on control reconnect (CORRECTED)

**Problem:** When the daemon's control socket drops and reconnects, the relay sends `sync` with live connection_ids. The daemon re-opens data sockets for each. But it may still hold stale data sockets from the previous control connection → duplicates, or the relay's old server-data sockets were never closed.

**Paseo's solution:** `closeExistingServerSockets` (`cloudflare-adapter.ts:181`) — on a new control connection, close any existing server-control/data sockets for that device_id.

**Fix (added to §5):** On a new daemon control connection, the relay closes prior control + data sockets for that `device_id` (close 1008 "Replaced"). The daemon's `RelayClient` must also clear its `dataSockets` map on control reconnect before processing `sync`, so it doesn't hold dead handles. Both sides converge to a clean state.

### Gap 3 — `connection_id` assignment unspecified for clients (CLARIFIED)

**Problem:** §5.2 shows the client URL with `connection_id=<relay-assigned>` but doesn't say who assigns it or how the daemon learns it.

**Fix (clarified):** The companion does NOT provide `connection_id`. The relay generates it (`conn_<16 hex>`) on the client WS upgrade and immediately sends `connected` to the daemon's control socket carrying that id. The daemon then opens its data socket with that id. The companion never needs to know its own connection_id — it just speaks Noise over its single client WS, and the relay transparently maps it. (This is exactly Paseo's v2 flow.)

## A.4 Abstraction correction — §6.2 is wrong about `AsyncRead + AsyncWrite`

**The error:** §6.2 proposes generalizing `handle_connection` to accept `S: AsyncRead + AsyncWrite + Unpin + Send`. This is incorrect.

**The reality (verified `transport.rs:255`):** `handle_connection` calls `accept_async(stream)` to produce a `WebSocketStream<TcpStream>`, then operates entirely on `Message::Binary` frames via `noise::decode_frame`. The Noise frame codec `[type:1][size:4][payload]` lives *inside* WS binary messages, not at the raw byte level. There is no raw `AsyncRead`/`AsyncWrite` in the post-upgrade path.

**Corrected abstraction:**

```rust
/// Noise_XX handshake → ConnectionCtx → frame loop → RPC dispatch.
/// Operates on an established WebSocketStream — agnostic to whether the
/// underlying WS came from accept_async (direct) or connect_async (relay).
async fn handle_ws_session<S>(
    mut socket: WebSocketStream<S>,
    server_static: Arc<Vec<u8>>,
    shutdown: CancellationToken,
    handler: Arc<RpcFrameHandler>,
    audit: Option<Arc<dyn CompanionAudit>>,
    peer_label: PeerLabel,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static;
```

The direct path: `TcpStream` → `accept_async` → `handle_ws_session`. The relay path: `connect_async(relay_url)` → `handle_ws_session`. **No WS-stream→AsyncRead adapter is needed** (§6.2's "~80 lines" is wrong). The generalization is smaller and cleaner than stated.

## A.5 Risks and honest limitations

1. **Relay is a new ops surface.** Even self-hosted, a relay binary is something to run, monitor, and keep available. RFC-044's Tailscale-only path has zero additional infrastructure. The relay trades ops-cost for zero-setup-client. This is the same trade Paseo made; it's defensible for a "reach from anywhere" feature, but it must be strictly opt-in (it is — `relay_enabled = false` default).
2. **Metadata leakage to the relay.** Online/offline status, timing, sizes, device_id. Inherent to any relay. Self-hosting on a user VPS removes the third-party trust entirely. An Oxios-operated relay retains this leak (acceptable for a personal tool, unacceptable for a multi-tenant SaaS — but we are not building a SaaS).
3. **No traffic-analysis padding in v1.** Message sizes leak through the ciphertext size. Paseo also omits padding. The 64 KiB frame cap bounds but does not eliminate the signal. Documented as a future option (§11.4).
4. **Relay server is single-point-of-failure for the relay path.** If the relay is down, the relay path is down — but LAN and Tailscale paths still work (they're independent). The endpoint race means a relay outage degrades to "use Tailscale/LAN," not "total outage." Acceptable by design.
5. **Web client vs native companion.** The web client (browser) connecting via the relay needs a Noise_XX initiator in the browser. RFC-044 §7.3 already addresses Noise-in-JS (snow-wasm). This RFC doesn't add browser-specific complexity, but the browser relay path depends on that prior work landing.

## A.6 Cross-check: does this contradict RFC-044's relay rejection (N1)?

**No — it revisits the decision with new evidence.** RFC-044 N1 rejected the relay *bundle* (relay + OAuth + accounts + Cloudflare ops) as disproportionate. The Paseo analysis decouples the relay from that bundle: a zero-knowledge relay needs no accounts, and Oxios's existing pairing is the trust anchor. This RFC re-proposes *just the relay*, stripped of everything RFC-044 objected to. RFC-044's LAN/Tailscale paths are untouched; the relay is additive. If the user still prefers Tailscale-only, this RFC's `relay_enabled = false` default means it ships inert.

## A.7 Cross-check: vs Paseo analysis findings

| Paseo finding (from analysis report) | Oxios RFC-045 |
|---|---|
| Outbound-only daemon defeats firewall | Adopted verbatim (§4.2, §5) |
| Zero-knowledge relay (opaque byte routing) | Adopted (§5.4, §7) |
| Curve25519 ECDH + NaCl box E2EE | **Upgraded** to Noise_XX (forward secrecy) — already in RFC-044 |
| Pairing via URL fragment (`#offer=`) | Reused: `oxios://pair?code=` (RFC-044 §6.2), extended with `relay_endpoint` |
| Re-handshake key mismatch → close 1008 | Noise_XX responder + companion pubkey pin achieves this inherently (§8.2) |
| v2 control + data socket split | Adopted (§5.2) — necessary for multi-device |
| Relay DO hibernation + protocol-ping keepalive | Adopted shape: control keepalive + staleness cutoff (§5.5); Rust relay uses in-memory map (no hibernation needed) |
| Transport transparency (relay socket = direct socket to session layer) | Adopted and **verified** against `transport.rs` (§A.4) |
| Within-session replay unprotected (Paseo limitation) | **Fixed** — Noise counters reject within-session replay (§8.4) |
| No OAuth/accounts | Adopted as the core simplification vs the deferred doc (§2, §9) |

The design faithfully adapts Paseo's relay pattern while upgrading its crypto and integrating into Oxios's existing surface. No Paseo weakness is inherited.
