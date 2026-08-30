// use-state-workspace — legacy state-workspace browser hooks.
//
// These target `/api/workspace/*` (the kernel state directory browser:
// sessions, skills, restore artifacts). They were moved out of
// `use-workspace.ts` in Task 7 to make room for the project-workspace
// hooks (`/api/project/workspace/*`, Task 4) that power the workbench
// shell. The two surfaces share no endpoints or scope rules — keep them
// separated to avoid mixing admin-browse auth from session-scoped auth.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type { TreeEntry } from '@/types'
import type { CreateFileRequest } from '@/types/workspace'

export function useWorkspaceTree(dir?: string) {
  return useQuery({
    queryKey: ['workspace-tree', dir],
    queryFn: async () => {
      const params = dir ? { dir } : undefined
      const res = await api.get<TreeEntry[]>('/api/workspace/tree', params)
      return Array.isArray(res) ? res : []
    },
    refetchInterval: 15000,
  })
}

export function useWorkspaceFile(path: string | null) {
  return useQuery({
    queryKey: ['workspace-file', path],
    queryFn: async () => {
      if (!path) return null
      const res = await api.get<string>(`/api/workspace/file/${encodeURIComponent(path)}`)
      return { path, content: res }
    },
    enabled: !!path,
  })
}

export function useSaveFile() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ path, content }: { path: string; content: string }) => {
      await api.put(`/api/workspace/file/${encodeURIComponent(path)}`, content, true)
    },
    onSuccess: (_, vars) => {
      queryClient.invalidateQueries({ queryKey: ['workspace-file', vars.path] })
      queryClient.invalidateQueries({ queryKey: ['workspace-tree'] })
    },
  })
}

export function useCreateFile() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ path, isDir }: { path: string; isDir?: boolean }) => {
      await api.post(`/api/workspace/file/${encodeURIComponent(path)}`, {
        is_dir: isDir ?? false,
      } as CreateFileRequest)
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['workspace-tree'] })
    },
  })
}

export function useDeleteFile() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (path: string) => {
      await api.delete(`/api/workspace/file/${encodeURIComponent(path)}`)
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['workspace-tree'] })
    },
  })
}
