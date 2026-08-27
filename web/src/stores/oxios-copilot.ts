import { create } from 'zustand'
import { uuid } from '@/lib/uuid'
import type { ChatBlock, ChatMessage, StreamChunk } from '@/types'
import {
  applyContentChunk,
  applyTextFlush,
  buildWsUrl,
  getToken,
  parseChunk,
  patchAssistantModel,
} from './chat'

// ---------------------------------------------------------------------------
// Oxios Copilot — one-shot, non-persisted question/answer store.
//
// Opens its own short-lived WebSocket, sends `ephemeral: true` so the backend
// skips session persistence (chat.rs ephemeral flag), streams the reply with
// the same transparency chunks as /chat, and tears down on `done`/close.
// No session is ever written to StateStore; nothing appears in the sidebar.
// See docs/designs/2026-07-05-one-shot-oxios-copilot-design.md.
//
// Shares the pure chunk parsers / WS URL builder with chat.ts (no duplication).
// Token batching uses the same RAF pattern as chat.ts so the UX is identical.
// ---------------------------------------------------------------------------

export interface CapturedExchange {
  prompt: string
  reply: string
  blocks: ChatBlock[]
  model?: string
  sessionId?: string
}
export const COPILOT_PERSONA_ID = 'oxios' as const

/** Pure WS message payload — locked persona, ephemeral, selected model.
 *  Exported so tests can build the same payload without opening a WebSocket.
 *  Task 9 locks persona_id here so neither call site can forget it. */
export function buildCopilotMessagePayload(content: string, model: string | null) {
  return {
    type: 'message',
    content,
    ephemeral: true,
    persona_id: COPILOT_PERSONA_ID,
    model: model ?? '',
  } as const
}

/** Pure /api/chat/seed body — same shape the dialog builds today, plus
 *  `persona_id` so the seeded session is bound to the oxios persona.
 *  Trajectory/reasoning mapping preserved verbatim from handlePromote
 *  (tool blocks → {tool, input, output, duration_ms}; reasoning → joined text). */
export function buildSeedRequestBody(ex: CapturedExchange) {
  return {
    user_message: ex.prompt,
    agent_response: ex.reply,
    trajectory_steps: ex.blocks
      .filter((b) => b.type === 'tool')
      .map((b) => {
        const tb = b as Extract<ChatBlock, { type: 'tool' }>
        return {
          tool: tb.apiName,
          input: tb.arguments,
          output: typeof tb.result === 'string' ? tb.result : undefined,
          duration_ms: tb.durationMs,
        }
      }),
    reasoning_text: ex.blocks
      .filter((b) => b.type === 'reasoning')
      .map((b) => {
        const rb = b as Extract<ChatBlock, { type: 'reasoning' }>
        return rb.text
      })
      .join('\n'),
    project_id: undefined,
    persona_id: COPILOT_PERSONA_ID,
  }
}
interface OxiosCopilotState {
  open: boolean
  messages: ChatMessage[]
  isStreaming: boolean
  pendingModel: string | null
  /** From engine config (Settings → One-shot model); falls back to default. */
  copilotModel: string | null
  /** Captured exchange for "promote to chat" (§5.6). */
  lastExchange: CapturedExchange | null
  /** Active tool-approval request id, if the one-shot triggers one. */
  activeToolApproval: { id: string; toolName: string; reason: string } | null
  /** User messages queued while an assistant turn is streaming. Drained
   *  (in order) when the turn completes via `done`/`error`; cleared on
   *  cancel / close / reset. Mirrors chat.ts _pendingQueue. */
  _pendingQueue: string[]
  _ws: WebSocket | null

  openCopilot: () => void
  closeCopilot: () => void
  setCopilotModel: (model: string | null) => void
  send: (content: string) => void
  /** Cancel the in-flight stream and discard the queue. */
  cancel: () => void
  resolveToolApproval: (id: string, approved: boolean, remember?: boolean) => Promise<void>
  reset: () => void
  _drainPendingQueue: () => void
}

