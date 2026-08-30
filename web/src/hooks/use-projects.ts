import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type { Project } from '@/types'

// ─── Types ────────────────────────────────────────────────────

export interface CreateProjectInput {
  name: string
  /** Absolute directories this project spans; the first root is the cwd. */
  root_paths?: string[]
  /** Custom instructions injected into the system prompt. */
  instructions?: string
}

export interface UpdateProjectInput {
  name?: string
  root_paths?: string[]
  instructions?: string
  /** Default brain space. Tri-state on the wire: omit = keep current,
   *  `null` = clear, string = set (mirrors PUT /api/projects/:id). */
  default_brain_space?: string | null
}

// ─── Hooks ────────────────────────────────────────────────────

/** List all projects with optional search. */
export function useProjects(search?: string) {
  return useQuery({
    queryKey: ['projects', search],
    queryFn: () => {
      const url = search ? `/api/projects?search=${encodeURIComponent(search)}` : '/api/projects'
      return api.get<{ items: Project[]; total: number }>(url)
    },
  })
}

/** Get a single project by ID. */
export function useProject(id: string | null) {
  return useQuery({
    queryKey: ['project', id],
    queryFn: () => api.get<Project>(`/api/projects/${id}`),
    enabled: !!id,
  })
}

/** Liveness of one project root (design §9 stale-root surfacing). */
export interface RootStatus {
  path: string
  exists: boolean
  is_dir: boolean
}

/** Per-root exists/is_dir for a project (GET /api/projects/:id/roots/status). */
export function useProjectRootsStatus(id: string | null | undefined) {
  return useQuery({
    queryKey: ['project-roots-status', id],
    queryFn: () => api.get<RootStatus[]>(`/api/projects/${id}/roots/status`),
    enabled: !!id,
  })
}

/** Create a new project. */
export function useCreateProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (input: CreateProjectInput) => api.post<Project>('/api/projects', input),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['projects'] })
    },
  })
}

/** Update an existing project. */
export function useUpdateProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...input }: UpdateProjectInput & { id: string }) =>
      api.put<Project>(`/api/projects/${id}`, input),
    onSuccess: (_, vars) => {
      qc.invalidateQueries({ queryKey: ['projects'] })
      qc.invalidateQueries({ queryKey: ['project', vars.id] })
    },
  })
}

/** Delete a project. */
export function useDeleteProject() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.delete(`/api/projects/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['projects'] })
    },
  })
}
