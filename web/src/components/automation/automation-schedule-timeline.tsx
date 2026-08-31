import { Pencil, Trash2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import {
  chartColor,
  computeJobFireData,
  computeTicks,
  formatTickLabel,
  LABEL_WIDTH,
  msToPx,
  PIXELS_PER_HOUR,
  TIME_RANGES,
  type TimeRange,
} from '@/lib/cron-timeline'
import { cn } from '@/lib/utils'

interface HoveredMarker {
  rect: DOMRect
  date: Date
  jobName: string
  isPrimary: boolean
}

/** Minimal structural shape the timeline renders — cron jobs and scheduled
 * automations both satisfy it. */
export interface TimelineEntry {
  id: string
  name: string
  schedule: string
  next_run?: string
  /** Optional — automation excludes paused/exhausted/failed entries. */
  enabled?: boolean
}

/**
 * Schedule timeline view for scheduled automations (cron/heartbeat).
 *
 * Shows each entry as a row with fire-time markers projected across a
 * selectable time window (24h / 48h / 7d). The authoritative `next_run`
 * from the backend is always shown as a highlighted primary marker;
 * additional projected fire times visualize the schedule pattern.
 */
export function AutomationScheduleTimeline({
  jobs,
  onEdit,
  onDelete,
}: {
  jobs: TimelineEntry[]
  onEdit?: (job: TimelineEntry) => void
  onDelete?: (job: TimelineEntry) => void
}) {
  const { t } = useTranslation()
  const [range, setRange] = useState<TimeRange>('24h')
  const [hovered, setHovered] = useState<HoveredMarker | null>(null)

  const rangeHours = TIME_RANGES.find((r) => r.value === range)!.hours
  const nowMs = Date.now()
  const trackWidth = rangeHours * PIXELS_PER_HOUR

  const fireData = useMemo(() => computeJobFireData(jobs, rangeHours), [jobs, rangeHours])
  const ticks = computeTicks(nowMs, rangeHours, range)
  const totalFires = useMemo(
    () => Array.from(fireData.values()).reduce((sum, d) => sum + d.fireTimes.length, 0),
    [fireData],
  )

  if (jobs.length === 0) {
    return (
      <div className="rounded-xl border border-dashed p-12 text-center">
        <p className="text-sm text-muted-foreground">{t('cronJobs.timeline.emptyHint')}</p>
      </div>
    )
  }

  return (
    <div className="space-y-3" data-testid="automation-schedule-timeline">
      {/* Controls */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="inline-flex gap-0.5 rounded-lg border bg-muted/50 p-0.5">
          {TIME_RANGES.map((r) => (
            <button
              key={r.value}
              type="button"
              onClick={() => setRange(r.value)}
              className={cn(
                'rounded-md px-3 py-1 text-xs font-medium transition-colors',
                range === r.value
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground',
              )}
            >
              {t(`cronJobs.timeline.range${r.value}`)}
            </button>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          {t('cronJobs.timeline.jobCount', { count: jobs.length })} ·{' '}
          {t('cronJobs.timeline.fireCount', { count: totalFires })}
        </p>
      </div>

      {/* Timeline */}
      <div className="max-h-[70vh] overflow-auto rounded-xl border bg-card shadow-sm">
        <div style={{ width: LABEL_WIDTH + trackWidth, minWidth: '100%' }}>
          {/* Header row: range labels */}
          <div className="sticky top-0 z-10 flex border-b bg-card">
            <div className="shrink-0 border-r px-3 py-1.5" style={{ width: LABEL_WIDTH }}>
              <span className="text-xs font-medium text-muted-foreground">
                {t('cronJobs.timeline.viewTimeline')}
              </span>
            </div>
            <div className="relative h-7" style={{ width: trackWidth }}>
              {ticks.map((tick) => (
                <div
                  key={tick.getTime()}
                  className="absolute top-0 h-full px-1 text-[10px] font-medium text-muted-foreground"
                  style={{ left: msToPx(tick.getTime() - nowMs) }}
                >
                  {formatTickLabel(tick, range)}
                </div>
              ))}
            </div>
          </div>

          {/* Rows */}
          {jobs.map((job, i) => {
            const data = fireData.get(job.id) ?? { fireTimes: [], isDense: false }
            const hasFires = data.fireTimes.length > 0
            const color = chartColor(i)
            return (
              <div key={job.id} className="flex border-b last:border-b-0">
                <div
                  className="flex shrink-0 items-center gap-1.5 border-r px-3 py-2"
                  style={{ width: LABEL_WIDTH }}
                >
                  <span
                    className="h-2 w-2 shrink-0 rounded-full"
                    style={{ backgroundColor: color }}
                    aria-hidden
                  />
                  <span className="min-w-0 flex-1 truncate text-xs font-medium" title={job.name}>
                    {job.name}
                  </span>
                  <Badge variant="outline" className="ml-auto shrink-0 text-[10px]">
                    {job.schedule}
                  </Badge>
                  {(onEdit || onDelete) && (
                    <div className="ml-1 flex shrink-0 items-center gap-0.5">
                      {onEdit && (
                        <button
                          type="button"
                          onClick={() => onEdit(job)}
                          className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                          aria-label={t('common.edit')}
                        >
                          <Pencil className="h-3 w-3" />
                        </button>
                      )}
                      {onDelete && (
                        <button
                          type="button"
                          onClick={() => onDelete(job)}
                          className="rounded p-0.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                          aria-label={t('common.delete')}
                        >
                          <Trash2 className="h-3 w-3" />
                        </button>
                      )}
                    </div>
                  )}
                </div>
                <div className="relative h-12" style={{ width: trackWidth }}>
                  {ticks.map((tick) => (
                    <div
                      key={tick.getTime()}
                      className="absolute top-0 h-full w-px bg-border/30"
                      style={{ left: msToPx(tick.getTime() - nowMs) }}
                    />
                  ))}

                  {hasFires && data.isDense && (
                    <DenseBar
                      data={data}
                      color={color}
                      dimmed={!job.enabled}
                      nowMs={nowMs}
                      jobName={job.name}
                      onHover={setHovered}
                      t={t}
                    />
                  )}

                  {hasFires &&
                    !data.isDense &&
                    data.fireTimes.map((ft) => {
                      const pos = msToPx(Math.max(0, ft.date.getTime() - nowMs))
                      return (
                        <div
                          key={ft.date.getTime()}
                          className="absolute top-1/2 -translate-y-1/2 cursor-pointer rounded-full transition-transform hover:z-20 hover:scale-[1.8]"
                          style={{
                            left: pos,
                            width: ft.isPrimary ? 6 : 3,
                            height: ft.isPrimary ? 22 : 14,
                            marginLeft: ft.isPrimary ? -3 : -1.5,
                            backgroundColor: color,
                            opacity: job.enabled === false ? 0.3 : 1,
                            zIndex: ft.isPrimary ? 5 : 1,
                            outline: ft.isPrimary
                              ? `2px solid color-mix(in oklch, ${color} 30%, transparent)`
                              : undefined,
                            outlineOffset: '1px',
                          }}
                          onMouseEnter={(e) =>
                            setHovered({
                              rect: e.currentTarget.getBoundingClientRect(),
                              date: ft.date,
                              jobName: job.name,
                              isPrimary: ft.isPrimary,
                            })
                          }
                          onMouseLeave={() => setHovered(null)}
                          role={ft.isPrimary ? 'img' : undefined}
                          aria-hidden={ft.isPrimary ? undefined : true}
                          aria-label={
                            ft.isPrimary
                              ? `${job.name}: ${t('cronJobs.timeline.nextRun')} ${ft.date.toLocaleString()}`
                              : undefined
                          }
                          onFocus={
                            ft.isPrimary
                              ? (e) =>
                                  setHovered({
                                    rect: e.currentTarget.getBoundingClientRect(),
                                    date: ft.date,
                                    jobName: job.name,
                                    isPrimary: true,
                                  })
                              : undefined
                          }
                          onBlur={ft.isPrimary ? () => setHovered(null) : undefined}
                        />
                      )
                    })}
                </div>
              </div>
            )
          })}
        </div>
      </div>

      {/* Floating tooltip */}
      {hovered && (
        <div
          className="pointer-events-none fixed z-50 rounded-md bg-foreground px-2.5 py-1.5 text-xs text-background shadow-lg"
          style={{
            left: hovered.rect.left + hovered.rect.width / 2,
            top: hovered.rect.top - 6,
            transform: 'translate(-50%, -100%)',
          }}
        >
          <span className="font-medium">{hovered.jobName}</span>
          <span className="mx-1 opacity-50">·</span>
          {hovered.date.toLocaleString(undefined, {
            month: 'short',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
            hour12: false,
          })}
          {hovered.isPrimary && (
            <span className="ml-1.5 rounded bg-background/20 px-1 text-[10px]">
              {t('cronJobs.timeline.nextRun')}
            </span>
          )}
        </div>
      )}
    </div>
  )
}

/** Dense continuous bar for high-frequency jobs (>25 fires in window). */
function DenseBar({
  data,
  color,
  dimmed,
  nowMs,
  jobName,
  onHover,
  t,
}: {
  data: { fireTimes: { date: Date; isPrimary: boolean }[] }
  color: string
  dimmed: boolean
  nowMs: number
  jobName: string
  onHover: (h: HoveredMarker | null) => void
  t: (key: string, opts?: Record<string, unknown>) => string
}) {
  const first = data.fireTimes[0]
  const last = data.fireTimes.at(-1)
  if (!first || !last) return null
  const leftPx = msToPx(Math.max(0, first.date.getTime() - nowMs))
  const rightPx = msToPx(last.date.getTime() - nowMs)
  const barWidth = Math.max(6, rightPx - leftPx)

  return (
    <>
      <div
        className="absolute top-1/2 h-2.5 -translate-y-1/2 cursor-pointer rounded-full"
        style={{
          left: leftPx,
          width: barWidth,
          backgroundColor: color,
          opacity: dimmed ? 0.15 : 0.5,
        }}
        onMouseEnter={(e) =>
          onHover({
            rect: e.currentTarget.getBoundingClientRect(),
            date: first.date,
            jobName,
            isPrimary: false,
          })
        }
        onMouseLeave={() => onHover(null)}
      />
      <span
        className="absolute top-1/2 -translate-y-1/2 whitespace-nowrap text-[10px] font-medium text-muted-foreground"
        style={{ left: rightPx + 8 }}
      >
        {t('cronJobs.timeline.fireCount', { count: data.fireTimes.length })}
      </span>
    </>
  )
}
