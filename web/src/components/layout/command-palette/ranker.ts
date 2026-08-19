import type { PaletteItem, QueryContext, Verb } from './types'

/**
 * Mode-primary verb: what a bare-text (no prefix) query resolves to per mode.
 * console → go (navigate), knowledge → capture (memo), brain → search, chat → run.
 */
export function modePrimaryVerb(mode: QueryContext['mode']): Verb {
  switch (mode) {
    case 'knowledge':
      return 'capture'
    case 'brain':
      return 'search'
    case 'chat':
      return 'run'
    default:
      return 'go'
  }
}

const MAX_RECENTS = 20
const RECENCY_BOOST_MAX = 25

/**
 * Recency log: tracks the last N selected item ids and yields a decay-weighted
 * score boost (most recent → max, oldest → ~0). Persisted by the host to
 * localStorage; the log itself is mode-agnostic.
 */
export class RecencyLog {
  private ids: string[] = []
  load(ids: string[]) {
    this.ids = ids.slice(-MAX_RECENTS)
  }
  snapshot(): string[] {
    return [...this.ids]
  }
  record(id: string) {
    this.ids = [id, ...this.ids.filter((x) => x !== id)].slice(0, MAX_RECENTS)
  }
  boost(id: string): number {
    const i = this.ids.indexOf(id)
    if (i === -1) return 0
    return Math.round(RECENCY_BOOST_MAX * (1 - i / MAX_RECENTS))
  }
}

/**
 * Sort items by final score = base + modeBoost + recencyBoost, descending.
 * Call this AFTER providers have already filtered to matching items and
 * computed their own base score (incl. prefix/verb/entity/fuzzy match terms).
 */
export function rank(items: PaletteItem[], ctx: QueryContext, recency: RecencyLog): PaletteItem[] {
  const primary = modePrimaryVerb(ctx.mode)
  return items
    .map((it) => {
      const modeBoost = it.verb === primary ? 15 : 0
      return { it, final: it.score + modeBoost + recency.boost(it.id) }
    })
    .sort((a, b) => b.final - a.final)
    .map((x) => x.it)
}

/** Subsequence fuzzy match score in [0, 20]; 0 = no match. */
function fuzzy(label: string, query: string): number {
  const l = label.toLowerCase()
  const q = query.toLowerCase().trim()
  if (!q) return 0
  if (l === q) return 20
  if (l.startsWith(q)) return 16
  if (l.includes(q)) return 12
  let i = 0
  for (const ch of l) {
    if (ch === q[i]) i++
    if (i === q.length) return 6
  }
  return 0
}

/**
 * Common match-score components a provider folds into its base score.
 * (Design §7 formula.) Providers may compute base differently, but most reuse
 * these terms against the raw query.
 */
export function matchScore(
  ctx: QueryContext,
  opts: {
    /** Item's trigger prefix matched exactly. */
    exactPrefix?: boolean
    /** Item's verb is the one the user explicitly typed. */
    verbExplicit?: boolean
    /** Entity name matched exactly. */
    entityExact?: boolean
    /** Label to fuzzy-match against the raw query. */
    label?: string
  },
): number {
  const { exactPrefix, verbExplicit, entityExact, label } = opts
  let s = 0
  if (exactPrefix) s += 100
  if (verbExplicit) s += 60
  if (entityExact) s += 40
  if (label) s += fuzzy(label, ctx.raw)
  return s
}
