import { waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, type Mock, vi } from 'vitest'
import { useAuthStore } from '@/stores/auth'
import {
  __clearAuthCacheForTesting,
  __clearResolvedApprovalIdsForTesting,
  __clearStreamProcessorsForTesting,
  appendTokenToMessages,
  ensureLastAssistant,
  finalizeStreamingMessage,
  KNOWN_CHUNK_TYPES,
  parseChunk,
  patchAssistantModel,
  TURN_WATCHDOG_MS,
  useChatStore,
} from '@/stores/chat'
import { usePortalStore } from '@/stores/portal'
import { useSidebarStore } from '@/stores/sidebar'
import type { ChatMessage, StreamChunk } from '@/types'

describe('useAuthStore', () => {
  beforeEach(() => {
    localStorage.clear()
    sessionStorage.clear()
  })

  it('starts unauthenticated when no token', () => {
    const state = useAuthStore.getState()
    expect(state.isAuthenticated).toBe(false)
    expect(state.token).toBeNull()
  })

  it('sets token and authenticates', () => {
    useAuthStore.getState().setToken('test-key')
    const state = useAuthStore.getState()
    expect(state.isAuthenticated).toBe(true)
    expect(state.token).toBe('test-key')
    expect(sessionStorage.getItem('oxios-api-key')).toBe('test-key')
  })

  it('logout clears token', () => {
    useAuthStore.getState().setToken('test-key')
    useAuthStore.getState().logout()
    const state = useAuthStore.getState()
    expect(state.isAuthenticated).toBe(false)
    expect(state.token).toBeNull()
    expect(sessionStorage.getItem('oxios-api-key')).toBeNull()
  })

  it('setToken(null) clears authentication', () => {
    useAuthStore.getState().setToken('test-key')
    useAuthStore.getState().setToken(null)
    expect(useAuthStore.getState().isAuthenticated).toBe(false)
  })
})

describe('useSidebarStore', () => {
  beforeEach(() => {
    localStorage.clear()
    useSidebarStore.setState({ collapsed: false, mobileOpen: false })
  })

  it('toggles collapsed state', () => {
    expect(useSidebarStore.getState().collapsed).toBe(false)
    useSidebarStore.getState().toggle()
    expect(useSidebarStore.getState().collapsed).toBe(true)
    useSidebarStore.getState().toggle()
    expect(useSidebarStore.getState().collapsed).toBe(false)
  })

  it('sets mobile open state', () => {
    useSidebarStore.getState().setMobileOpen(true)
    expect(useSidebarStore.getState().mobileOpen).toBe(true)
    useSidebarStore.getState().setMobileOpen(false)
    expect(useSidebarStore.getState().mobileOpen).toBe(false)
  })
})

