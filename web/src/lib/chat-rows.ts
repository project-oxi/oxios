// chat-rows — pure row model for the virtualized chat list (LobeHub borrow).
//
// Flattens the conversation into a heterogeneous row array consumed by virtua's
// <VList>: an optional collapse bar (older messages folded past a threshold),
// the message rows, and any active intervention cards (interview / tool
// approval / path access). `index` on a message row is its position in the
// FULL messages array — used to derive assistantIndex and minimap jumps.

import type { ChatMessage, CompressionInfo } from '@/types'

/** One renderable row in the virtualized chat list. */
export type ChatRow =
  | { kind: 'empty' }
  | {
      kind: 'collapse-bar'
      count: number
      foldedMessages: ChatMessage[]
      compression: CompressionInfo | null
    }
  | { kind: 'message'; message: ChatMessage; index: number }
  | { kind: 'interview' }
  | { kind: 'tool-approval' }
  | { kind: 'path-access' }
  /** Loading shimmer placeholder (Task 10) — emitted while a session history
   *  fetch is in flight and no messages have arrived yet. */
  | { kind: 'skeleton' }

export interface BuildChatRowsOptions {
  messages: ChatMessage[]
  /** Whether the collapse group is expanded (show all messages). */
  expanded: boolean
  /** Message count above which older messages collapse. */
  collapseThreshold: number
  /** Number of recent messages kept visible when collapsed. */
  visibleTail: number
  hasInterview: boolean
  hasToolApproval: boolean
  hasPathAccess: boolean
  /** LLM compression summary for the session (null = not generated yet). */
  compression: CompressionInfo | null
  /** True while a session history fetch is in flight (Task 10). When true and
   *  `messages` is empty, the row list collapses to a single `skeleton` row so
   *  the UI can render its shimmer placeholder instead of an empty pane. */
  isLoadingSession?: boolean
}

export function buildChatRows(opts: BuildChatRowsOptions): ChatRow[] {
  const { messages, expanded, collapseThreshold, visibleTail, compression } = opts
  const hasCard = opts.hasInterview || opts.hasToolApproval || opts.hasPathAccess

  // Task 10: while the session history fetch is in flight and nothing has
  // arrived yet, show the shimmer skeleton instead of the "empty" hint. Once
  // any message lands (or the fetch fails), the normal empty/message path
  // takes over.
  if (messages.length === 0 && !hasCard) {
    return opts.isLoadingSession ? [{ kind: 'skeleton' }] : [{ kind: 'empty' }]
  }

  const rows: ChatRow[] = []
  const collapseCount = messages.length > collapseThreshold ? messages.length - visibleTail : 0

  if (collapseCount > 0) {
    rows.push({
      kind: 'collapse-bar',
      count: collapseCount,
      foldedMessages: messages.slice(0, collapseCount),
      compression,
    })
    const start = expanded ? 0 : collapseCount
    for (let i = start; i < messages.length; i++) {
      rows.push({ kind: 'message', message: messages[i]!, index: i })
    }
  } else {
    for (let i = 0; i < messages.length; i++) {
      rows.push({ kind: 'message', message: messages[i]!, index: i })
    }
  }

  if (opts.hasInterview) rows.push({ kind: 'interview' })
  if (opts.hasToolApproval) rows.push({ kind: 'tool-approval' })
  if (opts.hasPathAccess) rows.push({ kind: 'path-access' })
  return rows
}
