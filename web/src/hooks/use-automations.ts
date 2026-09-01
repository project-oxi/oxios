// use-automations — React Query hooks for the Automation API.
//
// Endpoint map (mirrors the Rust `automation_routes` module):
//   GET    /api/automations                  → list
//   POST   /api/automations                  → create
//   GET    /api/automations/:id              → get one
//   PUT    /api/automations/:id              → partial update
//   DELETE /api/automations/:id              → delete
//   PUT    /api/automations/:id/status       → set status (pause/resume)
//   PUT    /api/automations/:id/trigger      → set trigger config (replaces /schedule)
//   PUT    /api/automations/:id/verify       → set verify gate
//   POST   /api/automations/:id/run          → manual synchronous run
//   GET    /api/automations/:id/runs         → run history
//
// Query keys are rooted at `['automations']` so React Query can invalidate
// every list view by key prefix. Mutations invalidate both the list and the
// per-id detail key plus the per-automation run-history key — keeps list,
// detail, and run views consistent after any state change.
//
// Deprecated endpoints from the prior Task* domain (batch create, comments,
// dependencies) are gone — there is no migration alias and no fallback.
// /api/tasks routes are unregistered on this branch (the kernel's Router
// returns 404 for /api/tasks* requests).

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'
import type {
  Automation,
  AutomationRun,
  AutomationStatus,
  CreateAutomationParams,
  ListAutomationsParams,
  SetTriggerParams,
  SetVerifyParams,
  UpdateAutomationParams,
} from '@/types/automation'

// ── List ──

export function useAutomations(params?: ListAutomationsParams) {
  const query = new URLSearchParams()
  if (params?.status) query.set('status', params.status)
  if (params?.limit) query.set('limit', String(params.limit))
  if (params?.offset) query.set('offset', String(params.offset))

  const qs = query.toString()
  return useQuery({
    queryKey: ['automations', qs],
    queryFn: () =>
      api.get<{ automations: Automation[]; count: number }>(
        `/api/automations${qs ? `?${qs}` : ''}`,
      ),
  })
}

// ── Get ──

export function useAutomation(id: string | null) {
  return useQuery({
    queryKey: ['automation', id],
    queryFn: () => api.get<Automation>(`/api/automations/${id}`),
    enabled: !!id,
  })
}

// ── Create ──

export function useCreateAutomation() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (params: CreateAutomationParams) =>
      api.post<Automation>('/api/automations', params),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['automations'] })
    },
  })
}

// ── Update (partial) ──

export function useUpdateAutomation() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...params }: { id: string } & UpdateAutomationParams) =>
      api.put<Automation>(`/api/automations/${id}`, params),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['automations'] })
      qc.invalidateQueries({ queryKey: ['automation', vars.id] })
    },
  })
}

// ── Delete ──

export function useDeleteAutomation() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => api.delete(`/api/automations/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['automations'] })
    },
  })
}

// ── Update status (pause / resume) ──

export function useUpdateAutomationStatus() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, status }: { id: string; status: AutomationStatus }) =>
      api.put(`/api/automations/${id}/status`, { status }),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['automations'] })
      qc.invalidateQueries({ queryKey: ['automation', vars.id] })
    },
  })
}

// ── Set trigger ──

export function useSetAutomationTrigger() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...params }: { id: string } & SetTriggerParams) =>
      api.put<Automation>(`/api/automations/${id}/trigger`, params),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['automations'] })
      qc.invalidateQueries({ queryKey: ['automation', vars.id] })
    },
  })
}

// ── Set verify ──

export function useSetAutomationVerify() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id, ...params }: { id: string } & SetVerifyParams) =>
      api.put<Automation>(`/api/automations/${id}/verify`, params),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['automations'] })
      qc.invalidateQueries({ queryKey: ['automation', vars.id] })
    },
  })
}

// ── Run automation (manual synchronous) ──

export function useRunAutomation() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ id }: { id: string }) =>
      api.post<{ id: string; run_id: string; success: boolean; summary: string }>(
        `/api/automations/${id}/run`,
        {},
      ),
    onSuccess: (_data, vars) => {
      qc.invalidateQueries({ queryKey: ['automations'] })
      qc.invalidateQueries({ queryKey: ['automation', vars.id] })
      qc.invalidateQueries({ queryKey: ['automation-runs', vars.id] })
    },
  })
}

// ── Run history ──

export function useAutomationRuns(id: string | null) {
  return useQuery({
    queryKey: ['automation-runs', id],
    queryFn: () => api.get<{ runs: AutomationRun[]; count: number }>(`/api/automations/${id}/runs`),
    enabled: !!id,
  })
}