// RFC-015: chat transparency event handling
describe('useChatStore handleChunk (RFC-015)', () => {
  beforeEach(() => {
    localStorage.clear()
    // Phase 1: StreamProcessor instances are module-level and persist across
    // tests. Reset them so accumulation doesn't leak between tests.
    __clearStreamProcessorsForTesting()
    // Start each test with a single empty assistant message so chunks
    useChatStore.setState({
      messages: [
        {
          id: 'a1',
          role: 'assistant' as const,
          content: '',
          timestamp: new Date().toISOString(),
        },
      ],
      isStreaming: true,
    })
  })

  it('tool_start appends a tool block (P3 block-stream)', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'read_file',
      tool_call_id: 'c1',
      tool_args: { path: '/x' },
    })
    const last = useChatStore.getState().messages.at(-1)!
    expect(last.blocks?.filter((b) => b.type === 'tool')).toHaveLength(1)
    expect(last.blocks?.find((b) => b.type === 'tool')).toMatchObject({
      type: 'tool',
      apiName: 'read_file',
      id: 'c1',
    })
  })

  it('tool_end attaches duration and output to the same tool block', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'bash',
      tool_call_id: 'c1',
      tool_args: {},
    })
    useChatStore.getState().handleChunk({
      type: 'tool_end',
      tool_name: 'bash',
      tool_call_id: 'c1',
      duration_ms: 50,
      is_error: false,
      output_summary: 'ok',
    })
    const last = useChatStore.getState().messages.at(-1)!
    // tool_end collapses into the same tool block (no duplicate).
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect(tools[0]).toMatchObject({
      type: 'tool',
      id: 'c1',
      apiName: 'bash',
      durationMs: 50,
      result: 'ok',
    })
  })

  it('tool_start marks the tool block as loading', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'browse',
      tool_call_id: 'c1',
      tool_args: {},
    })
    const last = useChatStore.getState().messages.at(-1)!
    expect(last.blocks?.find((b) => b.type === 'tool')).toMatchObject({
      type: 'tool',
      id: 'c1',
      status: 'loading',
    })
  })

  it('tool_progress updates the existing tool block in place (RFC-015 v0.12)', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'browse',
      tool_call_id: 'c1',
      tool_args: { url: 'https://example.com' },
    })
    useChatStore.getState().handleChunk({
      type: 'tool_progress',
      tool_name: 'browse',
      tool_call_id: 'c1',
      progress: 'navigating to example.com',
      tab_id: 'tab-abc-123',
    })
    const last = useChatStore.getState().messages.at(-1)!
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect(tools[0]).toMatchObject({
      type: 'tool',
      id: 'c1',
      progress: 'navigating to example.com',
      tabId: 'tab-abc-123',
    })
    // Original toolArgs from tool_start are preserved across the merge.
    expect((tools[0] as { arguments: unknown }).arguments).toEqual({ url: 'https://example.com' })
  })

  it('tool_progress chunk without tab_id omits tabId on the tool block', () => {
    // Legacy oxicode-agent versions don't emit tab_id; the resulting tool block
    // must not have tabId at all (not tabId: undefined), so the frontend
    // doesn't render a badge.
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'browse',
      tool_call_id: 'c1',
      tool_args: {},
    })
    useChatStore.getState().handleChunk({
      type: 'tool_progress',
      tool_name: 'browse',
      tool_call_id: 'c1',
      progress: 'step 1',
    })
    const last = useChatStore.getState().messages.at(-1)!
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect((tools[0] as { tabId?: string }).tabId).toBeUndefined()
    // Defensive: the key should not even be present on the object literal.
    expect('tabId' in (tools[0] as object)).toBe(false)
  })

  it('tool_call_delta accumulates partial args before tool_start (oxi 0.58+)', () => {
    // The LLM streams raw JSON fragments for a tool call it is still
    // constructing. Two deltas for the same tool_call_id must concatenate.
    useChatStore.getState().handleChunk({
      type: 'tool_call_delta',
      tool_call_id: 'c1',
      args_delta: '{"path": "/tm',
    })
    useChatStore.getState().handleChunk({
      type: 'tool_call_delta',
      tool_call_id: 'c1',
      args_delta: 'p/foo"}',
    })
    const last = useChatStore.getState().messages.at(-1)!
    // A placeholder tool block exists with accumulated raw-JSON args.
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect(tools[0]).toMatchObject({
      type: 'tool',
      id: 'c1',
      apiName: '(constructing…)',
    })
    expect((tools[0] as { arguments: unknown }).arguments).toBe('{"path": "/tmp/foo"}')
  })

  it('tool_call_delta placeholder is replaced by tool_start', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_call_delta',
      tool_call_id: 'c1',
      args_delta: '{"q":"hi"}',
    })
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'web_search',
      tool_call_id: 'c1',
      tool_args: { q: 'hi' },
    })
    const last = useChatStore.getState().messages.at(-1)!
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect(tools[0]).toMatchObject({
      type: 'tool',
      id: 'c1',
      apiName: 'web_search',
    })
    expect((tools[0] as { arguments: unknown }).arguments).toEqual({ q: 'hi' })
  })

  it('subsequent tool_progress replaces the prior progress text', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'browse',
      tool_call_id: 'c1',
      tool_args: {},
    })
    useChatStore.getState().handleChunk({
      type: 'tool_progress',
      tool_name: 'browse',
      tool_call_id: 'c1',
      progress: 'step 1',
    })
    useChatStore.getState().handleChunk({
      type: 'tool_progress',
      tool_name: 'browse',
      tool_call_id: 'c1',
      progress: 'step 2',
    })
    const last = useChatStore.getState().messages.at(-1)!
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect((tools[0] as { progress?: string }).progress).toBe('step 2')
  })

  it('tool_end marks the matching tool block success', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'browse',
      tool_call_id: 'c1',
      tool_args: {},
    })
    useChatStore.getState().handleChunk({
      type: 'tool_end',
      tool_name: 'browse',
      tool_call_id: 'c1',
      duration_ms: 100,
      is_error: false,
      output_summary: 'done',
    })
    const last = useChatStore.getState().messages.at(-1)!
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect(tools[0]).toMatchObject({
      type: 'tool',
      id: 'c1',
      status: 'success',
    })
  })

  it('memory recall appends a memory block', () => {
    useChatStore.getState().handleChunk({
      type: 'memory',
      action: 'recall',
      query: 'rust errors',
      count: 3,
      source: 'warm',
    })
    const last = useChatStore.getState().messages.at(-1)!
    expect(last.blocks?.find((b) => b.type === 'memory')).toMatchObject({
      type: 'memory',
      action: 'recall',
      query: 'rust errors',
      count: 3,
      source: 'warm',
    })
  })

  it('usage accumulates input/output tokens via UsageBlocks', () => {
    // Each `usage` chunk overwrites the last UsageBlock (provider sends
    // cumulative totals). The chat store derives `totalInputTokens` /
    // `totalOutputTokens` from the final UsageBlock.
    useChatStore.getState().handleChunk({
      type: 'usage',
      input_tokens: 100,
      output_tokens: 30,
    })
    useChatStore.getState().handleChunk({
      type: 'usage',
      input_tokens: 50,
      output_tokens: 20,
    })
    const last = useChatStore.getState().messages.at(-1)!
    expect(last.totalInputTokens).toBe(50)
    expect(last.totalOutputTokens).toBe(20)
  })

  it('reasoning populates a positioned reasoning block (P2 block-stream)', () => {
    // Block-stream transparency: reasoning is captured as a positioned
    // ChatBlock (not a separate `reasoning` field) so it interleaves with
    // tool calls in execution order. See docs/designs/2026-07-27-block-stream...
    useChatStore.getState().handleChunk({
      type: 'reasoning',
      content: 'compaction complete',
      source: 'compaction',
    })
    const last = useChatStore.getState().messages.at(-1)!
    expect(
      last.blocks?.some((b) => b.type === 'reasoning' && b.text === 'compaction complete'),
    ).toBe(true)
    expect(last.blocks?.find((b) => b.type === 'reasoning')?.source).toBe('compaction')
  })

  it('accumulates reasoning deltas into a single positioned block', () => {
    // Reasoning deltas stream into a single open reasoning block; the
    // block is positioned by the number of tools started before it.
    useChatStore.getState().handleChunk({ type: 'reasoning', content: 'Thi', source: 'thinking' })
    useChatStore.getState().handleChunk({ type: 'reasoning', content: 's is', source: 'thinking' })
    useChatStore
      .getState()
      .handleChunk({ type: 'reasoning', content: ' a thought', source: 'thinking' })
    const last = useChatStore.getState().messages.at(-1)!
    const reasoning = last.blocks?.find((b) => b.type === 'reasoning')
    expect(reasoning?.type).toBe('reasoning')
    if (reasoning?.type === 'reasoning') expect(reasoning.text).toBe('This is a thought')
  })

  it('reasoning across sources still accumulates into one positioned block', () => {
    // All reasoning chunks accumulate into the same positioned block
    // (single open reasoning span at the current tool position).
    useChatStore
      .getState()
      .handleChunk({ type: 'reasoning', content: 'thinking…', source: 'thinking' })
    useChatStore
      .getState()
      .handleChunk({ type: 'reasoning', content: 'compacting…', source: 'compaction' })
    const last = useChatStore.getState().messages.at(-1)!
    const reasoning = last.blocks?.find((b) => b.type === 'reasoning')
    if (reasoning?.type === 'reasoning') expect(reasoning.text).toBe('thinking…compacting…')
  })
  it('tool_start closes the open reasoning block (reasoning_content models)', () => {
    // Reasoning_content providers go straight from reasoning to a tool call
    // with no Text, so the gateway's first-Text reasoning.end heuristic
    // never fires. The processor must close the open reasoning block on
    // tool transition or the spinner would run through the tool execution.
    const store = useChatStore.getState()
    store.handleChunk({ type: 'reasoning', content: 'Let me try a search. ' })
    store.handleChunk({ type: 'reasoning', content: 'Querying…' })
    store.handleChunk({
      type: 'tool_start',
      tool_name: 'web_search',
      tool_call_id: 'ws-1',
      tool_args: { q: 'weather' },
    })
    const last = useChatStore.getState().messages.at(-1)!
    const reasoning = last.blocks?.find((b) => b.type === 'reasoning')
    if (reasoning?.type === 'reasoning') {
      expect(reasoning.status).toBe('done')
      expect(reasoning.text).toBe('Let me try a search. Querying…')
    }
    expect(last.blocks?.filter((b) => b.type === 'tool')).toHaveLength(1)
  })

  it('token chunk does not add an activity', async () => {
    // F9: tokens are batched via requestAnimationFrame; wait one frame for flush.
    useChatStore.getState().handleChunk({ type: 'token', content: 'hello' })
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
    const last = useChatStore.getState().messages.at(-1)!
    expect(last.content).toBe('hello')
    expect(last.blocks?.filter((b) => b.type !== 'text') ?? []).toHaveLength(0)
  })

  it('mixed stream populates all first-class fields end-to-end (Phase 1+2 contract)', async () => {
    // Simulates a realistic stream: model announcement → reasoning deltas →
    // tool call (start/progress/end) → text tokens → usage → done.
    // Asserts the final ChatMessage shape matches the design's pipeline contract
    // (docs/designs/2026-07-21-lobehub-chat-port-design.md §6.1, §7).
    const store = useChatStore.getState()
    store.handleChunk({ type: 'model', model: 'zai/glm-5.2' })
    store.handleChunk({ type: 'reasoning', content: 'Thinking ' })
    store.handleChunk({ type: 'reasoning', content: 'about it…' })
    store.handleChunk({
      type: 'tool_start',
      tool_name: 'read_file',
      tool_call_id: 'tc-1',
      tool_args: { path: '/etc/hosts' },
    })
    store.handleChunk({
      type: 'tool_progress',
      tool_call_id: 'tc-1',
      progress: 'reading…',
    })
    store.handleChunk({
      type: 'tool_end',
      tool_call_id: 'tc-1',
      duration_ms: 42,
      output_summary: '127.0.0.1 localhost',
    })
    store.handleChunk({ type: 'token', content: 'Hello ' })
    store.handleChunk({ type: 'token', content: 'world' })
    // Allow RAF token flush.
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
    store.handleChunk({ type: 'usage', input_tokens: 12, output_tokens: 7 })
    store.handleChunk({ type: 'done', phase: 'Execute', evaluation_passed: true })

    const last = useChatStore.getState().messages.at(-1)!
    // First-class reasoning block — content accumulated into a positioned block.
    const reasoning = last.blocks?.find((b) => b.type === 'reasoning')
    if (reasoning?.type === 'reasoning') expect(reasoning.text).toBe('Thinking about it…')
    // Structured toolCalls[] with full lifecycle.
    const tools = last.blocks?.filter((b) => b.type === 'tool') ?? []
    expect(tools).toHaveLength(1)
    expect(tools[0]).toMatchObject({
      type: 'tool',
      apiName: 'read_file',
      status: 'success',
      durationMs: 42,
    })
    // Token stream accumulated into content.
    expect(last.content).toBe('Hello world')
    // Token totals carried through.
    expect(last.totalInputTokens).toBe(12)
    expect(last.totalOutputTokens).toBe(7)
    // Model badge stamped.
    expect(last.model).toBe('zai/glm-5.2')
  })

  it('done chunk keeps accumulated activities and sets isStreaming=false', () => {
    useChatStore.getState().handleChunk({
      type: 'tool_start',
      tool_name: 'grep',
      tool_call_id: 'g1',
      tool_args: {},
    })
    useChatStore.getState().handleChunk({
      type: 'done',
      session_id: 's1',
      phase: 'execute',
    })
    const state = useChatStore.getState()
    expect(state.isStreaming).toBe(false)
    const last = state.messages.at(-1)!
    expect(last.blocks?.filter((b) => b.type === 'tool')).toHaveLength(1)
    expect(last.metadata?.phase).toBe('execute')
  })

  it('error chunk appends an error message with isError metadata and resets streaming', () => {
    // Pre-seed a user message so the error path can place the assistant
    // error after it (mirrors the production layout).
    useChatStore.setState({
      messages: [
        { id: 'u1', role: 'user' as const, content: 'hello', timestamp: new Date().toISOString() },
      ],
      isStreaming: true,
    })
    const errorChunk = {
      type: 'error',
      message: 'rate limit exceeded',
      kind: 'provider_error',
      suggestion: 'try a different model',
    }
    useChatStore
      .getState()
      .handleChunk(
        errorChunk as unknown as Parameters<
          ReturnType<typeof useChatStore.getState>['handleChunk']
        >[0],
      )
    const state = useChatStore.getState()
    expect(state.isStreaming).toBe(false)
    const errMsg = state.messages.at(-1)!
    expect(errMsg.role).toBe('assistant')
    expect(errMsg.metadata?.isError).toBe(true)
    expect(errMsg.metadata?.errorKind).toBe('provider_error')
    expect(errMsg.content).toContain('rate limit exceeded')
    expect(errMsg.content).toContain('try a different model')
    // Regression: error messages must carry a blocks array — BlockStream maps
    // over it unconditionally, so undefined crashed the whole chat page.
    expect(Array.isArray(errMsg.blocks)).toBe(true)
    expect(errMsg.blocks).toHaveLength(0)
  })

  it('a cancelled terminal keeps the partial answer and marks it interrupted', () => {
    useChatStore.setState({
      messages: [
        { id: 'u1', role: 'user' as const, content: 'hi' },
        { id: 'a1', role: 'assistant' as const, content: 'partial ans', generating: true },
      ],
      isStreaming: true,
      activeToolApproval: { id: 'ap1', toolName: 'bash', reason: 'run ls' },
      activePathAccess: {
        id: 'pa1',
        path: '/x',
        mode: 'temp',
        toolName: 'read_file',
        reason: 'read',
      },
    })

    useChatStore.getState().handleChunk({
      type: 'error',
      kind: 'cancelled',
      message: 'Turn cancelled by user',
    } as never)

    const msgs = useChatStore.getState().messages
    expect(msgs).toHaveLength(2)
    expect(msgs[1]!.content).toBe('partial ans')
    expect(msgs[1]!.generating).toBe(false)
    expect(msgs[1]!.metadata?.cancelled).toBe(true)
    expect(msgs[1]!.metadata?.isError).toBeFalsy()
    const state = useChatStore.getState()
    expect(state.isStreaming).toBe(false)
    expect(state.activeToolApproval).toBeNull()
    expect(state.activePathAccess).toBeNull()
  })

  it('removeMessage drops a single message by id and leaves siblings intact', () => {
    useChatStore.setState({
      messages: [
        { id: 'u1', role: 'user' as const, content: 'first', timestamp: new Date().toISOString() },
        {
          id: 'a1',
          role: 'assistant' as const,
          content: 'first reply',
          timestamp: new Date().toISOString(),
        },
        { id: 'u2', role: 'user' as const, content: 'second', timestamp: new Date().toISOString() },
      ],
      isStreaming: false,
    })
    useChatStore.getState().removeMessage('a1')
    const ids = useChatStore.getState().messages.map((m) => m.id)
    expect(ids).toEqual(['u1', 'u2'])
  })

  it('removeMessage resets isStreaming when removing the streaming target', () => {
    useChatStore.setState({
      messages: [
        {
          id: 'a1',
          role: 'assistant' as const,
          content: '',
          timestamp: new Date().toISOString(),
        },
      ],
      isStreaming: true,
    })
    useChatStore.getState().removeMessage('a1')
    expect(useChatStore.getState().isStreaming).toBe(false)
    expect(useChatStore.getState().messages).toHaveLength(0)
  })
})

