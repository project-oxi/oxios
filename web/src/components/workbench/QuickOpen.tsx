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
        {data?.path && data.path !== data.roots[0] && (
          <CommandGroup heading="..">
            <CommandItem
              value=".."
              onSelect={() => {
                const parent = data.path.split('/').slice(0, -1).join('/') || '/'
                setPath(parent)
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
