import { useQuery } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { AlertTriangle, Brain, Cpu, HardDrive, Sparkles } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { AgentStatusCard } from '@/components/dashboard/agent-status-card'
import { AgentsActivityCard } from '@/components/dashboard/agents-activity-card'
import { ApprovalsQueue } from '@/components/dashboard/approvals-queue'
import { BrainDashboardCard } from '@/components/dashboard/brain-dashboard-card'
import { BudgetCard } from '@/components/dashboard/budget-card'
import { McpStatusCard } from '@/components/dashboard/mcp-status-card'
import { SkillsCronCard } from '@/components/dashboard/skills-cron-card'
import { StatCard } from '@/components/dashboard/stat-card'
import { SystemHealthCard } from '@/components/dashboard/system-health-card'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingStatCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { useAgentCountHistory } from '@/hooks/use-agent-count-history'
import { useApprovals } from '@/hooks/use-approvals'
import { useBrainStatus } from '@/hooks/use-brain'
import { computeDelta, seriesFromSnapshots, useResourceHistory } from '@/hooks/use-resource-history'
import { useTokenRate } from '@/hooks/use-token-rate'
import { api } from '@/lib/api-client'
import type { Agent, SystemStatus } from '@/types'

export const Route = createFileRoute('/')({
  component: DashboardPage,
})

