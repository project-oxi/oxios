// use-session-changes — flat list of file-change items derived from the
// active session's transcript (Task 7, design §7.2).
//
// Aggregation rule (tool registry aliases + standard edit/write/patch
// argument shapes — mirrors `InlineDiffViewer.isFileEditCall`):
//   * tool names matched case-insensitively across aliases:
//       edit, write, patch, apply_patch, multiedit, notebook_edit
//     (any tool whose apiName contains one of those substrings counts.)
//   * toolArgs are inspected for a path-like field: `path`, `file_path`,
//     `notebook_path`. First hit wins.
//   * diff text: `old_text`/`old_str`/`oldString` + `new_text`/`new_str`/
//     `newString` concatenated as a tiny unified-diff snippet; falls
//     back to `content` for write-style calls.
//   * status: 'pending' unless reviewedChangeIds contains the change id;
//     'reverted' after revertChange.
//   * id: `${messageId}::${toolCallId}` — stable across the same call.
//
// Order: chronological by message id (insertion order in messages[]),
// then tool id within a message so two edits on the same assistant turn
// keep their emitted order.

import { useMemo } from 'react'
import { useChatStore } from '@/stores/chat'
import { useWorkbenchStore } from '@/stores/workbench'
import type { ChatBlock, ChatMessage } from '@/types'

export interface ChangeItem {
  id: string
  path: string
  diff: string
  status: 'pending' | 'reviewed' | 'reverted'
  at: number
}

const EDIT_ALIASES = ['edit', 'write', 'patch', 'apply_patch', 'multiedit', 'notebook_edit']

function isEditTool(name: string | undefined): boolean {
  if (!name) return false
  const lower = name.toLowerCase()
  return EDIT_ALIASES.some((alias) => lower.includes(alias))
}

function pickPath(args: Record<string, unknown> | undefined): string | null {
  if (!args) return null
  for (const k of ['path', 'file_path', 'notebook_path']) {
    const v = args[k]
    if (typeof v === 'string' && v.length > 0) return v
  }
  return null
}

function pickDiffText(args: Record<string, unknown> | undefined): {
  before: string | null
  after: string | null
} {
  if (!args) return { before: null, after: null }
  const before =
    typeof args.old_text === 'string'
      ? args.old_text
      : typeof args.old_str === 'string'
        ? args.old_str
        : typeof args.oldString === 'string'
          ? args.oldString
          : null
  const after =
    typeof args.new_text === 'string'
      ? args.new_text
      : typeof args.new_str === 'string'
        ? args.new_str
        : typeof args.newString === 'string'
          ? args.newString
          : typeof args.content === 'string'
            ? args.content
            : null
  return { before, after }
}

function formatUnifiedDiff(path: string, before: string | null, after: string | null): string {
  if (before === null && after !== null) {
    return `--- a${path}\n+++ b${path}\n@@\n+${after}`
  }
  if (before !== null && after === null) {
    return `--- a${path}\n+++ b${path}\n@@\n-${before}`
  }
  if (before !== null && after !== null) {
    return `--- a${path}\n+++ b${path}\n@@\n-${before}\n+${after}`
  }
  return `--- a${path}\n+++ b${path}`
}

interface RawToolCall {
  id: string
  apiName: string | undefined
  args: Record<string, unknown> | undefined
}

function extractBlocks(message: ChatMessage): RawToolCall[] {
  const out: RawToolCall[] = []
  const blocks = message.blocks
  if (Array.isArray(blocks) && blocks.length > 0) {
    for (const b of blocks as ChatBlock[]) {
      if (b && b.type === 'tool') {
        const tool = b as unknown as {
          id: string
          apiName?: string
          arguments?: Record<string, unknown>
        }
        out.push({
          id: tool.id,
          apiName: tool.apiName,
          args: tool.arguments,
        })
      }
    }
    return out
  }
  // Legacy hydration: metadata.tool_calls[] — surfaced before the LobeHub
  // block-stream migration runs over an older session.
  const metadata = message.metadata
  if (metadata && Array.isArray(metadata.tool_calls)) {
    for (const tcRaw of metadata.tool_calls) {
      const tc = tcRaw as unknown as {
        tool_call_id?: string
        id?: string
        tool_args?: Record<string, unknown>
        tool_name?: string
      }
      const id = tc.tool_call_id ?? tc.id
      if (!id) continue
      out.push({ id, apiName: tc.tool_name, args: tc.tool_args })
    }
  }
  return out
}

export function useSessionChanges(): ChangeItem[] {
  const messages = useChatStore((s) => s.messages)
  const reviewed = useWorkbenchStore((s) => s.reviewedChangeIds)
  const revertedIds = useWorkbenchStore((s) => s.revertedChangeIds)

  return useMemo(() => {
    const items: ChangeItem[] = []
    for (const m of messages) {
      if (m?.role !== 'assistant') continue
      const blocks = extractBlocks(m)
      for (const b of blocks) {
        if (!isEditTool(b.apiName)) continue
        const path = pickPath(b.args)
        if (!path) continue
        const { before, after } = pickDiffText(b.args)
        const id = `${m.id}::${b.id}`
        const status: ChangeItem['status'] = revertedIds.has(id)
          ? 'reverted'
          : reviewed.has(id)
            ? 'reviewed'
            : 'pending'
        items.push({
          id,
          path,
          diff: formatUnifiedDiff(path, before, after),
          status,
          at: Date.parse(m.timestamp ?? '') || 0,
        })
      }
    }
    // messages[] is already chronological — preserve insertion order.
    // Sort by timestamp only when every item carries a parseable one
    // (Array.sort is stable, so ties keep insertion order).
    if (items.length > 0 && items.every((i) => i.at > 0)) {
      items.sort((a, b) => a.at - b.at)
    }
    return items
  }, [messages, reviewed, revertedIds])
}
