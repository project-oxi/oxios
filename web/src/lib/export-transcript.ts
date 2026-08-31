/**
 * `/export` — download the active session transcript as Markdown.
 *
 * Client-side only: fetches the session detail (same endpoint the history
 * loader uses), renders user/assistant pairs, and triggers a file download
 * via a temporary blob anchor.
 */
import { api } from '@/lib/api-client'

interface ExportSessionMessage {
  content: string
}

interface ExportSessionDetail {
  id: string
  created_at?: string
  user_messages: ExportSessionMessage[]
  agent_responses: ExportSessionMessage[]
}

function buildMarkdown(session: ExportSessionDetail): string {
  const lines: string[] = []
  lines.push('# Oxios conversation')
  lines.push('')
  lines.push(`- Session: \`${session.id}\``)
  if (session.created_at) lines.push(`- Started: ${session.created_at}`)
  lines.push(`- Exported: ${new Date().toISOString()}`)
  lines.push('')

  const count = Math.max(session.user_messages.length, session.agent_responses.length)
  for (let i = 0; i < count; i++) {
    const user = session.user_messages[i]
    const agent = session.agent_responses[i]
    if (user) {
      lines.push('## User')
      lines.push('')
      lines.push(user.content)
      lines.push('')
    }
    if (agent) {
      lines.push('## Assistant')
      lines.push('')
      lines.push(agent.content)
      lines.push('')
    }
  }
  return lines.join('\n')
}

/** Fetch `sessionId`, download it as Markdown, and resolve to the filename. */
export async function exportTranscriptMarkdown(sessionId: string): Promise<string> {
  const session = await api.get<ExportSessionDetail>(
    `/api/sessions/${encodeURIComponent(sessionId)}`,
  )
  const markdown = buildMarkdown(session)
  const filename = `oxios-session-${sessionId.slice(0, 8)}.md`

  const blob = new Blob([markdown], { type: 'text/markdown;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  URL.revokeObjectURL(url)
  return filename
}
