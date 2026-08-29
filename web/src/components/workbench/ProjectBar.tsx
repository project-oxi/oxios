// ProjectBar — the workbench header strip (Task 7, design §7.1).
//
// Layout (left → right):
//   project selector (name only, popover) | roots summary
//   | branch / dirty status | active model | ⌘K hint
//
// Folderless coding projects: the roots summary is replaced by an
// "Add folders" action that opens the existing EditProjectDialog so the
// user can grant filesystem roots without leaving the chat. There is no
// file rail / tree / terminal in the folderless state.

import { AlertCircle, FolderPlus, GitBranch } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { EditProjectDialog } from '@/components/project/edit-project-dialog'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { useProject } from '@/hooks/use-projects'
import { useProjectWorkspaceStatus } from '@/hooks/use-workspace'
import { useChatStore } from '@/stores/chat'
import { ProjectSelectorPopover } from './ProjectSelectorPopover'

interface ProjectBarProps {
  /** When true, the workbench is rendering for a coding-family persona. */
  isCoding: boolean
}

function basename(path: string): string {
  const parts = path.split('/').filter(Boolean)
  return parts.length === 0 ? path : parts[parts.length - 1]!
}

export function ProjectBar({ isCoding }: ProjectBarProps) {
  const { t } = useTranslation()
  const activeProjectId = useChatStore((s) => s.activeProjectId)
  const activeModelId = useChatStore((s) => s.activeModelId)
  const [editOpen, setEditOpen] = useState(false)
  const { data: project } = useProject(activeProjectId)
  const { data: status } = useProjectWorkspaceStatus()

  const isFolderless = isCoding && project && project.root_paths.length === 0

  return (
    <header
      data-testid="project-bar"
      className="flex h-9 shrink-0 items-center gap-3 border-b bg-background/95 px-3 text-xs"
    >
      <ProjectSelectorPopover
        activeProject={project ?? null}
        onSelect={() => {
          /* Project selector handles state mutation directly */
        }}
      />

      {project && !isFolderless && project.root_paths.length > 0 && (
        <div className="flex min-w-0 items-center gap-1 text-muted-foreground">
          <span className="truncate">{basename(project.root_paths[0]!)}</span>
          {project.root_paths.length > 1 && (
            <span className="text-2xs">+{project.root_paths.length - 1}</span>
          )}
        </div>
      )}

      {isFolderless && (
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => setEditOpen(true)}
                className="h-6 gap-1 px-2 text-2xs font-normal text-muted-foreground hover:text-primary"
                aria-label={t('workbench.addFolders')}
              >
                <FolderPlus className="h-3 w-3" />
                {t('workbench.addFolders')}
              </Button>
            </TooltipTrigger>
            <TooltipContent>{t('workbench.addFoldersHint')}</TooltipContent>
          </Tooltip>
        </TooltipProvider>
      )}

      {project && status && (
        <div className="flex items-center gap-1 text-2xs text-muted-foreground">
          {status.branch && (
            <span className="flex items-center gap-1">
              <GitBranch className="h-3 w-3" />
              {status.branch}
            </span>
          )}
          {typeof status.dirty_count === 'number' && status.dirty_count > 0 && (
            <span className="flex items-center gap-1 text-warning">
              <AlertCircle className="h-3 w-3" />
              {status.dirty_count}
            </span>
          )}
        </div>
      )}

      <div className="ml-auto flex items-center gap-2">
        {activeModelId && (
          <span className="text-2xs text-muted-foreground" title={activeModelId}>
            {/* F10: short display id (model-picker's shortModelId is private
                to that component; basename() is the identical one-liner). */}
            {basename(activeModelId)}
          </span>
        )}
        <span className="text-2xs text-muted-foreground">
          <kbd className="rounded border bg-muted px-1 py-0.5 text-2xs">⌘K</kbd>{' '}
          {t('workbench.commandHint')}
        </span>
      </div>

      {editOpen && project && (
        <EditProjectDialog project={project} open={editOpen} onOpenChange={setEditOpen} />
      )}
    </header>
  )
}
