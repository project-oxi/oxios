import { useQuery } from '@tanstack/react-query'
import { MessageSquare } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { EmptyState } from '@/components/shared/empty-state'
import { api } from '@/lib/api-client'
import { formatRelativeTime } from '@/lib/relative-time'
import { useChatStore } from '@/stores/chat'
import type { Session } from '@/types'

/**
 * Empty state shown when the chat has no messages yet.
 *
 * Greets the user and lists recent sessions for quick continuation.
 */
export function EmptyChatState() {
  const { t } = useTranslation()
  const loadSession = useChatStore((s) => s.loadSession)

  const { data: sessionsData } = useQuery({
    queryKey: ['sessions-recent'],
    queryFn: () => api.get<{ items: Session[]; total: number }>('/api/sessions'),
    refetchInterval: 30_000,
  })

  const sessions: Session[] = Array.isArray(sessionsData?.items)
    ? sessionsData.items.slice(0, 8)
    : []

  return (
    <EmptyState
      title={t('chat.greeting')}
      description={sessions.length > 0 ? t('chat.emptyHint') : undefined}
      className="px-4"
    >
      {sessions.length > 0 ? (
        <div className="mt-8 w-full max-w-md space-y-1 text-left">
          <p className="text-xs font-medium text-muted-foreground mb-2">
            {t('chat.recentSessions')}
          </p>
          <div className="space-y-1">
            {sessions.map((s) => {
              const timeStr = formatRelativeTime(s.created_at, t)

              return (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => loadSession(s.id)}
                  className="flex items-center gap-3 w-full rounded-lg border bg-card px-3 py-2.5 text-left text-sm transition-all hover:bg-accent hover:border-primary/20 hover:shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                >
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
                    <MessageSquare className="h-4 w-4" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-foreground">{s.title ?? `${s.id.slice(0, 8)}…`}</p>
                    <p className="text-2xs text-muted-foreground">{timeStr}</p>
                  </div>
                </button>
              )
            })}
          </div>
        </div>
      ) : null}
    </EmptyState>
  )
}
