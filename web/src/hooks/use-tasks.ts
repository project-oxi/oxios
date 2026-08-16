// use-tasks — React Query hooks for task API (RFC-043)

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type {
  AddCommentParams,
  AddDependencyParams,
  CreateTaskParams,
  CronMigrationReport,
  ListTasksParams,
  MigrateCronParams,
  SetScheduleParams,
  SetVerifyParams,
  Task,
  TaskComment,
  TaskRun,
  TaskStatus,
  UpdateTaskParams,
} from '@/types/task'

// ── List ──

export function useTasks(params?: ListTasksParams) {
  const query = new URLSearchParams()
  if (params?.statuses?.length) query.set('statuses', params.statuses.join(','))
  if (params?.assigneeAgentId) query.set('assignee', params.assigneeAgentId)
  if (params?.parentTaskId) query.set('parent', params.parentTaskId)
  if (params?.limit) query.set('limit', String(params.limit))
  if (params?.offset) query.set('offset', String(params.offset))

  const qs = query.toString()
  return useQuery({
    queryKey: ['tasks', qs],
    queryFn: () => api.get<{ tasks: Task[]; count: number }>(`/api/tasks${qs ? `?${qs}` : ''}`),
  })
}

// ── Get ──

export function useTask(id: string | null) {
  return useQuery({
    queryKey: ['task', id],
    queryFn: () => api.get<Task>(`/api/tasks/${id}`),
    enabled: !!id,
  })
}

// ── Create ──

export function useCreateTask() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (params: CreateTaskParams) => api.post<Task>('/api/tasks', params),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['tasks'] }),
  })
}

// ── Create batch ──

export function useCreateTasksBatch() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (tasks: CreateTaskParams[]) =>
      api.post<{ tasks: Task[] }>('/api/tasks/batch', { tasks }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['tasks'] }),
  })
}

// ── Update (partial) ──

export function useUpdateTask() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...params }: { id: string } & UpdateTaskParams) =>
      api.put<Task>(`/api/tasks/${id}`, params),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['tasks'] })
      qc.invalidateQueries({ queryKey: ['task', vars.id] })
    },
  })
}

// ── Delete ──

export function useDeleteTask() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.delete(`/api/tasks/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['tasks'] }),
  })
}

// ── Update status ──

export function useUpdateTaskStatus() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, status }: { id: string; status: TaskStatus }) =>
      api.put(`/api/tasks/${id}/status`, { status }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['tasks'] }),
  })
}

// ── Set schedule ──

export function useSetTaskSchedule() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...params }: { id: string } & SetScheduleParams) =>
      api.put(`/api/tasks/${id}/schedule`, params),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['tasks'] })
      qc.invalidateQueries({ queryKey: ['task', vars.id] })
    },
  })
}

// ── Set verify ──

export function useSetTaskVerify() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...params }: { id: string } & SetVerifyParams) =>
      api.put(`/api/tasks/${id}/verify`, params),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['tasks'] })
      qc.invalidateQueries({ queryKey: ['task', vars.id] })
    },
  })
}

// ── Run task ──

export function useRunTask() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id }: { id: string }) =>
      api.post<{ id: string; success: boolean; summary: string }>(`/api/tasks/${id}/run`, {}),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['tasks'] })
      qc.invalidateQueries({ queryKey: ['task-runs', vars.id] })
    },
  })
}

// ── Run history ──

export function useTaskRuns(id: string | null) {
  return useQuery({
    queryKey: ['task-runs', id],
    queryFn: () => api.get<{ runs: TaskRun[]; count: number }>(`/api/tasks/${id}/runs`),
    enabled: !!id,
  })
}

// ── Comments ──

export function useTaskComments(taskId: string | null) {
  return useQuery({
    queryKey: ['task-comments', taskId],
    queryFn: () => api.get<{ comments: TaskComment[] }>(`/api/tasks/${taskId}/comments`),
    enabled: !!taskId,
  })
}

export function useAddTaskComment(taskId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (params: AddCommentParams) =>
      api.post<TaskComment>(`/api/tasks/${taskId}/comments`, params),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['task-comments', taskId] }),
  })
}

export function useUpdateTaskComment(taskId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ commentId, content }: { commentId: string; content: string }) =>
      api.put<TaskComment>(`/api/tasks/${taskId}/comments/${commentId}`, { content }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['task-comments', taskId] }),
  })
}

export function useDeleteTaskComment(taskId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (commentId: string) => api.delete(`/api/tasks/${taskId}/comments/${commentId}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['task-comments', taskId] }),
  })
}

// ── Dependencies ──

export function useTaskDependencies(taskId: string | null) {
  return useQuery({
    queryKey: ['task-dependencies', taskId],
    queryFn: () => api.get<{ dependencies: Task[] }>(`/api/tasks/${taskId}/dependencies`),
    enabled: !!taskId,
  })
}

export function useAddTaskDependency(taskId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (params: AddDependencyParams) =>
      api.post<Task>(`/api/tasks/${taskId}/dependencies`, params),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['task-dependencies', taskId] })
      qc.invalidateQueries({ queryKey: ['task', taskId] })
      qc.invalidateQueries({ queryKey: ['tasks'] })
    },
  })
}

export function useRemoveTaskDependency(taskId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (dependsOnTaskId: string) =>
      api.delete(`/api/tasks/${taskId}/dependencies/${dependsOnTaskId}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['task-dependencies', taskId] })
      qc.invalidateQueries({ queryKey: ['task', taskId] })
      qc.invalidateQueries({ queryKey: ['tasks'] })
    },
  })
}

// ── Cron migration ──

export function useMigrateCronTasks() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (params?: MigrateCronParams) =>
      api.post<CronMigrationReport>('/api/tasks/migrate-cron', params ?? {}),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['tasks'] }),
  })
}
