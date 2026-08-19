import { describe, expect, it } from 'vitest'
import {
  deriveCurrentActivityFromBlocks,
  describeLiveActivity,
  type LiveActivityDescriptor,
  type Translator,
} from '@/lib/live-activity'
import type { ChatBlock, ReasoningBlock, ToolBlock } from '@/types/chat'

// ---------------------------------------------------------------------------
// Mock translator — returns the i18n key (with simple {{opt}} interpolation)
// so tests verify key selection + detail extraction without a full locale.
// ---------------------------------------------------------------------------
const mockT: Translator = (key, opts) => {
  if (!opts) return key
  return Object.entries(opts).reduce((s, [k, v]) => s.replaceAll(`{{${k}}}`, String(v)), key)
}

describe('describeLiveActivity', () => {
  it('returns thinking label for thinking kind', () => {
    const { label, detail } = describeLiveActivity({ kind: 'thinking' }, mockT)
    expect(label).toBe('chat.liveActivity.thinking')
    expect(detail).toBeUndefined()
  })

  it('returns reasoning label for reasoning kind', () => {
    const { label, detail } = describeLiveActivity({ kind: 'reasoning' }, mockT)
    expect(label).toBe('chat.liveActivity.reasoning')
    expect(detail).toBeUndefined()
  })

  it('derives label + detail from web_search context', () => {
    const { label, detail } = describeLiveActivity(
      {
        kind: 'tool_running',
        toolName: 'browser',
        context: { kind: 'web_search', query: 'rust async runtime', engine: 'google' },
      },
      mockT,
    )
    expect(label).toBe('chat.liveActivity.webSearch')
    expect(detail).toBe('rust async runtime')
  })

  it('derives label + shortened URL from page_visit context', () => {
    const { label, detail } = describeLiveActivity(
      {
        kind: 'tool_running',
        toolName: 'browser',
        context: { kind: 'page_visit', url: 'https://example.com/some/long/path' },
      },
      mockT,
    )
    expect(label).toBe('chat.liveActivity.pageVisit')
    expect(detail).toBe('example.com/some/long/path')
  })

  it('uses step text as label for script_step context', () => {
    const { label, detail } = describeLiveActivity(
      {
        kind: 'tool_running',
        toolName: 'browser',
        context: { kind: 'script_step', current: 2, total: 5, step: 'Clicking search button' },
      },
      mockT,
    )
    expect(label).toBe('Clicking search button')
    expect(detail).toBe('2/5')
  })

  it('falls back to progress text as detail when no context', () => {
    const { label, detail } = describeLiveActivity(
      {
        kind: 'tool_running',
        toolName: 'exec',
        progress: 'Building project...',
      },
      mockT,
    )
    expect(label).toBe('chat.liveActivity.exec')
    expect(detail).toBe('Building project...')
  })

  it('extracts file path from toolArgs as detail', () => {
    const { label, detail } = describeLiveActivity(
      {
        kind: 'tool_running',
        toolName: 'read',
        toolArgs: { path: '/home/user/projects/src/main.rs' },
      },
      mockT,
    )
    expect(label).toBe('chat.liveActivity.read')
    expect(detail).toBe('…/src/main.rs')
  })

  it('extracts command from toolArgs as detail', () => {
    const { label, detail } = describeLiveActivity(
      {
        kind: 'tool_running',
        toolName: 'exec',
        toolArgs: { command: 'cargo build --release' },
      },
      mockT,
    )
    expect(label).toBe('chat.liveActivity.exec')
    expect(detail).toBe('cargo build --release')
  })

  it('uses toolRunning fallback for unknown tool names', () => {
    const { label } = describeLiveActivity(
      { kind: 'tool_running', toolName: 'custom_mcp_tool' },
      mockT,
    )
    expect(label).toBe('chat.liveActivity.toolRunning')
  })

  it('uses toolDefault when toolName is absent', () => {
    const { label } = describeLiveActivity({ kind: 'tool_running', toolName: undefined }, mockT)
    expect(label).toBe('chat.liveActivity.toolDefault')
  })
})

// ---------------------------------------------------------------------------
// Block-stream variant — direct coverage for the selector that
// LiveActivityBar actually uses. deriveCurrentActivityFromBlocks() walks the
// `blocks[]` single-source-of-truth field backwards for the most recent
// (running tool → streaming reasoning → thinking).

/** Build a minimal tool block — the live variant the selector cares about. */
function loadingTool(overrides: Partial<ToolBlock> & { apiName?: string }): ToolBlock {
  return {
    type: 'tool',
    id: `tool-${Math.random().toString(36).slice(2)}`,
    identifier: 'kernel',
    apiName: 'read',
    arguments: { path: 'src/main.ts' },
    status: 'loading',
    ...overrides,
  } as ToolBlock
}

function doneTool(overrides: Partial<ToolBlock> & { apiName?: string }): ToolBlock {
  return {
    type: 'tool',
    id: `tool-${Math.random().toString(36).slice(2)}`,
    identifier: 'kernel',
    apiName: 'read',
    arguments: { path: 'src/main.ts' },
    status: 'success',
    ...overrides,
  } as ToolBlock
}

