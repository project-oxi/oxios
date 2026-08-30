import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type { Project } from '@/types'

// ─── Types ────────────────────────────────────────────────────

export interface CreateProjectInput {
  name: string
  emoji?: string
  /** RFC-025: custom instructions injected into the system prompt. */
  instructions?: string
}

export interface UpdateProjectInput {
  name?: string
  paths?: string[]
  tags?: string[]
  emoji?: string
  description?: string
  memory_visible?: boolean
  /** RFC-025: referenced Mount IDs. */
  mount_ids?: string[]
  /** RFC-025: custom instructions injected into the system prompt. */
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

/** Get memories linked to a project. */
export function useProjectMemories(projectId: string | null, page = 1) {
  return useQuery({
    queryKey: ['project-memories', projectId, page],
    queryFn: () =>
      api.get<{ items: unknown[]; total: number }>(
        `/api/projects/${projectId}/memories?page=${page}`,
      ),
    enabled: !!projectId,
  })
}

/** Link a memory to a project. */
export function useLinkProjectMemory() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ projectId, memoryId }: { projectId: string; memoryId: string }) =>
      api.post(`/api/projects/${projectId}/memories`, { memory_id: memoryId }),
    onSuccess: (_, vars) => {
      qc.invalidateQueries({ queryKey: ['project-memories', vars.projectId] })
    },
  })
}

/** Unlink a memory from a project. */
export function useUnlinkProjectMemory() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ projectId, memoryId }: { projectId: string; memoryId: string }) =>
      api.delete(`/api/projects/${projectId}/memories/${memoryId}`),
    onSuccess: (_, vars) => {
      qc.invalidateQueries({ queryKey: ['project-memories', vars.projectId] })
    },
  })
}
