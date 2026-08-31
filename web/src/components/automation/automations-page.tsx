// AutomationsPage — list + create + edit + detail + run for the Automation
// domain. Rendered standalone on this branch (WT-1): owns its own chrome
// (page header, status filter, list/grid, dialogs) until WT-3 embeds it in
// the Studio shell. Kept out of the route file so tests can mount it under
// a QueryClient without the router plugin's code-splitting rewriting the
// module's exports.

import { useMutation } from '@tanstack/react-query'
import { CalendarClock, Clock, History, LayoutGrid, Pencil, Play, Plus, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  AutomationBrainPicker,
  useAutomationBrainSpaces,
} from '@/components/automation/automation-brain-picker'
import {
  AutomationPersonaPicker,
  useAutomationPersonaRoster,
} from '@/components/automation/automation-persona-picker'
import {
  AutomationProjectPicker,
  useAutomationProjectRoster,
} from '@/components/automation/automation-project-picker'
import { AutomationScheduleTimeline } from '@/components/automation/automation-schedule-timeline'
import { AutomationTemplateGallery } from '@/components/automation/automation-template-gallery'
import { AutomationTriggerEditor } from '@/components/automation/automation-trigger-editor'
import { AutomationVerifyConfig } from '@/components/automation/automation-verify-config'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  useAutomationRuns,
  useAutomations,
  useCreateAutomation,
  useDeleteAutomation,
  useRunAutomation,
  useSetAutomationTrigger,
  useSetAutomationVerify,
  useUpdateAutomation,
  useUpdateAutomationStatus,
} from '@/hooks/use-automations'
import { api } from '@/lib/api-client'
import { DEFAULT_CRON } from '@/lib/cron-utils'
import { cn } from '@/lib/utils'
import {
  AUTOMATION_STATUS_META,
  AUTOMATION_STATUSES,
  type Automation,
  type AutomationRun,
  type AutomationStatus,
} from '@/types/automation'
import type { AutomationTemplate } from '@/types/automation-templates'

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

