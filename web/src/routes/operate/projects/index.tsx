import { createFileRoute } from '@tanstack/react-router'
import { Package, Plus } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CreateProjectDialog } from '@/components/project/create-project-dialog'
import { DeleteProjectDialog } from '@/components/project/delete-project-dialog'
import { EditProjectDialog } from '@/components/project/edit-project-dialog'
import { ProjectCard } from '@/components/project/project-card'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { RefreshButton } from '@/components/shared/refresh-button'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useProjects } from '@/hooks/use-projects'
import type { Project } from '@/types'

export const Route = createFileRoute('/operate/projects/')({ component: ProjectsPage })

function ProjectsPage() {
  const { t } = useTranslation()
  const [search, setSearch] = useState('')
  const [editTarget, setEditTarget] = useState<Project | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<Project | null>(null)
  const [showCreate, setShowCreate] = useState(false)

  const { data, isLoading, isError, refetch, isFetching } = useProjects(search || undefined)

  const projects = Array.isArray(data?.items) ? data.items : []

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('projects.title')}
        subtitle={t('projects.desc')}
        actions={
          <>
            <Button onClick={() => setShowCreate(true)} size="sm">
              <Plus className="h-4 w-4" />
              {t('projects.new')}
            </Button>
            <RefreshButton onClick={() => refetch()} isFetching={isFetching} />
          </>
        }
      />

      {/* Search */}
      <div className="flex items-center gap-2">
        <Input
          placeholder={t('projects.search')}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="max-w-sm"
        />
        {search && (
          <span className="text-xs text-muted-foreground">
            {t('projects.resultsCount', { count: data?.total ?? 0 })}
          </span>
        )}
      </div>

      {/* Content */}
      {isLoading ? (
        <LoadingCards count={6} />
      ) : isError ? (
        <ErrorState onRetry={() => refetch()} />
      ) : projects.length === 0 ? (
        <EmptyState
          icon={<Package className="h-10 w-10" />}
          title={search ? t('projects.noResults') : t('projects.empty')}
          description={search ? t('projects.noResultsDesc') : t('projects.emptyDesc')}
          action={
            !search ? (
              <Button onClick={() => setShowCreate(true)}>
                <Plus className="h-4 w-4" />
                {t('projects.new')}
              </Button>
            ) : undefined
          }
        />
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {projects.map((project) => (
            <ProjectCard
              key={project.id}
              project={project}
              onEdit={setEditTarget}
              onDelete={setDeleteTarget}
            />
          ))}
        </div>
      )}

      {/* Dialogs */}
      <CreateProjectDialog open={showCreate} onOpenChange={setShowCreate} />
      <EditProjectDialog
        project={editTarget}
        open={!!editTarget}
        onOpenChange={(open) => !open && setEditTarget(null)}
        onSuccess={() => setEditTarget(null)}
      />
      <DeleteProjectDialog
        project={deleteTarget}
        open={!!deleteTarget}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
      />
    </div>
  )
}