// Regression: multi-turn accumulation. Before the turn-aware fix, every
// streaming chunk was routed to "the last assistant message anywhere in the
// list" — which, once a new user message is appended, is the PREVIOUS turn's
// response. Turn 2's stream then overwrote turn 1's bubble and left the new
// user message with no answer beneath it (ordering scrambled, prior response
// gone). These guard the turn-boundary targeting fix.
describe('multi-turn accumulation (turn-boundary targeting)', () => {
  beforeEach(() => {
    __clearStreamProcessorsForTesting()
    localStorage.clear()
    sessionStorage.clear()
  })

  it('a second turn streams into a NEW assistant message, leaving the prior turn intact', async () => {
    // Turn 1 already completed.
    useChatStore.setState({
      messages: [
        { id: 'u1', role: 'user' as const, content: 'q1', timestamp: new Date().toISOString() },
        {
          id: 'a1',
          role: 'assistant' as const,
          content: 'response1',
          timestamp: new Date().toISOString(),
        },
      ],
      isStreaming: false,
      pendingModel: null,
    })
    // User sends turn 2 (mirrors sendMessage's optimistic append — no WS here).
    useChatStore.setState((s) => ({
      messages: [
        ...s.messages,
        { id: 'u2', role: 'user' as const, content: 'q2', timestamp: new Date().toISOString() },
      ],
      isStreaming: true,
    }))
    // Turn 2 streams a token.
    useChatStore.getState().handleChunk({ type: 'token', content: 'response2' })
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))

    const msgs = useChatStore.getState().messages
    expect(msgs).toHaveLength(4)
    expect(msgs[2]!.role).toBe('user')
    expect(msgs[3]!.role).toBe('assistant')
    expect(msgs[3]!.content).toBe('response2')
    // Prior turn's response is untouched (the pre-fix bug overwrote this).
    expect(msgs[1]!.role).toBe('assistant')
    expect(msgs[1]!.content).toBe('response1')
  })

  it('a model chunk before the first token stashes pendingModel instead of overwriting the prior turn', () => {
    useChatStore.setState({
      messages: [
        { id: 'u1', role: 'user' as const, content: 'q1', timestamp: '' },
        { id: 'a1', role: 'assistant' as const, content: 'r1', model: 'old-model', timestamp: '' },
        { id: 'u2', role: 'user' as const, content: 'q2', timestamp: '' },
      ],
      isStreaming: true,
      pendingModel: null,
    })
    useChatStore.getState().handleChunk({ type: 'model', model: 'new-model' })
    const s = useChatStore.getState()
    expect(s.pendingModel).toBe('new-model')
    expect(s.messages[1]!.model).toBe('old-model')
  })

  it('a done-only second turn creates a fresh assistant instead of reusing the prior turn', () => {
    useChatStore.setState({
      messages: [
        { id: 'u1', role: 'user' as const, content: 'q1', timestamp: '' },
        {
          id: 'a1',
          role: 'assistant' as const,
          content: 'response1',
          timestamp: '',
          metadata: { phase: 'execute', tool_calls: [] },
        },
        { id: 'u2', role: 'user' as const, content: 'q2', timestamp: '' },
      ],
      isStreaming: true,
      pendingModel: null,
    })
    useChatStore.getState().handleChunk({ type: 'done', phase: 'execute' })
    const msgs = useChatStore.getState().messages
    expect(msgs).toHaveLength(4)
    expect(msgs[3]!.role).toBe('assistant')
    expect(msgs[1]!.content).toBe('response1')
  })

  it('patchAssistantModel patches only the trailing assistant of the current turn', () => {
    const messages: ChatMessage[] = [
      { id: 'u1', role: 'user', content: 'q1', timestamp: '' },
      { id: 'a1', role: 'assistant', content: 'r1', model: 'old', timestamp: '' },
      { id: 'u2', role: 'user', content: 'q2', timestamp: '' },
      { id: 'a2', role: 'assistant', content: '', timestamp: '' },
    ]
    const r = patchAssistantModel(messages, 'new')
    expect(r.pendingModel).toBeUndefined()
    expect(r.messages[3]!.model).toBe('new')
    expect(r.messages[1]!.model).toBe('old')
  })

  it('patchAssistantModel returns pendingModel when the trailing message is a user message', () => {
    const messages: ChatMessage[] = [
      { id: 'u1', role: 'user', content: 'q1', timestamp: '' },
      { id: 'a1', role: 'assistant', content: 'r1', model: 'old', timestamp: '' },
      { id: 'u2', role: 'user', content: 'q2', timestamp: '' },
    ]
    const r = patchAssistantModel(messages, 'new')
    expect(r.pendingModel).toBe('new')
    expect(r.messages[1]!.model).toBe('old')
  })
})

