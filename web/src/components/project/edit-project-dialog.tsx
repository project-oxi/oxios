import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ProjectBrainSelect } from '@/components/brain/space-select'
import { RootPathsEditor } from '@/components/project/root-paths-editor'
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
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { useUpdateProject } from '@/hooks/use-projects'
import type { Project } from '@/types'

interface EditProjectDialogProps {
  project: Project | null
  open: boolean
  onOpenChange: (open: boolean) => void
  onSuccess?: () => void
}

/**
 * Edit dialog: the same root-list component as creation, pre-filled
 * (design §4.2). Folder removal here is separate from project deletion.
 */
export function EditProjectDialog({
  project,
  open,
  onOpenChange,
  onSuccess,
}: EditProjectDialogProps) {
  const { t } = useTranslation()
  const update = useUpdateProject()

  const [name, setName] = useState('')
  const [rootPaths, setRootPaths] = useState<string[]>([])
  const [defaultBrainSpace, setDefaultBrainSpace] = useState<string | null>(null)
  const [instructions, setInstructions] = useState('')

  // Sync state when the project prop or open state changes.
  useEffect(() => {
    if (project && open) {
      setName(project.name)
      setRootPaths(project.root_paths)
      setInstructions(project.instructions ?? '')
      setDefaultBrainSpace(project.default_brain_space ?? null)
    }
  }, [project?.id, open])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!project) return
    const n = name.trim()
    if (!n) return
    update.mutate(
      {
        id: project.id,
        name: n,
        root_paths: rootPaths,
        instructions: instructions.trim() || undefined,
        default_brain_space: defaultBrainSpace,
      },
      {
        onSuccess: () => {
          toast.success(t('projects.updateSuccess'))
          onOpenChange(false)
          onSuccess?.()
        },
        onError: (err) => {
          toast.error(t('projects.updateError', { error: err.message }))
        },
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t('projects.editTitle')}</DialogTitle>
          <DialogDescription>
            {t('projects.editDesc', { name: project?.name ?? '' })}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="project-edit-name">{t('projects.name')}</Label>
            <Input
              id="project-edit-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
            />
          </div>
          <div className="space-y-2">
            <Label>{t('projects.folders')}</Label>
            <p className="text-xs text-muted-foreground">{t('projects.foldersHint')}</p>
            <RootPathsEditor value={rootPaths} onChange={setRootPaths} />
          </div>
          <div className="space-y-2">
            <Label htmlFor="project-edit-instructions">{t('projects.instructions')}</Label>
            <Textarea
              id="project-edit-instructions"
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              placeholder={t('projects.instructionsPlaceholder')}
              rows={4}
            />
          </div>

          {/* Default brain — inherited by new chats in this project */}
          <div className="space-y-2">
            <Label>{t('projects.defaultBrain')}</Label>
            <ProjectBrainSelect
              value={defaultBrainSpace}
              onValueChange={setDefaultBrainSpace}
              className="w-full"
            />
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={update.isPending}
            >
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={!name.trim() || update.isPending}>
              {update.isPending ? t('common.saving') : t('projects.save')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
