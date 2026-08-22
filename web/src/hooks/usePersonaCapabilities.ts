// usePersonaCapabilities — derives the active session's persona capability set.
//
// RFC-044 Phase 3: each Persona carries a `capabilities: string[]` (backend
// `Vec<String>`); the chat substrate reads them to enable optional affordances
// (terminal toggle, worktree fanout, diff viewer, etc.).
//
// Source of truth:
//   - GET /api/personas        → list of personas (each with `capabilities`)
//   - GET /api/sessions/:id    → `active_persona_id` (per-session override)
//
// Falls back to the global active persona (PUT /api/personas/active response)
// when the session does not specify one. If neither yields a persona, the
// hook returns an empty Set — every affordance stays disabled.

import { useQuery } from '@tanstack/react-query'
import { useMemo } from 'react'
import { api } from '@/lib/api-client'
import { useChatStore } from '@/stores/chat'
import type { Persona, SessionDetail } from '@/types'

/** Persona with the fields we actually need (subset of full Persona). */
interface PersonaWithCapabilities {
  id: string
  name?: string
  capabilities?: string[]
}

/** Shape of the active-persona endpoint (if the backend exposes one). */
interface ActivePersona {
  id: string
  name?: string
  capabilities?: string[]
}

export interface UsePersonaCapabilitiesResult {
  /** Set of active capability strings for the session's persona. Empty when
   *  no persona is active or the persona has no capabilities. */
  capabilities: Set<string>
  /** The active persona id (session override first, then global). Null when
   *  no persona is active. */
  personaId: string | null
  /** The persona record itself, when resolvable. */
  persona: PersonaWithCapabilities | null
  /** True while the lookup query is in flight. */
  isLoading: boolean
}

/**
 * Resolve the active persona for the current chat session.
 *
 * Reads `activeSessionId` from the chat store, fetches the session detail to
 * learn `active_persona_id`, then finds the matching persona in the cached
 * `/api/personas` list. Returns a Set of capability strings.
 *
 * Stays query-cached: the same hook can be called from many components
 * without refetching. When the session changes, the hook re-resolves.
 */
export function usePersonaCapabilities(): UsePersonaCapabilitiesResult {
  const activeSessionId = useChatStore((s) => s.activeSessionId)

  // Persona roster. One shared query — every affordance subscribes to the
  // same cache entry. Returns the FULL persona records so every
  // `['personas']` consumer (picker, personas page, this hook) sees one
  // cache shape — a subset here used to starve other consumers of fields
  // (e.g. category) via react-query's key dedupe.
  const personasQuery = useQuery({
    queryKey: ['personas'],
    queryFn: async (): Promise<Persona[]> => {
      const res = await api.get<Persona[]>('/api/personas')
      return Array.isArray(res) ? res : []
    },
    staleTime: 60_000,
  })

  // Session detail — only when a session is active. Gives us the
  // session-scoped active_persona_id.
  const sessionQuery = useQuery({
    queryKey: ['session', activeSessionId],
    queryFn: () =>
      api.get<SessionDetail>(`/api/sessions/${encodeURIComponent(activeSessionId ?? '')}`),
    enabled: !!activeSessionId,
    staleTime: 30_000,
  })

  // Global active-persona fallback. Some builds track it server-side via
  // PUT /api/personas/active — when the session has no override, this
  // gives us a sensible default. Optional endpoint; we tolerate 404.
  const activePersonaQuery = useQuery({
    queryKey: ['persona', 'active'],
    queryFn: async (): Promise<ActivePersona | null> => {
      try {
        return await api.get<ActivePersona>('/api/personas/active')
      } catch {
        return null
      }
    },
    staleTime: 60_000,
    retry: false,
  })

  return useMemo<UsePersonaCapabilitiesResult>(() => {
    const personas = personasQuery.data ?? []
    const byId = new Map(personas.map((p) => [p.id, p]))

    const sessionPersonaId = sessionQuery.data?.active_persona_id ?? null
    const globalPersonaId = activePersonaQuery.data?.id ?? null
    const resolvedId = sessionPersonaId ?? globalPersonaId

    const persona = resolvedId ? (byId.get(resolvedId) ?? null) : null

    // Capabilities come from the persona record first; if the persona isn't
    // in the list cache yet, fall back to whatever the active-persona
    // endpoint returned.
    const capsList = persona?.capabilities ?? activePersonaQuery.data?.capabilities ?? []

    return {
      capabilities: new Set(capsList),
      personaId: resolvedId,
      persona,
      isLoading: personasQuery.isLoading || sessionQuery.isLoading,
    }
    // We deliberately exclude the query reference identities from deps —
    // we only care about their `data`. Tanstack query keeps `data` stable
    // across re-renders that don't change the resolved value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [personasQuery.data, sessionQuery.data?.active_persona_id, activePersonaQuery.data])
}