// ---------------------------------------------------------------------------
// Token batching — same RAF pattern as chat.ts.
//
// Each incoming token chunk previously rebuilt the entire messages array
// (O(n) per token → O(n×t) for a response of t tokens across n messages),
// triggering a Zustand subscriber re-render on every token. We instead
// accumulate token content in a module-scoped buffer and flush it at most once
// per animation frame. Any non-token chunk flushes synchronously first so
// streamed text is never lost when a tool/done/error event arrives mid-stream.
// ---------------------------------------------------------------------------
let _pendingTokens = ''
let _tokenRafId: number | null = null

function flushPendingTokens(): void {
  if (_tokenRafId !== null) {
    cancelAnimationFrame(_tokenRafId)
    _tokenRafId = null
  }
  if (!_pendingTokens) return
  const content = _pendingTokens
  _pendingTokens = ''
  useOxiosCopilotStore.setState((s) => ({
    messages: applyTextFlush(s.messages, content, {
      placeholderModel: s.pendingModel ?? s.copilotModel,
    }),
  }))
}

function scheduleTokenFlush(): void {
  if (_tokenRafId !== null) return
  _tokenRafId = requestAnimationFrame(() => {
    _tokenRafId = null
    flushPendingTokens()
  })
}

function discardPendingTokens(): void {
  if (_tokenRafId !== null) {
    cancelAnimationFrame(_tokenRafId)
    _tokenRafId = null
  }
  _pendingTokens = ''
}
// Tool-approval ids the user has already acted on. Oxios Copilot opens its own
// short-lived WS, so it keeps a separate set from chat.ts. A reconnect replay
// can re-deliver a tool_approval chunk for an approval already resolved on
// the backend; without dedup the replay re-arms a dead card whose every click
// returns 404. Bounded so a long session cannot grow it without limit.
const _resolvedApprovalIds = new Set<string>()
const RESOLVED_APPROVAL_IDS_MAX = 64
function markApprovalResolved(id: string): void {
  _resolvedApprovalIds.add(id)
  while (_resolvedApprovalIds.size > RESOLVED_APPROVAL_IDS_MAX) {
    const first = _resolvedApprovalIds.values().next().value
    if (first === undefined) break
    _resolvedApprovalIds.delete(first)
  }
}
/** Test-only: clear resolved-approval ids so module state doesn't leak
 *  between tests. */
export function __clearResolvedApprovalIdsForTesting(): void {
  _resolvedApprovalIds.clear()
}

