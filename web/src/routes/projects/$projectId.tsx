import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, Edit, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { DeleteProjectDialog } from '@/components/project/delete-project-dialog'
import { EditProjectDialog } from '@/components/project/edit-project-dialog'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useProject, useProjectMemories } from '@/hooks/use-projects'
import { formatRelativeTime } from '@/lib/utils'
import type { Project } from '@/types'

export const Route = createFileRoute('/projects/$projectId')({
  component: ProjectDetailPage,
})

const SOURCE_COLORS: Record<string, string> = {
  manual: 'bg-success-subtle text-success',
  auto_detected: 'bg-warning-subtle text-warning',
}

function formatDate(iso: string) {
  return new Date(iso).toLocaleString()
}

function ProjectPathsCard({ project }: { project: Project }) {
  const { t } = useTranslation()

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t('projects.paths')}</CardTitle>
      </CardHeader>
      <CardContent>
        {project.paths && project.paths.length > 0 ? (
          <div className="space-y-2">
            {project.paths.map((path, i) => (
              <div key={i} className="flex items-center gap-2 text-sm">
                <span>📁</span>
                <code className="text-xs bg-muted px-2 py-1 rounded font-mono truncate">
                  {path}
                </code>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">{t('projects.noPaths')}</p>
        )}
      </CardContent>
    </Card>
  )
}

function ProjectDetailsCard({ project }: { project: Project }) {
  const { t } = useTranslation()
  const sourceColor = SOURCE_COLORS[project.source ?? 'manual'] ?? SOURCE_COLORS.manual

  const details = [
    { label: t('projects.source'), value: project.source ?? 'manual' },
    {
      label: t('projects.memoryVisible'),
      value: project.memory_visible ? t('common.yes') : t('common.no'),
    },
    { label: t('projects.createdAt'), value: formatDate(project.created_at) },
    {
      label: t('projects.updatedAt'),
      value: formatDate(project.updated_at ?? project.created_at),
    },
    {
      label: t('projects.lastActive'),
      value: formatRelativeTime(
        project.last_active_at ?? project.updated_at ?? project.created_at,
        t,
      ),
    },
  ]

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t('projects.details')}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="space-y-3">
          {/* Description */}
          {project.description && (
            <div>
              <p className="text-xs text-muted-foreground mb-1">{t('projects.description')}</p>
              <p className="text-sm">{project.description}</p>
            </div>
          )}

          {/* Tags */}
          {project.tags && project.tags.length > 0 && (
            <div>
              <p className="text-xs text-muted-foreground mb-1">{t('projects.tags')}</p>
              <div className="flex flex-wrap gap-1">
                {project.tags.map((tag) => (
                  <Badge key={tag} variant="secondary" className="text-xs">
                    {tag}
                  </Badge>
                ))}
              </div>
            </div>
          )}

          {/* Other details */}
          <div className="grid gap-2 text-sm">
            {details.map((d) => (
              <div key={d.label} className="flex items-center justify-between">
                <span className="text-muted-foreground text-xs">{d.label}</span>
                <span
                  className={
                    d.label === 'Source'
                      ? `text-2xs px-1.5 py-0.5 rounded ${sourceColor}`
                      : 'text-xs'
                  }
                >
                  {d.value}
                </span>
              </div>
            ))}
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function ProjectMemoriesCard({ project }: { project: Project }) {
  const { t } = useTranslation()
  const { data, isLoading } = useProjectMemories(project.id)

  const memories = Array.isArray(data?.items) ? data.items : []

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base flex items-center gap-2">
          {t('projects.memories')}
          {memories.length > 0 && (
            <Badge variant="secondary" className="text-xs">
              {memories.length}
            </Badge>
          )}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="space-y-2">
            {[1, 2, 3].map((i) => (
              <div key={i} className="h-8 bg-muted rounded animate-pulse" />
            ))}
          </div>
        ) : memories.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t('projects.noMemories')}</p>
        ) : (
          <div className="space-y-2">
            {memories.map((mem: any) => (
              <div key={mem.id} className="flex items-center gap-2 p-2 rounded bg-muted/50">
                <Badge variant="outline" className="text-2xs shrink-0">
                  episode
                </Badge>
                <p className="text-xs truncate flex-1 font-mono">{mem.id}</p>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function ProjectActivityCard() {
  const { t } = useTranslation()

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t('projects.activity')}</CardTitle>
      </CardHeader>
      <CardContent>
        <p className="text-sm text-muted-foreground">{t('projects.activityDesc')}</p>
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
        <div className="flex items-center gap-2 flex-1 min-w-0">
          <span className="text-2xl">{project.emoji ?? '📦'}</span>
          <h1 className="text-2xl font-bold truncate">{project.name}</h1>
        </div>
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

      {/* Description */}
      {project.description && (
        <p className="text-muted-foreground text-sm mt-1">{project.description}</p>
      )}

      {/* Tabs */}
      <Tabs defaultValue="details" className="space-y-4">
        <TabsList>
          <TabsTrigger value="details">{t('projects.tabs.details')}</TabsTrigger>
          <TabsTrigger value="paths">{t('projects.tabs.paths')}</TabsTrigger>
          <TabsTrigger value="memories">{t('projects.tabs.memories')}</TabsTrigger>
          <TabsTrigger value="activity">{t('projects.tabs.activity')}</TabsTrigger>
        </TabsList>

        <TabsContent value="details">
          <ProjectDetailsCard project={project} />
        </TabsContent>

        <TabsContent value="paths">
          <ProjectPathsCard project={project} />
        </TabsContent>

        <TabsContent value="memories">
          <ProjectMemoriesCard project={project} />
        </TabsContent>

        <TabsContent value="activity">
          <ProjectActivityCard />
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
