# Oxios Design System

> **Single source of truth:** the **oxi-design-system** managed skill
> (`~/.omp/agent/managed-skills/oxi-design-system/DESIGN.md`, v1.0, project-agnostic).
> Token values, component specs, theming, type, motion, and accessibility rules all live there.
> This file holds **only** the oxios-specific surface identity, the project-specific token
> extensions the portable spec explicitly allows, and the migration status. Where this file
> and the canonical spec disagree, the canonical spec wins.

**In-ecosystem mirror:** `project-oxi/.github/DESIGN.md` (v1.0, 2026-07-31) — the oxi-family
spec. It is content-equivalent to the skill spec; the skill spec is the newer project-agnostic
form (refined 2026-08-03) and is authoritative.

---

## 1. What oxios inherits unchanged

The shared grammar comes from the canonical spec — oxios does not redefine any of it:

- OKLCH 3-tier token architecture (primitive → semantic → component → utility → code)
- Warm-paper neutrals (hue 95 light / 265 ink); six-hue label palette; APCA-optimized status
- SUIT (body) + SUITE (headline ≥20px) + Geist Mono (code), via jsDelivr + Fontsource
- `.dark` class as the single light/dark trigger; `oxi-theme` storage key; inline FOUC script
- Component radius tokens; box-shadow input borders; shared control size scale
- `bg-surface` / `text-text` / `border-line` semantic utilities — never raw `var()`, never `dark:`

Implementation: `web/src/index.css` (single-file 3-tier realization), `web/src/stores/theme.ts`,
`web/index.html` (fonts + FOUC).

## 2. oxios surface identity

oxios is an **agent operating-system dashboard**. Information density is high and every agent
state demands visual feedback. The shared grammar's **status-color mechanism** and **dense
dashboard density** are oxios's core surfaces.

- **Status is the primary channel, not decoration.** Every agent state surfaces as
  `text-status-{success|warning|error|info}` + a paired icon + label. Color is never the sole
  carrier (accessibility §9.2). The canonical status OKLCH values (light/dark) were measured and
  APCA-optimized on the oxios dashboard — oxios is the **source** of these values, not a consumer.
- **Dashboard density.** `gap-2` (8px) inside components, `gap-4` (16px) between sections.
  3-zone layout: sidebar + main + optional inspector. All sidebars share `sidebarPrimitives`.

## 3. oxios-specific tokens (project extensions)

These are oxios-only and live outside the shared grammar. They are the dashboard data-viz layer,
declared in `web/src/index.css`.

```css
/* Charts (5 hues) */
--chart-1: oklch(0.646 0.222 41.116);
--chart-2: oklch(0.6   0.118 184.704);
--chart-3: oklch(0.398 0.07  227.392);
--chart-4: oklch(0.828 0.189 84.429);
--chart-5: oklch(0.769 0.188 70.08);

/* Message-type hues */
--message-task:      oklch(0.623 0.214 259.815);
--message-status:    oklch(0.707 0.022 261.325);
--message-result:    oklch(0.723 0.219 149.579);
--message-query:     oklch(0.627 0.265 303.9);
--message-handshake: oklch(0.769 0.188 70.08);

/* Git-diff line colors */
--color-diff-add:  oklch(0.52 0.15 150);
--color-diff-del:  oklch(0.54 0.22 25);
--color-diff-hunk: oklch(0.55 0.11 220);

/* Settings panel */
--surface-section:   color-mix(in oklch, var(--muted) 30%, var(--background));
--modified-accent:   var(--primary);
--modified-row-bg:   color-mix(in oklch, var(--primary) 3%, transparent);

/* Knowledge editor prose typography */
--editor-font-size: 0.9375rem;   /* 15px desktop, 16px mobile */
--editor-line-height: 1.75;
--editor-font-body: var(--font-sans);
--editor-font-mono: var(--font-mono);

/* Status text on a PLAIN surface — oxios extension. The portable spec defines
   -on-subtle for text inside -subtle badges only; it has no token for status-colored
   TEXT on a neutral surface. Solid --color-status-* (L≈0.6) fails WCAG 4.5:1 as text
   on light surfaces, so -on-surface (L0.40 light / L0.80 dark) fills that role.
   Rule: icons/fills/dark-mode text use solid --color-status-*; status-colored text
   on a neutral surface uses --color-status-*-on-surface. */
--color-status-success-on-surface: /* L0.40 light / L0.80 dark — per-theme in index.css */
--color-status-warning-on-surface: /* L0.45 light / L0.82 dark */
--color-status-error-on-surface:   /* L0.42 light / L0.78 dark */
--color-status-info-on-surface:    /* L0.42 light / L0.78 dark */
```