// Exported for component-level tests and the thin route registration in
// routes/studio/automations.tsx.
export function AutomationsPage() {
  const { t } = useTranslation()
  const { data, isLoading, isError, refetch } = useAutomations()
  const [showCreate, setShowCreate] = useState(false)
  const [statusFilter, setStatusFilter] = useState<AutomationStatus | 'all'>('all')
  const [detailAutomation, setDetailAutomation] = useState<Automation | null>(null)
  const [editAutomation, setEditAutomation] = useState<Automation | null>(null)
  const [view, setView] = useState<'list' | 'timeline'>('list')
  const [createInitial, setCreateInitial] = useState<AutomationFormInitial | null>(null)

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const allAutomations = data?.automations ?? []
  const automations =
    statusFilter === 'all'
      ? allAutomations
      : allAutomations.filter((a) => a.status === statusFilter)
  const scheduled = allAutomations.filter((a) => a.trigger !== 'manual' && a.status === 'active')

  const handleTemplate = (tmpl: AutomationTemplate) => {
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
    setEditAutomation(null)
    setCreateInitial(null)
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('automations.title')}
        subtitle={t('automations.subtitle')}
        actions={
          <div className="flex items-center gap-2">
            <div className="inline-flex h-8 items-center rounded-lg border bg-muted/50 p-0.5 text-muted-foreground">
              <button
                type="button"
                onClick={() => setView('list')}
                aria-pressed={view === 'list'}
                data-testid="automations-view-list"
                className={cn(
                  'inline-flex h-7 items-center gap-1 rounded-md px-2.5 text-xs font-medium transition-colors',
                  view === 'list'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'hover:text-foreground',
                )}
              >
                <LayoutGrid className="h-3.5 w-3.5" />
                {t('automations.viewList')}
              </button>
              <button
                type="button"
                onClick={() => setView('timeline')}
                aria-pressed={view === 'timeline'}
                data-testid="automations-view-timeline"
                className={cn(
                  'inline-flex h-7 items-center gap-1 rounded-md px-2.5 text-xs font-medium transition-colors',
                  view === 'timeline'
                    ? 'bg-background text-foreground shadow-sm'
                    : 'hover:text-foreground',
                )}
              >
                <CalendarClock className="h-3.5 w-3.5" />
                {t('automations.viewTimeline')}
              </button>
            </div>
            <Button
              size="sm"
              className="gap-1.5"
              onClick={() => setShowCreate(true)}
              data-testid="automations-new"
            >
              <Plus className="h-3.5 w-3.5" />
              {t('automations.newAutomation')}
            </Button>
          </div>
        }
      />

      {/* Status filter chips */}
      <div className="flex items-center gap-1 overflow-x-auto pb-1">
        <StatusChip
          label={t('automations.all')}
          count={allAutomations.length}
          active={statusFilter === 'all'}
          onClick={() => setStatusFilter('all')}
        />
        {AUTOMATION_STATUSES.map((status) => {
          const count = allAutomations.filter((a) => a.status === status).length
          if (count === 0) return null
          const meta = AUTOMATION_STATUS_META[status]
          return (
            <StatusChip
              key={status}
              label={t(meta.label)}
              count={count}
              active={statusFilter === status}
              onClick={() => setStatusFilter(status)}
              colorClass={meta.color}
            />
          )
        })}
      </div>

      {view === 'timeline' ? (
        <AutomationScheduleTimeline
          jobs={scheduled.map((a) => ({
            id: a.id,
            name: a.name,
            schedule:
              a.cronPattern ?? (a.heartbeatIntervalSecs ? `every ${a.heartbeatIntervalSecs}s` : ''),
            next_run: a.nextRunAt ?? undefined,
            enabled: a.status === 'active',
          }))}
          onEdit={(j) => setEditAutomation(allAutomations.find((a) => a.id === j.id) ?? null)}
        />
      ) : automations.length === 0 && allAutomations.length === 0 ? (
        <div className="space-y-6">
          <EmptyState
            icon={<Plus className="h-8 w-8" />}
            title={t('automations.noAutomations')}
            description={t('automations.noAutomationsDescription')}
            action={
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setShowCreate(true)}
                  data-testid="automations-create-blank"
                >
                  {t('automations.createBlank')}
                </Button>
              </div>
            }
          />
          <div>
            <h2 className="text-sm font-semibold text-muted-foreground mb-3">
              {t('automations.fromTemplate')}
            </h2>
            <AutomationTemplateGallery onSelectTemplate={handleTemplate} />
          </div>
        </div>
      ) : automations.length === 0 ? (
        <EmptyState
          icon={<Plus className="h-8 w-8" />}
          title={t('automations.noAutomations')}
          description={t('automations.noAutomationsDescription')}
        />
      ) : (
        <div
          className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3"
          data-testid="automations-grid"
        >
          {automations.map((automation) => (
            <AutomationCard
              key={automation.id}
              automation={automation}
              onShowDetail={() => setDetailAutomation(automation)}
              onEdit={() => setEditAutomation(automation)}
            />
          ))}
        </div>
      )}

      {/* Detail dialog */}
      <AutomationDetailDialog
        automation={detailAutomation}
        onClose={() => setDetailAutomation(null)}
      />
      {/* Unified create/edit form */}
      <AutomationFormDialog
        open={showCreate || !!editAutomation}
        editAutomation={editAutomation}
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

// ── Automation card ──

