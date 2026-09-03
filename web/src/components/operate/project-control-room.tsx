import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, FolderOpen, Wrench } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { OperateInspector } from '@/components/operate/operate-inspector'
import { ProjectIssuesTab } from '@/components/project/project-issues-tab'
import { ProjectMilestonesTab } from '@/components/project/project-milestones-tab'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Select } from '@/components/ui/select'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { type OperateRun, useOperateProjectContext } from '@/hooks/use-operate'
import { api } from '@/lib/api-client'
import { formatRelativeTime } from '@/lib/utils'

/**
 * Project control room (design §6) — the operate view of one project.
 * The header and status line come ONLY from the operate context
 * endpoint; issues and milestones reuse the canonical tabs; roots and
 * active-run sections exist only when the backend records them. Unknown
 * project → error state with a back link, never a fake page.
 */
export function ProjectControlRoom({ projectId }: { projectId: string }) {
  const { t } = useTranslation()
  const context = useOperateProjectContext(projectId)
  const [selectedRun, setSelectedRun] = useState<OperateRun | null>(null)

  if (context.isLoading) return <LoadingCards count={4} />

  if (context.isError || !context.data) {
    return (
      <div className="space-y-4 animate-fade-in-up">
        <BackLink />
        <ErrorState onRetry={() => context.refetch()} />
      </div>
    )
  }

  const { project, roots, issuesOpen, milestones, activeRuns } = context.data

  return (
    <div className="space-y-6 animate-fade-in-up">
      {/* Header */}
      <div className="flex items-start gap-4">
        <BackLink />
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-2xl font-bold">{project.name}</h1>
          {/* Outcome/status line — only what the context endpoint records. */}
          <p className="mt-0.5 text-sm text-muted-foreground">
            {t('operate.project.statusLine', {
              issues: issuesOpen.length,
              milestones: milestones.length,
              runs: activeRuns.length,
            })}
          </p>
          {project.rootPaths.length > 0 && (
            <ul className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5">
              {project.rootPaths.map((p) => (
                <li key={p} className="truncate font-mono text-2xs text-muted-foreground" title={p}>
                  {p}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {project.instructions && (
        <p className="rounded-lg border bg-muted/40 px-3 py-2 text-sm text-muted-foreground">
          {project.instructions}
        </p>
      )}

      <ExecutionRecipeCard projectId={projectId} />

      {/* Roots — present only when the backend records root liveness. */}
      {roots && roots.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="flex items-center gap-2 text-base">
              <FolderOpen className="h-4 w-4" aria-hidden="true" />
              {t('operate.project.roots')}
            </CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            <ul className="space-y-1">
              {roots.map((root) => (
                <li
                  key={root.path}
                  className="flex items-center justify-between gap-2 rounded-md border px-2.5 py-1.5 text-xs"
                >
                  <span className="truncate font-mono" title={root.path}>
                    {root.path}
                  </span>
                  <span className="flex shrink-0 items-center gap-2 text-muted-foreground">
                    {root.branch && <span className="font-mono">{root.branch}</span>}
                    {typeof root.dirty === 'boolean' && (
                      <span>
                        {root.dirty ? t('operate.project.dirty') : t('operate.project.clean')}
                      </span>
                    )}
                  </span>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      )}

      {/* Active runs — rows only when the backend records running work. */}
      {activeRuns.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">{t('operate.project.activeRuns')}</CardTitle>
          </CardHeader>
          <CardContent className="pt-0">
            <ul className="space-y-1" data-testid="active-runs">
              {activeRuns.map((run) => (
                <li key={run.id}>
                  <button
                    type="button"
                    onClick={() => setSelectedRun(run)}
                    className="flex w-full items-center gap-2 rounded-md border px-2.5 py-1.5 text-left text-xs transition-colors hover:bg-accent/50"
                  >
                    <span className="min-w-0 flex-1 truncate font-medium">{run.name}</span>
                    <span className="shrink-0 text-muted-foreground">{run.status}</span>
                    {run.startedAt && (
                      <span className="shrink-0 font-mono text-2xs tabular-nums text-muted-foreground">
                        {formatRelativeTime(run.startedAt, t)}
                      </span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      )}

      {/* Project-scoped work tracking — canonical tabs, unchanged props. */}
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

      {selectedRun && (
        <OperateInspector
          key={selectedRun.id}
          run={selectedRun}
          onClose={() => setSelectedRun(null)}
          variant="dialog"
        />
      )}
    </div>
  )
}

function BackLink() {
  const { t } = useTranslation()
  return (
    <Button variant="ghost" size="icon" asChild className="shrink-0">
      <Link to="/operate/projects" aria-label={t('operate.allProjects')}>
        <ArrowLeft className="h-4 w-4" aria-hidden="true" />
      </Link>
    </Button>
  )
}

/**
 * Execution recipe binding (execution-recipe coding host design). One
 * select per project: the binding is written straight to the shared config
 * (single config Arc) + config.toml and observed by the recipe resolver on
 * the project's NEXT agent turn — no restart. `null` clears the binding.
 */
function ExecutionRecipeCard({ projectId }: { projectId: string }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const [error, setError] = useState(false)

  const bound = useQuery({
    queryKey: ['project-recipe', projectId],
    queryFn: async () => {
      const cfg = await api.get<{
        execution_recipe?: { project_bindings?: Record<string, string> }
      }>('/api/config')
      return cfg.execution_recipe?.project_bindings?.[projectId] ?? null
    },
  })

  const save = useMutation({
    mutationFn: (recipe: string) =>
      api
        .put<{ project_id: string; recipe: string | null }>(
          `/api/projects/${projectId}/execution-recipe`,
          { recipe: recipe === '' ? null : recipe },
        )
        .then((res) => res),
    onSuccess: (data) => {
      setError(false)
      qc.setQueryData(['project-recipe', projectId], data.recipe)
    },
    onError: () => setError(true),
  })

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Wrench className="h-4 w-4" aria-hidden="true" />
          {t('operate.project.executionRecipe')}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-1.5">
        <Select
          value={bound.data ?? ''}
          onValueChange={(v: string) => save.mutate(v)}
          disabled={bound.isLoading || save.isPending}
          placeholder={
            save.isPending ? t('operate.project.recipeSaving') : t('operate.project.recipeNone')
          }
          aria-label={t('operate.project.executionRecipe')}
          options={[
            { value: '', label: t('operate.project.recipeNone') },
            { value: 'coding-omp-v1', label: 'coding-omp-v1' },
            { value: 'general-v1', label: 'general-v1' },
          ]}
          className="w-72"
        />
        {error && (
          <p className="text-xs text-destructive">{bound.error ? String(bound.error) : ''}</p>
        )}
      </CardContent>
    </Card>
  )
}