// The message-transform primitives route every chunk through a single shared
// path in chat.ts; both the chat store and the quick-ask store call them, so
// these tests guard the Cmd+J (one-shot) path too (which has no store tests).
describe('message-transform primitives (shared by chat + quick-ask stores)', () => {
  const ctx = { placeholderModel: 'gpt-x' }
  const assistant = (over: Partial<ChatMessage> = {}): ChatMessage => ({
    id: 'a1',
    role: 'assistant',
    content: '',
    timestamp: 't',
    ...over,
  })
  const userMsg = (): ChatMessage => ({ id: 'u1', role: 'user', content: 'hi', timestamp: 't' })

  it('ensureLastAssistant returns the same array when last is already assistant', () => {
    const input = [assistant()]
    const { messages, index } = ensureLastAssistant(input, ctx)
    expect(messages).toBe(input)
    expect(index).toBe(0)
  })

  it('ensureLastAssistant appends a ctx-modelled placeholder when last is not assistant', () => {
    const { messages, index } = ensureLastAssistant([userMsg()], ctx)
    expect(messages).toHaveLength(2)
    expect(messages[1]).toMatchObject({ role: 'assistant', content: '', model: 'gpt-x' })
    expect(index).toBe(1)
  })

  it('appendTokenToMessages appends to the last assistant content', () => {
    const out = appendTokenToMessages([assistant({ content: 'foo' })], 'bar', ctx)
    expect(out[0]!.content).toBe('foobar')
  })

  it('appendTokenToMessages creates a placeholder when no assistant exists', () => {
    const out = appendTokenToMessages([userMsg()], 'hi', ctx)
    expect(out).toHaveLength(2)
    expect(out[1]).toMatchObject({ role: 'assistant', content: 'hi', model: 'gpt-x' })
  })

  it('appendTokenToMessages is a no-op returning the same array on empty content', () => {
    const input = [assistant({ content: 'foo' })]
    expect(appendTokenToMessages(input, '', ctx)).toBe(input)
  })

  it('patchAssistantModel patches the last assistant model', () => {
    const out = patchAssistantModel([assistant({ model: 'old' })], 'new')
    expect(out.messages[0]!.model).toBe('new')
    expect(out.pendingModel).toBeUndefined()
  })

  it('patchAssistantModel returns pendingModel when no assistant exists', () => {
    const input = [userMsg()]
    const out = patchAssistantModel(input, 'm')
    expect(out.messages).toBe(input)
    expect(out.pendingModel).toBe('m')
  })

  it('finalizeStreamingMessage drops an empty generating placeholder', () => {
    const msgs: ChatMessage[] = [userMsg(), assistant({ id: 'a1', generating: true })]
    // No content/reasoning/tools/activities → ghost → removed.
    expect(finalizeStreamingMessage(msgs)).toEqual([userMsg()])
  })

  it('finalizeStreamingMessage keeps a partial placeholder with generating cleared', () => {
    const msgs: ChatMessage[] = [
      userMsg(),
      assistant({ id: 'a1', content: 'partial answer', generating: true }),
    ]
    const out = finalizeStreamingMessage(msgs)
    expect(out).toHaveLength(2)
    expect(out[1]!.content).toBe('partial answer')
    expect(out[1]!.generating).toBe(false)
  })

  it('finalizeStreamingMessage is a no-op when the last message is not a generating assistant', () => {
    const done = [assistant({ id: 'a1', content: 'finished' })]
    expect(finalizeStreamingMessage(done)).toBe(done)
    const onlyUser = [userMsg()]
    expect(finalizeStreamingMessage(onlyUser)).toBe(onlyUser)
  })
  it('finalizeStreamingMessage clears isReasoning on an interrupted reasoning stream', () => {
    // Abnormal WS close mid-reasoning must not leave the Thinking spinner
    // stuck. Without clearing isReasoning/reasoning.thinking the block spins
    // forever — the reported "stuck Thinking..." symptom.
    const msgs: ChatMessage[] = [
      userMsg(),
      assistant({
        id: 'a1',
        generating: true,
        isToolCallGenerating: true,
        blocks: [
          {
            type: 'reasoning',
            id: 'r-1',
            text: 'Let me try',
            status: 'streaming',
            startedAt: Date.now() - 100,
          },
        ],
      }),
    ]
    const out = finalizeStreamingMessage(msgs)
    expect(out).toHaveLength(2)
    expect(out[1]!.generating).toBe(false)
    expect(out[1]!.isToolCallGenerating).toBe(false)
    // Blocks are preserved (no longer a separate `reasoning` field to
    // clear — the close happens at the block level when streaming ends).
    expect(out[1]!.blocks?.find((b) => b.type === 'reasoning')?.type).toBe('reasoning')
  })
})

