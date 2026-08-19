# Oxibrain First-Party Supervision + Brain Tab — Design

Status: approved (2026-08-19). Follows RFC-047 (brain migration) and RFC-048
(Foundation). This document is the oxios-side spec for making the oxibrain
daemon a first-party managed dependency and for restructuring the web UI into
Console / Brain / Chat.

## 1. Problem

1. **oxios does not work properly without oxibrain, yet treats it as
   optional.** When the daemon is missing or down, the Web UI shows a
   dead-end banner ("start it with `oxibrain serve --socket … --daemon`") and
   every memory operation silently degrades. The user experience contradicts
   the dependency reality.
2. **Knowledge and memory are split across surfaces.** oxibrain will soon own
   both markdown knowledge and vector memory, but the Web UI keeps Knowledge
   as a top-level tab and Brain as a Console sub-page (`/brain`, Storage
   group).

## 2. Decisions (settled with user, 2026-08-19)

| Question | Decision |
|---|---|
| Install method | Download prebuilt binary from GitHub Releases (`a7garden/oxibrain`). Requires an oxibrain repo change: attach `aarch64-apple-darwin` tarball + sha256 to releases. No cargo-install fallback in v1. |
| Supervision | Both: launchd LaunchAgent (KeepAlive) as the keeper, plus detached-spawn fallback in oxios when launchd is unavailable/removed. |
| Knowledge routes | Move `/knowledge/*` → `/brain/knowledge/*` with redirects preserving search params. |
| Memory views | Keep the existing four views (overview / search / entity / contradictions); restructure navigation only. brain-ui-v2-scale views (graph, merges, capture) are out of scope. |
| Architecture | Approach A: a dedicated `BrainSupervisor` module in `oxios-kernel`. `BrainConnection` stays a thin transport client. |

## 3. Architecture — `BrainSupervisor`

New module `crates/oxios-kernel/src/brain/supervisor.rs` (sibling of
`brain/mod.rs`, `brain/config.rs`). It owns the daemon lifecycle:

```
NotInstalled → Installing → Installed → Starting → Online
                                                 ↘ Failed(last_error)
```

`Disabled` when `[brain] enabled = false` or `auto_manage = false`.

### 3.1 `ensure(socket, config) -> SupervisorState` (idempotent)

1. Probe the socket (`BrainClient::connect` + ping). Alive → `Online`,
   `managed_by = external`. Never touch a daemon another app started.
2. Locate the binary: `binary_path` config → `~/.oxi/bin/oxibrain` → `PATH`.
3. Missing or protocol-incompatible → install (§4).
4. Ensure the launchd LaunchAgent (§5). If the socket is already alive after
   this (KeepAlive raced us) → `Online`.
5. Fallback: detached spawn (`setsid`, stdio → `~/.oxi/brain/daemon.log`) if
   launchctl failed.
6. Poll the socket for readiness, timeout 30 s → `Online`, else
   `Failed(timeout)`.

Every failure keeps the RFC-047 degradation contract: log, report state, agent
turns complete. The next `ensure` call retries.

### 3.2 Call sites

- **Boot** (`src/kernel.rs`, before `BrainConnection::connect`): when
  `config.brain.enabled && config.brain.auto_manage`, run `ensure()`. The
  foundation bootstrap hook (`foundation/bootstrap.rs:116-139`,
  `may_start_daemon`) is rewired to call the supervisor instead of logging
  "no installer is wired yet".
- **Runtime respawn**: `BrainConnection::call()` gains an optional
  `Arc<BrainSupervisor>` hook. When the lazy reconnect fails, it calls
  `supervisor.respawn_if_needed().await` (rate-limited to one attempt / 30 s;
  launchd KeepAlive normally makes this a no-op) and retries the connection
  once. Still `None` on failure — the degradation contract is unchanged.
- **CLI**: `oxios brain install | start | stop` invoke supervisor functions
  directly (they are process-agnostic: filesystem + launchctl + spawn);
  `oxios brain status` reads state from the filesystem (binary version,
  `launchctl print`, socket probe) so it works while the daemon is down.

