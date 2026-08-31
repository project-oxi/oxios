import { createFileRoute } from '@tanstack/react-router'
import { Activity } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  Tooltip as RechartsTooltip,
  ReferenceLine,
  ResponsiveContainer,
  XAxis,
  YAxis,
} from 'recharts'
import { BudgetManagement } from '@/components/budget/budget-management'
import { CostByModel } from '@/components/cost/cost-by-model'
import { CostByProject } from '@/components/cost/cost-by-project'
import { CostChart } from '@/components/cost/cost-chart'
import { CostSummaryCards } from '@/components/cost/cost-summary'
import { ProviderQuotaCards } from '@/components/cost/provider-quota-cards'
import { SpendLimitCard } from '@/components/cost/spend-limit-card'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { MetricGaugeCard } from '@/components/shared/metric-gauge-card'
import { PageHeader } from '@/components/shared/page-header'
import { RefreshButton } from '@/components/shared/refresh-button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useCostSummary } from '@/hooks/use-costs'
import { memoryPercent, useResourceHistory } from '@/hooks/use-resource-history'
import { cssVarToRgb } from '@/lib/utils'
import type { CostPeriod } from '@/types/cost'

export const Route = createFileRoute('/operate/system/resources')({
  component: ResourcesAndCostPage,
})

/**
 * Resources and cost (design §6) — the old /resources and /budget pages
 * composed under one route with local tabs. Both page bodies keep their
 * existing hooks and sections verbatim; only the page headers collapse
 * into the shared PageHeader.
 */
function ResourcesAndCostPage() {
  const { t } = useTranslation()
  const [tab, setTab] = useState<'resources' | 'cost'>('resources')

  return (
    <div className="space-y-4 animate-fade-in-up">
      <PageHeader
        title={t('operate.resourcesAndCost')}
        subtitle={t('operate.resourcesAndCostSubtitle')}
      />

      <Tabs
        value={tab}
        onValueChange={(v) => setTab(v as 'resources' | 'cost')}
        className="space-y-4"
      >
        <TabsList>
          <TabsTrigger value="resources">{t('resources.title')}</TabsTrigger>
          <TabsTrigger value="cost">{t('cost.pageTitle')}</TabsTrigger>
        </TabsList>

        <TabsContent value="resources" className="space-y-6">
          <ResourcesSection />
        </TabsContent>

        <TabsContent value="cost" className="space-y-6">
          <CostSection />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function getChartColor(token: string): string {
  if (typeof window === 'undefined') return cssVarToRgb('--color-text-muted')
  return (
    getComputedStyle(document.documentElement).getPropertyValue(token).trim() ||
    cssVarToRgb('--color-text-muted')
  )
}

/** The former /resources page body — same query cadence, gauges, and chart. */
function ResourcesSection() {
  const { t } = useTranslation()
  const { data, isLoading, isError, refetch, isFetching } = useResourceHistory(30, 5_000)

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const snapshots = Array.isArray(data) ? data : []
  const latest = snapshots.length > 0 ? snapshots[snapshots.length - 1] : null
  const chartData = snapshots.map((s) => ({
    time: new Date(s.timestamp).toLocaleTimeString(),
    cpu: s.cpu_percent,
    memory: memoryPercent(s),
  }))

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-lg font-semibold">{t('resources.title')}</h2>
        <RefreshButton onClick={() => refetch()} isFetching={isFetching} />
      </div>

      {/* Current Stats */}
      {latest && (
        <div className="grid gap-4 md:grid-cols-3">
          <MetricGaugeCard label={t('resources.cpu')} value={latest.cpu_percent} />
          <MetricGaugeCard label={t('resources.memory')} value={memoryPercent(latest)} />
          <MetricGaugeCard
            label={t('resources.disk')}
            display={`${latest.disk_used_gb.toFixed(1)} GB`}
          />
        </div>
      )}

      {/* Chart */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Activity className="h-4 w-4" /> {t('resources.resourceHistory')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {chartData.length > 1 ? (
            <ResponsiveContainer width="100%" height={300}>
              <AreaChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
                <XAxis dataKey="time" className="text-xs" tick={{ fontSize: 12 }} />
                <YAxis className="text-xs" tick={{ fontSize: 12 }} domain={[0, 100]} />
                <RechartsTooltip
                  contentStyle={{
                    backgroundColor: 'var(--card)',
                    border: '1px solid var(--border)',
                    borderRadius: '8px',
                    fontSize: '12px',
                    color: 'var(--foreground)',
                  }}
                />
                <Legend wrapperStyle={{ fontSize: 12 }} />
                <ReferenceLine
                  y={75}
                  stroke={getChartColor('--warning')}
                  strokeDasharray="4 4"
                  label={{
                    value: '75%',
                    position: 'right',
                    fontSize: 10,
                    fill: getChartColor('--warning'),
                  }}
                />
                <ReferenceLine
                  y={90}
                  stroke={getChartColor('--error')}
                  strokeDasharray="4 4"
                  label={{
                    value: '90%',
                    position: 'right',
                    fontSize: 10,
                    fill: getChartColor('--error'),
                  }}
                />
                <Area
                  type="monotone"
                  dataKey="cpu"
                  stroke={getChartColor('--chart-1')}
                  fill={getChartColor('--chart-1')}
                  fillOpacity={0.1}
                  name={`${t('resources.cpu')} %`}
                />
                <Area
                  type="monotone"
                  dataKey="memory"
                  stroke={getChartColor('--chart-2')}
                  fill={getChartColor('--chart-2')}
                  fillOpacity={0.1}
                  name={`${t('resources.memory')} %`}
                />
              </AreaChart>
            </ResponsiveContainer>
          ) : (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {t('resources.notEnoughData')}
            </p>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

const PERIODS: CostPeriod[] = ['today', 'week', 'month', 'all']

/** The former /budget page body — same hooks, cards, and budget panel. */
function CostSection() {
  const { t } = useTranslation()
  const [period, setPeriod] = useState<CostPeriod>('month')
  const { refetch, isFetching } = useCostSummary(period)

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-lg font-semibold">{t('cost.pageTitle')}</h2>
        <RefreshButton onClick={() => refetch()} isFetching={isFetching} />
      </div>

      {/* Spend limit + period selector */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <SpendLimitCard />
        <div className="flex items-end sm:col-span-1 lg:col-span-3">
          <Tabs value={period} onValueChange={(v) => setPeriod(v as CostPeriod)} className="w-full">
            <TabsList>
              {PERIODS.map((p) => (
                <TabsTrigger key={p} value={p}>
                  {t(`cost.period.${p}`)}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">{t('cost.spendLimitNote')}</p>
      <p className="text-xs text-muted-foreground">{t('cost.periodScopeNote')}</p>

      {/* Summary stat cards */}
      <CostSummaryCards period={period} />

      {/* Daily spend chart */}
      <CostChart days={30} />

      {/* Breakdowns */}
      <div className="grid gap-4 lg:grid-cols-2">
        <CostByModel period={period} />
        <CostByProject period={period} />
      </div>

      {/* Provider panel — all configured providers + quota data */}
      <ProviderQuotaCards />

      {/* Agent budget management — token/call rate limits */}
      <div className="space-y-2">
        <h2 className="text-lg font-semibold">{t('budget.title')}</h2>
        <BudgetManagement />
      </div>
    </div>
  )
}
