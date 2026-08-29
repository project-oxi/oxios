// usePersonaCapabilities — thin compat wrapper over useEffectiveProfile.
//
// Task 6 (project-roots persona workbench) rewired the web to consume the
// server-derived EffectiveProfile everywhere (design §6.1). The export name
// survives only to limit churn; the semantics changed from "Set of persona
// capability strings" to "Set of server-derived affordances". Gating code
// should prefer `useEffectiveProfile` + the `has()` helper in
// `@/lib/affordances`; persona names/categories/capability strings must
// never be consulted.

import { useMemo } from 'react'
import type { Affordance } from '@/types'
import { useEffectiveProfile } from './use-effective-profile'

export interface UsePersonaCapabilitiesResult {
  /** Server-derived affordances for the active session. Empty when no
   *  profile has resolved — every conditional control stays hidden. */
  capabilities: Set<Affordance>
}

export function usePersonaCapabilities(): UsePersonaCapabilitiesResult {
  const profile = useEffectiveProfile()
  return useMemo(
    () => ({ capabilities: new Set<Affordance>(profile?.affordances ?? []) }),
    [profile],
  )
}
