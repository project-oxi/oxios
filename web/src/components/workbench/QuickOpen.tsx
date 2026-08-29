// QuickOpen — ⌘P file picker over the project workspace (Task 7).
//
// Lists entries from `/api/project/workspace/tree` (Task 4 wire
// contract). cmdk handles client-side filtering for the currently loaded
// entries; the server `path=` query navigates to a subdirectory when the
// user expands a directory by pressing Enter on it.
//
// Picking a file opens the FileEditorModal via the workbench store.

import { FileText, Folder } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { useProjectWorkspaceTree } from '@/hooks/use-workspace'
import { useWorkbenchStore } from '@/stores/workbench'

export function QuickOpen() {
  const { t } = useTranslation()
  const open = useWorkbenchStore((s) => s.quickOpenOpen)
  const setOpen = useWorkbenchStore((s) => s.setQuickOpen)
  const openEditor = useWorkbenchStore((s) => s.openEditor)
  const [path, setPath] = useState<string | undefined>(undefined)
  const { data, isLoading } = useProjectWorkspaceTree(path)

  // Reset to root whenever the dialog closes.
  useEffect(() => {
    if (!open) setPath(undefined)
  }, [open])

  const entries = useMemo(() => data?.entries ?? [], [data])
  const roots = useMemo(() => data?.roots ?? [], [data])

  /** Parent of the served dir, clamped to the nearest containing root
   *  (fix F9): `..` from just inside a root must land ON the root, never
   *  escape to a dir outside every root. */
  const clampToRoot = (dir: string): string => {
    const containing = roots.find((r) => dir === r) ?? roots.find((r) => dir.startsWith(`${r}/`))
    const parent = dir.split('/').slice(0, -1).join('/')
    if (!containing) return dir
    if (parent === containing || parent.startsWith(`${containing}/`)) return parent
    return containing
  }

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput
        data-testid="quick-open-input"
        placeholder={t('workbench.quickOpen.placeholder')}
      />
      <CommandList>
        <CommandEmpty>
          {isLoading ? t('common.loading') : t('workbench.quickOpen.empty')}
        </CommandEmpty>
        {roots.length > 1 && (
          <CommandGroup heading={t('workbench.quickOpen.roots')}>
            {roots.map((r) => (
              <CommandItem key={r} value={`root ${r}`} onSelect={() => setPath(r)}>
                <Folder className="mr-2 h-3.5 w-3.5" />
                {r.split('/').filter(Boolean).pop() || r}
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        {data?.path && !roots.includes(data.path) && (
          <CommandGroup heading="..">
            <CommandItem
              value=".."
              onSelect={() => {
                setPath(clampToRoot(data.path))
              }}
            >
              <Folder className="mr-2 h-3.5 w-3.5" />
              ..
            </CommandItem>
          </CommandGroup>
        )}
        <CommandGroup heading={data?.path ?? '/'}>
          {entries.map((entry) => (
            <CommandItem
              key={entry.path}
              value={`${entry.name} ${entry.path}`}
              onSelect={() => {
                if (entry.is_dir) {
                  setPath(entry.path)
                  return
                }
                openEditor(entry.path)
                setOpen(false)
              }}
            >
              {entry.is_dir ? (
                <Folder className="mr-2 h-3.5 w-3.5" />
              ) : (
                <FileText className="mr-2 h-3.5 w-3.5" />
              )}
              {entry.name}
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  )
}
