import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
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
import { useCreateProject } from '@/hooks/use-projects'

interface CreateProjectDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/**
 * Create dialog (design §4.2): name + repeatable folder rows via the native
 * picker + optional instructions. Nothing else — no icon or tag metadata.
 */
export function CreateProjectDialog({ open, onOpenChange }: CreateProjectDialogProps) {
  const { t } = useTranslation()
  const create = useCreateProject()

  const [name, setName] = useState('')
  const [rootPaths, setRootPaths] = useState<string[]>([])
  const [instructions, setInstructions] = useState('')

  const reset = () => {
    setName('')
    setRootPaths([])
    setInstructions('')
  }

  useEffect(() => {
    if (open) reset()
  }, [open])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    const n = name.trim()
    if (!n) return
    create.mutate(
      {
        name: n,
        root_paths: rootPaths,
        instructions: instructions.trim() || undefined,
      },
      {
        onSuccess: () => {
          toast.success(t('projects.createSuccess'))
          reset()
          onOpenChange(false)
        },
        onError: (err) => {
          toast.error(t('projects.createError', { error: err.message }))
        },
      },
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t('projects.createTitle')}</DialogTitle>
          <DialogDescription>{t('projects.createDesc')}</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="project-name">{t('projects.name')}</Label>
            <Input
              id="project-name"
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
            <Label htmlFor="project-instructions">{t('projects.instructions')}</Label>
            <Textarea
              id="project-instructions"
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              placeholder={t('projects.instructionsPlaceholder')}
              rows={4}
            />
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={create.isPending}
            >
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={!name.trim() || create.isPending}>
              {create.isPending ? t('common.creating') : t('projects.create')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
