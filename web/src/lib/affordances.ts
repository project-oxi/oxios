// Affordance helpers — the typed gate vocabulary for conditional chat
// controls (design §6.1, project-roots persona workbench).
//
// Everything here reads ONLY the server-derived EffectiveProfile. Deriving
// UI from persona names, categories, or legacy capability strings is
// prohibited: the kernel resolves one immutable execution profile per turn
// and the web renders exactly what it emits.

import type { Affordance, EffectiveProfile, PresentationLens } from '@/types'

/** Whether the profile grants `a`. Null profile (no persona binds, or none
 *  resolved yet) grants nothing — every conditional control stays hidden. */
export const has = (p: EffectiveProfile | null, a: Affordance): boolean =>
  !!p?.affordances.includes(a)

/** Base/Code tool profiles — the family that renders the coding workbench
 *  (Task 7) around the shared conversation. */
export const isCodingFamily = (p: EffectiveProfile | null): boolean =>
  p?.tool_profile === 'base' || p?.tool_profile === 'code'

/** The profile's presentation lens (IA design §8.5). A profile missing the
 *  field (interrupted rolling deploy) reads as 'general' here — the ONLY
 *  place the default is applied. Presentation only: never gate on it. */
export const presentationLens = (p: EffectiveProfile | null): PresentationLens =>
  p?.presentation_lens ?? 'general'
