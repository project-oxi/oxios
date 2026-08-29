// Task 7 (project-roots persona workbench): use-session-changes aggregation.
//
// The hook derives a flat list of file-change items from the active
// session's transcript (ChatMessage[] → ChangeItem[]). The aggregation
// rule (design §7.2 + tool registry aliases):
//   * tool names matched case-insensitively across aliases:
//       edit, write, patch, apply_patch, multiedit, notebook_edit
//     (any tool whose apiName contains one of those substrings counts.)
//   * toolArgs are inspected for a path-like field: path, file_path,
//     notebook_path. The first hit wins.
//   * diff text: old_text/old_str/oldString + new_text/new_str/newString
//     concatenated; falls back to `content` for write-style calls.
//   * status: 'pending' unless reviewedChangeIds contains the change id;
//     'reverted' after revertChange.
//   * id: `${messageId}::${toolCallId}` — stable across the same call.
//
// The fixture builds a minimal transcript with both aliased and
// non-aliased tool names so the alias matcher is exercised.

import { renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useSessionChanges } from '@/hooks/use-session-changes'
import { useChatStore } from '@/stores/chat'
import { useWorkbenchStore } from '@/stores/workbench'
import type { ChatMessage, ToolBlock } from '@/types'

function toolBlock(partial: Partial<ToolBlock> & { apiName: string; id: string }): ToolBlock {
  return {
    type: 'tool',
    identifier: 'kernel',
    arguments: {},
    status: 'success',
    ...partial,
  } as ToolBlock
}

function msg(id: string, blocks: ToolBlock[]): ChatMessage {
  return {
    id,
    role: 'assistant',
    content: '',
    blocks,
  } as unknown as ChatMessage
}

function setTranscript(messages: ChatMessage[]) {
  // The hook reads messages from useChatStore via a selector. Set the
  // field directly — the store does not gate writes for tests, and
  // mutating through `setMessages` would require touching the store
  // shape which is not part of this test's surface.
  useChatStore.setState({ messages })
}

