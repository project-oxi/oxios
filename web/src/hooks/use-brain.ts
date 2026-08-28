import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type {
  Belief,
  BrainBriefResponse,
  BrainRecallResponse,
  BrainStats,
  BrainStatus,
  ContradictionDetail,
  DocumentRevision,
  ExplainBlock,
  ExtractionFailure,
  MergeRecord,
  OperationsSection,
  RetrievalMode,
  SearchResponse,
  SourceRow,
  SpaceOverview,
  SpaceSummary,
  TimelineEntry,
  TraversalResult,
} from '@/types/brain'

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

// ── Search (two-plane envelope, oxibrain 0.8) ──
export function useBrainSearch(
  q: string,
  mode: RetrievalMode | string = 'hybrid',
  limit = 20,
  enabled = false,
  planes: 'memory' | 'documents' | 'both' = 'both',
) {
  return useQuery({
    queryKey: ['brain', 'search', q, mode, limit, planes],
    queryFn: () =>
      api.get<SearchResponse | null>('/api/brain/search', {
        q,
        mode,
        limit: String(limit),
        planes,
      }),
    enabled: enabled && q.trim().length > 0,
  })
}

/** One-shot brain search (mutation form) — used by the @-mention flow. */
export function useBrainSearchMutation() {
  return useMutation({
    mutationFn: ({ query, limit }: { query: string; limit?: number }) =>
      api.get<SearchResponse | null>('/api/brain/search', {
        q: query,
        mode: 'hybrid',
        limit: String(limit ?? 5),
        planes: 'both',
      }),
  })
}

// ── Document history (gix-backed revisions of one tracked vault document) ──
export function useBrainDocumentHistory(
  alias: string,
  locator: string | null,
  limit = 32,
  enabled = true,
) {
  return useQuery({
    queryKey: ['brain', 'document-history', alias, locator, limit],
    queryFn: () =>
      api.get<DocumentRevision[] | null>('/api/brain/document-history', {
        alias,
        ...(locator ? { locator } : {}),
        limit: String(limit),
      }),
    enabled: enabled && !!locator,
  })
}

// ── Recall (agent context assembly) ──
export function useBrainRecall() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ query, budget }: { query: string; budget?: number }) =>
      api.post<BrainRecallResponse>('/api/brain/recall', { query, budget }),
    onSuccess: () => {
      // Recall writes new traces — refresh counters and search-indexed
      // queries, but leave entity/timeline/why detail pages alone (those
      // are content-stable per entity_id).
      qc.invalidateQueries({ queryKey: ['brain', 'status'] })
      qc.invalidateQueries({ queryKey: ['brain', 'stats'] })
      qc.invalidateQueries({ queryKey: ['brain', 'search'] })
    },
  })
}

// ── Entity (beliefs via entity:// resource) ──
export function useBrainEntity(entityId: string | null) {
  return useQuery({
    queryKey: ['brain', 'entity', entityId],
    queryFn: () => api.get<Belief[] | null>(`/api/brain/entity/${entityId}`),
    enabled: entityId !== null && entityId.length > 0,
  })
}

// ── Timeline (timeline:// resource) ──
export function useBrainTimeline(entityId: string | null) {
  return useQuery({
    queryKey: ['brain', 'timeline', entityId],
    queryFn: () =>
      api.get<TimelineEntry[] | null>('/api/brain/timeline', { entity: entityId ?? '' }),
    enabled: entityId !== null && entityId.length > 0,
  })
}

// ── Why (provenance) ──
export function useBrainWhy(statementId: string | null) {
  return useQuery({
    queryKey: ['brain', 'why', statementId],
    queryFn: () => api.get<ExplainBlock | null>(`/api/brain/why/${statementId}`),
    enabled: statementId !== null && statementId.length > 0,
  })
}

// ── Contradictions ──
export function useBrainContradictions() {
  return useQuery({
    queryKey: ['brain', 'contradictions'],
    queryFn: () => api.get<ContradictionDetail[] | null>('/api/brain/contradictions'),
  })
}

// ── Brief (entity/space/topic markdown page) ──
export function useBrainBrief(
  targetKind: 'entity' | 'space' | 'topic',
  entityId: string | null,
  topic: string | null,
) {
  return useQuery({
    queryKey: ['brain', 'brief', targetKind, entityId, topic],
    queryFn: () =>
      api.get<BrainBriefResponse>('/api/brain/brief', {
        target_kind: targetKind,
        ...(entityId ? { entity_id: entityId } : {}),
        ...(topic ? { topic } : {}),
      }),
    enabled:
      targetKind === 'space' ||
      (targetKind === 'entity' && (entityId?.length ?? 0) > 0) ||
      (targetKind === 'topic' && (topic?.length ?? 0) > 0),
  })
}

// ── Graph (traverse tool) ──
export function useBrainGraph(
  start: string | null,
  depth = 2,
  maxNodes = 64,
  direction: 'both' | 'out' | 'in' = 'both',
) {
  return useQuery({
    queryKey: ['brain', 'graph', start, depth, maxNodes, direction],
    queryFn: () =>
      api.get<TraversalResult | null>('/api/brain/graph', {
        start: start ?? '',
        depth: String(depth),
        max_nodes: String(maxNodes),
        direction,
      }),
    enabled: start !== null && start.length > 0,
  })
}

// ── Operations (review_merges console data) ──
export function useBrainMerges(enabled = true) {
  return useQuery({
    queryKey: ['brain', 'operations', 'merges'],
    queryFn: () => api.get<MergeRecord[] | null>('/api/brain/operations/merges'),
    enabled,
  })
}

export function useBrainFailures(enabled = true) {
  return useQuery({
    queryKey: ['brain', 'operations', 'failures'],
    queryFn: () => api.get<ExtractionFailure[] | null>('/api/brain/operations/failures'),
    enabled,
  })
}

export function useBrainSources(enabled = true) {
  return useQuery({
    queryKey: ['brain', 'operations', 'sources'],
    queryFn: () => api.get<SourceRow[] | null>('/api/brain/operations/sources'),
    enabled,
  })
}

export function useBrainOperations(section: OperationsSection) {
  const merges = useBrainMerges(section === 'merges')
  const failures = useBrainFailures(section === 'failures')
  const sources = useBrainSources(section === 'sources')
  if (section === 'merges') return merges
  if (section === 'failures') return failures
  return sources
}

// ── Spaces ──
export function useBrainSpaces() {
  return useQuery({
    queryKey: ['brain', 'spaces'],
    queryFn: () => api.get<SpaceSummary[] | null>('/api/brain/spaces'),
    staleTime: 60_000,
  })
}

// ── Configured-space overview (space:// resource) ──
export function useBrainSpaceOverview() {
  return useQuery({
    queryKey: ['brain', 'space'],
    queryFn: () => api.get<SpaceOverview | null>('/api/brain/space'),
    staleTime: 30_000,
  })
}