function streamingReasoning(): ReasoningBlock {
  return {
    type: 'reasoning',
    id: `reason-${Math.random().toString(36).slice(2)}`,
    text: 'thinking…',
    status: 'streaming',
    source: 'thinking',
    startedAt: 0,
  }
}

function doneReasoning(): ReasoningBlock {
  return {
    type: 'reasoning',
    id: `reason-${Math.random().toString(36).slice(2)}`,
    text: 'concluded',
    status: 'done',
    source: 'thinking',
    startedAt: 0,
    durationMs: 250,
  }
}

describe('deriveCurrentActivityFromBlocks (block-stream selector)', () => {
  it('falls back to thinking when blocks is undefined', () => {
    expect(deriveCurrentActivityFromBlocks(undefined)).toEqual<LiveActivityDescriptor>({
      kind: 'thinking',
    })
  })

  it('falls back to thinking when blocks is empty', () => {
    expect(deriveCurrentActivityFromBlocks([])).toEqual<LiveActivityDescriptor>({
      kind: 'thinking',
    })
  })

  it('reports a loading tool block as tool_running with the apiName', () => {
    const blocks: ChatBlock[] = [loadingTool({ apiName: 'browser' })]
    expect(deriveCurrentActivityFromBlocks(blocks)).toEqual<LiveActivityDescriptor>({
      kind: 'tool_running',
      toolName: 'browser',
      toolArgs: { path: 'src/main.ts' },
    })
  })

  it('ignores tool blocks that are not loading', () => {
    // Settled tool blocks must NOT keep the bar pinned to the tool label —
    // success/error/aborted all count as "done".
    const blocks: ChatBlock[] = [doneTool({ apiName: 'browser', status: 'success' })]
    expect(deriveCurrentActivityFromBlocks(blocks)).toEqual<LiveActivityDescriptor>({
      kind: 'thinking',
    })
  })

  it('reports a streaming reasoning block as in-progress reasoning', () => {
    const blocks: ChatBlock[] = [streamingReasoning()]
    expect(deriveCurrentActivityFromBlocks(blocks)).toEqual<LiveActivityDescriptor>({
      kind: 'reasoning',
    })
  })

  it('ignores reasoning blocks that have already settled', () => {
    // A "done" reasoning block is not live — the selector must keep walking.
    const blocks: ChatBlock[] = [doneReasoning()]
    expect(deriveCurrentActivityFromBlocks(blocks)).toEqual<LiveActivityDescriptor>({
      kind: 'thinking',
    })
  })

  it('prefers a loading tool over an older streaming reasoning block', () => {
    const blocks: ChatBlock[] = [streamingReasoning(), loadingTool({ apiName: 'bash' })]
    expect(deriveCurrentActivityFromBlocks(blocks)).toEqual<LiveActivityDescriptor>({
      kind: 'tool_running',
      toolName: 'bash',
      toolArgs: { path: 'src/main.ts' },
    })
  })

  it('returns reasoning when the most recent live block is streaming reasoning', () => {
    // [done tool, loading tool, streaming reasoning] — reasoning is the most
    // recent fire-and-forget fragment.
    const blocks: ChatBlock[] = [
      doneTool({ apiName: 'read', status: 'success' }),
      loadingTool({ apiName: 'browser' }),
      streamingReasoning(),
    ]
    expect(deriveCurrentActivityFromBlocks(blocks)).toEqual<LiveActivityDescriptor>({
      kind: 'reasoning',
    })
  })
  it('skips trailing settled blocks (e.g. text + usage) and falls back to thinking', () => {
    // No loading tool, no streaming reasoning — just done text + usage.
    // The selector must keep walking past them and land on thinking.
    const blocks: ChatBlock[] = [
      doneTool({ apiName: 'read', status: 'success' }),
      { type: 'text', id: 't1', text: 'hello', streaming: false },
      { type: 'usage', id: 'u1', inputTokens: 5, outputTokens: 3 },
    ]
    expect(deriveCurrentActivityFromBlocks(blocks)).toEqual<LiveActivityDescriptor>({
      kind: 'thinking',
    })
  })

  it('carries progress text from a loading tool block', () => {
    const blocks: ChatBlock[] = [
      loadingTool({ apiName: 'browser', progress: 'Navigating to example.com' }),
    ]
    const d = deriveCurrentActivityFromBlocks(blocks)
    expect(d.kind).toBe('tool_running')
    expect(d.progress).toBe('Navigating to example.com')
  })

  it('carries arguments from a loading tool block as toolArgs', () => {
    const blocks: ChatBlock[] = [
      loadingTool({ apiName: 'exec', arguments: { command: 'cargo test' } }),
    ]
    const d = deriveCurrentActivityFromBlocks(blocks)
    expect(d.toolArgs).toEqual({ command: 'cargo test' })
  })

  it('defaults toolArgs to {} when a loading tool has no arguments', () => {
    const blocks: ChatBlock[] = [loadingTool({ apiName: 'noop', arguments: undefined })]
    const d = deriveCurrentActivityFromBlocks(blocks)
    expect(d.toolArgs).toEqual({})
  })
})
