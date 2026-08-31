import { Link } from '@tanstack/react-router'
import { FolderOpen, Pencil, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { formatRelativeTime } from '@/lib/utils'
import type { Project } from '@/types'

interface ProjectCardProps {
  project: Project
  onEdit: (project: Project) => void
  onDelete: (project: Project) => void
}

/** Card: name + root count + instructions snippet (design §4.2). */
export function ProjectCard({ project, onEdit, onDelete }: ProjectCardProps) {
  const { t } = useTranslation()

  return (
    <div className="group rounded-lg border p-4 hover:bg-accent/30 transition-colors">
      {/* Header */}
      <div className="flex items-start justify-between gap-2 mb-2">
        <div className="flex items-center gap-2 min-w-0">
          <Link
            to="/operate/projects/$projectId"
            params={{ projectId: project.id }}
            className="font-semibold text-sm truncate hover:text-primary"
          >
            {project.name}
          </Link>
        </div>
        <div className="shrink-0 flex items-center gap-0.5">
          <button
            type="button"
            onClick={() => onEdit(project)}
            className="p-2 rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t('common.edit')}
          >
            <Pencil className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={() => onDelete(project)}
            className="p-2 rounded-md text-muted-foreground hover:bg-muted hover:text-destructive focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t('common.delete')}
          >
            <Trash2 className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* Folders */}
      <p className="text-2xs text-muted-foreground font-mono truncate mb-2 flex items-center gap-1">
        <FolderOpen className="h-3 w-3 shrink-0" />
        {project.root_paths.length > 0
          ? `${project.root_paths[0]}${project.root_paths.length > 1 ? ` +${project.root_paths.length - 1}` : ''}`
          : t('projects.noFoldersYet')}
      </p>

      {/* Instructions snippet */}
      {project.instructions && (
        <p className="text-xs text-muted-foreground mb-2 line-clamp-2">{project.instructions}</p>
      )}

      {/* Footer */}
      <div className="flex items-center justify-between text-2xs text-muted-foreground">
        <span>{formatRelativeTime(project.last_active_at, t)}</span>
        <Link
          to="/operate/projects/$projectId"
          params={{ projectId: project.id }}
          className="hover:text-primary font-medium"
        >
          {t('common.view')}
        </Link>
      </div>
    </div>
  )
}
