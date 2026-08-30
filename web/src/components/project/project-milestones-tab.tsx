import { Flag, Plus, Trash2 } from 'lucide-react'
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
  type Milestone,
  useCreateMilestone,
  useDeleteMilestone,
  useMilestones,
} from '@/hooks/use-issues'

function MilestoneCard({ milestone, projectId }: { milestone: Milestone; projectId: string }) {
  const { t } = useTranslation()
  const del = useDeleteMilestone(projectId)

  return (
    <Card>
      <CardContent className="space-y-2 pt-4">
        <div className="flex items-start gap-2">
          <Flag className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-sm font-medium">{milestone.title}</span>
              {milestone.status === 'closed' && (
                <Badge variant="secondary" className="text-2xs">
                  {t('milestones.closed')}
                </Badge>
              )}
            </div>
            {milestone.description && (
              <p className="mt-0.5 text-xs text-muted-foreground">{milestone.description}</p>
            )}
          </div>
          <Button
            variant="ghost"
            size="icon"
            onClick={() => del.mutate(milestone.slug)}
            disabled={del.isPending}
            aria-label={t('common.delete')}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>

        <div className="space-y-1">
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>
              {t('milestones.progress', { closed: milestone.closed, total: milestone.total })}
            </span>
            {milestone.due && <span>{t('milestones.due', { date: milestone.due })}</span>}
          </div>
          <span aria-hidden className="block h-1.5 overflow-hidden rounded-full bg-border">
            <span
              className="block h-full rounded-full bg-primary transition-all"
              style={{ width: `${milestone.percent}%` }}
            />
          </span>
        </div>

        {del.isError && <p className="text-xs text-destructive">{(del.error as Error).message}</p>}
      </CardContent>
    </Card>
  )
}

function NewMilestoneForm({ projectId, onDone }: { projectId: string; onDone: () => void }) {
  const { t } = useTranslation()
  const [title, setTitle] = useState('')
  const [due, setDue] = useState('')
  const create = useCreateMilestone(projectId)

  const submit = () => {
    if (!title.trim()) return
    create.mutate({ title: title.trim(), due: due || undefined }, { onSuccess: onDone })
  }

  return (
    <Card>
      <CardContent className="space-y-2 pt-4">
        <Input
          autoFocus
          placeholder={t('milestones.titlePlaceholder')}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') submit()
            if (e.key === 'Escape') onDone()
          }}
        />
        <div className="flex items-center gap-2">
          <Input
            type="date"
            className="h-8 max-w-[12rem]"
            value={due}
            onChange={(e) => setDue(e.target.value)}
          />
          <div className="ml-auto flex gap-1">
            <Button variant="ghost" size="sm" onClick={onDone}>
              {t('common.cancel')}
            </Button>
            <Button size="sm" onClick={submit} disabled={!title.trim() || create.isPending}>
              {t('milestones.create')}
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

export function ProjectMilestonesTab({ projectId }: { projectId: string }) {
  const { t } = useTranslation()
  const [creating, setCreating] = useState(false)
  const { data, isLoading, isError, refetch } = useMilestones(projectId)

  const milestones = data?.items ?? []

  return (
    <div className="space-y-3">
      <div className="flex items-center">
        <Button size="sm" className="ml-auto" onClick={() => setCreating(true)}>
          <Plus className="h-3 w-3" />
          {t('milestones.new')}
        </Button>
      </div>

      {creating && <NewMilestoneForm projectId={projectId} onDone={() => setCreating(false)} />}

      {isLoading ? (
        <LoadingCards count={2} />
      ) : isError ? (
        <ErrorState onRetry={() => refetch()} />
      ) : milestones.length === 0 ? (
        <EmptyState
          icon={<Flag className="h-10 w-10" />}
          title={t('milestones.empty')}
          description={t('milestones.emptyDesc')}
        />
      ) : (
        <div className="grid gap-3 md:grid-cols-2">
          {milestones.map((m) => (
            <MilestoneCard key={m.slug} milestone={m} projectId={projectId} />
          ))}
        </div>
      )}
    </div>
  )
}
