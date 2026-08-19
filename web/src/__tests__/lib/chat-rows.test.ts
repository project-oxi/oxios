import { describe, expect, it } from 'vitest'
import { buildChatRows } from '@/lib/chat-rows'
import type { ChatMessage } from '@/types'

const makeMsg = (id: string, role: 'user' | 'assistant' = 'user'): ChatMessage => ({
  id,
  role,
  content: `content-${id}`,
  timestamp: new Date(0).toISOString(),
})

const baseOpts = () => ({
  messages: [] as ChatMessage[],
  expanded: false,
  collapseThreshold: 40,
  visibleTail: 20,
  hasInterview: false,
  hasToolApproval: false,
  hasPathAccess: false,
  compression: null,
})

describe('buildChatRows', () => {
  it('returns an empty row by default when there are no messages', () => {
    const rows = buildChatRows(baseOpts())
    expect(rows).toEqual([{ kind: 'empty' }])
  })

  it('Task 10: returns a single skeleton row while loading with no messages', () => {
    const rows = buildChatRows({ ...baseOpts(), isLoadingSession: true })
    expect(rows).toEqual([{ kind: 'skeleton' }])
  })

  it('Task 10: emits skeleton only when isLoadingSession AND messages empty', () => {
    // With messages present, isLoadingSession is ignored — the real rows win.
    const opts = { ...baseOpts(), messages: [makeMsg('m1')], isLoadingSession: true }
    const rows = buildChatRows(opts)
    expect(rows).not.toEqual([{ kind: 'skeleton' }])
    expect(rows.find((r) => r.kind === 'skeleton')).toBeUndefined()
    expect(rows.find((r) => r.kind === 'message')).toBeTruthy()
  })

  it('preserves the legacy empty-row behaviour when not loading', () => {
    const rows = buildChatRows({ ...baseOpts(), isLoadingSession: false })
    expect(rows).toEqual([{ kind: 'empty' }])
  })
})
