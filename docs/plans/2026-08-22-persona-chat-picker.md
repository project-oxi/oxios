# Persona Chat Picker — Implementation Plan

**Goal:** Session-scoped persona selection in the web chat composer with a
persona category taxonomy (writing genres, coding, normal, …) and
persona-scoped default mounts.

**Architecture:** Additive Persona fields + `persona_id` threading along the
existing `model_override`/`role` conduit (WS → gateway metadata →
`handle_unified` → `MsgCtx`/`ExecEnv` → `execute_inner`), plus a
`PersonaPicker` composer pill and personas-page grouping. Design:
`docs/designs/2026-08-22-persona-chat-picker-design.md`.

**Tech Stack:** Rust 2024 kernel/gateway/ouroboros, React 19 + TS web,
react-query, i18next (ko/en).

## Tasks

1. **Kernel Persona fields** — `category: String` (default `general`),
   `genre: Option<String>`, `default_mount_ids: Vec<String>` on
   `crates/oxios-kernel/src/persona/mod.rs` (+ `Default` impl, constructors);
   assign categories to the 9 defaults; add 5 new defaults (normal, novelist,
   scenarist, essayist, blogger); persistence doc-comment update; unit tests
   (defaults carry categories; serde roundtrip with missing fields).
2. **Ouroboros conduit** — `MsgCtx.persona_id`, `ExecEnv.persona_id`
   (`#[serde(default, skip_serializing_if)]`); extend the existing serde
   roundtrip test.
3. **Orchestrator** — `handle_unified` gains `persona_id: Option<&str>` param;
   thread to `MsgCtx`; `resolve_exec_env` copies into `ExecEnv`.
4. **Gateway** — extract `persona_id` metadata; pass to `handle_unified`.
5. **Agent runtime** — `execute_inner` gains `persona_id: Option<&str>`;
   persona resolution = override (enabled) → global active; wire from
   `execute_directive_with_session` (`env.persona_id`).
6. **HTTP/WS ingress** — `ChatRequest.persona_id` (POST /api/chat);
   WS recv: parse + validate persona (exists+enabled → error frame);
   both paths insert metadata and persist `session.active_persona_id`
   (HTTP persist block + `persist_session` signature/callers).
7. **Persona routes** — summary/get/create/update gain the three fields.
8. **Web types + store** — `Persona` type fields; `activePersonaId` persisted
   state + setter; payload `persona_id`; `loadSession` rehydrate.
9. **PersonaPicker** — new component + container; mount auto-apply helper
   (pure, unit-tested); render in `chat-input.tsx` toolbar.
10. **Personas page + editor** — category grouping, badges; create/edit
    dialogs (category/genre/default mounts); i18n keys ko+en.
11. **Gates + browser verification** — cargo fmt (touched files
    only)/clippy/nextest; web typecheck/lint/test/build; browser drive.

## Conventions

- Commits: `feat(kernel): …`, `feat(web): …` split by layer.
- No `cargo fmt` on untouched files; no reformat of pre-existing drift.
- i18n parity enforced by `web/src/__tests__/i18n-coverage.test.ts`.
