import { Link } from '@tanstack/react-router'
import {
  Activity,
  AlertTriangle,
  Bot,
  CheckCircle,
  Loader2,
  type LucideIcon,
  Pause,
  Square,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ErrorState } from '@/components/shared/error-state'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import type { AgentListItem } from '@/types/agent'
import {
  formatElapsed,
  isStoppable,
  lastActivityAt,
  progressOf,
  statusMeta,
  statusRank,
} from './runs'

/** The dashboard is a triage surface, not the full inventory — /operate/runs owns the rest. */
const MAX_ROWS = 8

const ICON_BY_NAME: Record<string, LucideIcon> = {
  activity: Activity,
  loader: Loader2,
  alert: AlertTriangle,
  pause: Pause,
  check: CheckCircle,
  square: Square,
}

export interface ActiveExecutionListProps {
  runs?: AgentListItem[]
  isLoading: boolean
  isError: boolean
  onRetry: () => void
  selectedId: string | null
  onSelect: (id: string) => void
  onStop: (id: string) => void
  /** The agent whose stop mutation is in flight (disables its Stop control). */
  stoppingId: string | null
  /** Clock for elapsed formatting — owned by the page so rows update together. */
  now?: number
}

/**
 * Active execution — the dashboard's primary work surface (spec §3).
 *
 * Lists running, starting, and recently failed runs in priority order
 * (running first, most recent activity within a state). Each row: name +
 * short id, lifecycle icon + text label (never color alone), elapsed
 * time, and step progress only when the backend reports a real
 * completed/total. Selecting a row updates the inspector instead of
 * navigating. Missing wire fields are omitted, never invented.
 */
export function ActiveExecutionList({
  runs,
  isLoading,
  isError,
  onRetry,
  selectedId,
  onSelect,
  onStop,
  stoppingId,
  now = Date.now(),
}: ActiveExecutionListProps) {
  const { t } = useTranslation()
  const ordered = runs
    ? [...runs].sort(
        (a, b) =>
          statusRank(a.status) - statusRank(b.status) || lastActivityAt(b) - lastActivityAt(a),
      )
    : []

  return (
    <Card className="flex h-full min-w-0 flex-col">
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Bot className="h-4 w-4" aria-hidden="true" />
          {t('commandCenter.activeExecution.title')}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex-1 pt-0 min-h-[280px]">
        <section aria-label={t('commandCenter.activeExecution.title')}>
          <div className="space-y-1.5">
            {isError ? (
              <ErrorState onRetry={onRetry} className="py-8" />
            ) : isLoading ? (
              [0, 1, 2].map((i) => <Skeleton key={i} className="h-12 w-full rounded-md" />)
            ) : !runs || runs.length === 0 ? (
              <EmptyRuns />
            ) : (
              <div
                role="listbox"
                aria-label={t('commandCenter.activeExecution.title')}
                className="space-y-1.5"
              >
                {ordered.slice(0, MAX_ROWS).map((run) => (
                  <RunRow
                    key={run.id}
                    run={run}
                    now={now}
                    selected={run.id === selectedId}
                    stopping={stoppingId === run.id}
                    onSelect={onSelect}
                    onStop={onStop}
                  />
                ))}
              </div>
            )}
            {runs && runs.length > MAX_ROWS && (
              <Link
                to="/operate/runs"
                className="block pt-1 text-center text-xs text-muted-foreground transition-colors hover:text-foreground"
              >
                {t('dashboard.viewAllCount', { count: runs.length })}
              </Link>
            )}
          </div>
        </section>
      </CardContent>
    </Card>
  )
}

function EmptyRuns() {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-10 text-center">
      <Bot className="h-8 w-8 text-muted-foreground/40" aria-hidden="true" />
      <p className="text-sm text-muted-foreground">{t('commandCenter.activeExecution.empty')}</p>
      <p className="text-xs text-muted-foreground/70">
        {t('commandCenter.activeExecution.emptyHint')}
      </p>
      <Link to="/studio" className="text-xs font-medium text-primary hover:underline">
        {t('commandCenter.activeExecution.startTask')}
      </Link>
    </div>
  )
}

interface RunRowProps {
  run: AgentListItem
  now: number
  selected: boolean
  stopping: boolean
  onSelect: (id: string) => void
  onStop: (id: string) => void
}

function RunRow({ run, now, selected, stopping, onSelect, onStop }: RunRowProps) {
  const { t } = useTranslation()
  const meta = statusMeta(run.status)
  const Icon = ICON_BY_NAME[meta.icon] ?? Activity
  const progress = progressOf(run)
  const startMs = Date.parse(run.started_at ?? run.created_at)
  const elapsed = formatElapsed(
    startMs,
    now,
    run.completed_at ? Date.parse(run.completed_at) : null,
  )

  return (
    <div className="flex items-center gap-1">
      <button
        type="button"
        role="option"
        data-agent-id={run.id}
        aria-selected={selected}
        aria-label={t('commandCenter.activeExecution.selectRun', { name: run.name })}
        onClick={() => onSelect(run.id)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            onSelect(run.id)
          }
        }}
        className={cn(
          'flex min-w-0 flex-1 items-center gap-2.5 rounded-md border px-3 py-2 text-left transition-all',
          'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
          selected
            ? 'border-primary/40 bg-accent/50 shadow-sm'
            : 'hover:border-primary/20 hover:bg-accent',
        )}
      >
        <Icon
          className={cn(
            'h-4 w-4 shrink-0',
            meta.iconClass,
            meta.spin && 'motion-safe:animate-spin',
            meta.pulse && 'motion-safe:animate-pulse',
          )}
          aria-hidden="true"
        />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-1.5">
            <span className="truncate text-sm font-medium">{run.name}</span>
            <span className="shrink-0 font-mono text-2xs text-muted-foreground">
              {run.id.slice(0, 6)}
            </span>
          </span>
          <span className={cn('mt-0.5 flex items-center gap-1.5 text-2xs', meta.textClass)}>
            <span>{t(meta.labelKey)}</span>
            <span aria-hidden="true">·</span>
            <span className="tabular-nums text-muted-foreground">{elapsed}</span>
            {progress && (
              <>
                <span aria-hidden="true">·</span>
                <span className="tabular-nums">
                  {t('commandCenter.run.steps', {
                    completed: progress.completed,
                    total: progress.total,
                  })}
                </span>
              </>
            )}
            {run.error && (
              <span className="min-w-0 truncate text-muted-foreground" title={run.error}>
                {run.error}
              </span>
            )}
          </span>
        </span>
      </button>
      {isStoppable(run) && (
        <button
          type="button"
          onClick={() => onStop(run.id)}
          disabled={stopping}
          aria-label={
            stopping
              ? t('commandCenter.activeExecution.stopping')
              : t('commandCenter.activeExecution.stop')
          }
          title={
            stopping
              ? t('commandCenter.activeExecution.stopping')
              : t('commandCenter.activeExecution.stop')
          }
          className="shrink-0 rounded-md border p-1.5 text-muted-foreground transition-colors hover:border-status-error-subtle-border hover:bg-status-error-subtle hover:text-status-error-on-surface focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
        >
          {stopping ? (
            <Loader2 className="h-3.5 w-3.5 motion-safe:animate-spin" aria-hidden="true" />
          ) : (
            <Square className="h-3.5 w-3.5" aria-hidden="true" />
          )}
        </button>
      )}
    </div>
  )
}
