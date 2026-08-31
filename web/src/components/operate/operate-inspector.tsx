import { Link } from '@tanstack/react-router'
import { ExternalLink, Loader2, Square, X } from 'lucide-react'
import { Fragment } from 'react'
import { useTranslation } from 'react-i18next'
import { RunTraceDetail, StatusChip } from '@/components/dashboard/command-center/run-inspector'
import { isStoppable, progressOf } from '@/components/dashboard/command-center/runs'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { OperateRun } from '@/hooks/use-operate'
import { formatDuration, formatRelativeTime } from '@/lib/utils'

export interface OperateInspectorProps {
  run: OperateRun
  onClose: () => void
  /** 'panel' = fixed desktop aside; 'dialog' = mobile sheet. */
  variant: 'panel' | 'dialog'
  /** Present only for agent-kind rows that the page can stop. */
  onStop?: (id: string) => void
  stopping?: boolean
}

/**
 * Operate inspector — bounded triage detail of one operate run (design
 * §6.4). The section order is fixed: action/state, recorded origin, exact
 * scope, rationale/impact, safe actions, canonical detail — plus, for
 * agent runs, the embedded dashboard trace detail. Provenance rows with
 * no source record are omitted, never rendered as "unknown"; the Studio
 * launch appears only when an automation origin exists (typed route not
 * on this branch yet, so it is a plain anchor — integration converts).
 */
