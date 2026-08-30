import { useQuery } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { ArrowLeft, FolderKanban, MessageSquare } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useProjects } from '@/hooks/use-projects'
import { api } from '@/lib/api-client'
import type { SessionDetail } from '@/types'

export const Route = createFileRoute('/sessions/$sessionId')({
  component: SessionDetailPage,
})

function ProjectSelector({
  currentProjectId,
}: {
  sessionId?: string
  currentProjectId: string | null
}) {
  const { t } = useTranslation()
  const { data: projectsData } = useProjects()
  const projects = Array.isArray(projectsData?.items) ? projectsData.items : []
  const currentProject = projects.find((p) => p.id === currentProjectId)

  return (
    <div className="flex items-center gap-2">
      <FolderKanban className="h-4 w-4 text-muted-foreground shrink-0" />
      {currentProjectId && currentProject ? (
        <span className="text-sm">
          <span className="font-medium">{currentProject.name}</span>
        </span>
      ) : (
        <span className="text-xs text-muted-foreground">{t('sessions.noProject')}</span>
      )}
    </div>
  )
}

function SessionDetailPage() {
  const { t } = useTranslation()
  const { sessionId } = Route.useParams()
  const navigate = useNavigate()

  const {
    data: session,
    isLoading,
    isError,
    refetch,
  } = useQuery({
    queryKey: ['session', sessionId],
    queryFn: () => api.get<SessionDetail>(`/api/sessions/${sessionId}`),
  })

  if (isLoading) return <LoadingCards count={3} />
  if (isError) return <ErrorState onRetry={() => refetch()} />
  if (!session) return <p className="text-muted-foreground">{t('sessions.notFound')}</p>

  // Build interleaved messages from user_messages and agent_responses
  const messages: { role: 'user' | 'assistant'; content: string }[] = []
  const userMsgs: { content: string }[] = Array.isArray(session.user_messages)
    ? session.user_messages
    : []
  const agentMsgs: { content: string }[] = Array.isArray(session.agent_responses)
    ? session.agent_responses
    : []
  const maxLen = Math.max(userMsgs.length, agentMsgs.length)
  for (let i = 0; i < maxLen; i++) {
    const userMsg = userMsgs[i]
    const agentMsg = agentMsgs[i]
    if (userMsg != null) messages.push({ role: 'user', content: userMsg.content })
    if (agentMsg) messages.push({ role: 'assistant', content: agentMsg.content ?? '' })
  }

  const details = [
    { label: t('sessions.sessionId'), value: session.id },
    { label: t('sessions.userId'), value: session.user_id ?? '—' },
    { label: t('sessions.messages'), value: messages.length },
    { label: t('sessions.createdAt'), value: new Date(session.created_at).toLocaleString() },
    {
      label: t('sessions.updated'),
      value: session.updated_at ? new Date(session.updated_at).toLocaleString() : '—',
    },
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Button
          variant="ghost"
          size="icon"
          onClick={() => navigate({ to: '/sessions' })}
          aria-label={t('common.back')}
        >
          <ArrowLeft className="h-4 w-4" />
        </Button>
        <div className="flex-1">
          <h1 className="text-2xl font-bold">{t('sessions.sessionDetail')}</h1>
          <p className="text-muted-foreground font-mono text-xs">{sessionId}</p>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>{t('sessions.sessionInfo')}</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid gap-3 md:grid-cols-2">
            {/* Project row */}
            <div className="flex items-center justify-between rounded-lg border p-3 bg-muted/20">
              <span className="text-sm text-muted-foreground flex items-center gap-1">
                <FolderKanban className="h-3 w-3" />
                {t('sessions.project')}
              </span>
              <ProjectSelector currentProjectId={(session as any).project_id ?? null} />
            </div>
            {details.map((d) => (
              <div
                key={d.label}
                className="flex items-center justify-between rounded-lg border p-3"
              >
                <span className="text-sm text-muted-foreground">{d.label}</span>
                <span className="text-sm font-medium">{d.value}</span>
              </div>
            ))}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <MessageSquare className="h-4 w-4" /> {t('sessions.messages')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {messages.length > 0 ? (
            <div className="space-y-3">
              {messages.map((msg, i) => (
                <div key={`msg-${i}`} className="flex gap-3">
                  <Badge
                    variant={msg.role === 'user' ? 'default' : 'secondary'}
                    className="shrink-0 h-6"
                  >
                    {msg.role === 'user' ? t('chat.user') : t('chat.assistant')}
                  </Badge>
                  <div className="flex-1 rounded-lg bg-muted p-3">
                    <p className="text-sm whitespace-pre-wrap">{msg.content}</p>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{t('sessions.noMessages')}</p>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
