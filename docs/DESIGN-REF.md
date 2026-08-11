# Design System Reference

> **Pointer file** — this project follows the **Oxi Design System**.

## Canonical document

- **Living authority:** the **oxi-design-system** managed skill
  (`~/.omp/agent/managed-skills/oxi-design-system/DESIGN.md`, v1.0, project-agnostic, refined
  2026-08-03). Token values, component specs, theming, type, motion, accessibility.
- **In-ecosystem mirror:** `project-oxi/.github/DESIGN.md` (v1.0, 2026-07-31). Content-equivalent.

## This project's design doc

**[`../DESIGN.md`](../DESIGN.md)** (repo root) — the single oxios design doc. It holds the
oxios surface identity (dashboard density, status-as-mechanism), the project-specific token
extensions the portable spec allows (charts, messages, diff, editor, settings), the verified
migration status, and the documented token divergences from the portable spec.

## Migration status — COMPLETE

oxios adopted the oxi system in `92a416708` (2026-07-31). All canonical migration-plan steps are
done and verified (2026-08-11): 3-tier tokens, `dark:` sweep (zero in components), `oxi-theme`
storage key with one-time legacy migration, SUIT/SUITE/Geist-Mono fonts, editor-preset Serif
removal. oxios is the most complete implementation of the system — its measured dashboard status
values are the canonical source. See root `DESIGN.md` §4 for the per-step evidence.
