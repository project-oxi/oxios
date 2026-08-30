// use-workspace — project workspace API hooks (Task 7, design §7.1).
//
// IMPORTANT: this is NOT the same as the legacy `useWorkspaceTree` /
// `useWorkspaceFile` hooks (in the state-workspace browser page) that
// target `/api/workspace/*`. Project-scoped workspace lives at
// `/api/project/workspace/*` (Task 4, session-scoped endpoints). The
// legacy state-workspace hooks were moved to `use-state-workspace.ts`
// in this task so the two surfaces share no path or scope rule.
//
// Wire contracts (Task 4):
//   GET  /api/project/workspace/tree?session_id=&path=
//     → { roots: string[], path: string, entries: TreeEntry[] }
//   GET  /api/project/workspace/file?session_id=&path=
//     → { path: string, content: string, size: number, truncated: boolean }
//   PUT  /api/project/workspace/file body { session_id, path, content }
//     → { path: string, size: number }
//   GET  /api/project/workspace/status?session_id=&root=
//     → { branch: string | null, dirty_count: number | null }
// All four endpoints require `session_id` and reject scopeless callers
// with 409 — never silently fall back to ambient authority.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import { useChatStore } from '@/stores/chat'
import type { TreeEntry } from '@/types'

/** Project-workspace tree entry — Task 4 wire format adds a `path`
 *  field per entry (the legacy state-workspace browser does not return
 *  it; that surface uses the bare TreeEntry). */
export interface ProjectTreeEntry extends TreeEntry {
  path: string
}

export interface ProjectWorkspaceTreeResponse {
  roots: string[]
  path: string
  entries: ProjectTreeEntry[]
}

export interface ProjectWorkspaceFileResponse {
  path: string
  content: string
  size: number
  truncated: boolean
}

export interface ProjectWorkspaceStatusResponse {
  branch: string | null
  dirty_count: number | null
}

/** Project workspace tree — returns roots + entries for the requested path.
 *  Disabled until an active session exists (the server requires
 *  session_id to derive the scope). */
export function useProjectWorkspaceTree(dir?: string) {
  const sid = useChatStore((s) => s.activeSessionId)
  return useQuery({
    queryKey: ['project-workspace-tree', sid, dir],
    queryFn: async () => {
      if (!sid) return null
      const params: Record<string, string> = { session_id: sid }
      if (dir) params.path = dir
      return api.get<ProjectWorkspaceTreeResponse>('/api/project/workspace/tree', params)
    },
    enabled: !!sid,
  })
}

/** Project workspace file — read a single file. Disabled until path AND
 *  session are present. */
export function useProjectWorkspaceFile(path: string | null) {
  const sid = useChatStore((s) => s.activeSessionId)
  return useQuery({
    queryKey: ['project-workspace-file', sid, path],
    queryFn: async () => {
      if (!sid || !path) return null
      const params: Record<string, string> = { session_id: sid, path }
      return api.get<ProjectWorkspaceFileResponse>('/api/project/workspace/file', params)
    },
    enabled: !!sid && !!path,
  })
}

/** Project workspace status — branch + dirty count for the requested root
 *  (default = first root). */
export function useProjectWorkspaceStatus(root?: number) {
  const sid = useChatStore((s) => s.activeSessionId)
  return useQuery({
    queryKey: ['project-workspace-status', sid, root ?? 0],
    queryFn: async () => {
      if (!sid) return null
      const params: Record<string, string> = { session_id: sid }
      if (root !== undefined) params.root = String(root)
      return api.get<ProjectWorkspaceStatusResponse>('/api/project/workspace/status', params)
    },
    enabled: !!sid,
    refetchInterval: 30_000,
  })
}

/** Save a project-workspace file. Caller passes the full path (must be
 *  inside one of the active session's roots); the server enforces scope
 *  before writing. */
export function useSaveProjectFile() {
  const qc = useQueryClient()
  const sid = useChatStore((s) => s.activeSessionId)
  return useMutation({
    mutationFn: async ({ path, content }: { path: string; content: string }) => {
      if (!sid) throw new Error('No active session')
      // PUT takes { session_id, path, content } as JSON.
      await api.put<{ path: string; size: number }>('/api/project/workspace/file', {
        session_id: sid,
        path,
        content,
      })
    },
    onSuccess: (_, vars) => {
      qc.invalidateQueries({ queryKey: ['project-workspace-file', sid, vars.path] })
      qc.invalidateQueries({ queryKey: ['project-workspace-tree', sid] })
    },
  })
}
