import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'

// ─── Types ────────────────────────────────────────────────────

export interface IssueAssignment {
  session: string
  acquired_at: string
  /** False means the owning process is gone and the claim is reclaimable. */
  alive: boolean
}

export interface Issue {
  /** Project-scoped issue number. */
  number: number
  title: string
  status: 'open' | 'closed'
  priority: 'low' | 'medium' | 'high' | 'critical'
  labels: string[]
  /** Milestone slug, derived from the reserved `milestone:` label. */
  milestone?: string | null
  assignee?: string | null
  created_at: string
  updated_at: string
  closed_at?: string | null
  assigned_to?: IssueAssignment | null
  /** Present on single reads; omitted from list responses. */
  body?: string | null
  /** Send back on the next mutation so a concurrent edit is detected. */
  content_hash?: string | null
}

export interface Milestone {
  slug: string
  title: string
  description: string
  due?: string | null
  status: 'open' | 'closed'
  created_at: string
  updated_at: string
  total: number
  closed: number
  percent: number
}

export interface IssueFilters {
  status?: 'open' | 'closed'
  priority?: string
  label?: string
  milestone?: string
  text?: string
}

export interface CreateIssueInput {
  title: string
  body?: string
  priority?: string
  labels?: string[]
  milestone?: string
}

export interface UpdateIssueInput {
  title?: string
  body?: string
  priority?: string
  status?: string
  labels?: string[]
  /** From the last read. Omitting it skips the conflict check. */
  content_hash?: string
}

export interface CreateMilestoneInput {
  title: string
  description?: string
  /** `YYYY-MM-DD`. */
  due?: string
  slug?: string
}

// ─── Hooks ────────────────────────────────────────────────────

const issuesKey = (projectId: string, filters?: IssueFilters) =>
  ['project-issues', projectId, filters ?? {}] as const
const milestonesKey = (projectId: string) => ['project-milestones', projectId] as const

function issuesUrl(projectId: string, filters?: IssueFilters) {
  const qs = new URLSearchParams()
  for (const [k, v] of Object.entries(filters ?? {})) {
    if (v) qs.set(k, String(v))
  }
  const query = qs.toString()
  return `/api/projects/${projectId}/issues${query ? `?${query}` : ''}`
}

/** List a project's issues. */
export function useIssues(projectId: string, filters?: IssueFilters) {
  return useQuery({
    queryKey: issuesKey(projectId, filters),
    queryFn: () => api.get<{ items: Issue[]; total: number }>(issuesUrl(projectId, filters)),
    enabled: !!projectId,
  })
}

/** Read one issue, including its body and content hash. */
export function useIssue(projectId: string, number: number | null) {
  return useQuery({
    queryKey: ['project-issue', projectId, number],
    queryFn: () => api.get<Issue>(`/api/projects/${projectId}/issues/${number}`),
    enabled: !!projectId && number != null,
  })
}

/** A project's milestones with derived progress. */
export function useMilestones(projectId: string) {
  return useQuery({
    queryKey: milestonesKey(projectId),
    queryFn: () =>
      api.get<{ items: Milestone[]; total: number }>(`/api/projects/${projectId}/milestones`),
    enabled: !!projectId,
  })
}

/**
 * Invalidate everything derived from a project's issues.
 *
 * Milestone progress is computed from the issues, so an issue mutation has to
 * refresh the milestone list too — otherwise a closed issue leaves a stale
 * `3/7` on screen.
 */
function useIssueInvalidator(projectId: string) {
  const qc = useQueryClient()
  return () => {
    qc.invalidateQueries({ queryKey: ['project-issues', projectId] })
    qc.invalidateQueries({ queryKey: ['project-issue', projectId] })
    qc.invalidateQueries({ queryKey: milestonesKey(projectId) })
  }
}

export function useCreateIssue(projectId: string) {
  const invalidate = useIssueInvalidator(projectId)
  return useMutation({
    mutationFn: (input: CreateIssueInput) =>
      api.post<Issue>(`/api/projects/${projectId}/issues`, input),
    onSuccess: invalidate,
  })
}

export function useUpdateIssue(projectId: string) {
  const invalidate = useIssueInvalidator(projectId)
  return useMutation({
    mutationFn: ({ number, ...input }: UpdateIssueInput & { number: number }) =>
      api.patch<Issue>(`/api/projects/${projectId}/issues/${number}`, input),
    onSuccess: invalidate,
  })
}

export function useCloseIssue(projectId: string) {
  const invalidate = useIssueInvalidator(projectId)
  return useMutation({
    mutationFn: ({ number, content_hash }: { number: number; content_hash?: string }) =>
      api.post<Issue>(`/api/projects/${projectId}/issues/${number}/close`, { content_hash }),
    onSuccess: invalidate,
  })
}

export function useReopenIssue(projectId: string) {
  const invalidate = useIssueInvalidator(projectId)
  return useMutation({
    mutationFn: ({ number, content_hash }: { number: number; content_hash?: string }) =>
      api.post<Issue>(`/api/projects/${projectId}/issues/${number}/reopen`, { content_hash }),
    onSuccess: invalidate,
  })
}

/** Force-release a claim, e.g. one left by a session that is gone. */
export function useReleaseIssue(projectId: string) {
  const invalidate = useIssueInvalidator(projectId)
  return useMutation({
    mutationFn: ({ number }: { number: number }) =>
      api.post<Issue>(`/api/projects/${projectId}/issues/${number}/release`, {}),
    onSuccess: invalidate,
  })
}

/** File an issue under a milestone, or clear it with `null`. */
export function useSetIssueMilestone(projectId: string) {
  const invalidate = useIssueInvalidator(projectId)
  return useMutation({
    mutationFn: ({ number, milestone }: { number: number; milestone: string | null }) =>
      api.put<Issue>(`/api/projects/${projectId}/issues/${number}/milestone`, { milestone }),
    onSuccess: invalidate,
  })
}

export function useCreateMilestone(projectId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (input: CreateMilestoneInput) =>
      api.post<Milestone>(`/api/projects/${projectId}/milestones`, input),
    onSuccess: () => qc.invalidateQueries({ queryKey: milestonesKey(projectId) }),
  })
}

export function useDeleteMilestone(projectId: string) {
  const invalidate = useIssueInvalidator(projectId)
  return useMutation({
    // Deleting strips the label from every member, so the issue lists change
    // too — invalidate both.
    mutationFn: (slug: string) => api.delete(`/api/projects/${projectId}/milestones/${slug}`),
    onSuccess: invalidate,
  })
}
