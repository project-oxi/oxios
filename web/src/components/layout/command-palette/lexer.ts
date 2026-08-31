import type { SurfaceId } from '@/types/surfaces'
import type { PaletteEntity, QueryContext, Verb } from './types'

/** Map of leading prefix character → verb. */
const VERB_PREFIXES: Record<string, Verb> = {
  '>': 'run',
  '!': 'control',
  '~': 'switch',
  '+': 'new',
  '/': 'capture',
}

/** Inline action tokens valid only after a `!` control entity. */
const ACTIONS: Record<string, true> = { enable: true, disable: true, start: true, stop: true }

export interface LexResult {
  verb: Verb | null
  entity: PaletteEntity | null
  action?: string
  text: string
}

/**
 * Lex the raw query into verb / entity / action / text.
 *
 * Grammar (see design §5): `verb? entity? text?` where verb is a leading prefix
 * char (`>!~/+`) and entity is a `@` token (`@type:name` or `@name`). For the
 * `!` control verb, a leading action token (`enable|disable|start|stop`) in the
 * remainder is split out as `action`.
 *
 * Pure and side-effect free; safe to run on every keystroke.
 */
export function lex(raw: string): LexResult {
  const first = raw[0]
  const verb = first && VERB_PREFIXES[first] ? VERB_PREFIXES[first] : null
  let rest = verb ? raw.slice(1) : raw

  // Extract the first `@entity` token, wherever it appears.
  let entity: PaletteEntity | null = null
  const at = rest.indexOf('@')
  if (at !== -1) {
    const after = rest.slice(at + 1)
    const m = after.match(/^(\S+)/)
    const tok = m?.[1]
    if (tok) {
      const colon = tok.indexOf(':')
      entity =
        colon !== -1 ? { type: tok.slice(0, colon), name: tok.slice(colon + 1) } : { name: tok }
      // Splice the `@token` out of the remainder.
      rest = `${rest.slice(0, at)} ${after.slice(tok.length)}`.trim()
    }
  }

  // For `!` control, peel a leading action token off the remainder.
  let action: string | undefined
  let text = rest.trim()
  if (verb === 'control') {
    const toks = text.split(/\s+/)
    if (toks[0] && ACTIONS[toks[0].toLowerCase()]) {
      action = toks[0].toLowerCase()
      text = toks.slice(1).join(' ').trim()
    }
  }

  return { verb, entity, action, text }
}

/** Build a full `QueryContext` from raw input + current surface. */
export function buildContext(raw: string, surface: SurfaceId): QueryContext {
  const { verb, entity, action, text } = lex(raw)
  return { raw, verb, entity, action, text, surface }
}
