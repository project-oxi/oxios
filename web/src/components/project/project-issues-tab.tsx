import { CircleDot, Flag, Lock, Plus, Unlock } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import {
  type Issue,
  type IssueFilters,
  useCloseIssue,
  useCreateIssue,
  useIssues,
  useMilestones,
  useReleaseIssue,
  useReopenIssue,
} from '@/hooks/use-issues'
import { cn } from '@/lib/utils'

const PRIORITY_BADGE: Record<string, string> = {
  low: 'bg-muted text-muted-foreground',
  medium: 'bg-muted text-foreground/70',
  high: 'bg-warning-subtle text-warning',
  critical: 'bg-destructive/15 text-destructive',
}

function IssueRow({ issue, projectId }: { issue: Issue; projectId: string }) {
  const { t } = useTranslation()
  const close = useCloseIssue(projectId)
  const reopen = useReopenIssue(projectId)
  const release = useReleaseIssue(projectId)
  const closed = issue.status === 'closed'
  // A claim only means anything while its owning process is alive; a stale one
  // is shown as releasable rather than as a lock the user cannot clear.
  const heldLive = issue.assigned_to?.alive === true
  const heldStale = !!issue.assigned_to && !issue.assigned_to.alive

  return (
    <div className="flex items-start gap-3 border-b px-3 py-2 last:border-b-0">
      <CircleDot
        className={cn('mt-0.5 h-4 w-4 shrink-0', closed ? 'text-muted-foreground' : 'text-success')}
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
            #{issue.number}
          </span>
          <span className={cn('truncate text-sm', closed && 'text-muted-foreground line-through')}>
            {issue.title}
          </span>
          {heldLive && (
            <Lock
              className="h-3.5 w-3.5 shrink-0 text-warning"
              aria-label={t('issues.claimedBy', { session: issue.assigned_to?.session })}
            />
          )}
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-1.5">
          <Badge className={cn('text-2xs', PRIORITY_BADGE[issue.priority])} variant="secondary">
            {issue.priority}
          </Badge>
          {issue.milestone && (
            <Badge variant="outline" className="gap-1 text-2xs">
              <Flag className="h-2.5 w-2.5" />
              {issue.milestone}
            </Badge>
          )}
          {issue.labels.map((label) => (
            <Badge key={label} variant="secondary" className="text-2xs">
              {label}
            </Badge>
          ))}
          {heldStale && (
            <span className="text-2xs text-muted-foreground">{t('issues.staleClaim')}</span>
          )}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {heldStale && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => release.mutate({ number: issue.number })}
            disabled={release.isPending}
          >
            <Unlock className="h-3 w-3" />
            {t('issues.release')}
          </Button>
        )}
        {closed ? (
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              reopen.mutate({ number: issue.number, content_hash: issue.content_hash ?? undefined })
            }
            disabled={reopen.isPending}
          >
            {t('issues.reopen')}
          </Button>
        ) : (
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              close.mutate({ number: issue.number, content_hash: issue.content_hash ?? undefined })
            }
            disabled={close.isPending || heldLive}
            title={
              heldLive ? t('issues.claimedBy', { session: issue.assigned_to?.session }) : undefined
            }
          >
            {t('issues.close')}
          </Button>
        )}
      </div>
    </div>
  )
}

function NewIssueForm({ projectId, onDone }: { projectId: string; onDone: () => void }) {
  const { t } = useTranslation()
  const [title, setTitle] = useState('')
  const [priority, setPriority] = useState('medium')
  const [milestone, setMilestone] = useState('')
  const create = useCreateIssue(projectId)
  const { data: milestones } = useMilestones(projectId)

  const submit = () => {
    if (!title.trim()) return
    create.mutate(
      { title: title.trim(), priority, milestone: milestone || undefined },
      { onSuccess: onDone },
    )
  }

  return (
    <Card>
      <CardContent className="space-y-2 pt-4">
        <Input
          autoFocus
          placeholder={t('issues.titlePlaceholder')}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') submit()
            if (e.key === 'Escape') onDone()
          }}
        />
        <div className="flex flex-wrap items-center gap-2">
          <select
            className="h-8 rounded border bg-background px-2 text-xs"
            value={priority}
            onChange={(e) => setPriority(e.target.value)}
          >
            {['low', 'medium', 'high', 'critical'].map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
          <select
            className="h-8 rounded border bg-background px-2 text-xs"
            value={milestone}
            onChange={(e) => setMilestone(e.target.value)}
          >
            <option value="">{t('issues.noMilestone')}</option>
            {milestones?.items?.map((m) => (
              <option key={m.slug} value={m.slug}>
                {m.title}
              </option>
            ))}
          </select>
          <div className="ml-auto flex gap-1">
            <Button variant="ghost" size="sm" onClick={onDone}>
              {t('common.cancel')}
            </Button>
            <Button size="sm" onClick={submit} disabled={!title.trim() || create.isPending}>
              {t('issues.create')}
            </Button>
          </div>
        </div>
        {create.isError && (
          <p className="text-xs text-destructive">{(create.error as Error).message}</p>
        )}
      </CardContent>
    </Card>
  )
}

export function ProjectIssuesTab({ projectId }: { projectId: string }) {
  const { t } = useTranslation()
  const [filters, setFilters] = useState<IssueFilters>({ status: 'open' })
  const [creating, setCreating] = useState(false)
  const { data, isLoading, isError, refetch } = useIssues(projectId, filters)

  const issues = data?.items ?? []

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex rounded-md border p-0.5">
          {(['open', 'closed', undefined] as const).map((status) => (
            <button
              key={status ?? 'all'}
              type="button"
              onClick={() => setFilters((f) => ({ ...f, status }))}
              className={cn(
                'rounded px-2 py-1 text-xs transition-colors',
                filters.status === status
                  ? 'bg-muted font-medium text-foreground'
                  : 'text-muted-foreground hover:text-foreground',
              )}
            >
              {t(`issues.filter.${status ?? 'all'}`)}
            </button>
          ))}
        </div>
        <Input
          placeholder={t('issues.searchPlaceholder')}
          className="h-8 max-w-xs"
          value={filters.text ?? ''}
          onChange={(e) => setFilters((f) => ({ ...f, text: e.target.value || undefined }))}
        />
        <Button size="sm" className="ml-auto" onClick={() => setCreating(true)}>
          <Plus className="h-3 w-3" />
          {t('issues.new')}
        </Button>
      </div>

      {creating && <NewIssueForm projectId={projectId} onDone={() => setCreating(false)} />}

      {isLoading ? (
        <LoadingCards count={3} />
      ) : isError ? (
        <ErrorState onRetry={() => refetch()} />
      ) : issues.length === 0 ? (
        <EmptyState
          icon={<CircleDot className="h-10 w-10" />}
          title={t('issues.empty')}
          description={t('issues.emptyDesc')}
        />
      ) : (
        <Card>
          <CardContent className="p-0">
            {issues.map((issue) => (
              <IssueRow key={issue.number} issue={issue} projectId={projectId} />
            ))}
          </CardContent>
        </Card>
      )}
    </div>
  )
}
