import { useRouterState } from '@tanstack/react-router'
import { ArrowRightLeft } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useKnowledgeFile, useKnowledgeRecursiveTree, useMoveFile } from '@/hooks/use-knowledge'
import { cn } from '@/lib/utils'
import { useKnowledgeStore } from '@/stores/knowledge'

/**
 * MoveModal — Phase 4 (R3 resolution).
 *
 * Replaces the previous implementation that used `writeFile` + `deleteFile`
 * (2-step non-atomic move, git history fragmentation, no backlink reindex).
 * Now delegates to `POST /api/knowledge/move` which calls
 * `KnowledgeBase::note_move` (atomic rename + backlinks + change event).
 *
 * Directory listing now uses the recursive tree endpoint so all directories
 * (including nested ones) are selectable without typing paths manually.
 */
export function MoveModal() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [focusedIndex, setFocusedIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  const currentFilePath = useKnowledgeStore((s) => s.currentFilePath)
  const openFile = useKnowledgeStore((s) => s.openFile)
  const moveFile = useMoveFile()
  // Re-read current content only to verify the file still exists client-side
  // before the user attempts the move. The actual move uses the path-only API.
  const { data: currentContent } = useKnowledgeFile(currentFilePath)
  const { data: tree } = useKnowledgeRecursiveTree(open)

  // Every directory path in the tree (S7). Empty string represents the root.
  const allDirs = useMemo(() => {
    if (!tree) return ['/']
    return [
      '/',
      ...flattenTree(tree)
        .filter((n) => n.is_dir)
        .map((n) => n.path),
    ]
  }, [tree])

  // Manual path entry — if the user types a `/`-bearing query that
  // doesn't match an existing directory, treat it as a free-form target.
  const manualDir = query.trim().startsWith('/')
    ? query.trim()
    : query.trim().includes('/')
      ? query.trim()
      : null
  const extraDirs = manualDir && !allDirs.includes(manualDir) ? [manualDir] : []
  const candidateDirs = [...allDirs, ...extraDirs]

  const filteredDirs = query.trim()
    ? candidateDirs.filter((d) => d.toLowerCase().includes(query.trim().toLowerCase()))
    : candidateDirs

  // Global ⌘M listener (pathname guarded so the modal doesn't open on chat etc.)
  const router = useRouterState()
  const pathnameRef = useRef(router.location.pathname)
  pathnameRef.current = router.location.pathname

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'm') return
      if (!pathnameRef.current.startsWith('/brain/knowledge')) return
      e.preventDefault()
      setOpen(true)
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  useEffect(() => {
    if (open) inputRef.current?.focus()
  }, [open])

  const close = useCallback(() => setOpen(false), [])

  const handleMove = useCallback(
    async (targetDir: string) => {
      if (!currentFilePath) return
      const filename = currentFilePath.split('/').pop() ?? ''
      const newPath = targetDir === '/' || targetDir === '' ? filename : `${targetDir}/${filename}`
      if (newPath === currentFilePath) {
        close()
        return
      }
      try {
        await moveFile.mutateAsync({ from: currentFilePath, to: newPath })
        openFile(newPath)
      } catch (err) {
        console.error('move failed', err)
      }
      close()
    },
    [currentFilePath, moveFile, openFile, close],
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setFocusedIndex((i) => Math.min(i + 1, filteredDirs.length - 1))
      } else if (e.key === 'ArrowUp') {
        e.preventDefault()
        setFocusedIndex((i) => Math.max(i - 1, 0))
      } else if (e.key === 'Enter') {
        e.preventDefault()
        const dir = filteredDirs[focusedIndex]
        if (dir) handleMove(dir)
      } else if (e.key === 'Escape') {
        close()
      }
    },
    [filteredDirs, focusedIndex, close, handleMove],
  )

  if (!open) return null
  if (!currentFilePath) return null
  // Surface a friendly empty state if the active file no longer exists.
  // currentContent is undefined while loading and null if the file is gone.
  if (currentContent === null) return null

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center pt-[20vh]">
      <div className="fixed inset-0 bg-black/30" onClick={close} aria-hidden />
      <div className="relative z-10 w-full max-w-md rounded-lg border bg-popover shadow-lg">
        <div className="flex items-center gap-2 border-b px-3 py-2">
          <ArrowRightLeft className="h-4 w-4 text-muted-foreground" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setFocusedIndex(0)
            }}
            onKeyDown={handleKeyDown}
            placeholder={t('knowledge.movePlaceholder')}
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          <span className="text-2xs text-muted-foreground/60 font-mono">⌘M</span>
        </div>

        <ul className="max-h-64 overflow-y-auto p-1">
          {filteredDirs.length === 0 && (
            <li className="px-3 py-2 text-xs text-muted-foreground">
              {t('knowledge.moveNoMatch')}
            </li>
          )}
          {filteredDirs.map((dir, idx) => (
            <li key={dir}>
              <button
                type="button"
                onClick={() => handleMove(dir)}
                onMouseEnter={() => setFocusedIndex(idx)}
                className={cn(
                  'flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-left text-sm',
                  idx === focusedIndex ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/50',
                )}
              >
                <span className="font-mono text-2xs text-muted-foreground">/</span>
                <span className="truncate">{dir === '/' ? '/' : dir}</span>
              </button>
            </li>
          ))}
        </ul>

        <div className="border-t px-3 py-2 text-2xs text-muted-foreground">
          {t('knowledge.moveFooter')}
        </div>
      </div>
    </div>
  )
}

/** Pre-order traversal that returns every node (file + dir). */
function flattenTree<T extends { is_dir: boolean; path: string; children?: T[] }>(nodes: T[]): T[] {
  const out: T[] = []
  for (const node of nodes) {
    out.push(node)
    if (node.children?.length) out.push(...flattenTree(node.children))
  }
  return out
}
