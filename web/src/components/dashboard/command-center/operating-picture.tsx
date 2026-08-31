import { useQuery } from '@tanstack/react-query'
import { RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useCostDaily, useCostSummary } from '@/hooks/use-costs'
import { api } from '@/lib/api-client'
import { formatUsd } from '@/lib/utils'

/** Local midnight, ISO — computed inside the queryFn so a long-open tab never goes stale. */
function todayStartIso(): string {
  const d = new Date()
  d.setHours(0, 0, 0, 0)
  return d.toISOString()
}

interface OperatingPictureProps {
  /** Live active-run count from the page's shared agents query. */
  activeCount: number
}

/**
 * Today's operating picture (spec §6): four compact aggregates plus a
 * run-series chart. Every number is period-truthful — "today" metrics
 * come from today-scoped endpoints, the series is explicitly labeled
 * "last 7 days" because no intraday history exists. A failed metric
 * degrades alone with retry; it never takes the panel down.
 */
export function OperatingPicture({ activeCount }: OperatingPictureProps) {
  const { t } = useTranslation()

  const completedToday = useQuery({
    queryKey: ['agents', 'completed-today'],
    queryFn: async () => {
      const res = await api.get<{
        stats?: { count_completed?: number; count_failed?: number }
      }>(`/api/agents?date_from=${encodeURIComponent(todayStartIso())}&per_page=1`)
      return res.stats?.count_completed ?? 0
    },
    refetchInterval: 60_000,
    staleTime: 30_000,
  })
  const summary = useCostSummary('today')
  const daily = useCostDaily(7)

  const series = Array.isArray(daily.data?.items) ? (daily.data?.items.slice(-7) ?? []) : []
  const maxCount = Math.max(1, ...series.map((s) => s.agent_count))

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-base">{t('commandCenter.picture.title')}</CardTitle>
        <span className="text-2xs text-muted-foreground">
          {t('commandCenter.picture.last7days')}
        </span>
      </CardHeader>
      <CardContent className="space-y-4 pt-0">
        <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
          <Metric label={t('commandCenter.picture.activeAgents')} value={String(activeCount)} />
          <Metric
            label={t('commandCenter.picture.completedToday')}
            value={
              completedToday.isError
                ? null
                : String(completedToday.data ?? (completedToday.isLoading ? null : 0))
            }
            failed={completedToday.isError}
            onRetry={() => completedToday.refetch()}
          />
          <Metric
            label={t('commandCenter.picture.tokensToday')}
            value={summary.isError ? null : (summary.data?.total_tokens ?? 0).toLocaleString()}
            failed={summary.isError}
            onRetry={() => summary.refetch()}
          />
          <Metric
            label={t('commandCenter.picture.spendToday')}
            value={summary.isError ? null : formatUsd(summary.data?.total_cost_usd ?? 0)}
            failed={summary.isError}
            onRetry={() => summary.refetch()}
          />
        </div>

        <div
          role="img"
          aria-label={t('commandCenter.picture.last7days')}
          className="flex h-16 items-end gap-1"
        >
          {daily.isError ? (
            <p className="text-xs text-muted-foreground">
              {t('commandCenter.picture.unavailable')}
            </p>
          ) : (
            series.map((s) => (
              <div
                key={s.date}
                data-testid="picture-series-bar"
                title={`${s.date}: ${s.agent_count}`}
                style={{ height: `${Math.max(4, (s.agent_count / maxCount) * 100)}%` }}
                className="flex-1 rounded-sm bg-chart-2/70 transition-[height]"
              />
            ))
          )}
        </div>
      </CardContent>
    </Card>
  )
}

interface MetricProps {
  label: string
  /** Rendered value; null renders as loading dashes. */
  value: string | null
  failed?: boolean
  onRetry?: () => void
}

function Metric({ label, value, failed = false, onRetry }: MetricProps) {
  const { t } = useTranslation()
  return (
    <fieldset aria-label={label} className="rounded-md border p-2.5">
      <p className="truncate text-2xs text-muted-foreground">{label}</p>
      <div className="flex items-center justify-between gap-1">
        <p className="font-mono text-lg font-semibold tabular-nums">
          {failed ? t('commandCenter.picture.unavailable') : (value ?? '—')}
        </p>
        {failed && onRetry && (
          <button
            type="button"
            onClick={onRetry}
            aria-label={`${label} ${t('commandCenter.picture.retry')}`}
            title={t('commandCenter.picture.retry')}
            className="rounded-md border p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <RefreshCw className="h-3 w-3" aria-hidden="true" />
          </button>
        )}
      </div>
    </fieldset>
  )
}
