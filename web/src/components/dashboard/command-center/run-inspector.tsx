import {
  Activity,
  CheckCircle,
  Loader2,
  type LucideIcon,
  Square,
  Wrench,
  XCircle,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import { useAgentTrace } from '@/hooks/use-agent-trace'
import { cn, formatDuration, formatRelativeTime } from '@/lib/utils'
import type { AgentTrace, AgentTraceStep } from '@/types/agent'
import { statusMeta } from './runs'

/** Bounded summaries — the inspector is a triage view, not a trace viewer (spec §5). */
const MAX_STEPS = 10
const MAX_TOOLS = 5

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
 * Bounded execution-steps + recent-tool-calls detail (spec §5). The operate
 * inspector embeds it (via RunTraceDetail) for agent-kind runs; it stays a
 * triage view, not a full trace viewer.
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
 * embed for agent-kind rows. Polls only while the trace is incomplete.
 */
export function RunTraceDetail({ agentId }: { agentId: string }) {
  const { data: trace } = useAgentTrace(agentId)
  return <InspectorTraceSection trace={trace} />
}
