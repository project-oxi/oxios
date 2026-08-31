import { useMutation, useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { AlertTriangle, ExternalLink, Skull } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { AgentBudgetBar } from '@/components/agent/agent-budget-bar'
import { AgentHeader } from '@/components/agent/agent-header'
import { AgentLogs as AgentLogsComponent } from '@/components/agent/agent-logs'
import { ExecutionTrace } from '@/components/agent/execution-trace'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useAgentDetail, useAgentLogs, useAgentTrace } from '@/hooks/use-agent-trace'
import { api } from '@/lib/api-client'
import { defaultAgentSearch } from '@/routes/agents'

export const Route = createFileRoute('/agents/$agentId')({
  component: AgentDetailPage,
})

function AgentDetailPage() {
  const { t } = useTranslation()
  const { agentId } = Route.useParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const { data: agent, isLoading, isError, refetch } = useAgentDetail(agentId)
  const { data: trace, isLoading: traceLoading } = useAgentTrace(agentId)
  const { data: logs } = useAgentLogs(agentId)

  const killMutation = useMutation({
    mutationFn: () => api.post(`/api/agents/${agentId}/kill`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['agents'] })
      queryClient.invalidateQueries({ queryKey: ['agents', 'detail', agentId] })
      navigate({ to: '/agents', search: { ...defaultAgentSearch } })
    },
  })

  if (isLoading) return <LoadingCards count={3} />
  if (isError) return <ErrorState onRetry={() => refetch()} />
  if (!agent) return <p className="text-muted-foreground">{t('agents.notFound')}</p>

  return (
    <div className="space-y-6">
      <AgentHeader
        agent={agent}
        onBack={() => navigate({ to: '/agents', search: { ...defaultAgentSearch } })}
      >
        <Button
          variant="destructive"
          size="sm"
          onClick={() => {
            if (confirm(t('agents.terminateConfirm'))) killMutation.mutate()
          }}
          disabled={killMutation.isPending || agent.status?.toLowerCase() === 'stopped'}
        >
          <Skull className="h-4 w-4 mr-1" /> {t('agents.terminate')}
        </Button>
      </AgentHeader>

      {/* Error message */}
      {agent.error && (
        <Card className="border-destructive/50 bg-destructive/5">
          <CardContent className="pt-4">
            <div className="flex items-start gap-2 text-sm text-destructive">
              <AlertTriangle className="h-4 w-4 mt-0.5 shrink-0" />
              <div>
                <p className="font-medium">{t('agents.error')}</p>
                <p className="text-xs font-mono mt-1 whitespace-pre-wrap break-all">
                  {agent.error}
                </p>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Meta info */}
      <Card>
        <CardContent className="pt-4 space-y-3">
          <div className="grid gap-2 md:grid-cols-3 text-sm">
            {agent.project_id && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.project')}:</span>
                <span className="text-xs font-mono">{agent.project_id.slice(0, 8)}...</span>
              </div>
            )}
            {(agent as { session_id?: string | null }).session_id && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.session')}:</span>
                <Button
                  variant="link"
                  className="h-auto p-0 text-xs"
                  onClick={() =>
                    navigate({
                      to: '/studio/sessions/$sessionId',
                      params: { sessionId: (agent as { session_id: string }).session_id },
                    })
                  }
                >
                  {(agent as { session_id: string }).session_id.slice(0, 12)}...
                </Button>
              </div>
            )}
            <div className="flex items-center gap-1">
              <span className="text-muted-foreground">{t('agents.created')}:</span>
              <span>{new Date(agent.created_at).toLocaleString()}</span>
            </div>
            {agent.started_at && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.startedAt')}:</span>
                <span>{new Date(agent.started_at).toLocaleString()}</span>
              </div>
            )}
            {agent.completed_at && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.completedAt')}:</span>
                <span>{new Date(agent.completed_at).toLocaleString()}</span>
              </div>
            )}
            {agent.steps_completed > 0 && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.stepsCompleted')}:</span>
                <span>
                  {agent.steps_completed}
                  {agent.steps_total ? ` / ${agent.steps_total}` : ''}
                </span>
              </div>
            )}
            {agent.model_id && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.model')}:</span>
                <span className="text-xs font-mono">{agent.model_id}</span>
              </div>
            )}
            {agent.tokens_used > 0 && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.tokens')}:</span>
                <span>{agent.tokens_used.toLocaleString()}</span>
              </div>
            )}
            {agent.cost_usd > 0 && (
              <div className="flex items-center gap-1">
                <span className="text-muted-foreground">{t('agents.cost')}:</span>
                <span>${agent.cost_usd.toFixed(4)}</span>
              </div>
            )}
          </div>
          <AgentBudgetBar agent={agent} />
        </CardContent>
      </Card>

      {/* Tabs */}
      <Tabs defaultValue="trace" className="space-y-4">
        <TabsList>
          <TabsTrigger value="trace">{t('agents.trace')}</TabsTrigger>
          <TabsTrigger value="logs">{t('agents.logs')}</TabsTrigger>
        </TabsList>
        <TabsContent value="trace">
          <div className="flex items-center justify-between mb-2">
            <h3 className="text-lg font-semibold">{t('agents.executionTrace')}</h3>
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                navigate({
                  to: '/agents/$agentId/trace',
                  params: { agentId },
                })
              }
            >
              <ExternalLink className="h-4 w-4 mr-1" /> {t('agents.traceFullscreen')}
            </Button>
          </div>
          <ExecutionTrace trace={trace} isLoading={traceLoading} />
        </TabsContent>
        <TabsContent value="logs">
          <AgentLogsComponent logs={logs} />
        </TabsContent>
      </Tabs>
    </div>
  )
}