describe('useChatStore message queueing (while streaming)', () => {
  let sendSpy: Mock
  const mockWs = (): WebSocket =>
    ({ readyState: 1, send: sendSpy, close: vi.fn() }) as unknown as WebSocket

  beforeEach(() => {
    localStorage.clear()
    sendSpy = vi.fn()
    useChatStore.setState({
      messages: [
        { id: 'a1', role: 'assistant' as const, content: '', timestamp: new Date().toISOString() },
      ],
      isStreaming: true,
      connected: true,
      _ws: mockWs(),
      _pendingQueue: [],
      _reconnectTimer: null,
      _pingTimer: null,
      activeSessionId: 's1',
    })
  })

  it('queues a message sent while streaming instead of dispatching', () => {
    useChatStore.getState().sendMessage('follow-up')
    const s = useChatStore.getState()
    // Stashed in the pending queue — not yet on the wire or in the list.
    expect(s._pendingQueue).toEqual(['follow-up'])
    expect(s.messages.some((m) => m.role === 'user')).toBe(false)
    expect(sendSpy).not.toHaveBeenCalled()
  })

  it('drains the queue in order when the turn completes (done)', () => {
    useChatStore.getState().sendMessage('first')
    useChatStore.getState().sendMessage('second')
    // Turn ends → drain dispatches 'first', leaves 'second' queued.
    useChatStore.getState().handleChunk({ type: 'done', session_id: 's1', phase: 'execute' })
    const s = useChatStore.getState()
    expect(s._pendingQueue).toEqual(['second'])
    expect(s.isStreaming).toBe(true)
    expect(s.messages.some((m) => m.role === 'user' && m.content === 'first')).toBe(true)
    expect(sendSpy).toHaveBeenCalledTimes(1)
    expect(JSON.parse(sendSpy.mock.calls[0]![0] as string).content).toBe('first')
  })

  it('clears the queue on disconnect (cancel drops unsent messages)', () => {
    useChatStore.getState().sendMessage('ghost')
    useChatStore.getState().disconnect()
    expect(useChatStore.getState()._pendingQueue).toEqual([])
  })
})

// sendMessage turn start (lobehub-style holder model): no optimistic
// assistant placeholder is created — the LiveActivityBar holder pinned above
// the input owns the "what's happening" indicator for the whole turn,
// including the pre-chunk gap. The assistant message is created lazily on the
// first reasoning/tool/token chunk. sendMessage just stamps the user message,
// isStreaming, and a streamStartedAt timestamp the holder reads for elapsed.
describe('useChatStore sendMessage turn start', () => {
  let sendSpy: Mock
  const mockWs = (): WebSocket =>
    ({ readyState: 1, send: sendSpy, close: vi.fn() }) as unknown as WebSocket

  beforeEach(() => {
    localStorage.clear()
    __clearStreamProcessorsForTesting()
    sendSpy = vi.fn()
    useChatStore.setState({
      messages: [],
      isStreaming: false,
      streamStartedAt: null,
      connected: true,
      _ws: mockWs(),
      _pendingQueue: [],
      _reconnectTimer: null,
      _pingTimer: null,
      activeSessionId: 's1',
      activeModelId: 'gpt-test',
    })
  })

  it('appends only the user message and marks the turn live with a start timestamp', () => {
    useChatStore.getState().sendMessage('hello')
    const s = useChatStore.getState()
    expect(s.isStreaming).toBe(true)
    expect(s.streamStartedAt).toBeTypeOf('number')
    // No optimistic assistant placeholder — the holder covers the gap.
    expect(s.messages).toHaveLength(1)
    expect(s.messages[0]).toMatchObject({ role: 'user', content: 'hello' })
    expect(sendSpy).toHaveBeenCalledTimes(1)
  })

  it('creates the assistant message lazily on the first reasoning chunk', () => {
    useChatStore.getState().sendMessage('hello')
    expect(useChatStore.getState().messages).toHaveLength(1)
    // A reasoning model streams reasoning before any token; the chunk handler
    // must create the assistant message on demand (not drop the delta).
    useChatStore.getState().handleChunk({
      type: 'reasoning',
      content: 'thinking…',
      source: 'thinking',
    })
    const msgs = useChatStore.getState().messages
    expect(msgs).toHaveLength(2)
    expect(msgs[1]!.blocks?.find((b) => b.type === 'reasoning')?.type).toBe('reasoning')
  })

  it('disconnect before any chunk leaves only the user message (no ghost bubble)', () => {
    useChatStore.getState().sendMessage('hello')
    useChatStore.getState().disconnect()
    const msgs = useChatStore.getState().messages
    expect(msgs).toHaveLength(1)
    expect(msgs[0]!.role).toBe('user')
    expect(useChatStore.getState().isStreaming).toBe(false)
  })

  it('stamps a request_id on the first message (no session yet) so cancel can address the turn', () => {
    useChatStore.setState({ activeSessionId: null })
    useChatStore.getState().sendMessage('hello')
    const payload = JSON.parse(sendSpy.mock.calls[0]![0]!)
    expect(payload.request_id).toBeTypeOf('string')
    expect(payload.request_id!.length).toBeGreaterThan(0)
    expect(payload.session_id).toBe('')
  })

  it('omits request_id once a session exists (cancel keys by session_id)', () => {
    useChatStore.getState().sendMessage('hello')
    const payload = JSON.parse(sendSpy.mock.calls[0]![0]!)
    expect(payload.request_id).toBeUndefined()
    expect(payload.session_id).toBe('s1')
  })
})

