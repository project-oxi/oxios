import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Bot, CalendarClock, ExternalLink, Square } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ActiveExecutionList } from '@/components/dashboard/command-center/active-execution-list'
import { StatusChip } from '@/components/dashboard/command-center/run-inspector'
import { isStoppable, selectActiveRuns } from '@/components/dashboard/command-center/runs'
import { OperateInspector } from '@/components/operate/operate-inspector'
import { ErrorState } from '@/components/shared/error-state'
import { PageHeader } from '@/components/shared/page-header'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { useMediaQuery } from '@/hooks/use-media-query'
import { type OperateRun, useOperateRuns } from '@/hooks/use-operate'
import { api } from '@/lib/api-client'
import { cn, formatDuration, formatRelativeTime } from '@/lib/utils'
import type { AgentListItem } from '@/types/agent'

/** Elapsed-time refresh cadence; matches the dashboard. */
const CLOCK_MS = 15_000

/** The dashboard keeps its own row shape — adapt it for the inspector. */
function agentToOperateRun(a: AgentListItem): OperateRun {
  return {
    kind: 'agent',
    id: a.id,
    name: a.name,
    status: a.status,
    projectId: a.project_id ?? undefined,
    sessionId: a.session_id ?? undefined,
    error: a.error ?? undefined,
    stepsCompleted: a.steps_completed,
    stepsTotal: a.steps_total ?? undefined,
    tokensUsed: a.tokens_used,
    costUsd: a.cost_usd,
    modelId: a.model_id || undefined,
    startedAt: a.started_at ?? undefined,
    completedAt: a.completed_at ?? undefined,
    durationSecs: a.duration_secs ?? undefined,
  }
}

/**
 * Run center (design §6) — one place for everything currently executing:
 * the active agent executions (same ['agents'] family as the dashboard)
 * above the operate run inventory split into automation and agent
 * tables. Row selection opens the shared operate inspector; automation
 * rows deep-link to their canonical studio page via a plain anchor (the
 * typed route lands with integration). No trigger editing — triggers are
 * read-only facts here.
 */
export function RunCenter() {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const isDesktop = useMediaQuery('(min-width: 1024px)')
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), CLOCK_MS)
    return () => window.clearInterval(id)
  }, [])

  // Active executions — the SAME ['agents'] family the dashboard polls.
  const agents = useQuery({
    queryKey: ['agents'],
    queryFn: () => api.get<{ items: AgentListItem[] }>('/api/agents'),
    refetchInterval: 5_000,
    select: (d) => (Array.isArray(d.items) ? d.items : []),
  })
  const activeRuns = useMemo(() => selectActiveRuns(agents.data ?? [], now), [agents.data, now])

  const runs = useOperateRuns()
  const inventory = runs.data?.runs ?? []
  const automationRuns = useMemo(
    () => inventory.filter((r) => r.kind === 'automation'),
    [inventory],
  )
  const agentRunRows = useMemo(() => inventory.filter((r) => r.kind === 'agent'), [inventory])

  // Selection: page-local OperateRun snapshot (rows can leave the list
  // mid-inspection — keep the last snapshot so the inspector persists).
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [lastSelected, setLastSelected] = useState<OperateRun | null>(null)
  const selectedAgentRun = activeRuns.find((r) => r.id === selectedId)
  const selectedInventoryRun = inventory.find((r) => r.id === selectedId)
  const liveSelected =
    selectedInventoryRun ?? (selectedAgentRun ? agentToOperateRun(selectedAgentRun) : null)
  const inspectorRun = selectedId ? (liveSelected ?? lastSelected) : null
  useEffect(() => {
    if (liveSelected) setLastSelected(liveSelected)
  }, [liveSelected])

  // ONE kill mutation for the active list, the agent table and the
  // inspector, so a duplicate submit from any surface is impossible.
  const kill = useMutation({
    mutationFn: (id: string) => api.post(`/api/agents/${id}/kill`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['agents'] })
      qc.invalidateQueries({ queryKey: ['operate', 'runs'] })
      toast.success(t('commandPalette.killedAgent'))
    },
    onError: () => toast.error(t('commandPalette.controlFailed')),
  })
  const stoppingId = kill.isPending ? (kill.variables ?? null) : null

  return (
    <div className="animate-fade-in-up">
      <PageHeader title={t('operate.runCenter')} subtitle={t('operate.runCenterSubtitle')} />

      <div className="mt-4 flex items-start gap-4">
        <div className="min-w-0 flex-1 space-y-4">
          <ActiveExecutionList
            runs={activeRuns}
            isLoading={agents.isLoading}
            isError={agents.isError}
            onRetry={() => agents.refetch()}
            selectedId={selectedId}
            onSelect={setSelectedId}
            onStop={(id) => kill.mutate(id)}
            stoppingId={stoppingId}
            now={now}
          />

          {runs.isError ? (
            <Card>
              <CardContent className="pt-4">
                <ErrorState onRetry={() => runs.refetch()} className="py-6" />
              </CardContent>
            </Card>
          ) : (
            <>
              <RunTable
                testId="automation-runs-table"
                title={t('operate.runs.automation')}
                icon={<CalendarClock className="h-4 w-4" aria-hidden="true" />}
                runs={automationRuns}
                isLoading={runs.isLoading}
                onSelect={setSelectedId}
              />
              <RunTable
                testId="agent-runs-table"
                title={t('operate.runs.agents')}
                icon={<Bot className="h-4 w-4" aria-hidden="true" />}
                runs={agentRunRows}
                isLoading={runs.isLoading}
                onSelect={setSelectedId}
                onStop={(id) => kill.mutate(id)}
                stoppingId={stoppingId}
              />
            </>
          )}
        </div>

        {inspectorRun && isDesktop && (
          <OperateInspector
            key={inspectorRun.id}
            run={inspectorRun}
            onClose={() => setSelectedId(null)}
            variant="panel"
            onStop={(id) => kill.mutate(id)}
            stopping={stoppingId === inspectorRun.id}
          />
        )}
      </div>

      {inspectorRun && !isDesktop && (
        <OperateInspector
          key={`dialog-${inspectorRun.id}`}
          run={inspectorRun}
          onClose={() => setSelectedId(null)}
          variant="dialog"
          onStop={(id) => kill.mutate(id)}
          stopping={stoppingId === inspectorRun.id}
        />
      )}
    </div>
  )
}

