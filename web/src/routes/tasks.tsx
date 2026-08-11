// Task page — list + create + schedule + run + detail (RFC-043)

import { createFileRoute } from '@tanstack/react-router'
import { CalendarClock, Clock, History, Play, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { CronScheduleEditor } from '@/components/cron/cron-schedule-editor'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  useCreateTask,
  useDeleteTask,
  useRunTask,
  useSetTaskSchedule,
  useTaskRuns,
  useTasks,
  useUpdateTaskStatus,
} from '@/hooks/use-tasks'
import { DEFAULT_CRON } from '@/lib/cron-utils'
import { cn } from '@/lib/utils'
import {
  TASK_STATUS_META,
  TASK_STATUSES,
  type Task,
  type TaskAutomationMode,
  type TaskStatus,
} from '@/types/task'

export const Route = createFileRoute('/tasks')({ component: TasksPage })

function relativeTime(iso: string | undefined | null): string | null {
  if (!iso) return null
  const dt = new Date(iso)
  if (Number.isNaN(dt.getTime())) return null
  const diffMs = dt.getTime() - Date.now()
  const absMin = Math.round(Math.abs(diffMs) / 60000)
  const past = diffMs < 0
  if (absMin < 1) return 'now'
  if (absMin < 60) return past ? `${absMin}m ago` : `in ${absMin}m`
  const hours = Math.round(absMin / 60)
  if (hours < 24) return past ? `${hours}h ago` : `in ${hours}h`
  const days = Math.round(hours / 24)
  return past ? `${days}d ago` : `in ${days}d`
}

function TasksPage() {
  const { t } = useTranslation()
  const { data, isLoading, isError, refetch } = useTasks()
  const [showCreate, setShowCreate] = useState(false)
  const [statusFilter, setStatusFilter] = useState<TaskStatus | 'all'>('all')
  const [detailTask, setDetailTask] = useState<Task | null>(null)

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const allTasks = data?.tasks ?? []
  const tasks =
    statusFilter === 'all' ? allTasks : allTasks.filter((tk) => tk.status === statusFilter)

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('tasks.title')}
        subtitle={t('tasks.subtitle')}
        actions={
          <Dialog open={showCreate} onOpenChange={setShowCreate}>
            <DialogTrigger asChild>
              <Button size="sm" className="gap-1.5">
                <Plus className="h-3.5 w-3.5" />
                {t('tasks.newTask')}
              </Button>
            </DialogTrigger>
            <CreateTaskDialog onClose={() => setShowCreate(false)} />
          </Dialog>
        }
      />

      {/* Status filter chips */}
      <div className="flex items-center gap-1 overflow-x-auto pb-1">
        <StatusChip
          label={t('tasks.all')}
          count={allTasks.length}
          active={statusFilter === 'all'}
          onClick={() => setStatusFilter('all')}
        />
        {TASK_STATUSES.map((status) => {
          const count = allTasks.filter((tk) => tk.status === status).length
          if (count === 0) return null
          const meta = TASK_STATUS_META[status]
          return (
            <StatusChip
              key={status}
              label={meta.label}
              count={count}
              active={statusFilter === status}
              onClick={() => setStatusFilter(status)}
              colorClass={meta.color}
            />
          )
        })}
      </div>

      {/* Task list */}
      {tasks.length === 0 ? (
        <EmptyState
          icon={<Plus className="h-8 w-8" />}
          title={t('tasks.noTasks')}
          description={t('tasks.noTasksDescription')}
          action={
            <Button size="sm" onClick={() => setShowCreate(true)}>
              {t('tasks.createTask')}
            </Button>
          }
        />
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
          {tasks.map((task) => (
            <TaskCard key={task.id} task={task} onShowDetail={() => setDetailTask(task)} />
          ))}
        </div>
      )}

      {/* Detail dialog */}
      <TaskDetailDialog task={detailTask} onClose={() => setDetailTask(null)} />
    </div>
  )
}

// ── Status chip ──

function StatusChip({
  label,
  count,
  active,
  onClick,
  colorClass,
}: {
  label: string
  count: number
  active: boolean
  onClick: () => void
  colorClass?: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex items-center gap-1.5 px-3 py-1.5 rounded-full text-sm whitespace-nowrap transition-colors',
        active
          ? 'bg-primary text-primary-foreground'
          : 'bg-muted text-muted-foreground hover:bg-muted/80',
      )}
    >
      <span className={cn(!active && colorClass)}>{label}</span>
      <span
        className={cn(
          'text-xs px-1.5 py-0.5 rounded-full',
          active ? 'bg-primary-foreground/20' : 'bg-background/50',
        )}
      >
        {count}
      </span>
    </button>
  )
}