// Tool approval (RFC-017): resolveToolApproval must treat a 404 "already
// resolved" as benign idempotent success and NEVER re-arm a dead card. The
// prior code restored activeToolApproval on ANY non-OK and re-threw, so a
// single 404 re-armed the card → the next click 404ed again → the N×404
// cascade seen in the Web UI. A WS reconnect replay re-delivering a resolved
// approval id was the trigger; the restore-on-error was the amplifier.
describe('useChatStore tool approval (RFC-017)', () => {
  let fetchSpy: Mock

  beforeEach(() => {
    localStorage.clear()
    __clearStreamProcessorsForTesting()
    __clearResolvedApprovalIdsForTesting()
    fetchSpy = vi.fn()
    vi.stubGlobal('fetch', fetchSpy)
    useChatStore.setState({
      messages: [],
      activeToolApproval: { id: 'approval-1', toolName: 'exec', reason: 'shell command' },
      isStreaming: false,
    })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('dismisses the card on 200 success', async () => {
    fetchSpy.mockResolvedValueOnce({ ok: true, status: 200 })
    await useChatStore.getState().resolveToolApproval('approval-1', true)
    const s = useChatStore.getState()
    expect(s.activeToolApproval).toBeNull()
    expect(s.isStreaming).toBe(true)
  })

  it('treats 404 "already resolved" as benign — card stays dismissed, no throw', async () => {
    fetchSpy.mockResolvedValueOnce({
      ok: false,
      status: 404,
      text: () =>
        Promise.resolve('{"error":"tool approval approval-1 not found or already resolved"}'),
    })
    // Must not reject — a 404 is idempotent success (approval already gone).
    await expect(
      useChatStore.getState().resolveToolApproval('approval-1', true),
    ).resolves.toBeUndefined()
    const s = useChatStore.getState()
    // Card NOT restored — the old code set it back here, re-arming the loop.
    expect(s.activeToolApproval).toBeNull()
    expect(fetchSpy).toHaveBeenCalledTimes(1)
  })

  it('restores the card on a genuine server error (500) for retry, without throwing', async () => {
    fetchSpy.mockResolvedValueOnce({
      ok: false,
      status: 500,
      text: () => Promise.resolve('internal error'),
    })
    // Swallowed — no unhandled promise rejection.
    await expect(
      useChatStore.getState().resolveToolApproval('approval-1', true),
    ).resolves.toBeUndefined()
    const s = useChatStore.getState()
    expect(s.activeToolApproval?.id).toBe('approval-1')
    expect(s.isStreaming).toBe(false)
  })

  it('does not re-arm an already-resolved approval on WS replay', async () => {
    fetchSpy.mockResolvedValueOnce({ ok: true, status: 200 })
    await useChatStore.getState().resolveToolApproval('approval-1', true)
    expect(useChatStore.getState().activeToolApproval).toBeNull()
    // Reconnect replay re-delivers the same approval id.
    useChatStore.getState().handleChunk({
      type: 'tool_approval',
      id: 'approval-1',
      tool_name: 'exec',
      reason: 'shell command',
    })
    // Still dismissed — the dead card is NOT re-armed.
    expect(useChatStore.getState().activeToolApproval).toBeNull()
  })

  it('arms the card for a fresh tool_approval chunk', () => {
    useChatStore.setState({ activeToolApproval: null })
    useChatStore.getState().handleChunk({
      type: 'tool_approval',
      id: 'approval-2',
      tool_name: 'exec',
      reason: 'rm -rf',
    })
    const s = useChatStore.getState()
    expect(s.activeToolApproval?.id).toBe('approval-2')
    expect(s.isStreaming).toBe(false)
  })
})

// Reconnect state machine: the client must never permanently give up. After
// the fast exponential backoff exhausts it switches to a steady long-tail
// retry so a daemon restart that outlasts the ~31s backoff window still
// recovers instead of stranding the tab at connected === false forever.
describe('useChatStore reconnect (long-tail recovery)', () => {
  // Minimal fake WebSocket. The outcome is deferred via setTimeout(0) so fake
  // timers drive ordering deterministically and connect() can attach its
  // handlers before open/close fires. Statics are declared on the class body
  // (writable) because the DOM lib types `WebSocket.OPEN` &c. as readonly,
  // and the global is installed via vi.stubGlobal because jsdom exposes
  // `WebSocket` as a read-only property.
  let instances: unknown[]
  let FakeWS: ReturnType<typeof makeFakeWebSocket>

  function makeFakeWebSocket() {
    class FakeWebSocket {
      static OPEN = 1
      static CLOSED = 3
      static CONNECTING = 0
      static CLOSING = 2
      static mode: 'fail' | 'succeed' = 'fail'
      url: string
      readyState = 0
      onopen: (() => void) | null = null
      onclose: (() => void) | null = null
      onmessage: ((e: { data: string }) => void) | null = null
      onerror: (() => void) | null = null
      constructor(url: string) {
        this.url = url
        instances.push(this)
        setTimeout(() => {
          if (FakeWebSocket.mode === 'succeed') {
            this.readyState = 1
            this.onopen?.()
          } else {
            this.readyState = 3
            this.onclose?.()
          }
        }, 0)
      }
      close() {
        this.readyState = 3
      }
      send() {}
    }
    return FakeWebSocket
  }

  beforeEach(() => {
    localStorage.clear()
    sessionStorage.clear()
    __clearAuthCacheForTesting()
    __clearStreamProcessorsForTesting()
    __clearResolvedApprovalIdsForTesting()
    instances = []
    FakeWS = makeFakeWebSocket()
    vi.stubGlobal('WebSocket', FakeWS)
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({ auth_enabled: false }),
      })),
    )
    useChatStore.setState({
      connected: false,
      isStreaming: false,
      _ws: null,
      _sendQueue: [],
      _pendingQueue: [],
      _reconnectAttempts: 0,
      _reconnectTimer: null,
      _pingTimer: null,
      messages: [],
      activeSessionId: 's1',
      activeProjectId: 'p1',
    })
    vi.useFakeTimers()
  })

  afterEach(() => {
    try {
      useChatStore.getState().disconnect()
    } catch {
      // ignore — store may be mid-teardown
    }
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('keeps retrying past the fast-backoff cap instead of giving up forever', async () => {
    useChatStore.getState().connect()
    // Fast backoff is 1+2+4+8+16 = 31s across 5 attempts. Advancing 60s
    // runs the full backoff plus two long-tail (10s) retries.
    await vi.advanceTimersByTimeAsync(60_000)
    // Old behaviour stopped creating sockets at the cap (6 total). Long-tail
    // recovery must keep creating fresh sockets.
    expect(instances.length).toBeGreaterThanOrEqual(7)
    expect(useChatStore.getState()._reconnectAttempts).toBeGreaterThanOrEqual(5)
    expect(useChatStore.getState().connected).toBe(false)
  })

  it('recovers and resets the counter once the connection succeeds', async () => {
    useChatStore.getState().connect()
    await vi.advanceTimersByTimeAsync(60_000)
    expect(useChatStore.getState().connected).toBe(false)
    // Daemon returns — the next long-tail attempt opens.
    FakeWS.mode = 'succeed'
    await vi.advanceTimersByTimeAsync(15_000)
    expect(useChatStore.getState().connected).toBe(true)
    expect(useChatStore.getState()._reconnectAttempts).toBe(0)
  })
})

