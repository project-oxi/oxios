// TaskEditDialog — partial-edit dialog for an existing task (RFC-043 §D5/D9).
//
// Wired through useUpdateTask. Only sends fields the user actually changed
// (compared to the original task snapshot) so the route's partial update
// applies precisely the diff.

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { useUpdateTask } from '@/hooks/use-tasks'
import type { Task } from '@/types/task'

export interface TaskEditDialogProps {
  task: Task | null
  onClose: () => void
}

export function TaskEditDialog({ task, onClose }: TaskEditDialogProps) {
  const { t } = useTranslation()
  const update = useUpdateTask()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [instruction, setInstruction] = useState('')
  const [priority, setPriority] = useState(0)
  const [assigneeAgentId, setAssigneeAgentId] = useState('')

  // Re-seed local state every time the user opens a (different) task.
  useEffect(() => {
    if (!task) return
    setName(task.name)
    setDescription(task.description ?? '')
    setInstruction(task.instruction)
    setPriority(task.priority)
    setAssigneeAgentId(task.assigneeAgentId ?? '')
  }, [task?.id])

  const handleSave = () => {
    if (!task) return
    const params: Record<string, unknown> = {}
    if (name.trim() && name.trim() !== task.name) params.name = name.trim()
    if ((description.trim() || undefined) !== (task.description ?? undefined)) {
      params.description = description.trim() || undefined
    }
    if (instruction.trim() !== task.instruction) params.instruction = instruction.trim()
    if (priority !== task.priority) params.priority = priority
    if ((assigneeAgentId.trim() || undefined) !== (task.assigneeAgentId ?? undefined)) {
      params.assigneeAgentId = assigneeAgentId.trim() || undefined
    }
    if (Object.keys(params).length === 0) {
      onClose()
      return
    }
    update.mutate(
      { id: task.id, ...params },
      {
        onSuccess: () => {
          toast.success(t('tasks.editSaved'))
          onClose()
        },
        onError: () => toast.error(t('tasks.editSaveFailed')),
      },
    )
  }

  return (
    <Dialog open={!!task} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg max-h-[85vh] overflow-y-auto">
        {task && (
          <>
            <DialogHeader>
              <DialogTitle>{t('tasks.editDialogTitle')}</DialogTitle>
            </DialogHeader>
            <div className="space-y-3">
              <div>
                <Label className="text-sm font-medium mb-1 block">{t('tasks.nameLabel')}</Label>
                <Input value={name} onChange={(e) => setName(e.target.value)} />
              </div>
              <div>
                <Label className="text-sm font-medium mb-1 block">
                  {t('tasks.descriptionLabel')}
                </Label>
                <Input value={description} onChange={(e) => setDescription(e.target.value)} />
              </div>
              <div>
                <Label className="text-sm font-medium mb-1 block">
                  {t('tasks.instructionLabel')}
                </Label>
                <Textarea
                  value={instruction}
                  onChange={(e) => setInstruction(e.target.value)}
                  rows={4}
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label className="text-sm font-medium mb-1 block">
                    {t('tasks.priorityLabel')}
                  </Label>
                  <Input
                    type="number"
                    min={0}
                    value={priority}
                    onChange={(e) => setPriority(Math.max(0, Number(e.target.value) || 0))}
                  />
                </div>
                <div>
                  <Label className="text-sm font-medium mb-1 block">
                    {t('tasks.assigneeLabel')}
                  </Label>
                  <Input
                    value={assigneeAgentId}
                    onChange={(e) => setAssigneeAgentId(e.target.value)}
                    placeholder={t('tasks.assigneePlaceholder')}
                  />
                </div>
              </div>
              <div className="flex justify-end gap-2 pt-2">
                <Button variant="ghost" size="sm" onClick={onClose}>
                  {t('tasks.cancel')}
                </Button>
                <Button size="sm" onClick={handleSave} disabled={update.isPending}>
                  {update.isPending ? t('tasks.saving') : t('tasks.editSave')}
                </Button>
              </div>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