// ── Task card ──

function TaskCard({ task, onShowDetail }: { task: Task; onShowDetail: () => void }) {
  const { t } = useTranslation()
  const deleteMutation = useDeleteTask()
  const statusMutation = useUpdateTaskStatus()
  const runMutation = useRunTask()
  const meta = TASK_STATUS_META[task.status]

  const handleRun = () => {
    runMutation.mutate(
      { id: task.id },
      {
        onSuccess: (data) => {
          if (data.success) toast.success(t('tasks.run'))
          else toast.error(t('tasks.runFailed'))
        },
        onError: () => toast.error(t('tasks.runFailed')),
      },
    )
  }
  const handleDelete = () => deleteMutation.mutate(task.id)
  const handleComplete = () => statusMutation.mutate({ id: task.id, status: 'completed' })

  const nextRun = relativeTime(task.nextRunAt)

  return (
    <div className="flex flex-col rounded-xl border bg-card p-4 hover:border-primary/30 hover:shadow-sm transition-all">
      {/* Header */}
      <div className="flex items-start justify-between gap-2 mb-2">
        <button type="button" onClick={onShowDetail} className="min-w-0 flex-1 text-left">
          <h3 className="text-sm font-semibold truncate hover:text-primary transition-colors">
            {task.name}
          </h3>
          <p className="text-xs text-muted-foreground font-mono truncate">{task.identifier}</p>
        </button>
        <span
          className={cn(
            'text-xs px-2 py-0.5 rounded-full font-medium shrink-0',
            meta.bgColor,
            meta.color,
          )}
        >
          {meta.label}
        </span>
      </div>

      {/* Description */}
      {task.description && (
        <p className="text-xs text-muted-foreground line-clamp-2 mb-2">{task.description}</p>
      )}

      {/* Schedule info */}
      {task.automationMode && (
        <div className="flex items-center gap-1 text-xs text-muted-foreground mb-2">
          <Clock className="h-3 w-3" />
          <span>
            {task.automationMode === 'schedule'
              ? (task.schedulePattern ?? 'cron')
              : t('tasks.every', { secs: task.heartbeatIntervalSecs ?? 0 })}
          </span>
          {nextRun && (
            <span className="text-muted-foreground/70 ml-auto flex items-center gap-0.5">
              <CalendarClock className="h-3 w-3" />
              {nextRun}
            </span>
          )}
          {task.executionCount > 0 && !nextRun && (
            <span className="text-muted-foreground/60 ml-auto">
              {t('tasks.runs', { count: task.executionCount })}
            </span>
          )}
        </div>
      )}

      {/* Actions */}
      <div className="flex items-center gap-1 mt-auto pt-2">
        {task.status !== 'completed' && task.status !== 'running' && (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 text-xs gap-1"
            onClick={handleRun}
            disabled={runMutation.isPending}
          >
            <Play className="h-3 w-3" />
            {runMutation.isPending ? t('tasks.statusRunning') : t('tasks.run')}
          </Button>
        )}
        {task.status === 'running' && (
          <Button size="sm" variant="ghost" className="h-7 text-xs gap-1" onClick={handleComplete}>
            {t('tasks.complete')}
          </Button>
        )}
        <Button size="sm" variant="ghost" className="h-7 text-xs gap-1" onClick={onShowDetail}>
          <History className="h-3 w-3" />
          {t('tasks.details')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 text-xs text-muted-foreground hover:text-destructive ml-auto"
          onClick={handleDelete}
          disabled={deleteMutation.isPending}
        >
          <Trash2 className="h-3 w-3" />
        </Button>
      </div>
    </div>
  )
}

// ── Automation mode toggle ──

function AutomationToggle({
  value,
  onChange,
}: {
  value: TaskAutomationMode | 'none'
  onChange: (v: TaskAutomationMode | 'none') => void
}) {
  const { t } = useTranslation()
  const options: { value: TaskAutomationMode | 'none'; label: string }[] = [
    { value: 'none', label: t('tasks.automationNone') },
    { value: 'schedule', label: t('tasks.automationSchedule') },
    { value: 'heartbeat', label: t('tasks.automationHeartbeat') },
  ]
  return (
    <div className="flex gap-1 p-1 rounded-lg bg-muted">
      {options.map((opt) => (
        <button
          key={opt.value}
          type="button"
          onClick={() => onChange(opt.value)}
          className={cn(
            'flex-1 px-2 py-1.5 rounded-md text-xs font-medium transition-colors',
            value === opt.value
              ? 'bg-background text-foreground shadow-sm'
              : 'text-muted-foreground hover:text-foreground',
          )}
        >
          {opt.label}
        </button>
      ))}
    </div>
  )
}

// ── Schedule config section (shared by create + edit) ──

function ScheduleConfig({
  mode,
  setMode,
  schedule,
  setSchedule,
  intervalMin,
  setIntervalMin,
}: {
  mode: TaskAutomationMode | 'none'
  setMode: (v: TaskAutomationMode | 'none') => void
  schedule: string
  setSchedule: (v: string) => void
  intervalMin: number
  setIntervalMin: (v: number) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="space-y-3">
      <div>
        <Label className="text-sm font-medium mb-1.5 block">{t('tasks.scheduleLabel')}</Label>
        <AutomationToggle value={mode} onChange={setMode} />
      </div>
      {mode === 'schedule' && (
        <div className="rounded-lg border p-3">
          <CronScheduleEditor value={schedule} onChange={setSchedule} />
        </div>
      )}
      {mode === 'heartbeat' && (
        <div>
          <Label className="text-sm font-medium mb-1 block">{t('tasks.intervalMinutes')}</Label>
          <Input
            type="number"
            min={1}
            value={intervalMin}
            onChange={(e) => setIntervalMin(Math.max(1, Number(e.target.value)))}
            className="w-32"
          />
        </div>
      )}
    </div>
  )
}

// ── Create dialog ──

function CreateTaskDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const createMutation = useCreateTask()
  const scheduleMutation = useSetTaskSchedule()
  const [name, setName] = useState('')
  const [instruction, setInstruction] = useState('')
  const [description, setDescription] = useState('')
  const [mode, setMode] = useState<TaskAutomationMode | 'none'>('none')
  const [schedule, setSchedule] = useState(DEFAULT_CRON)
  const [intervalMin, setIntervalMin] = useState(30)

  const handleSubmit = () => {
    if (!name.trim() || !instruction.trim()) return
    createMutation.mutate(
      {
        name: name.trim(),
        instruction: instruction.trim(),
        description: description.trim() || undefined,
      },
      {
        onSuccess: (task) => {
          // If a schedule was configured, apply it after creation.
          if (mode !== 'none' && task?.id) {
            scheduleMutation.mutate({
              id: task.id,
              automationMode: mode,
              schedulePattern: mode === 'schedule' ? schedule : null,
              heartbeatIntervalSecs: mode === 'heartbeat' ? intervalMin * 60 : undefined,
            })
          }
          onClose()
        },
      },
    )
  }

  return (
    <DialogContent className="sm:max-w-lg max-h-[85vh] overflow-y-auto">
      <DialogHeader>
        <DialogTitle>{t('tasks.createDialogTitle')}</DialogTitle>
      </DialogHeader>
      <div className="space-y-3">
        <div>
          <Label className="text-sm font-medium mb-1 block">{t('tasks.nameLabel')}</Label>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t('tasks.namePlaceholder')}
          />
        </div>
        <div>
          <Label className="text-sm font-medium mb-1 block">{t('tasks.descriptionLabel')}</Label>
          <Input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder={t('tasks.descriptionPlaceholder')}
          />
        </div>
        <div>
          <Label className="text-sm font-medium mb-1 block">{t('tasks.instructionLabel')}</Label>
          <Textarea
            value={instruction}
            onChange={(e) => setInstruction(e.target.value)}
            placeholder={t('tasks.instructionPlaceholder')}
            rows={4}
          />
        </div>
        <ScheduleConfig
          mode={mode}
          setMode={setMode}
          schedule={schedule}
          setSchedule={setSchedule}
          intervalMin={intervalMin}
          setIntervalMin={setIntervalMin}
        />
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t('tasks.cancel')}
          </Button>
          <Button
            size="sm"
            onClick={handleSubmit}
            disabled={!name.trim() || !instruction.trim() || createMutation.isPending}
          >
            {createMutation.isPending ? t('tasks.creating') : t('tasks.createTask')}
          </Button>
        </div>
      </div>
    </DialogContent>
  )
}

