import { useRouterState } from '@tanstack/react-router'
import {
  ArrowLeft,
  ArrowRight,
  Calendar as CalendarIcon,
  Columns2,
  PanelRight,
  Save,
  X,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { EventEditor } from '@/components/calendar/event-editor'
import { EditorSettingsPopover } from '@/components/knowledge/editor-settings-popover'
import { NoteTitle } from '@/components/knowledge/note-title'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useCalendarCreate } from '@/hooks/use-calendar'
import { useKnowledgeStore } from '@/stores/knowledge'
import type { CreateEventRequest } from '@/types/calendar'

export function EditorToolbar() {
  const { t } = useTranslation()
  const {
    currentFilePath,
    history,
    historyIndex,
    goBack,
    goForward,
    infoPanelOpen,
    toggleInfoPanel,
    splitEditorOpen,
    closeSplit,
    openSplit,
  } = useKnowledgeStore()

  const [editorOpen, setEditorOpen] = useState(false)
  const createMutation = useCalendarCreate()

  const canGoBack = historyIndex > 0
  const canGoForward = historyIndex < history.length - 1
  const fileName =
    currentFilePath
      ?.split('/')
      .pop()
      ?.replace(/\.(md|html)$/, '') ?? ''

  const handleSave = () => {
    document.dispatchEvent(new CustomEvent('knowledge:save'))
  }

  // ⌘S keyboard shortcut — only on knowledge routes (B5 fix)
  const router = useRouterState()
  const pathnameRef = useRef(router.location.pathname)
  pathnameRef.current = router.location.pathname

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!pathnameRef.current.startsWith('/brain/knowledge')) return
      if ((e.metaKey || e.ctrlKey) && e.key === 's') {
        e.preventDefault()
        handleSave()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [handleSave])

  return (
    <div className="flex items-center gap-1 px-3 py-1.5 border-b bg-background min-h-[44px]">
      <Button
        variant="ghost"
        size="icon"
        className="h-8 w-8"
        onClick={() => goBack()}
        disabled={!canGoBack}
        title={t('knowledge.goBack')}
        aria-label={t('knowledge.goBack')}
      >
        <ArrowLeft className="h-4 w-4" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        className="h-8 w-8"
        onClick={() => goForward()}
        disabled={!canGoForward}
        title={t('knowledge.goForward')}
        aria-label={t('knowledge.goForward')}
      >
        <ArrowRight className="h-4 w-4" />
      </Button>

      <NoteTitle />

      <div className="flex-1" />

      {/* Save */}
      {currentFilePath && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              onClick={handleSave}
              aria-label={t('common.save')}
            >
              <Save className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{t('knowledge.saveWithShortcut')}</TooltipContent>
        </Tooltip>
      )}

      {/* Split editor */}
      {!splitEditorOpen && currentFilePath && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8 hidden md:inline-flex"
              onClick={() => openSplit(currentFilePath)}
              aria-label={t('knowledge.openSplitView')}
            >
              <Columns2 className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{t('knowledge.splitView')}</TooltipContent>
        </Tooltip>
      )}

      {/* Close split */}
      {splitEditorOpen && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              onClick={closeSplit}
              aria-label={t('knowledge.closeSplit')}
            >
              <X className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{t('knowledge.closeSplitWithShortcut')}</TooltipContent>
        </Tooltip>
      )}

      {/* Info panel toggle */}
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={toggleInfoPanel}
            aria-label={t('knowledge.toggleInfoPanel')}
          >
            <PanelRight className="h-4 w-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>
          {infoPanelOpen ? t('knowledge.hideInfoPanel') : t('knowledge.showInfoPanel')}
        </TooltipContent>
      </Tooltip>

      {/* Add to calendar — creates an event linked to this note */}
      {currentFilePath && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              onClick={() => setEditorOpen(true)}
              disabled={createMutation.isPending}
              aria-label={t('calendar.scheduleNote')}
            >
              <CalendarIcon className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{t('calendar.scheduleNote')}</TooltipContent>
        </Tooltip>
      )}

      {/* Editor appearance settings */}
      <EditorSettingsPopover />

      <EventEditor
        open={editorOpen}
        onClose={() => setEditorOpen(false)}
        defaultStart={new Date()}
        defaultTitle={fileName}
        isLoading={createMutation.isPending}
        onSubmit={(formData) => {
          createMutation.mutate(
            { ...(formData as CreateEventRequest), note_path: currentFilePath ?? undefined },
            { onSuccess: () => setEditorOpen(false) },
          )
        }}
      />
    </div>
  )
}
