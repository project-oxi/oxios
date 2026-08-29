// session-artifacts — extract renderable artifacts from the chat transcript.
//
// Artifacts are fenced code blocks whose language maps to a renderable
// ArtifactType (html / svg / mermaid / jsx / tsx). The transcript stores them
// inside assistant `message.content`, so both the PreviewStage and the stage
// emergence observer need the same lightweight scan — one implementation here
// keeps their "latest artifact" identity in lockstep with what the markdown
// pipeline renders inline (same languageToArtifactType mapping).

import type { ChatMessage } from '@/types'
import { type ArtifactMeta, type ArtifactType, languageToArtifactType } from '@/types/artifact'

export interface SessionArtifact {
  /** Stable identity: messageId + occurrence ordinal + type. */
  key: string
  meta: ArtifactMeta
  /** Raw code, title directive included (ArtifactRenderer parses it). */
  code: string
  /** Owning message index in messages[] (ordering cue). */
  messageIndex: number
}

const FENCE_RE = /```([a-zA-Z0-9_+-]*)([^\n]*)\n([\s\S]*?)```/g

/** Scan assistant messages (in transcript order) and return every completed
 *  renderable artifact. `generating` messages are skipped — a streaming fence
 *  is not a completed artifact yet. Pure. */
export function extractSessionArtifacts(messages: ChatMessage[]): SessionArtifact[] {
  const out: SessionArtifact[] = []
  messages.forEach((m, messageIndex) => {
    if (m?.role !== 'assistant') return
    if (m.generating === true) return
    const content = m.content ?? ''
    if (!content.includes('```')) return
    FENCE_RE.lastIndex = 0
    let ordinal = 0
    for (;;) {
      const match = FENCE_RE.exec(content)
      if (match === null) break
      const lang = match[1] ?? ''
      const info = match[2] ?? ''
      const code = match[3] ?? ''
      const type: ArtifactType | undefined = languageToArtifactType(lang)
      if (!type) continue
      // Title directive: first line of the info string after the language
      // (e.g. ```svg title=Logo) or a leading comment line handled by the
      // renderer itself — we carry the raw info string as a fallback title.
      const title = info.trim() === '' ? undefined : info.trim()
      const meta: ArtifactMeta = {
        messageId: m.id,
        type,
        title,
        language: lang,
        source: 'language',
        ordinal,
      }
      out.push({
        key: `${m.id}:${ordinal}:${type}`,
        meta,
        code,
        messageIndex,
      })
      ordinal += 1
    }
  })
  return out
}

/** The most recent completed renderable artifact, or null. */
export function findLatestSessionArtifact(messages: ChatMessage[]): SessionArtifact | null {
  const all = extractSessionArtifacts(messages)
  return all.length === 0 ? null : all[all.length - 1]!
}
