import { AlertTriangle, Bell } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useApproveApproval, usePendingApprovals, useRejectApproval } from '@/hooks/use-approvals'
import { formatRelativeTime } from '@/lib/utils'
import type { Approval } from '@/types'

/**
 * Needs your attention — pending approvals as first-class rows (spec §2).
 *
 * The section exists ONLY while there is work awaiting a decision or the
 * query failed: absence means nothing needs intervention, and no fake
 * "all clear" card is manufactured. A failed query localizes its error
 * here with retry rather than blanking the whole dashboard.
 */
export function AttentionQueue() {
  const { t } = useTranslation()
  const { items: pending, isLoading, isError, refetch } = usePendingApprovals()
  const approve = useApproveApproval()
  const reject = useRejectApproval()
  /** The approval whose decision is in flight — blocks duplicate submits per row. */
  const [busyId, setBusyId] = useState<string | null>(null)

  if (isLoading) return null
  if (!isError && pending.length === 0) return null

  const decide = (id: string, mutate: typeof approve.mutate, successKey: string) => {
    setBusyId(id)
    mutate(id, {
      onSuccess: () => toast.success(t(successKey)),
      onError: (err) => toast.error(t('approvals.mutationError', { error: String(err) })),
      onSettled: () => setBusyId(null),
    })
  }

  const handleApprove = (id: string) => decide(id, approve.mutate, 'approvals.approveSuccess')
  const handleDeny = (id: string) => decide(id, reject.mutate, 'approvals.rejectSuccess')

  return (
    <Card className="border-status-warning-subtle-border">
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Bell className="h-4 w-4 text-status-warning" aria-hidden="true" />
          {t('commandCenter.attention.title')}
          <Badge variant="warning" className="ml-1">
            {pending.length}
          </Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="pt-2">
        {isError ? (
          <div className="flex items-center justify-between gap-2 py-2">
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <AlertTriangle className="h-4 w-4 text-status-error" aria-hidden="true" />
              {t('commandCenter.picture.unavailable')}
            </p>
            <Button variant="outline" size="sm" onClick={() => refetch()}>
              {t('commandCenter.picture.retry')}
            </Button>
          </div>
        ) : (
          <ul className="space-y-2">
            {pending.map((approval) => (
              <ApprovalRow
                key={approval.id}
                approval={approval}
                busy={busyId === approval.id}
                onApprove={handleApprove}
                onDeny={handleDeny}
              />
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  )
}

interface ApprovalRowProps {
  approval: Approval
  busy: boolean
  onApprove: (id: string) => void
  onDeny: (id: string) => void
}

function ApprovalRow({ approval, busy, onApprove, onDeny }: ApprovalRowProps) {
  const { t } = useTranslation()
  const action = approval.action || ''
  const resource = approval.resource || ''
  const reason = approval.reason || action
  const approveLabel = t('approvals.approve')
  const denyLabel = t('approvals.deny')

  return (
    <li className="flex flex-wrap items-center gap-2 rounded-lg border bg-status-warning-subtle p-3">
      <AlertTriangle className="h-4 w-4 shrink-0 text-status-warning" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-foreground" title={reason}>
          <span className="mr-1.5 font-mono text-xs text-muted-foreground">{action}</span>
          {resource}
        </p>
        <p className="text-xs text-muted-foreground">
          {reason && reason !== action ? `${t('dashboard.risk')}: ${reason} · ` : ''}
          {formatRelativeTime(approval.created_at, t)}
        </p>
      </div>
      {/* Actions wrap below the reason on small screens (spec: responsive table). */}
      <div className="flex w-full items-center justify-end gap-2 sm:w-auto">
        <Button
          size="sm"
          variant="outline"
          className="border-status-success-subtle-border text-status-success-on-surface hover:bg-status-success-subtle"
          onClick={() => onApprove(approval.id)}
          disabled={busy}
          aria-label={approveLabel}
        >
          {approveLabel}
        </Button>
        <Button
          size="sm"
          variant="outline"
          className="border-status-error-subtle-border text-status-error-on-surface hover:bg-status-error-subtle"
          onClick={() => onDeny(approval.id)}
          disabled={busy}
          aria-label={denyLabel}
        >
          {denyLabel}
        </Button>
      </div>
    </li>
  )
}