// ── Detail dialog (instruction + schedule + run history) ──

function TaskDetailDialog({ task, onClose }: { task: Task | null; onClose: () => void }) {
  const { t } = useTranslation()
  const scheduleMutation = useSetTaskSchedule()
  const runMutation = useRunTask()
  const [mode, setMode] = useState<TaskAutomationMode | 'none'>('none')
  const [schedule, setSchedule] = useState(DEFAULT_CRON)
  const [intervalMin, setIntervalMin] = useState(30)

  return (
    <Dialog open={!!task} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-2xl max-h-[85vh] overflow-y-auto">
        {task && (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                {task.name}
                <Badge variant="secondary" className={cn(TASK_STATUS_META[task.status].color)}>
                  {TASK_STATUS_META[task.status].label}
                </Badge>
              </DialogTitle>
            </DialogHeader>
            <div className="space-y-4">
              {/* Instruction */}
              {task.description && (
                <p className="text-sm text-muted-foreground">{task.description}</p>
              )}
              <div>
                <Label className="text-xs font-medium mb-1 block text-muted-foreground">
                  {t('tasks.instructionLabel')}
                </Label>
                <pre className="text-xs whitespace-pre-wrap font-mono bg-muted/50 rounded-lg p-3 max-h-48 overflow-y-auto">
                  {task.instruction}
                </pre>
              </div>

              {/* Schedule config */}
              <ScheduleConfig
                mode={mode === 'none' ? (task.automationMode ?? 'none') : mode}
                setMode={setMode}
                schedule={schedule}
                setSchedule={setSchedule}
                intervalMin={intervalMin}
                setIntervalMin={setIntervalMin}
              />
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={scheduleMutation.isPending}
                  onClick={() => {
                    if (!task) return
                    scheduleMutation.mutate(
                      {
                        id: task.id,
                        automationMode: mode === 'none' ? null : mode,
                        schedulePattern: mode === 'schedule' ? schedule : null,
                        heartbeatIntervalSecs: mode === 'heartbeat' ? intervalMin * 60 : undefined,
                      },
                      { onSuccess: () => toast.success(t('tasks.scheduleSaved')) },
                    )
                  }}
                >
                  <CalendarClock className="h-3.5 w-3.5" />
                  {t('tasks.scheduleLabel')}
                </Button>
                <Button
                  size="sm"
                  disabled={runMutation.isPending || task.status === 'running'}
                  onClick={() =>
                    runMutation.mutate(
                      { id: task.id },
                      {
                        onSuccess: (data) =>
                          data.success
                            ? toast.success(t('tasks.run'))
                            : toast.error(t('tasks.runFailed')),
                      },
                    )
                  }
                >
                  <Play className="h-3.5 w-3.5" />
                  {runMutation.isPending ? t('tasks.statusRunning') : t('tasks.run')}
                </Button>
              </div>

              {/* Run history */}
              <RunHistory taskId={task.id} />
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

// ── Run history ──

function RunHistory({ taskId }: { taskId: string }) {
  const { t } = useTranslation()
  const { data, isLoading } = useTaskRuns(taskId)
  const runs = data?.runs ?? []

  return (
    <div>
      <div className="flex items-center gap-1.5 mb-2">
        <History className="h-3.5 w-3.5 text-muted-foreground" />
        <Label className="text-xs font-medium text-muted-foreground">{t('tasks.history')}</Label>
      </div>
      {isLoading ? (
        <div className="text-xs text-muted-foreground">…</div>
      ) : runs.length === 0 ? (
        <p className="text-xs text-muted-foreground/60">{t('tasks.noRuns')}</p>
      ) : (
        <div className="space-y-1.5 max-h-48 overflow-y-auto">
          {runs.map((run) => {
            const ok = run.status === 'completed'
            return (
              <div key={run.id} className="flex items-start gap-2 rounded-lg border p-2 text-xs">
                <span
                  className={cn(
                    'mt-0.5 h-1.5 w-1.5 rounded-full shrink-0',
                    ok
                      ? 'bg-status-success'
                      : run.status === 'running'
                        ? 'bg-status-warning'
                        : 'bg-status-error',
                  )}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="font-medium capitalize">{run.trigger}</span>
                    <span className="text-muted-foreground/60">{relativeTime(run.startedAt)}</span>
                  </div>
                  {(run.summary || run.error) && (
                    <p
                      className={cn(
                        'mt-0.5 line-clamp-2',
                        run.error ? 'text-status-error-on-surface' : 'text-muted-foreground',
                      )}
                    >
                      {run.error ?? run.summary}
                    </p>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