## 4. Migration status — COMPLETE

oxios adopted the oxi system in `92a416708` ("adopt oxi design system", 2026-07-31). Every step
of the canonical migration plan (§12.3) is done and verified (2026-08-11):

| Step | Status | Evidence |
|------|--------|----------|
| 3-tier token architecture | ✅ | `web/src/index.css` — `.dark` overrides live only in the token layer |
| `dark:` variant sweep | ✅ | grep over `web/src` — **zero** `dark:` in component code |
| Storage key `oxios-theme` → `oxi-theme` | ✅ | `theme.ts` + FOUC script both use `oxi-theme` with one-time legacy migration |
| Font migration Geist → SUIT/SUITE | ✅ | `index.html` loads SUIT+SUITE via jsDelivr; `--font-sans`=SUIT; Geist Mono retained via Fontsource |
| Editor presets — Serif removed, SUIT added | ✅ | `FONT_PRESETS`: SUIT first; Serif and Geist-Sans presets removed; legacy-guard deletes stale prefs |
| Forbidden patterns | ✅ | No `React.FC`/`ElementRef`/`defaultProps`; no hex/rgb in components (only SVG assets + a `<input type=color>` default + test fixtures); no `[data-theme=]` selectors; no card left-accent bars |

**Accessibility spot-check (WCAG 2.x, 2026-08-11):** every text/surface pair clears 4.5:1 in both
modes. Note: light `--color-text-muted` (L0.55) sits at 4.64:1 — passes the floor but is the
weakest pair; the canonical L0.35 would give 10.83:1 (see §5).

## 5. Known token divergences from the portable spec (for review)

oxios's values are a deliberate **2026-07-31 adoption snapshot** (git blame confirms commit
`92a416708`). The portable skill spec was refined on 2026-08-03, so a few values now differ. None
fail accessibility; all are aesthetic. Two coherent themes:

1. **Dark neutrals use hue ~286** (oxios dark canvas is `oklch(0.13 0.005 285.823)`, a slightly
   purple-blue), whereas the portable spec uses hue 265 (pure blue).
2. **Dark borders use translucent white overlays** (`oklch(1 0 0 / 10–15%)`) — an adaptive
   technique — vs the portable spec's solid `oklch(0.28…) ` values.

Notable pairs (oxios → portable canonical):

| Token | oxios (07-31) | portable (08-03) | Note |
|-------|---------------|------------------|------|
| `--color-text-muted` (light) | `--p-neutral-500` (L0.55, 4.64:1) | `--p-neutral-700` (L0.35, 10.83:1) | Weakest pair — candidate for tightening to canonical |
| `--color-surface-muted` (dark) | `oklch(0.274 0.006 286)` | `oklch(0.20 0.012 265)` | oxios lighter; visible on secondary surfaces |
| `--color-surface-raised` (dark) | `oklch(0.19 0.008 265)` | `oklch(0.22 0.016 265)` | Minor |
| `--color-border` (dark) | `oklch(1 0 0 / 10%)` | `oklch(0.28 0.015 265)` | Translucent vs solid |
| `--color-border-strong` (light) | `--p-neutral-200` (hue 95) | `oklch(0.82 0.008 265)` (hue 265) | Warm vs cool |
| `--color-focus-ring` (dark) | `oklch(0.6 0.05 265)` | `oklch(0.65 0.05 265)` | Negligible |

**Decision deferred to owner:** these were tuned by hand on a live dashboard; aligning them
requires a visual diff the agent cannot render headlessly. They are documented here so each can be
flipped to canonical selectively.

## 6. Open item

- **`dark:` lint enforcement.** The migration plan (§12.3 step 3) calls for a `no-restricted-syntax`
  rule banning `dark:` literals in component files. Biome (this project's linter) targets AST node
  types, not class-string content, so the rule isn't directly expressible. The codebase is verified
  clean by convention. Options for hard enforcement: a custom pre-commit grep, or a stylelint rule
  over the token layer. Not blocking.