describe('usePortalStore pushDocument (saved-doc chip)', () => {
  beforeEach(() => {
    usePortalStore.setState({ stack: [] })
  })

  it('pushes a document view onto an empty stack', () => {
    usePortalStore.getState().pushDocument('notes/foo.md')
    const { stack } = usePortalStore.getState()
    expect(stack).toHaveLength(1)
    expect(stack[0]).toEqual({ type: 'document', path: 'notes/foo.md' })
  })

  it('toggles off when the same document is already on top (peek)', () => {
    usePortalStore.getState().pushDocument('notes/foo.md')
    usePortalStore.getState().pushDocument('notes/foo.md')
    expect(usePortalStore.getState().stack).toHaveLength(0)
  })

  it('re-surfaces an existing deeper document instead of duplicating', () => {
    // Stack: [docA, filePreview] — a non-document view is on top.
    usePortalStore.getState().pushView({ type: 'document', path: 'a.md' })
    usePortalStore.getState().pushView({ type: 'filePreview', path: 'other.txt', content: 'x' })
    // The chip pushes docA again while the filePreview is on top.
    usePortalStore.getState().pushDocument('a.md')
    const { stack } = usePortalStore.getState()
    // Truncated back to the original docA — no duplicate, filePreview popped.
    expect(stack).toHaveLength(1)
    expect(stack[0]).toEqual({ type: 'document', path: 'a.md' })
  })

  it('pushes a different document on top (stack navigation)', () => {
    usePortalStore.getState().pushDocument('a.md')
    usePortalStore.getState().pushDocument('b.md')
    const { stack } = usePortalStore.getState()
    expect(stack).toHaveLength(2)
    expect(stack[1]).toEqual({ type: 'document', path: 'b.md' })
  })
})

describe('cancelTurn', () => {
  it('sends a cancel frame and does not tear down the socket', () => {
    const sent: string[] = []
    const fakeWs = {
      readyState: 1,
      send: (d: string) => sent.push(d),
      close: () => {
        throw new Error('cancelTurn must not close the socket')
      },
    }
    useChatStore.setState({
      _ws: fakeWs as unknown as WebSocket,
      connected: true,
      isStreaming: true,
      activeSessionId: 'sess-9',
    })

    useChatStore.getState().cancelTurn()

    // session_id keys the cancel for an existing session; request_id may be
    // null or a stale first-message id — either way the server resolves by
    // session_id first.
    const payload = JSON.parse(sent[0]!)
    expect(payload.type).toBe('cancel')
    expect(payload.session_id).toBe('sess-9')
    expect(payload).toHaveProperty('request_id')
  })

  it('reuses the first message request_id in the cancel frame (no session yet)', () => {
    const sent: string[] = []
    const fakeWs = {
      readyState: 1,
      send: (d: string) => sent.push(d),
      close: () => {
        throw new Error('cancelTurn must not close the socket')
      },
    }
    useChatStore.setState({
      _ws: fakeWs as unknown as WebSocket,
      connected: true,
      isStreaming: false,
      activeSessionId: null,
    })

    // First message with no session stamps the module-level request_id.
    useChatStore.getState().sendMessage('hello')
    const msgPayload = JSON.parse(sent[0]!)
    expect(msgPayload.request_id).toBeTypeOf('string')
    const requestId = msgPayload.request_id as string

    // The in-flight turn is cancelled using THAT id — the only key the
    // server knows (gateway keys first messages by msg.id).
    sent.length = 0
    useChatStore.setState({ isStreaming: true })
    useChatStore.getState().cancelTurn()
    const cancelPayload = JSON.parse(sent[0]!)
    expect(cancelPayload.type).toBe('cancel')
    expect(cancelPayload.session_id).toBeNull()
    expect(cancelPayload.request_id).toBe(requestId)
  })

  it('is a no-op when no turn is in flight', () => {
    const sent: string[] = []
    useChatStore.setState({
      _ws: { readyState: 1, send: (d: string) => sent.push(d) } as unknown as WebSocket,
      connected: true,
      isStreaming: false,
    })
    useChatStore.getState().cancelTurn()
    expect(sent).toHaveLength(0)
  })
})

describe('turn watchdog', () => {
  it('a hung turn is finalized by the watchdog instead of spinning forever', () => {
    vi.useFakeTimers()
    const sent: string[] = []
    useChatStore.setState({
      _ws: { readyState: 1, send: (d: string) => sent.push(d) } as unknown as WebSocket,
      connected: true,
      activeSessionId: 'sess-hang',
      messages: [],
    })

    useChatStore.getState().sendMessage('hello')
    expect(useChatStore.getState().isStreaming).toBe(true)

    vi.advanceTimersByTime(TURN_WATCHDOG_MS + 10)

    expect(useChatStore.getState().isStreaming).toBe(false)
    const last = useChatStore.getState().messages.at(-1)!
    expect(last.metadata?.isError).toBe(true)
    expect(last.metadata?.errorKind).toBe('timeout')
    expect(sent.some((s) => JSON.parse(s).type === 'cancel')).toBe(true)
    vi.useRealTimers()
  })
})

// Task 6: Loss-free socket close.
//
// When the WebSocket drops before the rAF token flush fires, the buffered
// token tail is silently dropped and the partial answer looks complete.
// The onclose handler must (a) flush the rAF-buffered tokens before
// finalizing, and (b) flag the trailing assistant message as
// `metadata.interrupted = true` so the UI can render a "connection lost"
// notice (Task 4's InterruptedNotice, picked by the metadata flag).
describe('useChatStore onclose (Task 6 — loss-free socket close)', () => {
  beforeEach(() => {
    localStorage.clear()
    sessionStorage.clear()
    __clearAuthCacheForTesting()
    __clearStreamProcessorsForTesting()
    __clearResolvedApprovalIdsForTesting()
    useChatStore.setState({
      connected: false,
      isStreaming: false,
      _ws: null,
      _sendQueue: [],
      _pendingQueue: [],
      _reconnectAttempts: 0,
      _reconnectTimer: null,
      _pingTimer: null,
      activeSessionId: 's1',
      activeProjectId: 'p1',
      messages: [],
    })
  })

  it('flushes buffered tokens and flags the turn interrupted when the socket drops', async () => {
    vi.useFakeTimers()
    let capturedOnClose: (() => void) | null = null
    class FakeWebSocket {
      static OPEN = 1
      static CLOSED = 3
      url: string
      readyState = 0
      onopen: (() => void) | null = null
      onclose: (() => void) | null = null
      onmessage: ((_e: { data: string }) => void) | null = null
      onerror: (() => void) | null = null
      constructor(url: string) {
        this.url = url
        // Open via the same setTimeout(0) schedule chat.ts uses so connect()
        // completes its onopen handler; we capture the onclose ref so we can
        // fire it manually from the test.
        setTimeout(() => {
          this.readyState = 1
          this.onopen?.()
        }, 0)
        capturedOnClose = () => this.onclose?.()
      }
      close() {
        this.readyState = 3
      }
      send() {}
    }
    vi.stubGlobal('WebSocket', FakeWebSocket)
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({ auth_enabled: false }),
      })),
    )

    await useChatStore.getState().connect()
    // Drive the FakeWebSocket opening setTimeout(0) without a real wait.
    await vi.advanceTimersByTimeAsync(1)
    expect(useChatStore.getState().connected).toBe(true)

    // Buffer a token so the rAF holds a tail that has not been flushed yet.
    // Pre-seed an in-flight streaming assistant so finalizeStreamingMessage
    // has a target to clear `generating` on and the onclose branch routes
    // the metadata.interrupted flag at it.
    useChatStore.setState({
      messages: [
        {
          id: 'a1',
          role: 'assistant' as const,
          content: '',
          timestamp: new Date().toISOString(),
          generating: true,
        },
      ],
      isStreaming: true,
    })
    const tokenChunk = { type: 'token', content: 'Hello ' } as unknown as StreamChunk
    useChatStore.getState().handleChunk(tokenChunk)

    // Trigger the WS close BEFORE the rAF flush.
    capturedOnClose!()
    // Drain any scheduled rAF work.
    await vi.advanceTimersByTimeAsync(1)

    const last = useChatStore.getState().messages.at(-1)!
    expect(last.role).toBe('assistant')
    expect(last.content).toBe('Hello ')
    expect(last.generating).toBe(false)
    expect(last.metadata?.interrupted).toBe(true)
    expect(useChatStore.getState().isStreaming).toBe(false)

    useChatStore.getState().disconnect()
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('does not flag interrupted when the socket closes while idle', async () => {
    vi.useFakeTimers()
    let capturedOnClose: (() => void) | null = null
    class FakeWebSocket {
      static OPEN = 1
      static CLOSED = 3
      url: string
      readyState = 0
      onopen: (() => void) | null = null
      onclose: (() => void) | null = null
      onmessage: ((_e: { data: string }) => void) | null = null
      onerror: (() => void) | null = null
      constructor(url: string) {
        this.url = url
        capturedOnClose = () => this.onclose?.()
      }
      close() {
        this.readyState = 3
      }
      send() {}
    }
    vi.stubGlobal('WebSocket', FakeWebSocket)
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({ auth_enabled: false }),
      })),
    )
    await useChatStore.getState().connect()
    await vi.advanceTimersByTimeAsync(1)

    useChatStore.setState({ isStreaming: false })
    capturedOnClose!()
    await vi.advanceTimersByTimeAsync(1)

    expect(useChatStore.getState().isStreaming).toBe(false)
    const last = useChatStore.getState().messages.at(-1)
    expect(last?.metadata?.interrupted).toBeUndefined()

    useChatStore.getState().disconnect()
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })
})

