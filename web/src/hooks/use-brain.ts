import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type {
  BrainRecallResponse,
  BrainSearchResponse,
  BrainStats,
  BrainStatus,
  Contradiction,
  EntityDetail,
  TimelineEntry,
  WhyDetail,
} from '@/types/brain'

// ── Status ──
export function useBrainStatus() {
  return useQuery({
    queryKey: ['brain', 'status'],
    queryFn: () => api.get<BrainStatus>('/api/brain/status'),
    staleTime: 15_000,
    refetchInterval: 30_000,
  })
}

// ── Stats ──
export function useBrainStats() {
  return useQuery({
    queryKey: ['brain', 'stats'],
    queryFn: () => api.get<BrainStats>('/api/brain/stats'),
    staleTime: 30_000,
  })
}

// ── Search ──
export function useBrainSearch(q: string, mode = 'hybrid', limit = 20, enabled = false) {
  return useQuery({
    queryKey: ['brain', 'search', q, mode, limit],
    queryFn: () =>
      api.get<BrainSearchResponse>('/api/brain/search', { q, mode, limit: String(limit) }),
    enabled: enabled && q.trim().length > 0,
  })
}

/** One-shot brain search (mutation form) — used by the @-mention flow. */
export function useBrainSearchMutation() {
  return useMutation({
    mutationFn: ({ query, limit }: { query: string; limit?: number }) =>
      api.get<BrainSearchResponse>('/api/brain/search', {
        q: query,
        mode: 'hybrid',
        limit: String(limit ?? 5),
      }),
  })
}

// ── Recall (agent context assembly) ──
export function useBrainRecall() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ query, budget }: { query: string; budget?: number }) =>
      api.post<BrainRecallResponse>('/api/brain/recall', { query, budget }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['brain'] })
    },
  })
}

// ── Entity ──
export function useBrainEntity(entityId: string | null) {
  return useQuery({
    queryKey: ['brain', 'entity', entityId],
    queryFn: () => api.get<EntityDetail>(`/api/brain/entity/${entityId}`),
    enabled: !!entityId,
  })
}

// ── Timeline ──
export function useBrainTimeline(
  entityId: string | null,
  from?: number | null,
  to?: number | null,
) {
  const params: Record<string, string> = {}
  if (from != null) params.from = String(from)
  if (to != null) params.to = String(to)
  return useQuery({
    queryKey: ['brain', 'timeline', entityId, from, to],
    queryFn: () => api.get<TimelineEntry[]>(`/api/brain/timeline?entity=${entityId}`, params),
    enabled: !!entityId,
  })
}

// ── Why (provenance) ──
export function useBrainWhy(statementId: string | null) {
  return useQuery({
    queryKey: ['brain', 'why', statementId],
    queryFn: () => api.get<WhyDetail>(`/api/brain/why/${statementId}`),
    enabled: !!statementId,
  })
}

// ── Contradictions ──
export function useBrainContradictions() {
  return useQuery({
    queryKey: ['brain', 'contradictions'],
    queryFn: () => api.get<Contradiction[]>('/api/brain/contradictions'),
    staleTime: 30_000,
  })
}
