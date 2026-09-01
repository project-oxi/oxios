import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api-client'

// ── Types (frozen contract — src/api/routes/operate_routes.rs serde) ────────

/** Why an item needs attention. `waiting_input` has no kernel source yet. */
export type OperateAttentionKind = 'pending_approval' | 'waiting_input' | 'failed_run'

/** Safe follow-up actions the backend may offer for an attention item. */
export type OperateAction = 'approve' | 'reject' | 'open_run' | 'open_agent' | 'open_automation'
export interface OperateOrigin {
  agentId?: string
  sessionId?: string
  automationId?: string
  automationName?: string
  projectId?: string
}

export interface OperateAttentionItem {
  kind: OperateAttentionKind
  /** Approval uuid, registry item id, or task run id. */
  id: string
  /** Exact backend state string ("pending", "failed", …). */
  state: string
  /** Redacted one-line description built only from stored fields. */
  summary: string
  origin: OperateOrigin
  /** RFC3339 timestamp of the source record. */
  timestamp: string
  actions: OperateAction[]
  /** Canonical in-app route, when one exists. */
  detailRoute?: string
}

export interface OperateAttentionResponse {
  items: OperateAttentionItem[]
  generatedAt: string
}

/** Which surface a run row belongs to. */
export type OperateRunKind = 'automation' | 'agent'

/** Automation run trigger (`manual | cron | heartbeat`). */
export type OperateRunTrigger = 'manual' | 'cron' | 'heartbeat'

/**
 * One run row in the merged operate run list. Only recorded fields are
 * present — automation rows never carry projectId, agent rows never carry
 * trigger/automationId.
 */
export interface OperateRun {
  kind: OperateRunKind
  id: string
  name: string
  /** Exact backend status string. */
  status: string
  trigger?: OperateRunTrigger
  automationId?: string
  projectId?: string
  sessionId?: string
  error?: string
  stepsCompleted?: number | null
  stepsTotal?: number | null
  tokensUsed?: number
  costUsd?: number
  modelId?: string
  startedAt?: string
  completedAt?: string
  durationSecs?: number
}

export interface OperateRunsResponse {
  runs: OperateRun[]
  total: number
}

export interface OperateProjectRoot {
  /** Root liveness supplies the path only — no branch/dirty today. */
  path: string
  branch?: string
  dirty?: boolean
}

export interface OperateIssueRef {
  id: string
  title: string
  state: string
}

export interface OperateMilestoneSummary {
  slug: string
  openCount: number
  closedCount: number
}

export interface OperateProjectSummary {
  id: string
  name: string
  rootPaths: string[]
  instructions?: string
  defaultBrainSpace?: string
  lastActiveAt?: string
  createdAt?: string
}

export interface OperateProjectContext {
  project: OperateProjectSummary
  /** Present only when the project has roots. */
  roots?: OperateProjectRoot[]
  issuesOpen: OperateIssueRef[]
  milestones: OperateMilestoneSummary[]
  activeRuns: OperateRun[]
}

/** Capability family — one row per real source record. */
export type OperateCapabilityFamily = 'mcp' | 'skill' | 'engine' | 'security' | 'channel'

/** Capability family — one row per real source record. */
export interface OperateCapability {
  family: OperateCapabilityFamily
  id: string
  name: string
  status: string
  /** e.g. the skill source: bundled/managed/workspace/foundation. */
  scope?: string
  /** Canonical `/operate/system/*` deep route. */
  deepRoute: string
}
export interface OperateCapabilitiesResponse {
  families: OperateCapability[]
}

// ── Hooks ───────────────────────────────────────────────────────────────────

/**
 * Items needing attention (pending approvals + failed/canceled automation
 * runs). `waiting_input` is part of the contract but the kernel has no
 * enumeration source yet, so no such rows arrive today.
 */
export function useOperateAttention() {
  return useQuery({
    queryKey: ['operate', 'attention'],
    queryFn: () => api.get<OperateAttentionResponse>('/api/operate/attention'),
    refetchInterval: 10_000,
  })
}

/** Backend page-size bounds for `/api/operate/runs` (mirrors the clamp). */
export const OPERATE_RUNS_DEFAULT_LIMIT = 50
export const OPERATE_RUNS_MAX_LIMIT = 200

/**
 * Merged automation + agent runs, running first. The limit is clamped to
 * the backend's contract (default 50, max 200) so a caller cannot send an
 * unbounded page.
 */
export function useOperateRuns(limit: number = OPERATE_RUNS_DEFAULT_LIMIT) {
  const raw = Number.isFinite(limit) ? Math.trunc(limit) : OPERATE_RUNS_DEFAULT_LIMIT
  const clamped = Math.min(Math.max(1, raw), OPERATE_RUNS_MAX_LIMIT)
  return useQuery({
    queryKey: ['operate', 'runs', clamped],
    queryFn: () => api.get<OperateRunsResponse>(`/api/operate/runs?limit=${clamped}`),
    refetchInterval: 5_000,
  })
}

/** Everything operate shows for one project. 404 when unknown. */
export function useOperateProjectContext(projectId: string | null | undefined) {
  return useQuery({
    queryKey: ['operate', 'project', projectId, 'context'],
    queryFn: () => api.get<OperateProjectContext>(`/api/operate/projects/${projectId}/context`),
    enabled: !!projectId,
  })
}

/** One row per real MCP server / skill / engine provider / security mode / channel. */
export function useOperateCapabilities() {
  return useQuery({
    queryKey: ['operate', 'capabilities'],
    queryFn: () => api.get<OperateCapabilitiesResponse>('/api/operate/capabilities'),
    staleTime: 60_000,
  })
}
