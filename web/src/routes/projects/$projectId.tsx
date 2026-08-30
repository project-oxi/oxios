import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, Edit, FolderOpen, FolderX, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { DeleteProjectDialog } from '@/components/project/delete-project-dialog'
import { EditProjectDialog } from '@/components/project/edit-project-dialog'
import { ProjectIssuesTab } from '@/components/project/project-issues-tab'
import { ProjectMilestonesTab } from '@/components/project/project-milestones-tab'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useProject, useProjectRootsStatus } from '@/hooks/use-projects'
import { formatRelativeTime } from '@/lib/utils'
import type { Project } from '@/types'

export const Route = createFileRoute('/projects/$projectId')({
  component: ProjectDetailPage,
})

/** Details tab: root list + instructions + record metadata (design §4.2). */
export function ProjectDetailsCard({ project }: { project: Project }) {
  const { t } = useTranslation()
  // Design §9: a stale project root is surfaced here — never silently
  // replaced by an ambient workspace. Null while the status query loads.
  const { data: rootStatus } = useProjectRootsStatus(project.id)
  const staleSet = new Set(
    (rootStatus ?? []).filter((r) => !r.exists || !r.is_dir).map((r) => r.path),
  )
  const details = [
    { label: t('projects.createdAt'), value: new Date(project.created_at).toLocaleString() },
    { label: t('projects.updatedAt'), value: new Date(project.updated_at).toLocaleString() },
    {
      label: t('projects.lastActive'),
      value: formatRelativeTime(project.last_active_at, t),
    },
  ]

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t('projects.details')}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="space-y-4">
          {/* Root folders */}
          <div>
            <p className="text-xs text-muted-foreground mb-2">{t('projects.folders')}</p>
            {project.root_paths.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t('projects.noFoldersYet')}</p>
            ) : (
              <div className="space-y-2">
                {project.root_paths.map((path) => {
                  const stale = staleSet.has(path)
                  return (
                    <div key={path} className="flex items-center gap-2 text-sm">
                      {stale ? (
                        <FolderX
                          className="h-3.5 w-3.5 shrink-0 text-destructive"
                          aria-hidden="true"
                        />
                      ) : (
                        <FolderOpen className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                      )}
                      <code className="text-xs bg-muted px-2 py-1 rounded font-mono truncate">
                        {path}
                      </code>
                      {stale && (
                        <span
                          className="shrink-0 inline-flex items-center rounded-full bg-destructive/10 px-2 py-0.5 text-2xs font-medium text-destructive"
                          title={t('projects.staleRootHint')}
                        >
                          {t('projects.staleRoot')}
                        </span>
                      )}
                    </div>
                  )
                })}
              </div>
            )}
          </div>

          {/* Instructions */}
          {project.instructions && (
            <div>
              <p className="text-xs text-muted-foreground mb-1">{t('projects.instructions')}</p>
              <p className="text-sm whitespace-pre-wrap">{project.instructions}</p>
            </div>
          )}

          {/* Metadata */}
          <div className="grid gap-2 text-sm">
            {details.map((d) => (
              <div key={d.label} className="flex items-center justify-between">
                <span className="text-muted-foreground text-xs">{d.label}</span>
                <span className="text-xs">{d.value}</span>
              </div>
            ))}
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function ProjectDetailPage() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { projectId } = Route.useParams()

  const { data: project, isLoading, isError, refetch } = useProject(projectId)

  const [showEdit, setShowEdit] = useState(false)
  const [showDelete, setShowDelete] = useState(false)

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />
  if (!project) return <p className="text-muted-foreground">{t('projects.notFound')}</p>

  return (
    <div className="space-y-6">
      {/* Back + Header */}
      <div className="flex items-center gap-4">
        <Button
          variant="ghost"
          size="icon"
          onClick={() => navigate({ to: '/projects' })}
          aria-label={t('common.back')}
        >
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <h1 className="text-2xl font-bold truncate flex-1">{project.name}</h1>
        <div className="flex items-center gap-1 shrink-0">
          <Button variant="outline" size="sm" onClick={() => setShowEdit(true)}>
            <Edit className="h-3 w-3" />
            {t('common.edit')}
          </Button>
          <Button variant="outline" size="sm" onClick={() => setShowDelete(true)}>
            <Trash2 className="h-3 w-3" />
            {t('common.delete')}
          </Button>
        </div>
      </div>

      {/* Details */}
      <ProjectDetailsCard project={project} />

      {/* Project-scoped work tracking */}
      <Tabs defaultValue="issues" className="space-y-4">
        <TabsList>
          <TabsTrigger value="issues">{t('projects.tabs.issues')}</TabsTrigger>
          <TabsTrigger value="milestones">{t('projects.tabs.milestones')}</TabsTrigger>
        </TabsList>

        <TabsContent value="issues">
          <ProjectIssuesTab projectId={projectId} />
        </TabsContent>

        <TabsContent value="milestones">
          <ProjectMilestonesTab projectId={projectId} />
        </TabsContent>
      </Tabs>

      {/* Dialogs */}
      <EditProjectDialog
        project={project}
        open={showEdit}
        onOpenChange={setShowEdit}
        onSuccess={() => {
          setShowEdit(false)
          refetch()
        }}
      />
      <DeleteProjectDialog project={project} open={showDelete} onOpenChange={setShowDelete} />
    </div>
  )
}