function AutomationCard({
  automation,
  onShowDetail,
  onEdit,
}: {
  automation: Automation
  onShowDetail: () => void
  onEdit: () => void
}) {
  const { t } = useTranslation()
  const deleteMutation = useDeleteAutomation()
  const statusMutation = useUpdateAutomationStatus()
  const runMutation = useRunAutomation()
  const meta = AUTOMATION_STATUS_META[automation.status]

  const handleRun = () => {
    runMutation.mutate(
      { id: automation.id },
      {
        onSuccess: (data) => {
          if (data.success) toast.success(t('automations.run'))
          else toast.error(t('automations.runFailed'))
        },
        onError: () => toast.error(t('automations.runFailed')),
      },
    )
  }
  const handleDelete = () => deleteMutation.mutate(automation.id)
  const handlePause = () => statusMutation.mutate({ id: automation.id, status: 'paused' })
  const handleResume = () => statusMutation.mutate({ id: automation.id, status: 'active' })

  const nextRun = relativeTime(automation.nextRunAt)

  return (
    <div
      className="flex flex-col rounded-xl border bg-card p-4 hover:border-primary/30 hover:shadow-sm transition-all"
      data-testid={`automation-card-${automation.id}`}
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-2 mb-2">
        <button type="button" onClick={onShowDetail} className="min-w-0 flex-1 text-left">
          <h3 className="text-sm font-semibold truncate hover:text-primary transition-colors">
            {automation.name}
          </h3>
          <p className="text-xs text-muted-foreground truncate">
            {automation.trigger === 'cron' && automation.cronPattern
              ? automation.cronPattern
              : automation.trigger === 'heartbeat' && automation.heartbeatIntervalSecs
                ? t('automations.every', { secs: automation.heartbeatIntervalSecs })
                : t('automations.manualTrigger')}
          </p>
        </button>
        <span
          className={cn(
            'text-xs px-2 py-0.5 rounded-full font-medium shrink-0',
            meta.bgColor,
            meta.color,
          )}
        >
          {t(meta.label)}
        </span>
      </div>

      {/* Description */}
      {automation.description && (
        <p className="text-xs text-muted-foreground line-clamp-2 mb-2">{automation.description}</p>
      )}

      {/* Bindings */}
      <div className="flex flex-wrap items-center gap-1 text-2xs text-muted-foreground mb-2">
        {automation.trigger !== 'manual' && (
          <span className="inline-flex items-center gap-0.5">
            <Clock className="h-3 w-3" />
            {nextRun && (
              <span className="ml-auto flex items-center gap-0.5">
                <CalendarClock className="h-3 w-3" />
                {nextRun}
              </span>
            )}
            {automation.executionCount > 0 && !nextRun && (
              <span className="ml-auto">
                {t('automations.runs', { count: automation.executionCount })}
              </span>
            )}
          </span>
        )}
      </div>

      {/* Actions */}
      <div className="flex items-center gap-1 mt-auto pt-2">
        <Button
          size="sm"
          variant="ghost"
          className="h-7 text-xs gap-1"
          onClick={handleRun}
          disabled={runMutation.isPending}
          data-testid={`automation-run-${automation.id}`}
        >
          <Play className="h-3 w-3" />
          {runMutation.isPending ? t('automations.statusRunning') : t('automations.run')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 text-xs gap-1"
          onClick={onShowDetail}
          data-testid={`automation-detail-${automation.id}`}
        >
          <History className="h-3 w-3" />
          {t('automations.details')}
        </Button>
        <Button size="sm" variant="ghost" className="h-7 text-xs gap-1" onClick={onEdit}>
          <Pencil className="h-3 w-3" />
          {t('common.edit')}
        </Button>
        {automation.status === 'active' ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 text-xs gap-1"
            onClick={handlePause}
            disabled={statusMutation.isPending}
            data-testid={`automation-pause-${automation.id}`}
          >
            {t('automations.pause')}
          </Button>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            className="h-7 text-xs gap-1"
            onClick={handleResume}
            disabled={statusMutation.isPending}
            data-testid={`automation-resume-${automation.id}`}
          >
            {t('automations.resume')}
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

// ── Automation create/edit form ──

interface AutomationFormInitial {
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

function AutomationFormDialog({
  open,
  editAutomation,
  initial,
  onClose,
}: {
  open: boolean
  editAutomation: Automation | null
  initial: AutomationFormInitial | null
  onClose: () => void
}) {
  const { t } = useTranslation()
  const createMutation = useCreateAutomation()
  const updateMutation = useUpdateAutomation()
  const triggerMutation = useSetAutomationTrigger()
  const goalSuggestions = useGoalSuggestions()

  const projectRoster = useAutomationProjectRoster()
  const personaRoster = useAutomationPersonaRoster()
  const brainRoster = useAutomationBrainSpaces()

  const [name, setName] = useState('')
  const [instruction, setInstruction] = useState('')
  const [description, setDescription] = useState('')
  // Trigger config as loaded from the definition. If the user did not touch
  // any trigger field, the save must NOT send PUT /:id/trigger — the server
  // keeps status/next_run coherent, and an unconditional re-send would
  // un-pause a paused automation's schedule (review finding F2).
  const loadedTriggerRef = useRef<{
    trigger: Automation['trigger']
    cronPattern: string
    heartbeatIntervalSecs: number
  } | null>(null)
  const [trigger, setTrigger] = useState<Automation['trigger']>('manual')
  const [cronPattern, setCronPattern] = useState<string>(DEFAULT_CRON)
  const [heartbeatIntervalSecs, setHeartbeatIntervalSecs] = useState<number>(1800)
  const [projectId, setProjectId] = useState<string | null>(null)
  const [personaId, setPersonaId] = useState<string | null>(null)
  const [brainSpace, setBrainSpace] = useState<string | null>(null)
  const [suggestions, setSuggestions] = useState<string[]>([])

  // Prefill on open: edit → existing fields; create → template initial.
  useEffect(() => {
    // The dialog stays mounted for the page lifetime, so stale AI suggestion
    // chips from a previous session must be dropped when it is closed or
    // (re)opened — a stale chip would overwrite the fresh prefill.
    if (!open) return
    setSuggestions([])
    if (editAutomation) {
      setName(editAutomation.name)
      setInstruction(editAutomation.instruction)
      setDescription(editAutomation.description ?? '')
      setTrigger(editAutomation.trigger)
      setCronPattern(editAutomation.cronPattern ?? DEFAULT_CRON)
      setHeartbeatIntervalSecs(editAutomation.heartbeatIntervalSecs ?? 1800)
      setProjectId(editAutomation.projectId ?? null)
      setPersonaId(editAutomation.personaId ?? null)
      setBrainSpace(editAutomation.brainSpace ?? null)
      loadedTriggerRef.current = {
        trigger: editAutomation.trigger,
        cronPattern: editAutomation.cronPattern ?? DEFAULT_CRON,
        heartbeatIntervalSecs: editAutomation.heartbeatIntervalSecs ?? 1800,
      }
    } else if (initial) {
      setName(initial.name ?? '')
      setInstruction(initial.instruction ?? '')
      setDescription(initial.description ?? '')
      setTrigger(initial.schedule ? 'cron' : 'manual')
      setCronPattern(initial.schedule ?? DEFAULT_CRON)
      setProjectId(null)
      setPersonaId(null)
      setBrainSpace(null)
      loadedTriggerRef.current = null
      setHeartbeatIntervalSecs(1800)
    } else {
      setName('')
      setInstruction('')
      setDescription('')
      setTrigger('manual')
      setCronPattern(DEFAULT_CRON)
      setHeartbeatIntervalSecs(1800)
      setProjectId(null)
      setPersonaId(null)
      setBrainSpace(null)
    }
  }, [open, editAutomation, initial])

  const isEdit = !!editAutomation
  const pending = createMutation.isPending || updateMutation.isPending || triggerMutation.isPending

  const runAiAssist = () => {
    const context = [name.trim(), description.trim(), instruction.trim()].filter(Boolean).join('\n')
    if (!context) return
    goalSuggestions.mutate(context, {
      onSuccess: (goals) => {
        setSuggestions(goals)
        if (goals.length === 0) toast.info(t('automations.aiNoSuggestions'))
      },
      onError: () => toast.error(t('automations.aiFailed')),
    })
  }

  const handleSubmit = () => {
    if (!name.trim() || !instruction.trim()) return

    if (isEdit && editAutomation) {
      const loaded = loadedTriggerRef.current
      const triggerChanged =
        !loaded ||
        loaded.trigger !== trigger ||
        (trigger === 'cron' && loaded.cronPattern !== cronPattern) ||
        (trigger === 'heartbeat' && loaded.heartbeatIntervalSecs !== heartbeatIntervalSecs)
      updateMutation.mutate(
        {
          id: editAutomation.id,
          name: name.trim(),
          instruction: instruction.trim(),
          description: description.trim() ? description.trim() : null,
          projectId,
          personaId,
          brainSpace,
        },
        {
          onSuccess: () => {
            // Only re-send the trigger config when the user actually changed
            // it. The server preserves status; an unconditional re-send would
            // silently re-arm a paused definition's schedule.
            if (triggerChanged) {
              triggerMutation.mutate(
                {
                  id: editAutomation.id,
                  trigger,
                  cronPattern: trigger === 'cron' ? cronPattern : null,
                  heartbeatIntervalSecs: trigger === 'heartbeat' ? heartbeatIntervalSecs : null,
                },
                { onSuccess: () => onClose() },
              )
            } else {
              onClose()
            }
          },
        },
      )
      return
    }

    createMutation.mutate(
      {
        name: name.trim(),
        instruction: instruction.trim(),
        description: description.trim() ? description.trim() : null,
        trigger,
        cronPattern: trigger === 'cron' ? cronPattern : null,
        heartbeatIntervalSecs: trigger === 'heartbeat' ? heartbeatIntervalSecs : null,
        projectId,
        personaId,
        brainSpace,
      },
      { onSuccess: () => onClose() },
    )
  }

  const projectList = useMemo(() => projectRoster.data ?? [], [projectRoster.data])
  const personaList = useMemo(() => personaRoster.data ?? [], [personaRoster.data])
  const brainList = useMemo(() => brainRoster.data ?? [], [brainRoster.data])

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        className="sm:max-w-lg max-h-[85vh] overflow-y-auto"
        data-testid="automation-form-dialog"
      >
        <DialogHeader>
          <DialogTitle>
            {isEdit ? t('automations.editDialogTitle') : t('automations.createDialogTitle')}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div>
            <Label className="text-sm font-medium mb-1 block">{t('automations.nameLabel')}</Label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('automations.namePlaceholder')}
              autoFocus
              data-testid="automation-form-name"
            />
          </div>
          <div>
            <Label className="text-sm font-medium mb-1 block">
              {t('automations.descriptionLabel')}
            </Label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t('automations.descriptionPlaceholder')}
            />
          </div>
          <div>
            <div className="flex items-center justify-between mb-1">
              <Label className="text-sm font-medium">{t('automations.instructionLabel')}</Label>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-6 gap-1 px-2 text-xs text-primary"
                onClick={runAiAssist}
                disabled={goalSuggestions.isPending || !instruction.trim()}
                title={t('automations.aiAssistHint')}
              >
                {goalSuggestions.isPending
                  ? t('automations.aiThinking')
                  : t('automations.aiAssist')}
              </Button>
            </div>
            <Textarea
              value={instruction}
              onChange={(e) => setInstruction(e.target.value)}
              placeholder={t('automations.instructionPlaceholder')}
              rows={5}
              data-testid="automation-form-instruction"
            />
            {suggestions.length > 0 && (
              <div className="mt-2 space-y-1.5">
                <p className="text-xs text-muted-foreground">{t('automations.aiPickOne')}</p>
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

          {/* Scoped bindings — persisted on the definition, not inferred */}
          <div className="space-y-2 rounded-lg border p-3">
            <Label className="text-xs font-medium text-muted-foreground">
              {t('automations.bindingsTitle')}
            </Label>
            <div className="flex flex-wrap items-center gap-2">
              <AutomationProjectPicker
                projects={projectList}
                value={projectId}
                onChange={setProjectId}
              />
              <AutomationPersonaPicker
                personas={personaList}
                value={personaId}
                onChange={setPersonaId}
              />
              <AutomationBrainPicker
                spaces={brainList}
                value={brainSpace}
                onChange={setBrainSpace}
              />
            </div>
            <p className="text-2xs text-muted-foreground/80">{t('automations.bindingsHint')}</p>
          </div>

          <AutomationTriggerEditor
            trigger={trigger}
            cronPattern={cronPattern}
            heartbeatIntervalSecs={heartbeatIntervalSecs}
            onChange={(p) => {
              setTrigger(p.trigger)
              if (p.cronPattern !== undefined) setCronPattern(p.cronPattern ?? DEFAULT_CRON)
              if (p.heartbeatIntervalSecs !== undefined) {
                setHeartbeatIntervalSecs(p.heartbeatIntervalSecs ?? 1800)
              }
            }}
            id="automation-form"
          />

          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" size="sm" onClick={onClose}>
              {t('automations.cancel')}
            </Button>
            <Button
              size="sm"
              onClick={handleSubmit}
              disabled={!name.trim() || !instruction.trim() || pending}
              data-testid="automation-form-submit"
            >
              {pending
                ? t('automations.creating')
                : isEdit
                  ? t('common.save')
                  : t('automations.createAutomation')}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}

// ── Detail dialog (instruction + trigger + verify + last run) ──

function AutomationDetailDialog({
  automation,
  onClose,
}: {
  automation: Automation | null
  onClose: () => void
}) {
  const { t } = useTranslation()
  const triggerMutation = useSetAutomationTrigger()
  const verifyMutation = useSetAutomationVerify()
  const runMutation = useRunAutomation()

  // Local mirror so editing doesn't refetch the definition on every keystroke.
  const [trigger, setTrigger] = useState<Automation['trigger']>('manual')
  const [cronPattern, setCronPattern] = useState<string>(DEFAULT_CRON)
  const [heartbeatIntervalSecs, setHeartbeatIntervalSecs] = useState<number>(1800)

  useEffect(() => {
    if (automation) {
      setTrigger(automation.trigger)
      setCronPattern(automation.cronPattern ?? DEFAULT_CRON)
      setHeartbeatIntervalSecs(automation.heartbeatIntervalSecs ?? 1800)
    }
  }, [automation])

  return (
    <Dialog open={!!automation} onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        className="sm:max-w-3xl max-h-[85vh] overflow-y-auto"
        data-testid="automation-detail-dialog"
      >
        {automation && (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                {automation.name}
                <Badge
                  variant="secondary"
                  className={AUTOMATION_STATUS_META[automation.status].color}
                >
                  {t(AUTOMATION_STATUS_META[automation.status].label)}
                </Badge>
              </DialogTitle>
            </DialogHeader>
            <div className="space-y-4">
              {/* Description */}
              {automation.description && (
                <p className="text-sm text-muted-foreground">{automation.description}</p>
              )}
              {/* Last failure — surfaced so a Failed automation explains itself */}
              {automation.lastError && (
                <p
                  className="text-xs text-status-error-on-surface"
                  data-testid="automation-last-error"
                >
                  {automation.lastError}
                </p>
              )}
              {/* Instruction */}
              <div>
                <Label className="text-xs font-medium mb-1 block text-muted-foreground">
                  {t('automations.instructionLabel')}
                </Label>
                <pre className="text-xs whitespace-pre-wrap font-mono bg-muted/50 rounded-lg p-3 max-h-48 overflow-y-auto">
                  {automation.instruction}
                </pre>
              </div>

              {/* Trigger config */}
              <AutomationTriggerEditor
                trigger={trigger}
                cronPattern={cronPattern}
                heartbeatIntervalSecs={heartbeatIntervalSecs}
                onChange={(p) => {
                  setTrigger(p.trigger)
                  if (p.cronPattern !== undefined) setCronPattern(p.cronPattern ?? DEFAULT_CRON)
                  if (p.heartbeatIntervalSecs !== undefined) {
                    setHeartbeatIntervalSecs(p.heartbeatIntervalSecs ?? 1800)
                  }
                }}
                id={`automation-detail-${automation.id}`}
              />

              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={triggerMutation.isPending}
                  onClick={() => {
                    triggerMutation.mutate(
                      {
                        id: automation.id,
                        trigger,
                        cronPattern: trigger === 'cron' ? cronPattern : null,
                        heartbeatIntervalSecs:
                          trigger === 'heartbeat' ? heartbeatIntervalSecs : null,
                      },
                      { onSuccess: () => toast.success(t('automations.triggerSaved')) },
                    )
                  }}
                  data-testid="automation-save-trigger"
                >
                  <CalendarClock className="h-3.5 w-3.5" />
                  {t('automations.triggerLabel')}
                </Button>
                <Button
                  size="sm"
                  disabled={runMutation.isPending}
                  onClick={() =>
                    runMutation.mutate(
                      { id: automation.id },
                      {
                        onSuccess: (data) =>
                          data.success
                            ? toast.success(t('automations.run'))
                            : toast.error(t('automations.runFailed')),
                      },
                    )
                  }
                  data-testid="automation-detail-run"
                >
                  <Play className="h-3.5 w-3.5" />
                  {runMutation.isPending ? t('automations.statusRunning') : t('automations.run')}
                </Button>
              </div>

              {/* Verify gate */}
              <AutomationVerifyConfig
                enabled={automation.verify.enabled}
                requirement={automation.verify.requirement ?? undefined}
                maxIterations={automation.verify.maxIterations}
                isPending={verifyMutation.isPending}
                onSave={(params) =>
                  verifyMutation.mutate(
                    { id: automation.id, ...params },
                    {
                      onSuccess: () => toast.success(t('automations.verifySaved')),
                      onError: () => toast.error(t('automations.verifySaveFailed')),
                    },
                  )
                }
              />

              {/* Immutable run-context snapshot display */}
              <LastRunSnapshot automationId={automation.id} />
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

// ── Run context snapshot + run-history list ──

// The latest run's immutable contextSnapshot is the header card; the full
// history (newest first, kernel caps at 50) renders below as a compact list.
function LastRunSnapshot({ automationId }: { automationId: string }) {
  const { t } = useTranslation()
  const { data, isLoading } = useAutomationRuns(automationId)
  const runs: AutomationRun[] = data?.runs ?? []
  const latest = runs[0]

  return (
    <div data-testid="automation-last-run-context">
      <div className="flex items-center gap-1.5 mb-2">
        <History className="h-3.5 w-3.5 text-muted-foreground" />
        <Label className="text-xs font-medium text-muted-foreground">
          {t('automations.lastRunContext')}
        </Label>
      </div>
      {isLoading ? (
        <div className="text-xs text-muted-foreground">…</div>
      ) : !latest ? (
        <p className="text-xs text-muted-foreground/60">{t('automations.noRuns')}</p>
      ) : (
        <>
          <RunContextCard run={latest} />
          {runs.length > 0 && (
            <div className="mt-3" data-testid="automation-run-history">
              <Label className="text-xs font-medium text-muted-foreground mb-1.5 block">
                {t('automations.runHistory')}
              </Label>
              <div className="space-y-1.5 max-h-48 overflow-y-auto">
                {runs.map((run) => (
                  <RunHistoryRow key={run.id} run={run} />
                ))}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  )
}

// One compact history row: status, trigger, summary-or-error, relative time.
function RunHistoryRow({ run }: { run: AutomationRun }) {
  const ok = run.status === 'succeeded'
  return (
    <div
      className="flex items-start gap-2 rounded-lg border p-2 text-xs"
      data-testid="automation-run-history-row"
    >
      <span
        className={cn(
          'mt-0.5 h-1.5 w-1.5 rounded-full shrink-0',
          ok
            ? 'bg-status-success'
            : run.status === 'running'
              ? 'bg-status-warning'
              : 'bg-status-error',
        )}
        aria-label={run.status}
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="font-medium capitalize">{run.status}</span>
          <Badge variant="outline" className="capitalize px-1.5 py-0 text-2xs">
            {run.trigger}
          </Badge>
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
}

function RunContextCard({ run }: { run: AutomationRun }) {
  const { t } = useTranslation()
  const snap = run.contextSnapshot
  return (
    <div
      className="rounded-lg border p-3 space-y-1.5 text-xs"
      data-testid="automation-run-context-snapshot"
    >
      <div className="flex items-center gap-2">
        <Badge variant="outline" className="capitalize">
          {snap.trigger}
        </Badge>
        <span className="text-muted-foreground/70">{relativeTime(run.startedAt)}</span>
      </div>
      <div className="grid grid-cols-3 gap-2">
        <div>
          <span className="text-muted-foreground">{t('automations.projectPickerLabel')}</span>
          <p className="font-medium">{snap.projectId ?? t('automations.noProject')}</p>
        </div>
        <div>
          <span className="text-muted-foreground">{t('automations.personaPickerLabel')}</span>
          <p className="font-medium">{snap.personaId ?? t('automations.noPersona')}</p>
        </div>
        <div>
          <span className="text-muted-foreground">{t('automations.brainPickerLabel')}</span>
          <p className="font-medium">{snap.brainSpace ?? t('automations.noBrain')}</p>
        </div>
      </div>
      <pre className="text-2xs whitespace-pre-wrap font-mono bg-muted/50 rounded p-2 max-h-32 overflow-y-auto">
        {snap.instruction}
      </pre>
    </div>
  )
}