Concurrent `ensure()` from the CLI and daemon is safe: socket-alive
short-circuit, atomic binary rename, idempotent launchctl bootstrap.

## 4. Installer

Mirrors `src/commands/update.rs` (GitHub release download) and
`skill/host_tools/provisioner.rs` (archive extraction).

- Source: `https://api.github.com/repos/a7garden/oxibrain/releases` — latest
  release with asset `oxibrain-<version>-aarch64-apple-darwin.tar.gz` plus
  its `.sha256`.
- Target: `~/.oxi/bin/oxibrain`, mode `0o755`. Flow: tmp download → sha256
  verify → tar extract (single binary inside) → atomic rename.
- **Version policy**: install only when the binary is absent or the daemon
  reports a protocol version outside
  `MIN_BRAIN_PROTOCOL_VERSION..=MAX_BRAIN_PROTOCOL_VERSION`
  (`foundation/bootstrap.rs`). No silent unattended upgrades.
- Dependency bump: `oxibrain-client` `0.2 → 0.3` — the typed
  `ClientHello`/`ServerInfo` handshake (`connect_endpoint`) drives the
  compatibility classification.
- Offline / download failure: `Failed(download)` state, warning log,
  degradation continues. Retry on next boot or CLI invocation.

**Prerequisite (oxibrain repo, separate PR):** `publish.yml` builds the
release binary and uploads `oxibrain-<ver>-aarch64-apple-darwin.tar.gz` +
`.sha256` to the GitHub Release (mirroring oxios `release.yml` tarball
packaging). Until that ships, `auto_manage` installs nothing and the banner
reports `Failed(no-release-asset)` with the manual command.

## 5. Keeper — launchd + fallback

Mirrors `src/daemon.rs` (plist write with XML escaping, `launchctl bootstrap`
/ `bootout` / legacy `unload` fallbacks). Label: `com.oxi.oxibrain`.

- Plist: `~/Library/LaunchAgents/com.oxi.oxibrain.plist`,
  `ProgramArguments = [<binary>, "serve", "--daemon"]` (omitted `--socket`
  binds the canonical `~/.oxi/brain/oxibrain.sock`), `RunAtLoad = true`,
  `KeepAlive = true`.
- If the socket is already alive, skip plist installation entirely (shared
  daemon — oximemo/oxiline may own it).
- Rewrite + `bootout`/`bootstrap` when the plist content changes (binary path
  moves).
- Supervision never kills a daemon it did not start. Removal is explicit:
  `oxios brain stop` (bootout for the session) / `oxios brain uninstall`
  (bootout + delete plist; binary and store data remain).

## 6. Kernel integration & observability

- `BrainApi` gains `supervisor_state() -> SupervisorStatus`:
  `{ state, installed_version, daemon_version, managed_by: launchd|spawn|external|none, last_error }`.
  Exposed by merging into `GET /api/brain/status`.