// Task 10: session loading surfaces failures instead of swallowing them.
describe('useChatStore loadSession (Task 10 — surfacing load failures)', () => {
  beforeEach(() => {
    localStorage.clear()
    sessionStorage.clear()
    // Reset the loading/error slots so a prior test's failure does not leak.
    useChatStore.setState({
      isLoadingSession: false,
      sessionLoadError: null,
      _lastLoadedSessionId: null,
    } as Partial<typeof useChatStore.getState>)
    // Wipe any WebSocket instance from a prior test so connect() inside
    // loadSession's call chain stays inert.
    useChatStore.getState().disconnect()
  })

  it('exposes loading state and surfaces a load failure instead of swallowing it', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('offline'))
    try {
      const p = useChatStore.getState().loadSession('sess-x')
      expect(useChatStore.getState().isLoadingSession).toBe(true)
      await p
      const state = useChatStore.getState()
      expect(state.isLoadingSession).toBe(false)
      expect(state.sessionLoadError).toBeTruthy()
    } finally {
      fetchMock.mockRestore()
    }
  })

  it('treats a non-OK HTTP response as a load failure', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: false,
      status: 500,
      json: async () => ({}),
      text: async () => '',
    } as Response)
    try {
      await useChatStore.getState().loadSession('sess-y')
      const state = useChatStore.getState()
      expect(state.isLoadingSession).toBe(false)
      expect(state.sessionLoadError).toBeTruthy()
    } finally {
      fetchMock.mockRestore()
    }
  })

  it('retryLoadSession re-runs the last load', async () => {
    let calls = 0
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(async () => {
      calls++
      if (calls === 1) throw new Error('offline')
      return {
        ok: true,
        status: 200,
        json: async () => ({
          user_messages: [],
          agent_responses: [],
          trajectory_steps: [],
          reasoning_records: [],
        }),
        text: async () => '',
      } as Response
    })
    const loadSpy = vi.spyOn(useChatStore.getState(), 'loadSession')
    try {
      // First attempt fails.
      await useChatStore.getState().loadSession('sess-retry')
      expect(useChatStore.getState().sessionLoadError).toBeTruthy()
      expect(useChatStore.getState()._lastLoadedSessionId).toBe('sess-retry')
      // Snapshot spy counts BEFORE retry — the first load already counted itself.
      const callsBeforeRetry = loadSpy.mock.calls.length
      // Trigger the retry. retryLoadSession returns void; wait on the
      // observable consequence (isLoadingSession false after second fetch).
      useChatStore.getState().retryLoadSession()
      await waitFor(() => expect(useChatStore.getState().isLoadingSession).toBe(false))
      // Allow the post-finally set() to flush before reading sessionLoadError.
      await waitFor(() => expect(useChatStore.getState().sessionLoadError).toBeNull())
      expect(loadSpy.mock.calls.length).toBeGreaterThan(callsBeforeRetry)
      expect(loadSpy).toHaveBeenCalledWith('sess-retry')
      expect(calls).toBeGreaterThanOrEqual(2)
    } finally {
      loadSpy.mockRestore()
      fetchMock.mockRestore()
    }
  })
})

describe('useChatStore branchFrom (Task 22)', () => {
  beforeEach(() => {
    useChatStore.setState({
      activeSessionId: 'sess-a',
      activeProjectId: null,
      messages: [],
    })
  })

  it('branching copies history up to the chosen message into a new session', () => {
    useChatStore.setState({
      activeSessionId: 'sess-a',
      messages: [
        { id: 'u1', role: 'user', content: 'one' },
        { id: 'a1', role: 'assistant', content: 'two' },
        { id: 'u2', role: 'user', content: 'three' },
      ] as never,
    })
    useChatStore.getState().branchFrom('a1')
    const s = useChatStore.getState()
    expect(s.activeSessionId).not.toBe('sess-a')
    expect(s.messages.map((m) => m.id)).toEqual(['u1', 'a1'])
  })

  it('branchFrom is a no-op for an unknown message id', () => {
    useChatStore.setState({
      activeSessionId: 'sess-a',
      messages: [{ id: 'u1', role: 'user', content: 'one' }] as never,
    })
    useChatStore.getState().branchFrom('ghost')
    const s = useChatStore.getState()
    expect(s.activeSessionId).toBe('sess-a')
    expect(s.messages.map((m) => m.id)).toEqual(['u1'])
  })
})

describe('parseChunk', () => {
  it('accepts agent_start/agent_end/tool_start frames', () => {
    // Regression 1 (T21): agent_start/agent_end were missing from
    // KNOWN_CHUNK_TYPES, so parseChunk coerced the opening agent_start frame
    // of EVERY turn into a "Malformed chunk" error.
    // Regression 2 (f1538a369): tool_start was dropped together with the
    // intentional 'phase' removal, so every tool-using turn got a fake error.
    for (const type of ['agent_start', 'agent_end', 'tool_start'] as const) {
      const chunk = parseChunk({ type })
      expect(chunk.type).toBe(type)
      expect(KNOWN_CHUNK_TYPES.has(type)).toBe(true)
    }
  })

  it('still coerces unknown types to an error chunk', () => {
    const chunk = parseChunk({ type: 'not_a_real_chunk_type' })
    expect(chunk.type).toBe('error')
    expect('error' in chunk).toBe(true)
  })
})
