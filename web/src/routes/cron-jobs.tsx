import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import {
  CalendarClock,
  List,
  Pencil,
  Plus,
  Power,
  PowerOff,
  Sparkles,
  Timer,
  Trash2,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { CronScheduleEditor } from '@/components/cron/cron-schedule-editor'
import { CronTimelineView } from '@/components/cron/cron-timeline-view'
import { EditCronDialog } from '@/components/cron/edit-cron-dialog'
import { TaskTemplateGallery } from '@/components/cron/task-template-gallery'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { RefreshButton } from '@/components/shared/refresh-button'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import { Input } from '@/components/ui/input'
import { useMigrateCronTasks } from '@/hooks/use-tasks'
import { api } from '@/lib/api-client'
import { DEFAULT_CRON } from '@/lib/cron-utils'
import { cn } from '@/lib/utils'
import type { CronJob } from '@/types'
import type { CronMigrationReport } from '@/types/task'
import type { TaskTemplate } from '@/types/task-templates'

export const Route = createFileRoute('/cron-jobs')({ component: CronJobsPage })

function CronJobsPage() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [showCreate, setShowCreate] = useState(false)
  const [editingJob, setEditingJob] = useState<CronJob | null>(null)
  const [name, setName] = useState('')
  const [schedule, setSchedule] = useState(DEFAULT_CRON)
  const [goal, setGoal] = useState('')
  const [viewMode, setViewMode] = useState<'list' | 'timeline'>('timeline')
  const [migrationPreview, setMigrationPreview] = useState<CronMigrationReport | null>(null)
  const [confirmMigrate, setConfirmMigrate] = useState(false)
  const [lastReport, setLastReport] = useState<CronMigrationReport | null>(null)
  const migrateMutation = useMigrateCronTasks()

  const { data, isLoading, isError, refetch, isFetching } = useQuery({
    queryKey: ['cron-jobs'],
    queryFn: async () => {
      const res = await api.get<{ jobs: CronJob[] }>('/api/cron-jobs')
      return Array.isArray(res?.jobs) ? res.jobs : []
    },
    refetchInterval: 10000,
  })

  const createMutation = useMutation({
    mutationFn: (job: { name: string; schedule: string; goal: string }) =>
      api.post('/api/cron-jobs', job),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['cron-jobs'] })
      setShowCreate(false)
      setName('')
      setSchedule(DEFAULT_CRON)
      setGoal('')
    },
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/api/cron-jobs/${id}`),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['cron-jobs'] }),
  })

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      api.post(`/api/cron-jobs/${id}/edit`, { enabled }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['cron-jobs'] }),
  })

  const updateMutation = useMutation({
    mutationFn: (job: { id: string; name: string; schedule: string; goal: string }) =>
      api.post(`/api/cron-jobs/${job.id}/edit`, job),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['cron-jobs'] })
      setEditingJob(null)
    },
  })

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const jobs = Array.isArray(data) ? data : []

  const handleSelectTemplate = (template: TaskTemplate) => {
    setName(template.title)
    setSchedule(template.cronPattern)
    setGoal(template.instruction)
    setShowCreate(true)
  }

  const handleMigrateDryRun = () => {
    migrateMutation.mutate(
      { dryRun: true },
      {
        onSuccess: (report) => setMigrationPreview(report),
        onError: () => toast.error(t('cronJobs.migrateDryRunFailed')),
      },
    )
  }

  const handleMigrateConfirm = () => {
    setConfirmMigrate(false)
    migrateMutation.mutate(
      { dryRun: false },
      {
        onSuccess: (report) => {
          setLastReport(report)
          setMigrationPreview(null)
          toast.success(
            t('cronJobs.migrateSuccess', {
              count: report.created.length,
              created: report.created.length,
              skipped: report.skipped.length,
            }),
          )
        },
        onError: () => toast.error(t('cronJobs.migrateFailed')),
      },
    )
  }

  return (
    <div className="space-y-6">
      {/* Task templates */}
      {jobs.length === 0 && (
        <div>
          <h2 className="text-lg font-semibold mb-3">
            {t('cronJobs.templates.galleryTitle')}
          </h2>
          <TaskTemplateGallery onSelectTemplate={handleSelectTemplate} />
        </div>
      )}
      <PageHeader
        title={t('cronJobs.title')}
        subtitle={t('cronJobs.subtitle')}
        actions={
          <>
            <div className="inline-flex gap-0.5 rounded-lg border bg-muted/50 p-0.5">
              <button
                type="button"
                onClick={() => setViewMode('list')}
                className={cn(
                  'flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
                  viewMode === 'list'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground',
                )}
              >
                <List className="h-3.5 w-3.5" />
                {t('cronJobs.timeline.viewList')}
              </button>
              <button
                type="button"
                onClick={() => setViewMode('timeline')}
                className={cn(
                  'flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
                  viewMode === 'timeline'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'text-muted-foreground hover:text-foreground',
                )}
              >
                <CalendarClock className="h-3.5 w-3.5" />
                {t('cronJobs.timeline.viewTimeline')}
              </button>
            </div>
            <RefreshButton onClick={() => refetch()} isFetching={isFetching} />
            <Button
              size="sm"
              variant="outline"
              className="gap-1.5"
              onClick={handleMigrateDryRun}
              disabled={migrateMutation.isPending || jobs.length === 0}
            >
              <Sparkles className="h-4 w-4" /> {t('cronJobs.migrateToTasks')}
            </Button>
            <Button size="sm" onClick={() => setShowCreate(true)}>
              <Plus className="h-4 w-4" /> {t('cronJobs.newJob')}
            </Button>
          </>
        }
      />

      {showCreate && (
        <Card>
          <CardHeader>
            <CardTitle>{t('cronJobs.createCronJob')}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('cronJobs.jobNamePlaceholder')}
            />
            <CronScheduleEditor value={schedule} onChange={setSchedule} />
            <span className="text-xs text-muted-foreground">{t('cronJobs.goalLabel')}</span>
            <Input
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder={t('cronJobs.goalPlaceholder')}
            />
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={() => createMutation.mutate({ name, schedule, goal })}
                disabled={!name || !schedule || !goal || createMutation.isPending}
              >
                {t('common.create')}
              </Button>
              <Button variant="ghost" size="sm" onClick={() => setShowCreate(false)}>
                {t('common.cancel')}
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {jobs.length === 0 && !showCreate ? (
        <EmptyState
          icon={<Timer className="h-10 w-10" />}
          title={t('cronJobs.noCronJobs')}
          description={t('cronJobs.description')}
        />
      ) : viewMode === 'timeline' ? (
        <CronTimelineView
          jobs={jobs}
          onEdit={setEditingJob}
          onToggle={(job) => toggleMutation.mutate({ id: job.id, enabled: !job.enabled })}
          onDelete={(job) => deleteMutation.mutate(job.id)}
        />
      ) : (
        <div className="space-y-3">
          {jobs.map((job) => (
            <Card key={job.id} className={cn('transition-opacity', !job.enabled && 'opacity-60')}>
              <CardContent className="flex items-center justify-between p-4">
                <div>
                  <div className="font-medium flex items-center gap-2">
                    <Timer className="h-4 w-4" /> {job.name}
                    <Badge variant={job.enabled ? 'success' : 'secondary'}>
                      {job.enabled ? t('common.enabled') : t('common.disabled')}
                    </Badge>
                  </div>
                  <p className="text-sm text-muted-foreground mt-1">
                    <code className="text-xs bg-muted px-1 py-0.5 rounded">{job.schedule}</code>
                    {' → '}
                    <code className="text-xs bg-muted px-1 py-0.5 rounded">{job.goal}</code>
                  </p>
                  <div className="flex gap-4 text-xs text-muted-foreground mt-1">
                    {job.last_run && (
                      <span>
                        {t('cronJobs.lastRunLabel')} {new Date(job.last_run).toLocaleString()}
                      </span>
                    )}
                    {job.next_run && (
                      <span>
                        {t('cronJobs.nextRunLabel')} {new Date(job.next_run).toLocaleString()}
                      </span>
                    )}
                  </div>
                </div>
                <div className="flex gap-1">
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => setEditingJob(job)}
                    aria-label={t('common.edit')}
                  >
                    <Pencil className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => toggleMutation.mutate({ id: job.id, enabled: !job.enabled })}
                    aria-label={job.enabled ? t('cronJobs.disableJob') : t('cronJobs.enableJob')}
                  >
                    {job.enabled ? <PowerOff className="h-4 w-4" /> : <Power className="h-4 w-4" />}
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => deleteMutation.mutate(job.id)}
                    aria-label={t('cronJobs.deleteJob')}
                  >
                    <Trash2 className="h-4 w-4 text-destructive" />
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
      <EditCronDialog
        job={editingJob}
        onOpenChange={(open) => !open && setEditingJob(null)}
        onSave={(patch) => {
          if (!editingJob) return
          updateMutation.mutate({
            id: editingJob.id,
            name: patch.name,
            schedule: patch.schedule,
            goal: patch.goal,
          })
        }}
        isPending={updateMutation.isPending}
      />

      {/* Migration preview dialog */}
      {migrationPreview && (
        <Card>
          <CardHeader>
            <CardTitle>{t('cronJobs.migratePreviewTitle')}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <p className="text-muted-foreground">{t('cronJobs.migratePreviewIntro')}</p>
            <div className="rounded-lg border p-3 bg-muted/30 space-y-1">
              <div className="font-medium">
                {t('cronJobs.migrateCreatedCount', { count: migrationPreview.created.length })}
              </div>
              {migrationPreview.created.length > 0 && (
                <ul className="list-disc pl-5 text-xs text-muted-foreground">
                  {migrationPreview.created.map((tk) => (
                    <li key={tk.id}>
                      <span className="font-mono">{tk.name}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            {migrationPreview.skipped.length > 0 && (
              <div className="rounded-lg border p-3 bg-muted/30 space-y-1">
                <div className="font-medium">
                  {t('cronJobs.migrateSkippedCount', { count: migrationPreview.skipped.length })}
                </div>
                <ul className="list-disc pl-5 text-xs text-muted-foreground">
                  {migrationPreview.skipped.map((s) => (
                    <li key={s.name}>
                      <span className="font-mono">{s.name}</span> — {s.reason}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <div className="flex justify-end gap-2 pt-2">
              <Button variant="ghost" size="sm" onClick={() => setMigrationPreview(null)}>
                {t('common.cancel')}
              </Button>
              <Button
                size="sm"
                disabled={migrationPreview.created.length === 0 || migrateMutation.isPending}
                onClick={() => setConfirmMigrate(true)}
              >
                {t('cronJobs.migrateConfirm', {
                  count: migrationPreview.created.length,
                })}
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Final report */}
      {lastReport && (
        <div className="rounded-lg border bg-status-success/10 px-3 py-2 text-xs text-status-success-on-surface">
          {t('cronJobs.migrateSummary', {
            count: lastReport.created.length,
            created: lastReport.created.length,
            skipped: lastReport.skipped.length,
          })}
        </div>
      )}

      {/* Confirm dialog */}
      <ConfirmDialog
        open={confirmMigrate}
        onOpenChange={setConfirmMigrate}
        title={t('cronJobs.migrateConfirmTitle')}
        description={t('cronJobs.migrateConfirmDescription', {
          count: migrationPreview?.created.length ?? 0,
        })}
        confirmLabel={t('cronJobs.migrateConfirmAction')}
        onConfirm={handleMigrateConfirm}
      />
    </div>
  )
}
