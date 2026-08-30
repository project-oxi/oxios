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
export function useBrainStats(space: string) {
  return useQuery({
    queryKey: ['brain', 'stats', space],
    queryFn: () => api.get<BrainStats>('/api/brain/stats', { space }),
    staleTime: 30_000,
    enabled: space.length > 0,
  })
}

// ── Search (two-plane envelope, oxibrain 0.8) ──
export function useBrainSearch(
  q: string,
  mode: RetrievalMode | string = 'hybrid',
  limit = 20,
  enabled = false,
  planes: 'memory' | 'documents' | 'both' = 'both',
  space = '',
) {
  return useQuery({
    queryKey: ['brain', 'search', q, mode, limit, planes, space],
    queryFn: () =>
      api.get<SearchResponse | null>('/api/brain/search', {
        q,
        space,
        mode,
        limit: String(limit),
        planes,
      }),
    enabled: enabled && q.trim().length > 0 && space.length > 0,
  })
}

/** One-shot brain search (mutation form) — used by the @-mention flow. */
export function useBrainSearchMutation() {
  return useMutation({
    mutationFn: ({ query, limit, space }: { query: string; limit?: number; space: string }) =>
      api.get<SearchResponse | null>('/api/brain/search', {
        q: query,
        space,
        mode: 'hybrid',
        limit: String(limit ?? 5),
        planes: 'both',
      }),
  })
}

// ── Document history (gix-backed revisions of one tracked vault document) ──
export function useBrainDocumentHistory(
  space: string,
  alias: string,
  locator: string | null,
  limit = 32,
  enabled = true,
) {
  return useQuery({
    queryKey: ['brain', 'document-history', space, alias, locator, limit],
    queryFn: () =>
      api.get<DocumentRevision[] | null>('/api/brain/document-history', {
        space,
        alias,
        ...(locator ? { locator } : {}),
        limit: String(limit),
      }),
    enabled: enabled && !!locator && space.length > 0,
  })
}

// ── Recall (agent context assembly) ──
export function useBrainRecall() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ query, budget, space }: { query: string; budget?: number; space: string }) =>
      api.post<BrainRecallResponse>('/api/brain/recall', { query, budget, space }),
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
export function useBrainEntity(entityId: string | null, space: string) {
  return useQuery({
    queryKey: ['brain', 'entity', space, entityId],
    queryFn: () => api.get<Belief[] | null>(`/api/brain/entity/${entityId}`, { space }),
    enabled: entityId !== null && entityId.length > 0 && space.length > 0,
  })
}

// ── Timeline (timeline:// resource) ──
export function useBrainTimeline(entityId: string | null, space: string) {
  return useQuery({
    queryKey: ['brain', 'timeline', space, entityId],
    queryFn: () =>
      api.get<TimelineEntry[] | null>('/api/brain/timeline', {
        space,
        entity: entityId ?? '',
      }),
    enabled: entityId !== null && entityId.length > 0 && space.length > 0,
  })
}

// ── Why (provenance) ──
export function useBrainWhy(statementId: string | null, space: string) {
  return useQuery({
    queryKey: ['brain', 'why', space, statementId],
    queryFn: () => api.get<ExplainBlock | null>(`/api/brain/why/${statementId}`, { space }),
    enabled: statementId !== null && statementId.length > 0 && space.length > 0,
  })
}

// ── Contradictions ──
export function useBrainContradictions(space: string) {
  return useQuery({
    queryKey: ['brain', 'contradictions', space],
    queryFn: () => api.get<ContradictionDetail[] | null>('/api/brain/contradictions', { space }),
    enabled: space.length > 0,
  })
}

// ── Brief (entity/space/topic markdown page) ──
export function useBrainBrief(
  targetKind: 'entity' | 'space' | 'topic',
  entityId: string | null,
  topic: string | null,
  space: string,
) {
  return useQuery({
    queryKey: ['brain', 'brief', targetKind, entityId, topic, space],
    queryFn: () =>
      api.get<BrainBriefResponse>('/api/brain/brief', {
        space,
        target_kind: targetKind,
        ...(entityId ? { entity_id: entityId } : {}),
        ...(topic ? { topic } : {}),
      }),
    enabled:
      space.length > 0 &&
      (targetKind === 'space' ||
        (targetKind === 'entity' && (entityId?.length ?? 0) > 0) ||
        (targetKind === 'topic' && (topic?.length ?? 0) > 0)),
  })
}

// ── Graph (traverse tool) ──
export function useBrainGraph(
  start: string | null,
  depth = 2,
  maxNodes = 64,
  direction: 'both' | 'out' | 'in' = 'both',
  space = '',
) {
  return useQuery({
    queryKey: ['brain', 'graph', start, depth, maxNodes, direction, space],
    queryFn: () =>
      api.get<TraversalResult | null>('/api/brain/graph', {
        space,
        start: start ?? '',
        depth: String(depth),
        max_nodes: String(maxNodes),
        direction,
      }),
    enabled: start !== null && start.length > 0 && space.length > 0,
  })
}

// ── Operations (review_merges console data) ──
export function useBrainMerges(space: string, enabled = true) {
  return useQuery({
    queryKey: ['brain', 'operations', space, 'merges'],
    queryFn: () => api.get<MergeRecord[] | null>('/api/brain/operations/merges', { space }),
    enabled: enabled && space.length > 0,
  })
}

export function useBrainFailures(space: string, enabled = true) {
  return useQuery({
    queryKey: ['brain', 'operations', space, 'failures'],
    queryFn: () => api.get<ExtractionFailure[] | null>('/api/brain/operations/failures', { space }),
    enabled: enabled && space.length > 0,
  })
}

export function useBrainSources(space: string, enabled = true) {
  return useQuery({
    queryKey: ['brain', 'operations', space, 'sources'],
    queryFn: () => api.get<SourceRow[] | null>('/api/brain/operations/sources', { space }),
    enabled: enabled && space.length > 0,
  })
}

export function useBrainOperations(section: OperationsSection, space: string) {
  const merges = useBrainMerges(space, section === 'merges')
  const failures = useBrainFailures(space, section === 'failures')
  const sources = useBrainSources(space, section === 'sources')
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
export function useBrainSpaceOverview(space: string) {
  return useQuery({
    queryKey: ['brain', 'space', space],
    queryFn: () => api.get<SpaceOverview | null>('/api/brain/space', { space }),
    staleTime: 30_000,
    enabled: space.length > 0,
  })
}
