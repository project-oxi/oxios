// TaskDependencies — list + add/remove + graph (RFC-043 §D3 + §D9)

import { GitBranch, Link2, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { EmptyState } from '@/components/shared/empty-state'
import { LoadingCards } from '@/components/shared/loading'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Select } from '@/components/ui/select'
import {
  useAddTaskDependency,
  useRemoveTaskDependency,
  useTaskDependencies,
  useTasks,
} from '@/hooks/use-tasks'
import { cn } from '@/lib/utils'
import { TASK_STATUS_META, type Task } from '@/types/task'
import { TaskDependencyGraph } from './task-dependency-graph'

export interface TaskDependenciesProps {
  task: Task
}

export function TaskDependencies({ task }: TaskDependenciesProps) {
  const { t } = useTranslation()
  const { data, isLoading } = useTaskDependencies(task.id)
  const allTasks = useTasks()
  const add = useAddTaskDependency(task.id)
  const remove = useRemoveTaskDependency(task.id)
  const [pickId, setPickId] = useState<string>('')

  const dependencies = data?.dependencies ?? []
  const all = allTasks.data?.tasks ?? []
  // Available to add: not the task itself, not already a dependency.
  const available = all.filter(
    (tk) => tk.id !== task.id && !dependencies.some((d) => d.id === tk.id),
  )

  const handleAdd = () => {
    if (!pickId) return
    add.mutate(
      { dependsOnTaskId: pickId },
      {
        onSuccess: () => {
          setPickId('')
          toast.success(t('tasks.dependencyAdded'))
        },
        onError: (err) =>
          toast.error(err instanceof Error ? err.message : t('tasks.dependencyAddFailed')),
      },
    )
  }

  const handleRemove = (depId: string) => {
    remove.mutate(depId, {
      onSuccess: () => toast.success(t('tasks.dependencyRemoved')),
      onError: () => toast.error(t('tasks.dependencyRemoveFailed')),
    })
  }

  const selectOptions = [
    ...(available.length === 0
      ? [{ value: '__none__', label: t('tasks.noAvailableDependencies') }]
      : []),
    ...available.map((tk) => ({ value: tk.id, label: tk.name })),
  ]

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-1.5">
        <GitBranch className="h-3.5 w-3.5 text-muted-foreground" />
        <Label className="text-xs font-medium text-muted-foreground">
          {t('tasks.dependenciesTitle')}
        </Label>
      </div>

      {/* Add row */}
      <div className="flex items-center gap-2">
        <Select
          className="flex-1"
          value={pickId}
          onValueChange={(v) => setPickId(v === '__none__' ? '' : v)}
          placeholder={t('tasks.dependencyPlaceholder')}
          options={selectOptions}
        />
        <Button
          size="sm"
          variant="outline"
          className="gap-1.5"
          onClick={handleAdd}
          disabled={!pickId || add.isPending}
        >
          <Plus className="h-3 w-3" />
          {t('tasks.dependencyAdd')}
        </Button>
      </div>

      {/* List */}
      {isLoading ? (
        <LoadingCards count={1} />
      ) : dependencies.length === 0 ? (
        <EmptyState
          size="compact"
          icon={<Link2 className="h-6 w-6" />}
          title={t('tasks.noDependencies')}
          description={t('tasks.noDependenciesDescription')}
        />
      ) : (
        <div className="space-y-1.5">
          {dependencies.map((dep) => {
            const meta = TASK_STATUS_META[dep.status]
            return (
              <div
                key={dep.id}
                className="flex items-center justify-between gap-2 rounded-lg border p-2 text-xs"
              >
                <div className="min-w-0 flex-1">
                  <div className="font-medium truncate">{dep.name}</div>
                  <div className="text-muted-foreground font-mono truncate">{dep.identifier}</div>
                </div>
                <Badge variant="secondary" className={cn(meta.color, 'shrink-0')}>
                  {meta.label}
                </Badge>
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-7 text-muted-foreground hover:text-destructive"
                  onClick={() => handleRemove(dep.id)}
                  disabled={remove.isPending}
                  aria-label={t('tasks.dependencyRemove')}
                >
                  <Trash2 className="h-3 w-3" />
                </Button>
              </div>
            )
          })}
        </div>
      )}

      {/* Graph (renders null when no deps) */}
      <TaskDependencyGraph task={task} dependencies={dependencies} />
    </div>
  )
}
