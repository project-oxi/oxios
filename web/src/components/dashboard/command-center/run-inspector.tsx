import { Link } from '@tanstack/react-router'
import {
  Activity,
  CheckCircle,
  ExternalLink,
  Loader2,
  type LucideIcon,
  Square,
  Wrench,
  X,
  XCircle,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { useAgentDetail, useAgentTrace } from '@/hooks/use-agent-trace'
import { cn, formatDuration, formatRelativeTime } from '@/lib/utils'
import type { AgentListItem, AgentTrace, AgentTraceStep } from '@/types/agent'
import { formatElapsed, isStoppable, statusMeta } from './runs'

/** Bounded summaries — the inspector is a triage view, not a trace viewer (spec §5). */
const MAX_STEPS = 10
const MAX_TOOLS = 5
const TERMINAL_STATUSES = ['completed', 'failed', 'stopped']

export interface RunInspectorProps {
  run: AgentListItem
  /** True when the selected run no longer appears among active runs. */
  stale: boolean
  onClose: () => void
  /** 'panel' = fixed desktop aside; 'dialog' = mobile sheet. */
  variant: 'panel' | 'dialog'
  onStop: (id: string) => void
  stopping: boolean
  /** Shared clock (page-owned) so elapsed stays consistent with the list. */
  now?: number
}

/**
 * Run inspector — desktop right column / mobile sheet (spec §5).
 *
 * An operational summary of the selected run: identity, lifecycle, model,
 * session, elapsed, bounded step list, and a bounded recent-tool-call
 * list. Deep investigation stays in the canonical /agents/$agentId and
 * trace routes; this component only links there.
 */
export function RunInspector({
  run,
  stale,
  onClose,
  variant,
  onStop,
  stopping,
  now = Date.now(),
}: RunInspectorProps) {
  const { t } = useTranslation()
  const { data: detail } = useAgentDetail(run.id)
  const { data: trace } = useAgentTrace(run.id)

  const body = (
    <div className="flex h-full min-h-0 flex-col gap-4 p-4">
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

      <StatusChip status={run.status} />

      {(stale || TERMINAL_STATUSES.includes(run.status.toLowerCase())) && (
        <p
          className={cn(
            'rounded-md border px-2.5 py-1.5 text-xs',
            'border-status-info-subtle-border bg-status-info-subtle text-status-info-on-surface',
          )}
          role="status"
        >
          {stale
            ? t('commandCenter.inspector.noLongerActive')
            : t('commandCenter.inspector.finished')}
        </p>
      )}

      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
        <dt className="text-muted-foreground">{t('commandCenter.inspector.model')}</dt>
        <dd className="truncate font-mono">{detail?.model_id ?? run.model_id ?? '—'}</dd>
        {run.session_id && (
          <>
            <dt className="text-muted-foreground">{t('commandCenter.inspector.session')}</dt>
            <dd className="truncate font-mono">{run.session_id.slice(0, 12)}</dd>
          </>
        )}
        <dt className="text-muted-foreground">{t('commandCenter.inspector.started')}</dt>
        <dd className="truncate font-mono tabular-nums">
          {run.started_at ? new Date(run.started_at).toLocaleTimeString() : '—'}
        </dd>
        <dt className="text-muted-foreground">{t('commandCenter.inspector.elapsed')}</dt>
        <dd className="font-mono tabular-nums">
          {formatElapsed(
            Date.parse(run.started_at ?? run.created_at),
            now,
            run.completed_at ? Date.parse(run.completed_at) : null,
          )}
        </dd>
      </dl>

      <InspectorTraceSection trace={trace} />

      <footer className="flex items-center gap-2 border-t pt-3">
        <Button variant="outline" size="sm" asChild>
          <Link to="/agents/$agentId" params={{ agentId: run.id }}>
            <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
            {t('commandCenter.inspector.openRun')}
          </Link>
        </Button>
        {isStoppable(run) && (
          <Button
            variant="destructive"
            size="sm"
            className="ml-auto"
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
      </footer>
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
      className="w-[340px] shrink-0 overflow-hidden rounded-lg border bg-background"
    >
      {body}
    </aside>
  )
}

export function StatusChip({ status }: { status: string }) {
  const { t } = useTranslation()
  const meta = statusMeta(status)
  const icons: Record<string, LucideIcon> = {
    activity: Activity,
    loader: Loader2,
    alert: XCircle,
    pause: Activity,
    check: CheckCircle,
    square: Square,
  }
  const Icon = icons[meta.icon] ?? Activity
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-xs font-medium', meta.textClass)}>
      <Icon
        className={cn(
          'h-3.5 w-3.5',
          meta.iconClass,
          meta.spin && 'motion-safe:animate-spin',
          meta.pulse && 'motion-safe:animate-pulse',
        )}
        aria-hidden="true"
      />
      {t(meta.labelKey)}
    </span>
  )
}

function stepVisual(step: AgentTraceStep): { icon: LucideIcon; cls: string } {
  if (step.status === 'completed') return { icon: CheckCircle, cls: 'text-status-success' }
  if (step.status === 'failed') return { icon: XCircle, cls: 'text-status-error' }
  return { icon: Loader2, cls: 'text-status-info' }
}

function InspectorStep({ step }: { step: AgentTraceStep }) {
  const { icon: Icon, cls } = stepVisual(step)
  return (
    <li
      data-testid="inspector-step"
      className="flex items-center gap-2 rounded-md border px-2 py-1.5 text-xs"
    >
      <Icon
        className={cn(
          'h-3 w-3 shrink-0',
          cls,
          step.status !== 'completed' && step.status !== 'failed' && 'motion-safe:animate-spin',
        )}
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1 truncate">{step.tool_name ?? step.action}</span>
      <span className="shrink-0 font-mono text-2xs tabular-nums text-muted-foreground">
        {formatDuration(step.duration_ms)}
      </span>
    </li>
  )
}

function InspectorTool({ step }: { step: AgentTraceStep }) {
  const { icon: Icon, cls } = stepVisual(step)
  return (
    <li
      data-testid="inspector-tool"
      className="flex items-center gap-2 rounded-md bg-muted/40 px-2 py-1.5 text-xs"
    >
      <Wrench className="h-3 w-3 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate font-mono">{step.tool_name}</span>
      <Badge variant="outline" className="px-1 py-0 text-2xs">
        {step.status}
      </Badge>
      <Icon className={cn('h-3 w-3 shrink-0', cls)} aria-hidden="true" />
      <span className="shrink-0 text-2xs tabular-nums text-muted-foreground">
        {formatRelativeTime(step.started_at)}
      </span>
    </li>
  )
}

/**
 * Bounded execution-steps + recent-tool-calls detail (spec §5). Shared by
 * the dashboard RunInspector and the operate inspector, which embeds it
 * for agent-kind runs; both stay triage views — deep investigation
 * remains in the canonical /agents routes.
 */
export function InspectorTraceSection({ trace }: { trace?: AgentTrace }) {
  const { t } = useTranslation()
  const steps = trace?.steps ?? []
  const recentSteps = [...steps].slice(-MAX_STEPS).reverse()
  const recentTools = [...steps]
    .filter((s) => s.kind === 'tool')
    .slice(-MAX_TOOLS)
    .reverse()

  return (
    <section
      className="min-h-0 flex-1 space-y-2 overflow-y-auto"
      aria-label={t('commandCenter.inspector.steps')}
    >
      <h3 className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        {t('commandCenter.inspector.steps')}
      </h3>
      {recentSteps.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t('commandCenter.inspector.noSteps')}</p>
      ) : (
        <ol className="space-y-1">
          {recentSteps.map((step) => (
            <InspectorStep key={step.index} step={step} />
          ))}
        </ol>
      )}

      <h3 className="pt-2 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        {t('commandCenter.inspector.recentTools')}
      </h3>
      {recentTools.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t('commandCenter.inspector.toolsEmpty')}</p>
      ) : (
        <ul className="space-y-1">
          {recentTools.map((step) => (
            <InspectorTool key={`tool-${step.index}`} step={step} />
          ))}
        </ul>
      )}
    </section>
  )
}

/**
 * Self-fetching trace detail for one agent run — the operate inspector's
 * embed for agent-kind rows. Polls only while the trace is incomplete
 * (same cadence as the dashboard inspector via the shared hook).
 */
export function RunTraceDetail({ agentId }: { agentId: string }) {
  const { data: trace } = useAgentTrace(agentId)
  return <InspectorTraceSection trace={trace} />
}