- **Metrics gauge fix**: `oxibrain_available` is updated on every supervisor
  state transition (and in `BrainConnection::call()`'s failure path), not
  only at boot — closes the drift found in review
  (`ARCHITECTURE.md:682` claims "boot + on reconnect"; only boot sets it
  today).
- i18n: delete the "start it manually" copy
  (`brainPage.degradedDescription` in `ko.json`/`en.json`). New banner states
  derive from the supervisor state machine: 설치 중… / 시작 중… / 온라인(배너
  숨김) / 실패(원인 표시). The manual-command hint appears only when
  `auto_manage = false`.

## 7. Web UI restructure

### 7.1 Modes

`console | brain | chat` replaces `console | knowledge | chat`:

- `mode-tabs.tsx` (desktop), `bottom-nav.tsx` (mobile): `brain` entry, Brain
  icon, href `/brain`. `switch.tsx` `MODE_HREF` updated. ⌃1/⌃2/⌃3
  (`use-tab-shortcuts.ts`) map to the new three surfaces.
- `sidebar.tsx`: `mode === 'brain'` renders `BrainNav`; the Console
  "Storage" group drops the Brain item (it is now a mode).
- `app-layout.tsx` doc comments updated.

### 7.2 Routes

```
/brain                      개요 — overview + supervisor StatusBanner
                             (layout route, sidebar = BrainNav)
/brain/search               검색
/brain/entity               엔티티
/brain/contradictions       모순
/brain/knowledge            지식 홈        (moved from /knowledge)
/brain/knowledge/graph      지식 그래프     (moved from /knowledge/graph)
/knowledge, /knowledge/graph  → beforeLoad redirect, search params preserved
```

- The existing `brain.tsx` page (internal Tabs) is split into four route
  files; view components (`components/brain/*`) are reused unchanged.
- Knowledge routes move under `routes/brain/knowledge/`; `KnowledgeLayout`,
  editor, store, journal, habits, settings are untouched. Internal knowledge
  links (`sidebar.tsx:369`, knowledge components) repoint to the new prefix.
- Redirects use `beforeLoad: () => { throw redirect({ to, search: (s) => s }) }`
  so file-path deep links survive.

### 7.3 Sidebar — BrainNav

Two sections:

- **메모리** (memory): 개요 `/brain`, 검색, 엔티티, 모순.
- **지식** (knowledge): the existing `KnowledgeNav` content (home, chat,
  journal, graph, new file, habits, settings) transplanted verbatim under the
  section header.

### 7.4 Command palette

Mode inference moves from the three-tab mapping to route prefixes:
`/brain/knowledge/*` behaves as today's knowledge mode (bare-text → capture),
`/brain/*` (memory views) maps to a brain mode (bare-text → memory search),
console/chat unchanged. `ranker.ts` `modePrimaryVerb` and palette mode getters
updated accordingly.

## 8. CLI

`oxios brain` subcommands: existing `status | ingest | ask` (unchanged
behavior; `status` gains supervisor state) plus `install | start | stop |
uninstall`. `export` remains delegated to the oxibrain CLI.

## 9. Config

```toml
[brain]
enabled = true
auto_manage = true   # first-party: install + launchd + spawn fallback
# binary_path = ""   # explicit binary; skips download when set
```

`auto_manage = false` restores today's behavior exactly (degrade, manual
banner). Default config (`share/default-config.toml`) documents both keys.

## 10. Testing

- **Supervisor unit tests**: state machine transitions against a
  trait-abstracted prober/installer (fake socket, failing download).
- **Spawn-fallback integration test**: a stub daemon script (listens on a
  tempdir Unix socket, answers ping) is spawned via the supervisor fallback
  path; assert socket readiness and `Online` (launchd path skipped under
  test). Pattern: oxibrain's `serve.rs` integration tests.
- **Web**: route redirect tests (params preserved), mode-switch smoke,
  `bun run typecheck` + `biome` + build gates.
- **Full gates**: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-features -- -D warnings`,
  `cargo nextest run --workspace --no-fail-fast`.
- **Manual smoke** (after oxibrain artifacts ship): fresh `~/.oxi/bin`,
  `oxios brain install` → `start` → `/api/brain/status` shows
  `state: online`, `managed_by: launchd`; kill the daemon → KeepAlive
  restarts it → banner recovers without page reload.

## 11. Out of scope

- brain-ui-v2-scale memory views (sigma graph, merge review, capture) —
  tracked separately in the oxibrain repo.
- Unifying KnowledgeBase data with the oxibrain knowledge projection (vault
  sync) — oxibrain-side work.
- Unattended daemon upgrades beyond the compatibility floor.
- Non-macOS keepers (target is `aarch64-apple-darwin` only).

## 12. Risks

- **Release asset availability**: until oxibrain's `publish.yml` attaches
  binaries, first-run install reports `Failed(no-release-asset)`; the banner
  keeps the manual hint as an interim. Sequencing: oxibrain PR first.
- **Shared-daemon ownership**: oximemo/oxiline may manage the same daemon
  later; the socket-alive short-circuit and never-kill rules keep oxios a
  polite co-owner.
- **launchctl domain quirks** (GUI session required): the detached-spawn
  fallback covers headless/SSH contexts where `gui/$UID` bootstrap fails.
- **Search-param fidelity in redirects**: knowledge file deep links must
  round-trip; covered by redirect tests.