function DashboardPage() {
  const { t } = useTranslation()

  const {
    data: status,
    isLoading: statusLoading,
    isError: statusError,
    refetch: refetchStatus,
  } = useQuery({
    queryKey: ['status'],
    queryFn: () => api.get<SystemStatus>('/api/status'),
    refetchInterval: 10_000,
  })

  const {
    data: agents,
    isError: agentsError,
    refetch: refetchAgents,
  } = useQuery({
    queryKey: ['agents'],
    queryFn: () => api.get<{ items: Agent[] }>('/api/agents'),
    refetchInterval: 5_000,
  })

  // Brain daemon status (RFC-047)
  const { data: brainStatus } = useBrainStatus()

  // Resource history (last 30 samples) → sparklines
  const { data: snapshots } = useResourceHistory(30, 10_000)
  const cpuSeries = seriesFromSnapshots(Array.isArray(snapshots) ? snapshots : [], 'cpu_percent')
  const memSeries = seriesFromSnapshots(Array.isArray(snapshots) ? snapshots : [], 'memory_percent')
  const cpuDelta = computeDelta(cpuSeries)
  const memDelta = computeDelta(memSeries)
  const cpuNow = cpuSeries.length > 0 ? (cpuSeries[cpuSeries.length - 1] ?? 0) : 0
  const memNow = memSeries.length > 0 ? (memSeries[memSeries.length - 1] ?? 0) : 0
  // Threshold-based severity: normal load stays neutral (info); only high load
  // escalates to warning/error. Avoids false-alarm amber/red on healthy metrics.
  const sevText = { error: 'text-error', warning: 'text-warning', info: 'text-info' } as const
  const sevOf = (v: number): 'error' | 'warning' | 'info' =>
    v >= 90 ? 'error' : v >= 75 ? 'warning' : 'info'
  const cpuSev = sevOf(cpuNow)
  const memSev = sevOf(memNow)

  // Token rate from the SSE stream
  const { tokensPerMin, history: tokenHistory } = useTokenRate()
  const tokenDelta = computeDelta(tokenHistory)

  // Pending approvals
  const { data: approvals } = useApprovals()
  const pendingApprovals = (Array.isArray(approvals?.items) ? approvals.items : []).filter(
    (a) => a.status === 'pending',
  )

  // derived data — computed before early returns for stable hook order
  const allAgents = Array.isArray(agents?.items) ? agents.items : []
  const runningAgents = allAgents.filter((a) => a.status?.toLowerCase() === 'running')
  const totalForked: number | null =
    typeof status?.components?.agents?.total_forked === 'number'
      ? status.components.agents.total_forked
      : null
  const totalFailed: number =
    typeof status?.components?.agents?.total_failed === 'number'
      ? status.components.agents.total_failed
      : 0
  const { runningSeries } = useAgentCountHistory(totalForked, runningAgents.length, {
    trackTotal: false,
  })

  // Brain episode count (null when the daemon is offline)
  const brainTotal = brainStatus?.episodes ?? '—'

  if (statusLoading) return <LoadingStatCards count={6} />
  if (statusError) return <ErrorState onRetry={() => refetchStatus()} />

  return (
    <div className="space-y-6 animate-fade-in-up">
      <PageHeader
        title={t('dashboard.title')}
        subtitle={t('dashboard.subtitle')}
        display
        actions={
          status && (
            <div className="flex items-center gap-1.5">
              <span className="inline-flex items-center rounded-full bg-primary/10 px-2 py-0.5 text-2xs font-mono font-medium text-primary whitespace-nowrap">
                {t('dashboard.binaryVersion', { version: status.version })}
              </span>
              {status.web_version && (
                <span className="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-2xs font-mono font-medium text-muted-foreground whitespace-nowrap">
                  {t('dashboard.webVersion', { version: status.web_version })}
                </span>
              )}
            </div>
          )
        }
      />

      {/* Row 1: KPI — 6 cards */}
      <div className="grid gap-3 grid-cols-2 sm:grid-cols-3 xl:grid-cols-6 animate-stagger">
        <AgentStatusCard
          total={totalForked}
          running={runningAgents.length}
          failed={totalFailed}
          runningSeries={runningSeries}
        />
        <StatCard
          label={t('dashboard.tokensPerMin')}
          value={formatTokensPerMin(tokensPerMin)}
          icon={<Sparkles className="h-4 w-4" />}
          iconClassName="text-info"
          delta={tokenHistory.length > 1 ? tokenDelta : undefined}
          sparkline={tokenHistory}
          sparkColor="primary"
          hint={t('dashboard.lastWindow')}
        />
        <StatCard
          label={t('dashboard.cpu')}
          value={
            cpuSeries.length > 0 ? `${(cpuSeries[cpuSeries.length - 1] ?? 0).toFixed(0)}%` : '—'
          }
          icon={<Cpu className="h-4 w-4" />}
          iconClassName={sevText[cpuSev]}
          delta={cpuSeries.length > 1 ? cpuDelta : undefined}
          sparkline={cpuSeries}
          sparkColor={cpuSev}
          href="/resources"
        />
        <StatCard
          label={t('dashboard.ram')}
          value={
            memSeries.length > 0 ? `${(memSeries[memSeries.length - 1] ?? 0).toFixed(0)}%` : '—'
          }
          icon={<HardDrive className="h-4 w-4" />}
          iconClassName={sevText[memSev]}
          delta={memSeries.length > 1 ? memDelta : undefined}
          sparkline={memSeries}
          sparkColor={memSev}
          href="/resources"
        />
        <StatCard
          label={t('dashboard.brain')}
          value={brainTotal}
          icon={<Brain className="h-4 w-4" />}
          iconClassName="text-info"
          sparkColor="accent"
          href="/brain"
        />
        <StatCard
          label={t('dashboard.pendingApprovals')}
          value={pendingApprovals.length}
          icon={<AlertTriangle className="h-4 w-4" />}
          iconClassName={pendingApprovals.length > 0 ? 'text-error' : 'text-muted-foreground'}
          sparkColor={pendingApprovals.length > 0 ? 'error' : 'accent'}
          hint={
            pendingApprovals.length > 0 ? t('dashboard.needsAttention') : t('dashboard.allClear')
          }
        />
      </div>

      {/* Row 2: Agents & Activity (3/5) + System Health (2/5) */}
      <div className="grid gap-4 lg:grid-cols-5">
        <div className="lg:col-span-3">
          <AgentsActivityCard
            runningAgents={runningAgents}
            isAgentsError={agentsError}
            onRetryAgents={() => refetchAgents()}
          />
        </div>
        <div className="lg:col-span-2">
          <SystemHealthCard status={status} className="h-full" />
        </div>
      </div>

      {/* Row 3: MCP (1/4) + Budget (1/4) + Brain (1/4) + Skills/Seeds/Cron (1/4) */}
      <div className="grid gap-4 grid-cols-2 lg:grid-cols-4 animate-stagger">
        <McpStatusCard />
        <BudgetCard />
        <BrainDashboardCard />
        <SkillsCronCard />
      </div>

      {/* Row 4: Pending approvals (full-width, only when needed) */}
      {pendingApprovals.length > 0 && <ApprovalsQueue />}
    </div>
  )
}

function formatTokensPerMin(n: number): string {
  if (n <= 0) return '0'
  if (n < 1000) return `${Math.round(n)}`
  if (n < 10_000) return `${(n / 1000).toFixed(1)}k`
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`
  return `${(n / 1_000_000).toFixed(1)}M`
}
