// useEffectiveProfile — the single source for conditional chat controls
// (design §6.1, project-roots persona workbench).
//
// Resolution order (Task 6 decisions):
//   1. chat-store `effectiveProfile` — rehydrated from SessionDetail on
//      loadSession and refreshed on every WS done-chunk `effective_profile`.
//   2. GET /api/sessions/:id fallback (TanStack Query, staleTime 30s) when a
//      session is active but the store holds no profile yet (e.g. a reload
//      racing the history fetch).
//   3. null — no session, or no persona binds: every conditional control
//      stays hidden.
//
// NEVER derives from persona names, categories, or legacy capability
// strings — the kernel's per-turn EffectiveProfile is the only truth.

import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import { useChatStore } from '@/stores/chat'
import type { EffectiveProfile, SessionDetail } from '@/types'

export function useEffectiveProfile(): EffectiveProfile | null {
  const activeSessionId = useChatStore((s) => s.activeSessionId)
  const stored = useChatStore((s) => s.effectiveProfile)

  const sessionQuery = useQuery({
    queryKey: ['session', activeSessionId],
    queryFn: () =>
      api.get<SessionDetail>(`/api/sessions/${encodeURIComponent(activeSessionId ?? '')}`),
    // Fetch only while a session is live and the store cannot answer yet.
    enabled: !!activeSessionId && stored === null,
    staleTime: 30_000,
  })

  if (!activeSessionId) return null
  return stored ?? sessionQuery.data?.effective_profile ?? null
}