export function OperateInspector({
  run,
  onClose,
  variant,
  onStop,
  stopping = false,
}: OperateInspectorProps) {
  const { t } = useTranslation()
  const isAgent = run.kind === 'agent'
  const progress = progressOf({
    steps_completed: run.stepsCompleted ?? 0,
    steps_total: run.stepsTotal ?? null,
  })
  const body = (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-y-auto p-4">
      <header className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <h2 className="truncate text-base font-semibold">{run.name}</h2>
          <p className="font-mono text-2xs text-muted-foreground">{run.id}</p>
        </div>
        {variant === 'panel' && (
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 shrink-0"
            onClick={onClose}
            aria-label={t('common.close')}
          >
            <X className="h-3.5 w-3.5" aria-hidden="true" />
          </Button>
        )}
      </header>

      {/* 1. Action / state */}
      <section data-section="state" aria-label={t('operate.sections.state')} className="space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <StatusChip status={run.status} />
          {run.trigger && (
            <span className="rounded-md border px-1.5 py-0.5 text-2xs text-muted-foreground">
              {t(`operate.runs.trigger.${run.trigger}`)}
            </span>
          )}
        </div>
      </section>

      {/* 2. Recorded origin — only what the source records */}
      <OriginSection run={run} />

      {/* 3. Exact scope */}
      <section data-section="scope" aria-label={t('operate.sections.scope')}>
        <SectionTitle>{t('operate.sections.scope')}</SectionTitle>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
          {progress && (
            <>
              <dt className="text-muted-foreground">{t('commandCenter.run.steps', progress)}</dt>
              <dd className="text-right font-mono tabular-nums">
                {progress.completed}/{progress.total}
              </dd>
            </>
          )}
          {typeof run.tokensUsed === 'number' && (
            <>
              <dt className="text-muted-foreground">{t('operate.runs.tokens')}</dt>
              <dd className="text-right font-mono tabular-nums">
                {run.tokensUsed.toLocaleString()}
              </dd>
            </>
          )}
          {typeof run.costUsd === 'number' && (
            <>
              <dt className="text-muted-foreground">{t('operate.runs.cost')}</dt>
              <dd className="text-right font-mono tabular-nums">${run.costUsd.toFixed(4)}</dd>
            </>
          )}
          {run.modelId && (
            <>
              <dt className="text-muted-foreground">{t('commandCenter.inspector.model')}</dt>
              <dd className="truncate text-right font-mono">{run.modelId}</dd>
            </>
          )}
          {typeof run.durationSecs === 'number' && (
            <>
              <dt className="text-muted-foreground">{t('operate.runs.duration')}</dt>
              <dd className="text-right font-mono tabular-nums">
                {formatDuration(run.durationSecs * 1000)}
              </dd>
            </>
          )}
          {run.startedAt && (
            <>
              <dt className="text-muted-foreground">{t('commandCenter.inspector.started')}</dt>
              <dd className="truncate text-right font-mono tabular-nums">
                {formatRelativeTime(run.startedAt, t)}
              </dd>
            </>
          )}
          {run.completedAt && (
            <>
              <dt className="text-muted-foreground">{t('operate.runs.completed')}</dt>
              <dd className="truncate text-right font-mono tabular-nums">
                {formatRelativeTime(run.completedAt, t)}
              </dd>
            </>
          )}
        </dl>
      </section>

      {/* 4. Rationale / impact — the recorded error, when there is one */}
      {run.error && (
        <section data-section="impact" aria-label={t('operate.sections.impact')}>
          <SectionTitle>{t('operate.sections.impact')}</SectionTitle>
          <p className="rounded-md border border-status-error-subtle-border bg-status-error-subtle px-2.5 py-1.5 font-mono text-xs break-words text-status-error-on-surface">
            {run.error}
          </p>
        </section>
      )}

      {/* 5. Safe actions */}
      {(isAgent && isStoppable(run) && onStop) || run.automationId ? (
        <section data-section="actions" aria-label={t('operate.sections.actions')}>
          <div className="flex flex-wrap items-center gap-2">
            {isAgent && isStoppable(run) && onStop && (
              <Button
                variant="destructive"
                size="sm"
                onClick={() => onStop(run.id)}
                disabled={stopping}
                aria-label={t('commandCenter.inspector.stop')}
              >
                {stopping ? (
                  <Loader2 className="h-3.5 w-3.5 motion-safe:animate-spin" aria-hidden="true" />
                ) : (
                  <Square className="h-3.5 w-3.5" aria-hidden="true" />
                )}
                {t('commandCenter.inspector.stop')}
              </Button>
            )}
            {run.automationId && (
              <Button variant="outline" size="sm" asChild>
                <a href={`/studio/automations/${run.automationId}`}>
                  <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
                  {t('operate.actions.openAutomation')}
                </a>
              </Button>
            )}
          </div>
        </section>
      ) : null}

      {/* 6. Runs inventory link — agent runs surface in the operate runs list */}
      {isAgent && (
        <section data-section="details" aria-label={t('operate.sections.details')}>
          <Button variant="outline" size="sm" asChild className="w-full">
            <Link to="/operate/runs">
              <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
              {t('commandCenter.inspector.openRun')}
            </Link>
          </Button>
        </section>
      )}

      {/* Trace/logs detail — agent runs only, same bounded body as the dashboard inspector */}
      {isAgent && (
        <section data-section="trace" aria-label={t('commandCenter.inspector.steps')}>
          <RunTraceDetail agentId={run.id} />
        </section>
      )}
    </div>
  )

  if (variant === 'dialog') {
    return (
      <Dialog open onOpenChange={(open) => !open && onClose()}>
        <DialogContent className="flex max-h-[85dvh] flex-col p-0" mobileSheet>
          <DialogHeader className="sr-only">
            <DialogTitle>{run.name}</DialogTitle>
          </DialogHeader>
          {body}
        </DialogContent>
      </Dialog>
    )
  }

  return (
    <aside
      aria-label={t('commandCenter.inspector.title')}
      className="max-h-[80dvh] w-[340px] shrink-0 overflow-hidden rounded-lg border bg-background"
    >
      {body}
    </aside>
  )
}

/**
 * Recorded origin (design §6.4 §2). Rendered only when at least one
 * origin field exists — an orphan row with no provenance shows nothing
 * rather than "unknown".
 */
function OriginSection({ run }: { run: OperateRun }) {
  const { t } = useTranslation()
  const rows: Array<[string, string]> = []
  if (run.kind === 'automation' && run.automationId)
    rows.push([t('operate.origin.automation'), run.name])
  if (run.automationId) rows.push([t('operate.origin.automationId'), run.automationId])
  if (run.projectId) rows.push([t('operate.origin.project'), run.projectId])
  if (run.sessionId) rows.push([t('commandCenter.inspector.session'), run.sessionId])
  if (rows.length === 0) return null

  return (
    <section data-section="origin" aria-label={t('operate.sections.origin')}>
      <SectionTitle>{t('operate.sections.origin')}</SectionTitle>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
        {rows.map(([label, value]) => (
          <Fragment key={label}>
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="truncate text-right font-mono" title={value}>
              {value}
            </dd>
          </Fragment>
        ))}
      </dl>
    </section>
  )
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mb-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
      {children}
    </h3>
  )
}