export const useOxiosCopilotStore = create<OxiosCopilotState>((set, get) => ({
  open: false,
  messages: [],
  isStreaming: false,
  pendingModel: null,
  copilotModel: null,
  lastExchange: null,
  activeToolApproval: null,
  _pendingQueue: [],
  _ws: null,

  openCopilot: () => {
    // Opening while streaming focuses the existing dialog (singleton).
    if (get().isStreaming) {
      set({ open: true })
      return
    }
    set({ open: true, messages: [], lastExchange: null, pendingModel: null })
  },

  closeCopilot: () => {
    discardPendingTokens()
    const ws = get()._ws
    if (ws && ws.readyState === WebSocket.OPEN) ws.close()
    set({ open: false, _ws: null, isStreaming: false, _pendingQueue: [] })
  },

  setCopilotModel: (model) => set({ copilotModel: model }),

  send: (content) => {
    const { copilotModel, isStreaming, _ws, messages } = get()
    if (!content.trim()) return

    // Queue if currently streaming (same pattern as chat.ts _pendingQueue).
    // The caller has already cleared the textarea; we just stash the content.
    // It is dispatched (and added to the message list) when the done/error
    // handler drains the queue — so there are no ghost messages to clean up.
    if (isStreaming) {
      set((s) => ({ _pendingQueue: [...s._pendingQueue, content] }))
      return
    }

    const now = new Date().toISOString()
    const userMsg: ChatMessage = {
      id: uuid(),
      role: 'user',
      content,
      timestamp: now,
    }
    // Optimistic assistant placeholder for streaming tokens.
    const assistantMsg: ChatMessage = {
      id: uuid(),
      role: 'assistant',
      content: '',
      timestamp: now,
      blocks: [],
    }
    const baseMessages = [...messages, userMsg, assistantMsg]
    set({ messages: baseMessages, isStreaming: true, pendingModel: null })

    const connectAndSend = async () => {
      try {
        const ws =
          _ws && _ws.readyState === WebSocket.OPEN ? _ws : new WebSocket(await buildWsUrl())
        set({ _ws: ws })
        const sendPayload = () => {
          ws.send(JSON.stringify(buildCopilotMessagePayload(content, copilotModel)))
        }

        if (ws.readyState === WebSocket.OPEN) {
          sendPayload()
          return
        }

        ws.onopen = sendPayload
        ws.onmessage = (ev) => {
          let raw: unknown
          try {
            raw = JSON.parse(ev.data)
          } catch {
            return
          }
          handleChunk(parseChunk(raw), set, get)
        }
        ws.onerror = () => ws.close()
        ws.onclose = () => {
          if (get()._ws !== ws) return
          set({ _ws: null })
          if (get().isStreaming) {
            flushPendingTokens()
            appendError(set, '연결이 끊겼습니다. 다시 시도해 주세요.')
            get()._drainPendingQueue()
          }
        }
      } catch (err) {
        appendError(set, err instanceof Error ? err.message : '연결할 수 없습니다.')
        get()._drainPendingQueue()
      }
    }

    void connectAndSend()
  },

  cancel: () => {
    discardPendingTokens()
    const ws = get()._ws
    if (ws && ws.readyState === WebSocket.OPEN) ws.close()
    set({ isStreaming: false, _ws: null, _pendingQueue: [] })
  },

  resolveToolApproval: async (id, approved, remember) => {
    // Record as resolved before the fetch so a WS replay cannot re-arm the
    // card with this id. 404 "already resolved" is benign (prior click,
    // replay re-arm, or the exec_tool 120 s timeout auto-denying it) — leave
    // the card dismissed; only genuine errors warrant a retry.
    markApprovalResolved(id)
    try {
      const res = await fetch(`/api/chat/tool-approval/${encodeURIComponent(id)}/respond`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(getToken() ? { Authorization: `Bearer ${getToken()}` } : {}),
        },
        body: JSON.stringify({ approved, ...(remember ? { remember: true } : {}) }),
      })
      if (!res.ok && res.status !== 404) {
        console.warn('[oxios-copilot] tool approval respond failed:', res.status)
      }
    } catch (e) {
      // Non-blocking — the backend oneshot times out on its own.
      console.warn('[oxios-copilot] tool approval respond error:', e)
    }
    set({ activeToolApproval: null })
  },

  reset: () => {
    discardPendingTokens()
    const ws = get()._ws
    if (ws && ws.readyState === WebSocket.OPEN) ws.close()
    set({
      messages: [],
      isStreaming: false,
      pendingModel: null,
      lastExchange: null,
      activeToolApproval: null,
      _pendingQueue: [],
      _ws: null,
    })
  },

  _drainPendingQueue: () => {
    const { _pendingQueue } = get()
    if (_pendingQueue.length === 0) return
    // Shift the head before dispatching: send's normal path runs here because
    // isStreaming was just cleared by the done/error handler.
    const next = _pendingQueue[0]
    if (next === undefined) return
    set({ _pendingQueue: _pendingQueue.slice(1) })
    get().send(next)
  },
}))

// ---------------------------------------------------------------------------
// Chunk handling — operates on the store via set/get closures.
//
// Message transforms (token append, activity merge, model patch, placeholder
// creation) route through shared pure primitives imported from chat.ts
// (appendTokenToMessages / appendActivityToMessages / patchAssistantModel) so
// this store and the chat store cannot drift apart. Token batching uses the
// same RAF pattern as chat.ts. What stays oxios-copilot-specific: promote-capture
// on done, and the divergent done/interview/error/tool_approval side effects.
// ---------------------------------------------------------------------------

type SetFn = (
  partial: Partial<OxiosCopilotState> | ((s: OxiosCopilotState) => Partial<OxiosCopilotState>),
) => void
type GetFn = () => OxiosCopilotState

