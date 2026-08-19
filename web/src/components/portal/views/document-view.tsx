// DocumentView — portal view: rendered (read-only) preview of a KnowledgeBase
// markdown file.
//
// LobeHub analogue: features/Portal/Document. Oxios keeps the chat-side portal
// read-only: editing happens in the full CodeMirror editor on the Knowledge
// Base page. The "Edit" action opens a leave-chat confirmation, then navigates
// to /knowledge with the file open (same openFile → navigate pattern as
// event-detail.tsx).
//
// No new fetcher is built — reuses useKnowledgeFile (string via select) and the
// shared MarkdownPreview renderer.

import { useNavigate } from '@tanstack/react-router'
import { FileX, Loader2, Pencil } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { ConfirmDialog } from '@/components/ui/confirm-dialog'
import { useKnowledgeFile } from '@/hooks/use-knowledge'
import { useKnowledgeStore } from '@/stores/knowledge'
import { type PortalView, usePortalStore } from '@/stores/portal'
import { MarkdownPreview } from './markdown-preview'

interface DocumentViewProps {
  view: Extract<PortalView, { type: 'document' }>
}

export function DocumentView({ view }: DocumentViewProps) {
  const { t } = useTranslation()
  const { path } = view
  const { data: content, isError, isLoading } = useKnowledgeFile(path)
  const navigate = useNavigate()
  const openFile = useKnowledgeStore((s) => s.openFile)
  const clearStack = usePortalStore((s) => s.clearStack)
  const [confirmEdit, setConfirmEdit] = useState(false)

  const handleEdit = () => {
    // Open the file in the KB store, then close the portal and route to /knowledge
    // (KnowledgeLayout reads mode='editor' + currentFilePath). Order matters:
    // openFile first so the target page mounts with the file already selected.
    openFile(path)
    clearStack()
    navigate({ to: '/brain/knowledge' })
  }

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        <Loader2 className="size-5 animate-spin" />
      </div>
    )
  }

  if (isError) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
        <FileX className="size-6" />
        <span>{t('portal.document.notFound')}</span>
        <span className="font-mono text-xs">{path}</span>
      </div>
    )
  }

  return (
    <div className="flex h-full flex-col">
      {/* Sub-header: full path + edit action. Mirrors the file-preview /
          artifact-view action rows so the portal chrome reads as one family. */}
      <div className="flex items-center justify-between gap-2 border-b px-3 py-1.5">
        <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          <span className="truncate font-mono" title={path}>
            {path}
          </span>
          <span className="shrink-0 rounded border px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
            MD
          </span>
        </div>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 shrink-0 gap-1 px-2 text-xs"
          onClick={() => setConfirmEdit(true)}
        >
          <Pencil className="size-3.5" />
          {t('portal.document.edit')}
        </Button>
      </div>

      {/* Rendered markdown body (read-only). */}
      <div className="min-h-0 flex-1 overflow-auto">
        <MarkdownPreview content={content ?? ''} className="p-4" />
      </div>

      <ConfirmDialog
        open={confirmEdit}
        onOpenChange={setConfirmEdit}
        title={t('portal.document.editWarningTitle')}
        description={t('portal.document.editWarningDesc')}
        confirmLabel={t('portal.document.editConfirm')}
        onConfirm={handleEdit}
      />
    </div>
  )
}
