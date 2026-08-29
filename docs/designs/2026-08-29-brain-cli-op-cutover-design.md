# Brain CLI-op cutover — design

**Date:** 2026-08-29
**Status:** Implemented
**Supersedes:** the transport half of `2026-08-27-daemonless-brain-design.md`
(the store layout, documents plane, and degradation contract carry over
unchanged).
**Upstream basis:** oxibrain 0.10.0/0.10.1 — the agent-first CLI contract
(`doc/spec/agent-first-cli-v1.md`, ADR-012/013, ARCHITECTURE.md v2.13).

## Problem

oxibrain 0.10 replaced its CLI human verbs with a 14-op payload dispatch
(`oxibrain <op> --json`), removed the `stats` / `review_merges` MCP tools,
made `space` required in every payload, and moved machine lifecycle verbs
under `oxibrain admin`. oxios was still integrated against 0.8: the kernel
held one lazily-spawned `serve --stdio` child for everything, and the
oneshot admin verbs (`index`, `extract --pending`) no longer exist at the
top level. Against a 0.10 binary the old integration is broken.

## Decision — one process per operation, everywhere

Nothing resident. Every brain interaction spawns a short-lived process:

1. **Agent path → CLI op dispatch.** `recall`, `ingest` (kernel
   `remember`), `search`, `brief`, `traverse`, `why`, `contradictions`
   run as one-shot `oxibrain --dir <dir> <op> --json -` subprocesses.
   Payloads go over stdin (spec §2: prose bodies never travel as argv);
   stdout carries the stable envelope `{api, ok, op, space?, data, meta}`.
   A well-formed envelope proves the transport alive even when the op
   failed, so availability tracks the transport, not op success.
   `ingest` adds `wait_lock_ms: 2000` to ride short writer locks; every
   op is bounded by a 90 s timeout (`kill_on_drop` reaps stragglers).
2. **Console path → per-call stdio child.** The reads the 14-op CLI
   surface deliberately does not expose — native `stats`,
   `pending_stats`, `document_history`, `spaces/list`, and the
   `entity://` / `timeline://` / `space://` resources — speak JSON-RPC
   over a caller-owned `serve --stdio` child that is spawned per call
   and reaped when the call returns (`oxibrain-client` 0.10.1,
   `StdioChild` impl). The web console endpoints keep their shapes: the
   `entity://` / `timeline://` resources return the same `Belief[]` /
   `TimelineEntry[]` payloads the removed `get_entity` / `timeline`
   tools used to return.
3. **Known upstream gap — `review_merges`.** The tool was removed
   (v2.13 slot accounting) and its `admin review` replacement is not
   shipped in 0.10.1. `BrainSession::review_merges` degrades to `None`;
   the console merges/failures/sources panels render empty. Revisit on
   the next oxibrain bump.

### Deleted

- The persistent session child and all its machinery: client mutex,
  respawn-on-failure, `ChildSpawn`, `NoopSpawn`, `SpawnBackoff`.
- The Foundation JSON-RPC handshake probe (`spawn_local` + `bring_up` at
  bootstrap) — replaced by a one-shot `oxibrain describe` probe
  (`exit 0` + envelope → `Compatible`; `api` field → protocol version).
- The binary crate's production use of `oxibrain-client`:
  `oxios brain status` reads `admin stats` output; the dependency moved
  to `[dev-dependencies]` (DTO shape-guard test only).

### Admin verbs

`oxibrain index --documents [--embed]` → `oxibrain admin index
--documents [--embed]`; `oxibrain extract --pending` → `oxibrain admin
extract --pending` (boot warm-up, 600 s drain timer, `oxios brain
reindex` / `extract`). The `embed_gap` degradation (retry without
`--embed`) is unchanged.

### Agent skill

`share/default-skills/brain/SKILL.md` now teaches the op contract:
scratch-file JSON payloads (`--json @file`), required `space`,
`--dir` before the op name, envelope/exit-code reading, and the
`extraction: pending` semantics (the kernel drains the backlog; the CLI
one-shot has no sampling client).

## Tradeoffs

- **Spawn overhead per op** (~10–30 ms) instead of a warm child. Agent
  recall runs once per turn and console reads poll every 15–30 s; the
  overhead is noise. In exchange: no resident process, no respawn or
  backoff bookkeeping, and a dead binary fails fast per call.
- **Two transports** (CLI ops + per-call child) instead of one. The
  split follows upstream's own doctrine — CLI ops are the agent
  transport; native RPC/resources remain the host/console transport
  (ECOSYSTEM.md C8) — and it keeps every console endpoint working
  without a web frontend rewrite.

## Files

- `crates/oxios-kernel/src/brain/session.rs` — rewritten transport.
- `crates/oxios-kernel/src/brain/oneshot.rs` — admin verbs.
- `crates/oxios-kernel/src/brain/{mod.rs}`, `src/lib.rs`,
  `src/config.rs` — docs + re-exports.
- `crates/oxios-kernel/src/foundation/bootstrap.rs` — describe probe.
- `src/kernel.rs`, `src/main.rs` — admin verbs, status probe rewire.
- `Cargo.toml` — `oxibrain-client` 0.8 → 0.10 (root dep → dev-dep for
  the binary; kernel keeps the workspace dep).
- `share/default-skills/brain/SKILL.md` — op-contract rewrite.