interface RunTableProps {
  testId: string
  title: string
  icon: React.ReactNode
  runs: OperateRun[]
  isLoading: boolean
  onSelect: (id: string) => void
  /** Stop affordance — agent table only; gated per row by isStoppable. */
  onStop?: (id: string) => void
  stoppingId?: string | null
}

/**
 * Read-only run inventory table. Automation rows expose their canonical
 * studio page as a plain anchor; agent rows select into the inspector
 * and keep the stop affordance only while stoppable.
 */
function RunTable({
  testId,
  title,
  icon,
  runs,
  isLoading,
  onSelect,
  onStop,
  stoppingId = null,
}: RunTableProps) {
  const { t } = useTranslation()
  const isAutomation = testId === 'automation-runs-table'

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          {icon}
          {title}
        </CardTitle>
        {!isLoading && runs.length > 0 && (
          <span className="text-2xs text-muted-foreground">
            {t('operate.runs.count', { count: runs.length })}
          </span>
        )}
      </CardHeader>
      <CardContent className="pt-2">
        {isLoading ? (
          <div className="space-y-1.5" aria-busy="true">
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} className="h-9 w-full rounded-md" />
            ))}
          </div>
        ) : runs.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            {t('operate.runs.empty')}
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table data-testid={testId} className="w-full text-sm">
              <thead>
                <tr className="border-b text-left text-2xs uppercase tracking-wide text-muted-foreground">
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t('operate.runs.name')}
                  </th>
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t('operate.runs.status')}
                  </th>
                  {isAutomation && (
                    <th scope="col" className="py-2 pr-3 font-medium">
                      {t('operate.runs.trigger')}
                    </th>
                  )}
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t('operate.runs.started')}
                  </th>
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t('operate.runs.completed')}
                  </th>
                  <th scope="col" className="py-2 pr-3 font-medium">
                    {t('operate.runs.duration')}
                  </th>
                  <th scope="col" className="py-2 font-medium">
                    <span className="sr-only">{t('operate.sections.actions')}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {runs.map((run) => (
                  <tr
                    key={run.id}
                    tabIndex={0}
                    onClick={() => onSelect(run.id)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault()
                        onSelect(run.id)
                      }
                    }}
                    className="cursor-pointer border-b transition-colors last:border-b-0 hover:bg-accent/50"
                    aria-label={t('commandCenter.activeExecution.selectRun', { name: run.name })}
                  >
                    <td className="max-w-[16rem] truncate py-2 pr-3 font-medium" title={run.name}>
                      {run.name}
                    </td>
                    <td className="py-2 pr-3">
                      <StatusChip status={run.status} />
                    </td>
                    {isAutomation && (
                      <td className="py-2 pr-3 text-muted-foreground">
                        {run.trigger ? t(`operate.runs.trigger.${run.trigger}`) : '—'}
                      </td>
                    )}
                    <td className="py-2 pr-3 font-mono text-xs tabular-nums text-muted-foreground">
                      {run.startedAt ? formatRelativeTime(run.startedAt, t) : '—'}
                    </td>
                    <td className="py-2 pr-3 font-mono text-xs tabular-nums text-muted-foreground">
                      {run.completedAt ? formatRelativeTime(run.completedAt, t) : '—'}
                    </td>
                    <td className="py-2 pr-3 font-mono text-xs tabular-nums text-muted-foreground">
                      {typeof run.durationSecs === 'number'
                        ? formatDuration(run.durationSecs * 1000)
                        : '—'}
                    </td>
                    <td className="py-2 text-right">
                      <div className="flex items-center justify-end gap-1">
                        {isAutomation && run.automationId && (
                          <Button variant="ghost" size="sm" asChild>
                            <a
                              href={`/studio/automations/${run.automationId}`}
                              aria-label={t('operate.actions.openAutomation')}
                              onClick={(e) => e.stopPropagation()}
                            >
                              <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
                            </a>
                          </Button>
                        )}
                        {!isAutomation && isStoppable(run) && onStop && (
                          <Button
                            variant="ghost"
                            size="sm"
                            className={cn('text-status-error-on-surface')}
                            aria-label={t('commandCenter.inspector.stop')}
                            disabled={stoppingId === run.id}
                            onClick={(e) => {
                              e.stopPropagation()
                              onStop(run.id)
                            }}
                          >
                            <Square className="h-3.5 w-3.5" aria-hidden="true" />
                          </Button>
                        )}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
