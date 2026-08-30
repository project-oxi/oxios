// Issue render — shows what an `issue` tool call did, as a card.
//
// The kernel tool publishes a tagged payload on `tool_end.results`
// (see crates/oxios-kernel/src/tools/issue_tool.rs): `kind` is one of
// `issue` | `list` | `milestone` | `milestones`.
import { CircleDot, Flag, Lock } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import type { ToolRenderComponent } from './registry'

export interface IssueAssignment {
  session: string
  acquired_at: string
  alive: boolean
}

export interface IssueSummary {
  number: number
  title: string
  status: string
  priority: string
  labels?: string[]
  milestone?: string | null
  assigned_to?: IssueAssignment | null
  body?: string | null
}

export interface MilestoneSummary {
  slug: string
  title: string
  status: string
  total: number
  closed: number
  percent: number
}

const PRIORITY_CLASS: Record<string, string> = {
  low: 'text-muted-foreground',
  medium: 'text-foreground/70',
  high: 'text-warning',
  critical: 'text-destructive',
}

function IssueRow({ issue }: { issue: IssueSummary }) {
  const { t } = useTranslation()
  const closed = issue.status === 'closed'
  return (
    <div className="flex items-start gap-2 py-1">
      <span className={cn('mt-0.5 shrink-0', closed ? 'text-muted-foreground' : 'text-success')}>
        <CircleDot className="h-3.5 w-3.5" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
            #{issue.number}
          </span>
          <span className={cn('truncate text-xs', closed && 'line-through text-muted-foreground')}>
            {issue.title}
          </span>
          {issue.assigned_to?.alive && (
            <Lock className="h-3 w-3 shrink-0 text-warning" aria-label={t('chat.issue.claimed')} />
          )}
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-2xs">
          <span className={PRIORITY_CLASS[issue.priority] ?? 'text-muted-foreground'}>
            {issue.priority}
          </span>
          {issue.milestone && (
            <span className="inline-flex items-center gap-0.5 rounded bg-muted px-1 py-px text-muted-foreground">
              <Flag className="h-2.5 w-2.5" />
              {issue.milestone}
            </span>
          )}
          {issue.labels?.map((label) => (
            <span key={label} className="rounded bg-muted px-1 py-px text-muted-foreground">
              {label}
            </span>
          ))}
        </div>
      </div>
    </div>
  )
}

function MilestoneRow({ milestone }: { milestone: MilestoneSummary }) {
  return (
    <div className="py-1">
      <div className="flex items-center gap-1.5">
        <Flag className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="truncate text-xs">{milestone.title}</span>
        <span className="ml-auto shrink-0 text-2xs tabular-nums text-muted-foreground">
          {milestone.closed}/{milestone.total}
        </span>
      </div>
      <span aria-hidden className="mt-1 block h-1 overflow-hidden rounded-full bg-border">
        <span
          className="block h-full rounded-full bg-primary"
          style={{ width: `${milestone.percent}%` }}
        />
      </span>
    </div>
  )
}

export const IssueRender: ToolRenderComponent = ({ args, result, isRunning }) => {
  const { t } = useTranslation()
  const action = (args?.action ?? '') as string

  if (isRunning) {
    return (
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-status-warning" />
        {t('chat.issue.working', { action })}
      </div>
    )
  }

  const payload = result && typeof result === 'object' ? (result as Record<string, unknown>) : null
  const kind = payload && typeof payload.kind === 'string' ? payload.kind : null

  // Unknown or absent payload: fall back to the tool's own text, which the
  // kernel always produces.
  if (!payload || !kind) {
    return (
      <pre className="whitespace-pre-wrap text-xs text-muted-foreground">
        {typeof result === 'string' ? result : t('chat.issue.done', { action })}
      </pre>
    )
  }

  if (kind === 'list') {
    const issues = (payload.issues ?? []) as IssueSummary[]
    return (
      <div className="space-y-1 text-sm">
        <div className="text-xs text-muted-foreground">
          {t('chat.issue.count', { count: issues.length })}
        </div>
        {issues.map((issue) => (
          <IssueRow key={issue.number} issue={issue} />
        ))}
      </div>
    )
  }

  if (kind === 'issue') {
    const issue = payload.issue as IssueSummary | undefined
    return issue ? <IssueRow issue={issue} /> : null
  }

  if (kind === 'milestones') {
    const milestones = (payload.milestones ?? []) as MilestoneSummary[]
    return (
      <div className="space-y-1 text-sm">
        {milestones.map((m) => (
          <MilestoneRow key={m.slug} milestone={m} />
        ))}
      </div>
    )
  }

  if (kind === 'milestone') {
    const milestone = payload.milestone as MilestoneSummary | undefined
    return milestone ? <MilestoneRow milestone={milestone} /> : null
  }

  return null
}
