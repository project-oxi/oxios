// Task page — list + create + schedule + run + detail (RFC-043)

import { useMutation } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import {
  CalendarClock,
  Clock,
  GanttChartSquare,
  History,
  LayoutGrid,
  Pencil,
  Play,
  Plus,
  ShieldCheck,
  Sparkles,
  Trash2,
} from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { CronScheduleEditor } from '@/components/cron/cron-schedule-editor'
import { CronTimelineView } from '@/components/cron/cron-timeline-view'
import { TaskTemplateGallery } from '@/components/cron/task-template-gallery'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { TaskComments } from '@/components/task/task-comments'
import { TaskDependencies } from '@/components/task/task-dependencies'

import { TaskVerifyConfig } from '@/components/task/task-verify-config'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  useCreateTask,
  useDeleteTask,
  useRunTask,
  useSetTaskSchedule,
  useSetTaskVerify,
  useTaskRuns,
  useTasks,
  useUpdateTask,
  useUpdateTaskStatus,
} from '@/hooks/use-tasks'
import { api } from '@/lib/api-client'
import { DEFAULT_CRON } from '@/lib/cron-utils'
import { cn } from '@/lib/utils'
import {
  TASK_STATUS_META,
  TASK_STATUSES,
  type Task,
  type TaskAutomationMode,
  type TaskStatus,
} from '@/types/task'
import type { TaskTemplate } from '@/types/task-templates'

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
  const [editTask, setEditTask] = useState<Task | null>(null)
  const [view, setView] = useState<'list' | 'timeline'>('list')
  const [createInitial, setCreateInitial] = useState<TaskFormInitial | null>(null)

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const allTasks = data?.tasks ?? []
  const tasks =
    statusFilter === 'all' ? allTasks : allTasks.filter((tk) => tk.status === statusFilter)
  const scheduled = allTasks.filter((tk) => tk.automationMode === 'schedule')

  const handleTemplate = (tmpl: TaskTemplate) => {
    setCreateInitial({
      name: tmpl.title,
      description: tmpl.description,
      instruction: tmpl.instruction,
      schedule: tmpl.cronPattern,
    })
    setShowCreate(true)
  }

  const closeForm = () => {
    setShowCreate(false)
    setEditTask(null)
    setCreateInitial(null)
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('tasks.title')}
        subtitle={t('tasks.subtitle')}
        actions={
          <div className="flex items-center gap-2">
            {/* View toggle — Gantt timeline of scheduled tasks */}
            <div className="inline-flex h-8 items-center rounded-lg border bg-muted/50 p-0.5 text-muted-foreground">
              <button
                type="button"
                onClick={() => setView('list')}
                aria-pressed={view === 'list'}
                className={cn(
                  'inline-flex h-7 items-center gap-1 rounded-md px-2.5 text-xs font-medium transition-colors',
                  view === 'list'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'hover:text-foreground',
                )}
              >
                <LayoutGrid className="h-3.5 w-3.5" />
                {t('tasks.viewList')}
              </button>
              <button
                type="button"
                onClick={() => setView('timeline')}
                aria-pressed={view === 'timeline'}
                className={cn(
                  'inline-flex h-7 items-center gap-1 rounded-md px-2.5 text-xs font-medium transition-colors',
                  view === 'timeline'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'hover:text-foreground',
                )}
              >
                <GanttChartSquare className="h-3.5 w-3.5" />
                {t('tasks.viewTimeline')}
              </button>
            </div>
            <Button size="sm" className="gap-1.5" onClick={() => setShowCreate(true)}>
              <Plus className="h-3.5 w-3.5" />
              {t('tasks.newTask')}
            </Button>
          </div>
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

      {view === 'timeline' ? (
        <CronTimelineView
          jobs={scheduled.map((tk) => ({
            id: tk.id,
            name: tk.name,
            schedule: tk.schedulePattern ?? '',
            next_run: tk.nextRunAt ?? undefined,
          }))}
          onEdit={(j) => setEditTask(allTasks.find((tk) => tk.id === j.id) ?? null)}
        />
      ) : tasks.length === 0 && allTasks.length === 0 ? (
        /* First-time empty state: template gallery + blank create */
        <div className="space-y-6">
          <EmptyState
            icon={<Plus className="h-8 w-8" />}
            title={t('tasks.noTasks')}
            description={t('tasks.noTasksDescription')}
            action={
              <div className="flex gap-2">
                <Button size="sm" variant="outline" onClick={() => setShowCreate(true)}>
                  {t('tasks.createBlank')}
                </Button>
              </div>
            }
          />
          <div>
            <h2 className="text-sm font-semibold text-muted-foreground mb-3">
              {t('tasks.fromTemplate')}
            </h2>
            <TaskTemplateGallery onSelectTemplate={handleTemplate} />
          </div>
        </div>
      ) : tasks.length === 0 ? (
        <EmptyState
          icon={<Plus className="h-8 w-8" />}
          title={t('tasks.noTasks')}
          description={t('tasks.noTasksDescription')}
        />
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
          {tasks.map((task) => (
            <TaskCard
              key={task.id}
              task={task}
              onShowDetail={() => setDetailTask(task)}
              onEdit={() => setEditTask(task)}
            />
          ))}
        </div>
      )}

      {/* Detail dialog */}
      <TaskDetailDialog task={detailTask} onClose={() => setDetailTask(null)} />
      {/* Unified create/edit form */}
      <TaskFormDialog
        open={showCreate || !!editTask}
        editTask={editTask}
        initial={createInitial}
        onClose={closeForm}
      />
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

function TaskCard({
  task,
  onShowDetail,
  onEdit,
}: {
  task: Task
  onShowDetail: () => void
  onEdit: () => void
}) {
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
        <Button size="sm" variant="ghost" className="h-7 text-xs gap-1" onClick={onShowDetail}>
          <History className="h-3 w-3" />
          {t('tasks.details')}
        </Button>
        <Button size="sm" variant="ghost" className="h-7 text-xs gap-1" onClick={onEdit}>
          <Pencil className="h-3 w-3" />
          {t('common.edit')}
        </Button>
        {task.status === 'running' && (
          <Button size="sm" variant="ghost" className="h-7 text-xs gap-1" onClick={handleComplete}>
            {t('tasks.complete')}
          </Button>
        )}
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

// ── Unified create/edit form dialog ──

interface TaskFormInitial {
  name?: string
  description?: string
  instruction?: string
  schedule?: string
}

/** Draft goal candidates via the engine sidecar (degrades to none on error). */
function useGoalSuggestions() {
  return useMutation({
    mutationFn: async (context: string) => {
      const r = await api.post<{ goals: string[] }>('/api/engine/goal-suggestions', {
        content: context,
      })
      return r?.goals ?? []
    },
  })
}

function TaskFormDialog({
  open,
  editTask,
  initial,
  onClose,
}: {
  open: boolean
  editTask: Task | null
  initial: TaskFormInitial | null
  onClose: () => void
}) {
  const { t } = useTranslation()
  const createMutation = useCreateTask()
  const updateMutation = useUpdateTask()
  const scheduleMutation = useSetTaskSchedule()
  const goalSuggestions = useGoalSuggestions()

  const [name, setName] = useState('')
  const [instruction, setInstruction] = useState('')
  const [description, setDescription] = useState('')
  const [mode, setMode] = useState<TaskAutomationMode | 'none'>('none')
  const [schedule, setSchedule] = useState(DEFAULT_CRON)
  const [intervalMin, setIntervalMin] = useState(30)
  const [suggestions, setSuggestions] = useState<string[]>([])

  // Prefill on open: edit → existing task fields; create → template initial.
  useEffect(() => {
    if (!open) return
    setSuggestions([])
    if (editTask) {
      setName(editTask.name)
      setInstruction(editTask.instruction)
      setDescription(editTask.description ?? '')
      setMode(editTask.automationMode ?? 'none')
      setSchedule(editTask.schedulePattern ?? DEFAULT_CRON)
      setIntervalMin(Math.max(1, Math.round((editTask.heartbeatIntervalSecs ?? 1800) / 60)))
    } else if (initial) {
      setName(initial.name ?? '')
      setInstruction(initial.instruction ?? '')
      setDescription(initial.description ?? '')
      setMode(initial.schedule ? 'schedule' : 'none')
      setSchedule(initial.schedule ?? DEFAULT_CRON)
    } else {
      setName('')
      setInstruction('')
      setDescription('')
      setMode('none')
      setSchedule(DEFAULT_CRON)
    }
  }, [open, editTask, initial])

  const isEdit = !!editTask
  const pending = createMutation.isPending || updateMutation.isPending

  const runAiAssist = () => {
    const context = [name.trim(), description.trim(), instruction.trim()].filter(Boolean).join('\n')
    if (!context) return
    goalSuggestions.mutate(context, {
      onSuccess: (goals) => {
        setSuggestions(goals)
        if (goals.length === 0) toast.info(t('tasks.aiNoSuggestions'))
      },
      onError: () => toast.error(t('tasks.aiFailed')),
    })
  }

  const handleSubmit = () => {
    if (!name.trim() || !instruction.trim()) return
    if (isEdit && editTask) {
      updateMutation.mutate(
        {
          id: editTask.id,
          name: name.trim(),
          instruction: instruction.trim(),
          description: description.trim() ? description.trim() : undefined,
        },
        {
          onSuccess: () => {
            const modeChanged =
              (editTask.automationMode ?? 'none') !== mode ||
              (mode === 'schedule' && editTask.schedulePattern !== schedule)
            if (mode !== 'none' || modeChanged) {
              scheduleMutation.mutate({
                id: editTask.id,
                automationMode: mode === 'none' ? null : mode,
                schedulePattern: mode === 'schedule' ? schedule : null,
                heartbeatIntervalSecs: mode === 'heartbeat' ? intervalMin * 60 : undefined,
              })
            }
            onClose()
          },
        },
      )
      return
    }
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
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {isEdit ? t('tasks.editDialogTitle') : t('tasks.createDialogTitle')}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div>
            <Label className="text-sm font-medium mb-1 block">{t('tasks.nameLabel')}</Label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('tasks.namePlaceholder')}
              autoFocus
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
            <div className="flex items-center justify-between mb-1">
              <Label className="text-sm font-medium">{t('tasks.instructionLabel')}</Label>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-6 gap-1 px-2 text-xs text-primary"
                onClick={runAiAssist}
                disabled={goalSuggestions.isPending || !instruction.trim()}
                title={t('tasks.aiAssistHint')}
              >
                <Sparkles className="h-3 w-3" />
                {goalSuggestions.isPending ? t('tasks.aiThinking') : t('tasks.aiAssist')}
              </Button>
            </div>
            <Textarea
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              placeholder={t('tasks.instructionPlaceholder')}
              rows={5}
            />
            {suggestions.length > 0 && (
              <div className="mt-2 space-y-1.5">
                <p className="text-xs text-muted-foreground">{t('tasks.aiPickOne')}</p>
                {suggestions.map((goal, i) => (
                  <button
                    key={i}
                    type="button"
                    onClick={() => {
                      setInstruction(goal)
                      setSuggestions([])
                    }}
                    className="w-full rounded-lg border border-primary/25 bg-primary/5 p-2.5 text-left text-xs leading-relaxed hover:border-primary/50 hover:bg-primary/10 transition-colors"
                  >
                    {goal}
                  </button>
                ))}
              </div>
            )}
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
              disabled={!name.trim() || !instruction.trim() || pending}
            >
              {pending ? t('tasks.creating') : isEdit ? t('common.save') : t('tasks.createTask')}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

// ── Detail dialog (instruction + schedule + run history) ──

function TaskDetailDialog({ task, onClose }: { task: Task | null; onClose: () => void }) {
  const { t } = useTranslation()
  const scheduleMutation = useSetTaskSchedule()
  const verifyMutation = useSetTaskVerify()
  const runMutation = useRunTask()
  const [mode, setMode] = useState<TaskAutomationMode | 'none'>('none')
  const [schedule, setSchedule] = useState(DEFAULT_CRON)
  const [intervalMin, setIntervalMin] = useState(30)

  return (
    <Dialog open={!!task} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-3xl max-h-[85vh] overflow-y-auto">
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

              {/* Verify gate */}
              <TaskVerifyConfig
                enabled={task.verifyEnabled}
                requirement={task.verifyRequirement}
                maxIterations={task.verifyMaxIterations}
                isPending={verifyMutation.isPending}
                onSave={(params) =>
                  verifyMutation.mutate(
                    { id: task.id, ...params },
                    {
                      onSuccess: () => toast.success(t('tasks.verifySaved')),
                      onError: () => toast.error(t('tasks.verifySaveFailed')),
                    },
                  )
                }
              />

              {/* Comments */}
              <TaskComments taskId={task.id} />

              {/* Dependencies */}
              <TaskDependencies task={task} />

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
            const verified = run.status === 'verified'
            const ok = verified || run.status === 'completed'
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
                    {verified && (
                      <Badge variant="success" className="gap-1 px-1.5 py-0 text-2xs">
                        <ShieldCheck className="h-2.5 w-2.5" />
                        {t('tasks.runVerified')}
                      </Badge>
                    )}
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
