// ProjectSelectorPopover — the project picker anchored in the workbench
// header (Task 7, design §7.3).
//
// Replaces the legacy `projects` page as the canonical project picker:
// "No project" detaches the active session (`setActiveProject(null)`),
// every project is listed via the same `useProjects` query that drives
// the sidebar, and create / edit-folder entries jump to the existing
// `CreateProjectDialog` / `EditProjectDialog` so the picker does not
// duplicate their logic.

import { FolderPlus, Plus, Settings } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CreateProjectDialog } from '@/components/project/create-project-dialog'
import { EditProjectDialog } from '@/components/project/edit-project-dialog'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useProjects } from '@/hooks/use-projects'
import { useChatStore } from '@/stores/chat'
import type { Project } from '@/types'

interface ProjectSelectorPopoverProps {
  /** Currently bound project (or null for the "No project" state). */
  activeProject: Project | null
  /** Called when the user picks a project (or null to detach). */
  onSelect: (project: Project | null) => void
}

/** Resolve the project's display name (basename of the first root or
 *  "Unnamed"). Pure. */
function projectLabel(p: Project | null): string {
  if (!p) return 'workbench.noProject'
  return p.name
}

export function ProjectSelectorPopover({ activeProject, onSelect }: ProjectSelectorPopoverProps) {
  const { t } = useTranslation()
  const { data: projectsData } = useProjects()
  const projects = projectsData?.items ?? []
  const [createOpen, setCreateOpen] = useState(false)
  const [editProject, setEditProject] = useState<Project | null>(null)
  const detachSession = useChatStore((s) => s.setActiveProject)

  const handleSelect = (project: Project | null) => {
    onSelect(project)
    if (project === null) {
      // Detach the session: clears activeProjectId + activeSessionId so the
      // next send runs without a project binding (project_id: '').
      detachSession(null)
    }
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 px-2 text-xs font-normal"
            aria-label={t('workbench.projectSelector.label')}
          >
            {t(projectLabel(activeProject))}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-[200px]">
          <DropdownMenuLabel>{t('workbench.projectSelector.title')}</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => handleSelect(null)}>
            <span className="text-muted-foreground">{t('workbench.noProject')}</span>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          {projects.map((p) => (
            <DropdownMenuItem key={p.id} onSelect={() => handleSelect(p)}>
              <span className={p.id === activeProject?.id ? 'font-medium' : ''}>{p.name}</span>
            </DropdownMenuItem>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => setCreateOpen(true)}>
            <Plus className="mr-2 h-3.5 w-3.5" />
            {t('workbench.projectSelector.create')}
          </DropdownMenuItem>
          {activeProject && (
            <DropdownMenuItem onSelect={() => setEditProject(activeProject)}>
              <FolderPlus className="mr-2 h-3.5 w-3.5" />
              {t('workbench.projectSelector.editFolders')}
            </DropdownMenuItem>
          )}
          {activeProject && (
            <DropdownMenuItem onSelect={() => setEditProject(activeProject)}>
              <Settings className="mr-2 h-3.5 w-3.5" />
              {t('workbench.projectSelector.settings')}
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>

      <CreateProjectDialog open={createOpen} onOpenChange={setCreateOpen} />
      {editProject && (
        <EditProjectDialog
          project={editProject}
          open={!!editProject}
          onOpenChange={(o) => !o && setEditProject(null)}
        />
      )}
    </>
  )
}
