import { useNavigate } from '@tanstack/react-router'
import { AlertTriangle } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { useDeleteProject } from '@/hooks/use-projects'
import type { Project } from '@/types'

interface DeleteProjectDialogProps {
  project: Project | null
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function DeleteProjectDialog({ project, open, onOpenChange }: DeleteProjectDialogProps) {
  const { t } = useTranslation()
  const deleteProject = useDeleteProject()
  const navigate = useNavigate()

  const handleDelete = () => {
    if (!project) return

    deleteProject.mutate(project.id, {
      onSuccess: () => {
        toast(t('projects.deleteSuccess'))
        onOpenChange(false)
        navigate({ to: '/operate/projects' })
      },
      onError: () => {
        toast.error(t('projects.deleteError'))
      },
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>
            {t('projects.deleteTitle', 'Delete "{{name}}"?', { name: project?.name ?? '' })}
          </DialogTitle>
          <DialogDescription>{t('projects.deleteDesc')}</DialogDescription>
        </DialogHeader>

        <ul className="text-sm text-muted-foreground list-disc pl-5 space-y-1">
          <li>{t('projects.deleteMemories')}</li>
          <li>{t('projects.deleteFiles')}</li>
        </ul>

        <p className="text-xs text-destructive font-medium">
          <AlertTriangle className="h-4 w-4 shrink-0" /> {t('projects.undoWarning')}
        </p>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t('common.cancel')}
          </Button>
          <Button variant="destructive" onClick={handleDelete} disabled={deleteProject.isPending}>
            {deleteProject.isPending ? '...' : t('projects.delete')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
