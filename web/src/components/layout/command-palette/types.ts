import type { ReactNode } from 'react'
import type { SidebarMode } from '@/stores/sidebar'

/**
 * Command palette core types.
 *
 * The palette is a federated provider registry: each provider contributes
 * `PaletteItem`s for a subset of verbs/modes given a parsed `QueryContext`.
 * The host lexes the raw query, asks every active provider, ranks by score,
 * and renders. See `docs/designs/2026-06-29-command-palette-design.md`.
 *
 * NOTE: the type is named `PaletteItem` (not `CommandItem`) to avoid colliding
 * with the cmdk/shadcn `CommandItem` component imported alongside it in hosts.
 */

/** The palette verbs. `go` is implicit (bare-text / nav match). `search` is the
 * brain-memory primary (bare text in the Brain surface searches memories). */
export type Verb = 'go' | 'capture' | 'run' | 'switch' | 'control' | 'new' | 'search'

/** A parsed `@entity` target. */
export interface PaletteEntity {
  /** Explicit type namespace if the user typed `@type:name` (e.g. `@skill:…`), else undefined. */
  type?: string
  /** The (possibly partial) entity name. */
  name: string
}

/** The result of lexing the raw query string (mode attached by the caller). */
export interface QueryContext {
  /** Original raw input. */
  raw: string
  /** Leading verb prefix, or `null` for bare-text (→ mode-primary resolution). */
  verb: Verb | null
  /** Resolved `@entity`, or `null`. */
  entity: PaletteEntity | null
  /** For `!` control only: inline action token (`enable|disable|start|stop`). */
  action?: string
  /** Remainder free text — the intent / payload. */
  text: string
  /** Current sidebar mode (drives mode-primary + modeBoost). */
  mode: SidebarMode
}

/** A single selectable palette entry. */
export interface PaletteItem {
  id: string
  verb: Verb
  /** Already-rendered icon element (providers supply `<Icon className=…/>`). */
  icon: ReactNode
  /** Resolved title string (provider runs `t()` itself, incl. interpolation). */
  title: string
  /** Optional subtitle shown muted after the title (e.g. ` · 빨래`). */
  subtitle?: string
  /** Optional trailing hint node, e.g. a `<kbd>`. */
  hint?: ReactNode
  /**
   * If set, selecting this item sets the palette query to this string (and keeps
   * the palette open + focused) instead of running `onSelect` and closing. Used
   * by "compose" entries like the capture destination picker (`/later` → `/Later `).
   */
  compose?: string
  /** Provider-computed base score; the ranker adds modeBoost + recencyBoost. */
  score: number
  onSelect: () => void | Promise<void>
}

/**
 * A provider contributes commands for a subset of verbs/modes.
 * `resolve` builds items (and onSelect closures) but MUST NOT execute side
 * effects itself — it runs on every keystroke. Side effects live in `onSelect`.
 */
export interface CommandProvider {
  id: string
  /** Verbs this provider answers. */
  verbs: Verb[]
  /** Modes in which the provider is active; omit/empty for all modes. */
  modes?: SidebarMode[]
  /** Return only items matching the context. */
  resolve(ctx: QueryContext): PaletteItem[]
}
