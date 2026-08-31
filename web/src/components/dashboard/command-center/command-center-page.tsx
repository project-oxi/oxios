import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ActiveExecutionList } from '@/components/dashboard/command-center/active-execution-list'
import { AttentionQueue } from '@/components/dashboard/command-center/attention-queue'
import { CommandCenterHeader } from '@/components/dashboard/command-center/command-center-header'
import { LiveTimelinePanel } from '@/components/dashboard/command-center/live-timeline-panel'
import { OperatingPicture } from '@/components/dashboard/command-center/operating-picture'
import { RunInspector } from '@/components/dashboard/command-center/run-inspector'
import { selectActiveRuns, systemHealth } from '@/components/dashboard/command-center/runs'
import { SystemLinks } from '@/components/dashboard/command-center/system-links'
import { useMediaQuery } from '@/hooks/use-media-query'
import { api } from '@/lib/api-client'
import type { SystemStatus } from '@/types'
import type { AgentListItem } from '@/types/agent'

/** Elapsed-time refresh cadence; matches the status poll cadence. */
const CLOCK_MS = 15_000

/**
 * Command center dashboard (spec: 2026-08-31 redesign).
 *
 * Composition root for the operational layout: attention queue first,
 * then active execution beside the live timeline, the operating picture,
 * and de-emphasized system links. Desktop gets a right-hand run
 * inspector when a row is selected; below `lg` it becomes a sheet and
 * the two work surfaces stack. All queries, SSE, and mutations stay in
 * the existing hooks/stores — selection is dashboard-local state keyed
 * by agentId.
 */
export function CommandCenterPage() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const isDesktop = useMediaQuery('(min-width: 1024px)')

  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())

  const status = useQuery({
    queryKey: ['status'],
    queryFn: () => api.get<SystemStatus>('/api/status'),
    refetchInterval: 10_000,
  })
  const agents = useQuery({
    queryKey: ['agents'],
    queryFn: () => api.get<{ items: AgentListItem[] }>('/api/agents'),
    refetchInterval: 5_000,
    select: (d) => (Array.isArray(d.items) ? d.items : []),
  })

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), CLOCK_MS)
    return () => window.clearInterval(id)
  }, [])

  const runs = useMemo(() => selectActiveRuns(agents.data ?? [], now), [agents.data, now])

  // Selection: dashboard-local. When the selected run leaves the active
  // set, keep its last snapshot so the inspector can show a terminal
  // state instead of vanishing mid-inspection.
  const selectedRun = runs.find((r) => r.id === selectedId) ?? null
  const [lastSelected, setLastSelected] = useState<AgentListItem | null>(null)
  useEffect(() => {
    if (selectedRun) setLastSelected(selectedRun)
  }, [selectedRun])
  const inspectorRun = selectedId ? (selectedRun ?? lastSelected) : null

  // ONE kill mutation serves both the list rows and the inspector, so a
  // duplicate submit from either surface is impossible.
  const kill = useMutation({
    mutationFn: (id: string) => api.post(`/api/agents/${id}/kill`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['agents'] })
      toast.success(t('commandPalette.killedAgent'))
    },
    onError: () => toast.error(t('commandPalette.controlFailed')),
  })
  const stoppingId = kill.isPending ? (kill.variables ?? null) : null

  const health =
    status.isLoading && !status.data ? null : systemHealth(status.data, Boolean(status.isError))

  return (
    <div className="space-y-4 animate-fade-in-up">
      <CommandCenterHeader health={health} />

      <AttentionQueue />

      <div className="flex items-start gap-4">
        <div className="min-w-0 flex-1 space-y-4">
          <div className="grid gap-4 lg:grid-cols-2">
            <ActiveExecutionList
              runs={runs}
              isLoading={agents.isLoading}
              isError={agents.isError}
              onRetry={() => agents.refetch()}
              selectedId={selectedId}
              onSelect={setSelectedId}
              onStop={(id) => kill.mutate(id)}
              stoppingId={stoppingId}
              now={now}
            />
            <LiveTimelinePanel selectedAgentId={selectedId} />
          </div>

          <OperatingPicture activeCount={runs.length} />

          <SystemLinks status={status.data} />
        </div>

        {inspectorRun && isDesktop && (
          <RunInspector
            key={inspectorRun.id}
            run={inspectorRun}
            stale={!selectedRun}
            onClose={() => setSelectedId(null)}
            variant="panel"
            onStop={(id) => kill.mutate(id)}
            stopping={stoppingId === inspectorRun.id}
            now={now}
          />
        )}
      </div>

      {inspectorRun && !isDesktop && (
        <RunInspector
          key={`dialog-${inspectorRun.id}`}
          run={inspectorRun}
          stale={!selectedRun}
          onClose={() => setSelectedId(null)}
          variant="dialog"
          onStop={(id) => kill.mutate(id)}
          stopping={stoppingId === inspectorRun.id}
          now={now}
        />
      )}
    </div>
  )
}
