import { Link } from '@tanstack/react-router'
import { FolderOpen } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ProjectBrainSelect } from '@/components/brain/space-select'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { useMounts } from '@/hooks/use-mounts'
import { useUpdateProject } from '@/hooks/use-projects'
import type { Project } from '@/types'

interface EditProjectDialogProps {
  project: Project | null
  open: boolean
  onOpenChange: (open: boolean) => void
  onSuccess?: () => void
}

import { ICON_OPTIONS } from './create-project-dialog'

export function EditProjectDialog({
  project,
  open,
  onOpenChange,
  onSuccess,
}: EditProjectDialogProps) {
  const { t } = useTranslation()
  const update = useUpdateProject()

  const [name, setName] = useState('')
  const [icon, setIcon] = useState('package')
  // RFC-025: mount_ids + instructions
  const [mountIds, setMountIds] = useState<string[]>([])
  const [defaultBrainSpace, setDefaultBrainSpace] = useState<string | null>(null)
  const [instructions, setInstructions] = useState('')
  const { data: mountsData } = useMounts()
  const availableMounts = useMemo(() => mountsData?.items ?? [], [mountsData?.items])

  // Sync state when the project prop or open state changes.
  useEffect(() => {
    if (project && open) {
      setName(project.name)
      setIcon(project.emoji ?? 'package')
      setMountIds(project.mount_ids ?? [])
      setInstructions(project.instructions ?? '')
      setDefaultBrainSpace(project.default_brain_space ?? null)
    }
  }, [project?.id, open])

  const handleSubmit = () => {
    if (!project || !name.trim()) return

    update.mutate(
      {
        id: project.id,
        name: name.trim(),
        emoji: icon,
        mount_ids: mountIds,
        instructions: instructions.trim() || undefined,
        default_brain_space: defaultBrainSpace,
      },
      {
        onSuccess: () => {
          toast(t('projects.updateSuccess'))
          onOpenChange(false)
          onSuccess?.()
        },
        onError: () => {
          toast.error(t('projects.updateError'))
        },
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t('projects.editTitle')}</DialogTitle>
          <DialogDescription>
            {project?.name
              ? t('projects.editDesc', 'Update "{{name}}"', { name: project.name })
              : ''}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="space-y-1">
            <label className="text-sm font-medium">{t('projects.name')}</label>
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="oxios" />
          </div>

          <div className="space-y-1">
            <label className="text-sm font-medium">{t('projects.icon')}</label>
            <div className="flex flex-wrap gap-1">
              {ICON_OPTIONS.map((opt) => (
                <button
                  key={opt.name}
                  type="button"
                  onClick={() => setIcon(opt.name)}
                  className={`w-8 h-8 rounded flex items-center justify-center border transition-colors ${
                    icon === opt.name
                      ? 'border-primary bg-primary/10'
                      : 'border-transparent hover:bg-muted'
                  }`}
                >
                  {opt.icon}
                </button>
              ))}
            </div>
          </div>

          {/* RFC-025: Mount references — click-toggle chips */}
          <div className="space-y-1">
            <label className="text-sm font-medium">{t('projects.mounts')}</label>
            {availableMounts.length > 0 ? (
              <div className="flex flex-wrap gap-1">
                {availableMounts.map((m) => (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => {
                      setMountIds((prev) =>
                        prev.includes(m.id) ? prev.filter((id) => id !== m.id) : [...prev, m.id],
                      )
                    }}
                    className={`rounded px-2 py-1 text-xs border transition-colors ${
                      mountIds.includes(m.id)
                        ? 'border-primary bg-primary/10 text-primary'
                        : 'border-transparent hover:bg-muted'
                    }`}
                  >
                    <span className="inline-flex items-center gap-1">
                      <FolderOpen className="h-3 w-3" /> {m.name}
                    </span>
                  </button>
                ))}
              </div>
            ) : (
              <div className="rounded-md border border-dashed p-3 text-center">
                <p className="text-xs text-muted-foreground mb-2">{t('projects.noMountsYet')}</p>
                <Link
                  to="/mounts"
                  onClick={() => onOpenChange(false)}
                  className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
                >
                  <FolderOpen className="h-3 w-3" />
                  {t('mounts.create')}
                </Link>
              </div>
            )}
          </div>

          {/* RFC-025: Custom instructions */}
          <div className="space-y-1">
            <label className="text-sm font-medium">{t('projects.instructions')}</label>
            <Textarea
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              rows={3}
              placeholder={t('projects.instructionsPlaceholder')}
            />
          </div>

          {/* Default brain — inherited by new chats in this project */}
          <div className="space-y-1">
            <label className="text-sm font-medium">{t('projects.defaultBrain')}</label>
            <ProjectBrainSelect
              value={defaultBrainSpace}
              onValueChange={setDefaultBrainSpace}
              className="w-full"
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t('common.cancel')}
          </Button>
          <Button onClick={handleSubmit} disabled={!name.trim() || update.isPending}>
            {update.isPending ? '...' : t('projects.save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