describe('useSessionChanges — alias matching + aggregation', () => {
  it('returns an empty list when the transcript has no edit tool calls', () => {
    setTranscript([])
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current).toEqual([])
  })

  it('collects a single edit-style tool call with path + diff', () => {
    setTranscript([
      msg('m1', [
        toolBlock({
          id: 't1',
          apiName: 'edit',
          arguments: {
            path: '/repo/src/foo.ts',
            old_text: 'a',
            new_text: 'b',
          },
        }),
      ]),
    ])
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current).toHaveLength(1)
    const item = result.current[0]!
    expect(item.id).toBe('m1::t1')
    expect(item.path).toBe('/repo/src/foo.ts')
    expect(item.diff).toContain('--- a')
    expect(item.diff).toContain('+++ b')
    expect(item.status).toBe('pending')
  })

  it('matches aliases case-insensitively (EDIT, Write, Apply_Patch, MULTIEDIT, NOTEBOOK_EDIT)', () => {
    setTranscript([
      msg('m1', [
        toolBlock({
          id: 't1',
          apiName: 'EDIT',
          arguments: { path: '/a', old_text: '1', new_text: '2' },
        }),
        toolBlock({
          id: 't2',
          apiName: 'Write',
          arguments: { path: '/b', content: 'rewritten' },
        }),
        toolBlock({
          id: 't3',
          apiName: 'Apply_Patch',
          arguments: { path: '/c', old_text: 'x', new_text: 'y' },
        }),
        toolBlock({
          id: 't4',
          apiName: 'MULTIEDIT',
          arguments: { path: '/d', old_text: 'm', new_text: 'n' },
        }),
        toolBlock({
          id: 't5',
          apiName: 'Notebook_Edit',
          arguments: { notebook_path: '/e.ipynb', old_text: 'a', new_text: 'b' },
        }),
      ]),
    ])
    const { result } = renderHook(() => useSessionChanges())
    const ids = result.current.map((c) => c.id).sort()
    expect(ids).toEqual(['m1::t1', 'm1::t2', 'm1::t3', 'm1::t4', 'm1::t5'])
    expect(result.current.find((c) => c.id === 'm1::t5')?.path).toBe('/e.ipynb')
  })

  it('ignores tool calls whose name does not match any edit alias', () => {
    setTranscript([
      msg('m1', [
        toolBlock({ id: 't1', apiName: 'bash', arguments: { command: 'ls' } }),
        toolBlock({ id: 't2', apiName: 'web_search', arguments: { q: 'hi' } }),
      ]),
    ])
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current).toEqual([])
  })

  it('reflects reviewedChangeIds — reviewed ids show status=reviewed', () => {
    setTranscript([
      msg('m1', [
        toolBlock({
          id: 't1',
          apiName: 'edit',
          arguments: { path: '/a', old_text: 'a', new_text: 'b' },
        }),
      ]),
    ])
    useWorkbenchStore.getState().reset()
    useWorkbenchStore.getState().markReviewed('m1::t1')
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current[0]?.status).toBe('reviewed')
  })

  it('reflects reverted change ids — after revertChange the status flips to reverted', () => {
    setTranscript([
      msg('m1', [
        toolBlock({
          id: 't1',
          apiName: 'edit',
          arguments: { path: '/a', old_text: 'a', new_text: 'b' },
        }),
      ]),
    ])
    useWorkbenchStore.getState().reset()
    useWorkbenchStore.getState().markReviewed('m1::t1')
    useWorkbenchStore.getState().revertChange('m1::t1')
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current[0]?.status).toBe('reverted')
  })

  it('preserves transcript insertion order (messages[] is chronological)', () => {
    setTranscript([
      msg('m1', [
        toolBlock({ id: 't1', apiName: 'edit', arguments: { path: '/a' } }),
        toolBlock({ id: 't0', apiName: 'edit', arguments: { path: '/a0' } }),
      ]),
      msg('m2', [toolBlock({ id: 't2', apiName: 'edit', arguments: { path: '/b' } })]),
    ])
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current.map((c) => c.id)).toEqual(['m1::t1', 'm1::t0', 'm2::t2'])
  })

  it('sorts by timestamp only when every message carries a parseable one', () => {
    setTranscript([
      msg('m2', [toolBlock({ id: 't2', apiName: 'edit', arguments: { path: '/b' } })]),
      msg('m1', [toolBlock({ id: 't1', apiName: 'edit', arguments: { path: '/a' } })]),
    ])
    // Stamp timestamps out of insertion order; the stable timestamp sort
    // must reorder them (m1 earlier than m2).
    useChatStore.setState({
      messages: [
        {
          ...msg('m2', [toolBlock({ id: 't2', apiName: 'edit', arguments: { path: '/b' } })]),
          timestamp: '2026-08-29T10:00:00Z',
        },
        {
          ...msg('m1', [toolBlock({ id: 't1', apiName: 'edit', arguments: { path: '/a' } })]),
          timestamp: '2026-08-29T09:00:00Z',
        },
      ] as ChatMessage[],
    })
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current.map((c) => c.id)).toEqual(['m1::t1', 'm2::t2'])
  })

  it('falls back to message metadata tool_calls when blocks are absent', () => {
    // The store sometimes hydrates history with the legacy toolCalls metadata
    // and no blocks yet — we still surface the change.
    const m: ChatMessage = {
      id: 'm1',
      role: 'assistant',
      content: '',
      metadata: {
        tool_calls: [
          {
            tool_call_id: 'tc-1',
            tool_name: 'write',
            tool_args: { path: '/m.txt', content: 'hi' },
          },
        ],
      },
    } as unknown as ChatMessage
    setTranscript([m])
    const { result } = renderHook(() => useSessionChanges())
    expect(result.current).toHaveLength(1)
    expect(result.current[0]?.path).toBe('/m.txt')
  })
})
