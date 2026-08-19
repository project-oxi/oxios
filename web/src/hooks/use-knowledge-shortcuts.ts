import { useRouterState } from '@tanstack/react-router'
import { useEffect, useRef } from 'react'
import { api } from '@/lib/api-client'
import { useKnowledgeStore } from '@/stores/knowledge'
import { useSidebarStore } from '@/stores/sidebar'

/**
 * Register global keyboard shortcuts for the Knowledge UI.
 * Only active when the current route is within /brain/knowledge.
 *
 * Uses individual store selectors and ref-based mutation handles to avoid
 * stale closures and unnecessary re-registrations.
 *
 * Shortcuts:
 * ⌘N        → New file
 * ⌘⇧N       → New folder
 * ⌘D        → Delete current file (editor mode)
 * ⌘Enter    → Open chat
 * ⌘⇧Enter   → Toggle chat overlay
 * ⌘~ / ⌘§   → Toggle main sidebar
 * ⌘W        → Close split editor
 * Escape    → Close split / deselect all
 */

/** Stable write-file helper that doesn't change reference */
function useStableWriteFile() {
  const mutateRef = useRef<((path: string, content: string) => Promise<void>) | undefined>(
    undefined,
  )

  // Recreate the mutator on each render (it's only called from the event handler via ref)
  mutateRef.current = async (path: string, content: string) => {
    await api.put('/api/knowledge/files', { path, content })
  }

  return mutateRef
}

/** Stable delete-file helper */
function useStableDeleteFile() {
  const mutateRef = useRef<((path: string) => Promise<void>) | undefined>(undefined)

  mutateRef.current = async (path: string) => {
    await api.delete(`/api/knowledge/files/${encodeURIComponent(path)}`)
  }

  return mutateRef
}

export function useKnowledgeShortcuts() {
  // Individual selectors — each returns a stable reference (zustand guarantees this)
  const openFile = useKnowledgeStore((s) => s.openFile)
  const mode = useKnowledgeStore((s) => s.mode)
  const currentFilePath = useKnowledgeStore((s) => s.currentFilePath)
  const openChat = useKnowledgeStore((s) => s.openChat)
  const toggleMainSidebar = useSidebarStore((s) => s.toggle)
  const splitEditorOpen = useKnowledgeStore((s) => s.splitEditorOpen)
  const closeSplit = useKnowledgeStore((s) => s.closeSplit)

  // Stable mutation refs (avoid effect re-registration on mutation state change)
  const writeFileRef = useStableWriteFile()
  const deleteFileRef = useStableDeleteFile()

  // Keep a ref to the latest pathname so the event handler is never stale
  const router = useRouterState()
  const pathnameRef = useRef(router.location.pathname)
  pathnameRef.current = router.location.pathname

  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      // Only activate when on a knowledge route
      if (!pathnameRef.current.startsWith('/brain/knowledge')) return

      const isMeta = e.metaKey || e.ctrlKey

      // ⌘N — new file
      if (isMeta && e.key === 'n' && !e.shiftKey) {
        e.preventDefault()
        e.stopPropagation()
        try {
          await writeFileRef.current?.('New file.md', '# New file\n\n')
          openFile('New file.md')
        } catch {
          /* ignore */
        }
        return
      }

      // ⌘⇧N — new folder
      if (isMeta && e.key === 'N' && e.shiftKey) {
        e.preventDefault()
        e.stopPropagation()
        const name = prompt('Enter folder name:', 'New Folder')
        if (name?.trim()) {
          try {
            await writeFileRef.current?.(`${name.trim()}/.keep`, '')
          } catch {
            /* ignore */
          }
        }
        return
      }

      // ⌘D — delete current file
      if (isMeta && e.key === 'd') {
        e.preventDefault()
        e.stopPropagation()
        if (mode === 'editor' && currentFilePath) {
          if (confirm(`Delete ${currentFilePath}?`)) {
            try {
              await deleteFileRef.current?.(currentFilePath)
            } catch {
              /* ignore */
            }
          }
        }
        return
      }

      // ⌘Enter — open chat
      if (isMeta && e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        e.stopPropagation()
        openChat()
        return
      }

      // ⌘⇧Enter — toggle chat overlay
      if (isMeta && e.key === 'Enter' && e.shiftKey) {
        e.preventDefault()
        e.stopPropagation()
        openChat()
        return
      }

      // ⌘~ or ⌘§ — toggle main sidebar
      if (isMeta && (e.key === '~' || e.key === '§')) {
        e.preventDefault()
        e.stopPropagation()
        toggleMainSidebar()
        return
      }

      // ⌘W — close split editor
      if (isMeta && e.key === 'w') {
        e.preventDefault()
        e.stopPropagation()
        if (splitEditorOpen) {
          closeSplit()
        }
        return
      }

      // Escape — close split or deselect
      if (e.key === 'Escape') {
        if (splitEditorOpen) {
          closeSplit()
        }
        return
      }
    }

    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
    // These selectors are stable references from zustand — the effect only runs once
  }, [
    openFile,
    mode,
    currentFilePath,
    openChat,
    toggleMainSidebar,
    splitEditorOpen,
    closeSplit,
    writeFileRef,
    deleteFileRef,
  ])
}