function updateAssistant(set: SetFn, fn: (m: ChatMessage) => ChatMessage): void {
  set((s) => {
    const msgs = [...s.messages]
    for (let i = msgs.length - 1; i >= 0; i--) {
      const m = msgs[i]
      if (m && m.role === 'assistant') {
        msgs[i] = fn(m)
        break
      }
    }
    return { messages: msgs }
  })
}

function appendError(set: SetFn, message: string): void {
  set({ isStreaming: false })
  updateAssistant(set, (m) => ({
    ...m,
    content: m.content ? `${m.content}\n\n⚠️ ${message}` : `⚠️ ${message}`,
  }))
}

function handleChunk(chunk: StreamChunk, set: SetFn, get: GetFn): void {
  // Flush any buffered token content before a non-token chunk so streamed text
  // is committed to the message before a tool/done/error event reads or
  // replaces the last assistant message. (Same guard as chat.ts.)
  if (chunk.type !== 'token') {
    flushPendingTokens()
  }

  switch (chunk.type) {
    case 'model': {
      // Patch the live assistant message, or stash as pendingModel for the
      // placeholder created on first token/activity (shared logic w/ chat.ts).
      const modelId = chunk.model
      if (!modelId) break
      set((s) => {
        const r = patchAssistantModel(s.messages, modelId)
        return r.pendingModel !== undefined
          ? { pendingModel: r.pendingModel }
          : { messages: r.messages }
      })
      break
    }
    case 'token':
      if (!chunk.content) break
      _pendingTokens += chunk.content
      scheduleTokenFlush()
      break
    case 'reasoning':
    case 'tool_start':
    case 'tool_progress':
    case 'tool_end':
    case 'memory':
    case 'usage': {
      set((s) => ({
        messages: applyContentChunk(s.messages, chunk, {
          placeholderModel: s.pendingModel ?? s.copilotModel ?? undefined,
        }).messages,
      }))
      break
    }
    case 'tool_approval': {
      // Backend sends `id` (chat.rs), NOT tool_call_id. Dedup: a WS replay
      // can re-deliver a tool_approval chunk for an approval already resolved
      // on the backend; re-arming it guarantees a 404 on the next click. Skip
      // ids already acted on, already active, or missing a real id (the former
      // crypto.randomUUID() fallback guaranteed a 404 since no backend entry
      // ever matched it).
      const approvalId = chunk.id
      if (
        approvalId &&
        !_resolvedApprovalIds.has(approvalId) &&
        get().activeToolApproval?.id !== approvalId
      ) {
        set({
          activeToolApproval: {
            id: approvalId,
            toolName: chunk.tool_name || 'tool',
            reason: chunk.reason ?? '',
          },
          isStreaming: false,
        })
      }
      break
    }
    case 'interview':
      // Rare for one-shot; surface as a plain message and stop streaming.
      appendError(set, '이 질문은 추가 정보가 필요합니다. 채팅에서 다시 시도해 주세요.')
      break
    case 'error':
      appendError(set, chunk.error ?? '오류가 발생했습니다.')
      get()._drainPendingQueue()
      break
    case 'done': {
      const state = get()
      const assistant = [...state.messages].reverse().find((m) => m.role === 'assistant')
      const user = [...state.messages].reverse().find((m) => m.role === 'user')
      if (assistant && user) {
        const prompt = user.content
        const reply = assistant.content
        const blocks = assistant.blocks ?? []
        set({
          lastExchange: {
            prompt,
            reply,
            blocks,
            model: assistant.model ?? state.pendingModel ?? state.copilotModel ?? undefined,
            sessionId: chunk.session_id,
          },
        })
      }
      set({ isStreaming: false })
      const ws = get()._ws
      if (ws && ws.readyState === WebSocket.OPEN) ws.close()
      // Queue drain: if the user queued follow-ups while this turn streamed,
      // dispatch the next one now that the turn is idle.
      get()._drainPendingQueue()
      break
    }
    default:
      break
  }
}
