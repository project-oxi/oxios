// /studio/sessions/:sessionId — deep link to an existing Studio
// conversation (IA design §10.1). Loads the session into the same shared
// chat store the Studio shell consumes, then renders the identical shell
// — a session deep link is the same conversation, not a second viewer.

import { createFileRoute } from '@tanstack/react-router'
import { useEffect } from 'react'
import { StudioShell } from '@/components/studio/studio-shell'
import { useChatStore } from '@/stores/chat'

export const Route = createFileRoute('/studio/sessions/$sessionId')({
  component: StudioSessionRoute,
})

function StudioSessionRoute() {
  const { sessionId } = Route.useParams()
  const activeSessionId = useChatStore((s) => s.activeSessionId)

  useEffect(() => {
    if (sessionId && activeSessionId !== sessionId) {
      void useChatStore.getState().loadSession(sessionId)
    }
  }, [sessionId, activeSessionId])

  return <StudioShell />
}
