// FileEditorModal — read/edit a project-workspace file (Task 7).
//
// Wire flow (Task 4 contracts, `/api/project/workspace/*`):
//   1. useProjectWorkspaceFile(path) loads the file content
//   2. edits are buffered in local textarea state until the user clicks Save
//   3. useSaveProjectFile PUTs the content back to the server
//   4. >512 KiB files come back with `truncated: true` and a read-only
//      banner; the textarea is disabled and Save is hidden
//   5. save failures surface via `toast.error(workbench.editor.saveFailed)`
//      and the modal stays open with the user's edits intact

import { Save, X } from 'lucide-react'
import { useEffect, useState } from 'react'
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
import { useProjectWorkspaceFile, useSaveProjectFile } from '@/hooks/use-workspace'
import { useWorkbenchStore } from '@/stores/workbench'

export function FileEditorModal() {
  const { t } = useTranslation()
  const path = useWorkbenchStore((s) => s.editingPath)
  const closeEditor = useWorkbenchStore((s) => s.closeEditor)
  const { data, isLoading } = useProjectWorkspaceFile(path)
  const save = useSaveProjectFile()
  const [content, setContent] = useState<string>('')

  // Reset the textarea when the file path or the loaded content changes.
  useEffect(() => {
    if (data) setContent(data.content)
  }, [data?.path, data?.content])

  const isTruncated = data?.truncated === true

  const handleSave = async () => {
    if (!path) return
    try {
      await save.mutateAsync({ path, content })
      toast.success(t('workbench.editor.saved'))
    } catch {
      toast.error(t('workbench.editor.saveFailed'))
    }
  }

  return (
    <Dialog open={!!path} onOpenChange={(o) => !o && closeEditor()}>
      <DialogContent data-testid="file-editor-modal" className="flex h-[80vh] max-w-3xl flex-col">
        <DialogHeader>
          <DialogTitle className="truncate font-mono text-xs">{path}</DialogTitle>
          <DialogDescription>
            {data && (
              <span className="text-2xs">
                {data.size} bytes
                {data.truncated ? ` · ${t('workbench.editor.truncated')}` : ''}
              </span>
            )}
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-hidden rounded border bg-background">
          {isLoading ? (
            <div className="p-3 text-2xs text-muted-foreground">{t('common.loading')}</div>
          ) : (
            <textarea
              readOnly={isTruncated}
              value={content}
              onChange={(e) => setContent(e.target.value)}
              className="h-full w-full resize-none bg-transparent p-3 font-mono text-xs outline-none"
              spellCheck={false}
            />
          )}
        </div>

        {isTruncated && (
          <div className="rounded border border-warning/40 bg-warning/10 px-3 py-2 text-2xs text-warning">
            {t('workbench.editor.truncatedHint')}
          </div>
        )}

        <DialogFooter className="gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => closeEditor()}
            className="h-7 gap-1 px-2 text-2xs"
          >
            <X className="h-3 w-3" />
            {t('common.cancel')}
          </Button>
          {!isTruncated && (
            <Button
              type="button"
              size="sm"
              onClick={handleSave}
              disabled={save.isPending || isLoading || !data}
              className="h-7 gap-1 px-2 text-2xs"
            >
              <Save className="h-3 w-3" />
              {t('workbench.editor.save')}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
