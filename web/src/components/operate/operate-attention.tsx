import { Link } from '@tanstack/react-router'
import { AlertTriangle, Bell, ChevronRight, Inbox } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { AttentionQueue } from '@/components/dashboard/command-center/attention-queue'
import { LiveTimelinePanel } from '@/components/dashboard/command-center/live-timeline-panel'
import { PageHeader } from '@/components/shared/page-header'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { usePendingApprovals } from '@/hooks/use-approvals'
import { type OperateAttentionItem, useOperateAttention } from '@/hooks/use-operate'
import { cn, formatRelativeTime } from '@/lib/utils'

/**
 * Operate home (design §6) — exception/decision first: the canonical
 * approval queue, then the operate attention endpoint's own categories
 * (waiting-input + failed runs; endpoint approval rows are NOT duplicated
 * here), then the live timeline on the shared SSE singleton. Categories
 * with zero items render no panel at all — absence is truthful.
 */
export function OperateAttention() {
  const { t } = useTranslation()
  const attention = useOperateAttention()
  const pending = usePendingApprovals()

  const items: OperateAttentionItem[] = attention.data?.items ?? []
  const waitingInput = items.filter((i: OperateAttentionItem) => i.kind === 'waiting_input')
  const failedRuns = items.filter((i: OperateAttentionItem) => i.kind === 'failed_run')
  const approvalsQuiet = !pending.isError && !pending.isLoading && pending.items.length === 0
  // Only the renderable categories count toward "nothing to show" —
  // endpoint approval rows are surfaced through the canonical AttentionQueue.
  const nothingAnywhere =
    !attention.isError && waitingInput.length === 0 && failedRuns.length === 0 && approvalsQuiet

  return (
    <div className="space-y-4 animate-fade-in-up">
      <PageHeader title={t('operate.attention')} subtitle={t('operate.attentionSubtitle')} />

      <AttentionQueue />

      {attention.isError && (
        <Card className="border-status-error-subtle-border">
          <CardContent className="flex items-center justify-between gap-2 py-3">
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <AlertTriangle className="h-4 w-4 text-status-error" aria-hidden="true" />
              {t('operate.attentionUnavailable')}
            </p>
            <Button variant="outline" size="sm" onClick={() => attention.refetch()}>
              {t('common.retry')}
            </Button>
          </CardContent>
        </Card>
      )}

      {waitingInput.length > 0 && (
        <AttentionGroup
          kind="waiting_input"
          title={t('operate.groupWaitingInput')}
          items={waitingInput}
        />
      )}
      {failedRuns.length > 0 && (
        <AttentionGroup kind="failed_run" title={t('operate.groupFailedRun')} items={failedRuns} />
      )}

      {nothingAnywhere && (
        <div className="flex items-center gap-2 rounded-lg border px-3 py-4 text-sm text-muted-foreground">
          <Inbox className="h-4 w-4 shrink-0" aria-hidden="true" />
          <span>{t('operate.attentionEmpty')}</span>
          <Link
            to="/operate/runs"
            className="ml-auto inline-flex shrink-0 items-center gap-0.5 text-xs transition-colors hover:text-foreground"
          >
            {t('operate.runCenter')}
            <ChevronRight className="h-3 w-3" aria-hidden="true" />
          </Link>
        </div>
      )}

      <LiveTimelinePanel />
    </div>
  )
}

/** One attention category — rendered only when it has rows. */
function AttentionGroup({
  kind,
  title,
  items,
}: {
  kind: 'waiting_input' | 'failed_run'
  title: string
  items: OperateAttentionItem[]
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Bell className="h-4 w-4 text-status-warning" aria-hidden="true" />
          {title}
          <span className="ml-1 rounded-full bg-status-warning-subtle px-1.5 py-0.5 text-2xs font-medium text-status-warning-on-subtle">
            {items.length}
          </span>
        </CardTitle>
      </CardHeader>
      <CardContent className="pt-2">
        <ul className="space-y-2" data-testid={`attention-group-${kind}`}>
          {items.map((item) => (
            <AttentionRow key={item.id} item={item} />
          ))}
        </ul>
      </CardContent>
    </Card>
  )
}

function ActionButton({ action, item }: { action: string; item: OperateAttentionItem }) {
  const { t } = useTranslation()
  const label =
    action === 'open_run'
      ? t('operate.actions.openRun')
      : action === 'open_agent'
        ? t('operate.actions.openAgent')
        : t('operate.actions.openAutomation')

  if (action === 'open_agent' && item.origin.agentId) {
    return (
      <Button variant="outline" size="sm" asChild>
        <Link to="/operate/runs">{label}</Link>
      </Button>
    )
  }
  if (action === 'open_run' && item.detailRoute && !item.detailRoute.startsWith('/studio')) {
    return (
      <Button variant="outline" size="sm" asChild>
        <Link to={item.detailRoute}>{label}</Link>
      </Button>
    )
  }
  // open_automation, and open_run into the (typed-route-less) studio area.
  const href =
    action === 'open_run' && item.detailRoute
      ? item.detailRoute
      : item.origin.automationId
        ? `/studio/automations/${item.origin.automationId}`
        : null
  if (!href) return null
  return (
    <Button variant="outline" size="sm" asChild>
      <a href={href}>{label}</a>
    </Button>
  )
}

function AttentionRow({ item }: { item: OperateAttentionItem }) {
  const { t } = useTranslation()
  const parts = originParts(item.origin)
  return (
    <li className="flex flex-wrap items-center gap-2 rounded-lg border p-3">
      <span className={cn('rounded-md border px-1.5 py-0.5 text-2xs', stateChipClass(item.state))}>
        {item.state}
      </span>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-foreground" title={item.summary}>
          {item.summary}
        </p>
        <p className="truncate text-2xs text-muted-foreground">
          {parts.join(' · ')}
          {parts.length > 0 ? ' · ' : ''}
          {formatRelativeTime(item.timestamp, t)}
        </p>
      </div>
      <div className="flex w-full items-center justify-end gap-2 sm:w-auto">
        {item.actions.map((action) => (
          <ActionButton key={action} action={action} item={item} />
        ))}
      </div>
    </li>
  )
}

/** Origin parts actually recorded on the item — nothing invented. */
function originParts(origin: OperateAttentionItem['origin']): string[] {
  const parts: string[] = []
  if (origin.automationName) parts.push(origin.automationName)
  if (origin.automationId) parts.push(origin.automationId)
  if (origin.agentId) parts.push(origin.agentId)
  if (origin.projectId) parts.push(origin.projectId)
  if (origin.sessionId) parts.push(origin.sessionId)
  return parts
}

/** State color follows the exact backend state string; label is text + border, never color alone. */
function stateChipClass(state: string): string {
  const s = state.toLowerCase()
  if (s === 'failed' || s === 'canceled' || s === 'cancelled')
    return 'border-status-error-subtle-border bg-status-error-subtle text-status-error-on-surface'
  if (s === 'pending' || s === 'waiting')
    return 'bg-status-warning-subtle text-status-warning-on-surface border-status-warning-subtle-border'
  return 'border-border bg-muted text-muted-foreground'
}
