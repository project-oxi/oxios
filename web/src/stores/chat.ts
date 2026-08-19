import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { adaptChunk } from '@/lib/stream/adapter'
import type { ProcessorResult } from '@/lib/stream/StreamProcessor'
import { StreamProcessor } from '@/lib/stream/StreamProcessor'
import { uuid } from '@/lib/uuid'
import { usePortalStore } from '@/stores/portal'
import type {
  ChatActivity,
  ChatBlock,
  ChatMessage,
  CompressionInfo,
  InterviewAnswer,
  InterviewQuestion,
  Project,
  StreamChunk,
  ToolCallContext,
} from '@/types'
import { useAuthStore } from './auth'

// ---------------------------------------------------------------------------
// Persisted state (survives tab switches)
// ---------------------------------------------------------------------------

interface PersistedState {
  /** Last active session ID (null = no conversation started yet). */
  activeSessionId: string | null
  /** Project ID associated with the active session (grouping). */
  activeProjectId: string | null
  /** RFC-025: Active Mount IDs (comma-separated, primary first). */
  activeMountIds: string | null
  /** RFC-032: Active role hint (null = no role; uses default model). */
  activeRole: string | null
  /** Model override id (null = follow default / role). Persisted across reloads. */
  activeModelId: string | null
  /** Per-message sampling temperature. Persisted. `null` = use provider default. */
  temperature: number | null
  /** Per-message max output tokens. Persisted. `null` = use provider default. */
  maxTokens: number | null
}

const PERSIST_KEY = 'oxios-chat-persist'

// ---------------------------------------------------------------------------
// Runtime state
// ---------------------------------------------------------------------------

interface ChatRuntimeState {
  /** All messages in the current session (restored from /api/sessions/:id). */
  messages: ChatMessage[]
  isStreaming: boolean
  /** Epoch-ms when the current assistant turn started (set in sendMessage).
   *  Read by the LiveActivityBar holder to render an elapsed timer. Stale
   *  values are harmless — it's only consulted while isStreaming is true, and
   *  the next send overwrites it. */
  streamStartedAt: number | null
  /** Buffer for the backend model-announcement chunk (`type: 'model'`) that
   *  arrives before the first token; consumed when the assistant placeholder
   *  is created. Null when no turn is in flight. */
  pendingModel: string | null
  /** WebSocket connection state. */
  connected: boolean
  /** Queue of messages waiting for WS connection. */
  _sendQueue: string[]
  /** User messages queued while an assistant turn is streaming. Drained (in
   *  order) when the turn completes via `done`/`error`; cleared on a hard
   *  reset (disconnect / new session). The matching user message is added to
   *  the list only when the queue drains, so there are no ghost messages to
   *  clean up. */
  _pendingQueue: string[]
  /** The session ID from the last "done" chunk. */
  _lastDoneSessionId: string | null
  /** The project ID from the last "done" chunk. */
  _lastDoneProjectId: string | null
  /** AI-detected project (Phase 2 stub, always null). */
  detectedProject: Project | null
  /** RFC-025: detected mount tag from the last orchestrator response. */
  detectedMountTag: string | null
  /** RFC-025: detected mount IDs from the last orchestrator response. */
  detectedMountIds: string[]
  /** IDs of dismissed detection badges. */
  dismissedProjectIds: string[]
  /** Active structured interview questions (null = no interview active). */
  activeInterview: InterviewQuestion[] | null
  /** Active tool approval request awaiting user response (RFC-017). */
  activeToolApproval: {
    id: string
    toolName: string
    reason: string
  } | null
  /** Active path-access request awaiting user response (Mount/temp/deny). */
  activePathAccess: {
    id: string
    path: string
    mode: string
    toolName: string
    reason: string
  } | null
  /** Interview round number. */
  interviewRound: number
  /** Interview ambiguity score. */
  interviewAmbiguity: number
  /** LLM compression summary for the active session. */
  compression: CompressionInfo | null

  // ── WebSocket lifecycle (encapsulated, not persisted) ──
  /** WebSocket instance managed by the store. */
  _ws: WebSocket | null
  /** Reconnect timer (exponential backoff). */
  _reconnectTimer: number | null
  /** Reconnect attempt counter. */
  _reconnectAttempts: number
  /** RFC-024 SP2 (B4): client-side keepalive ping timer. Fires every
   *  `WS_CLIENT_PING_MS` so the server's pong-deadline is reset even
   *  when no app-level message is flowing. */
  _pingTimer: number | null
  /** RFC-024 SP2 (C2): highest `seq` we have observed on this WS.
   *  Persisted in `sessionStorage` so a hard refresh / tab reopen can
   *  resume the stream from the next message. */
  _lastSeq: number
  /** RFC-024 SP2 (C3): ring of recently-seen `msg.id` values for
   *  dedup. The replay buffer can return the same message twice
   *  (e.g. during a fast reconnect), so we drop ids we've already
   *  applied. Capacitied at `DEDUP_RING_MAX`. */
  _seenMsgIds: string[]
}

// ---------------------------------------------------------------------------
// Chat store — single source of truth for all chat state
// ---------------------------------------------------------------------------

interface ChatActions {
  /** Start or continue a WebSocket connection. */
  connect: () => Promise<void>
  /** Close the WebSocket and reset connection state. */
  disconnect: () => void
  /** RFC-024 SP2 (B4): cancel the client-side keepalive interval. */
  stopPingTimer: () => void
  /** Send a message using the active session. */
  sendMessage: (content: string) => void
  /** Dispatch the next queued user message (if any) now that the in-flight
   *  turn has completed. Reuses sendMessage's normal path. */
  _drainPendingQueue: () => void
  /** Load a previous session's message history from the API. */
  loadSession: (sessionId: string) => Promise<void>
  /** Start a fresh session (clears messages). */
  newSession: () => void
  /** Set the active project explicitly. */
  setActiveProject: (projectId: string | null) => void
  /** RFC-032: Set the active role hint. */
  setActiveRole: (role: string | null) => void
  /** Set the per-message model override id (null = no override). */
  setActiveModelId: (modelId: string | null) => void
  /** Set the per-message sampling temperature (null = provider default). */
  setTemperature: (temperature: number | null) => void
  /** Set the per-message max output tokens (null = provider default). */
  setMaxTokens: (maxTokens: number | null) => void
  /** RFC-025: accept detected mount IDs into the active binding. */
  setActiveMountIds: (mountIds: string[] | null) => void
  /** RFC-025: Clear detected mount tag and IDs (e.g. on badge accept/dismiss). */
  clearDetectedMount: () => void
  setDetectedProject: (project: Project | null) => void
  /** Dismiss a detection badge (don't show again for this project). */
  dismissDetection: (projectId: string) => void
  /** Remove a single message by id. Used by the inline error retry flow (RFC-032). */
  removeMessage: (id: string) => void
  /** Clear persisted state (e.g. on logout). */
  clearPersist: () => void
  /** Submit interview answers and send them as a message. */
  submitInterviewResponse: (answers: InterviewAnswer[]) => void
  /** Resolve a pending tool approval (RFC-017). */
  resolveToolApproval: (id: string, approved: boolean, remember?: boolean) => Promise<void>
  /** Resolve a pending path-access request (mount/temp/deny). */
  resolvePathAccess: (id: string, action: 'mount' | 'temp' | 'deny') => Promise<void>
  /** Handle an incoming WS chunk. */
  handleChunk: (chunk: StreamChunk) => void
}

export type ChatStore = PersistedState & ChatRuntimeState & ChatActions

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// F6: known StreamChunk type values. Unknown types are coerced to an error
// chunk so downstream handlers never operate on an unrecognised shape (which
// could produce undefined activity IDs and React key collisions).
const KNOWN_CHUNK_TYPES = new Set<StreamChunk['type']>([
  'token',
  'tool_call',
  'tool_result',
  'done',
  'error',
  'phase',
  'tool_start',
  'tool_end',
  'tool_progress',
  'tool_call_delta',
  'memory',
  'reasoning',
  'usage',
  'interview',
  'grounding',
  'tool_approval',
  'path_access',
  'model',
  'compression_delta',
  'compression_done',
  'compression_failed',
])

export function parseChunk(raw: unknown): StreamChunk {
  if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
    const obj = raw as Record<string, unknown>
    const t = obj.type
    if (typeof t === 'string' && KNOWN_CHUNK_TYPES.has(t as StreamChunk['type'])) {
      return obj as unknown as StreamChunk
    }
  }
  return { type: 'error', error: 'Malformed chunk' }
}

// ---------------------------------------------------------------------------
// Shared message-transform primitives
// ---------------------------------------------------------------------------
// Pure helpers that mutate a message list by transforming the last assistant
// message (creating a placeholder if absent). chat.ts (RAF token flush,
// activity/done cases) and quick-ask.ts (token/activity cases) all route
// through these so the "find-or-create assistant + copy array + replace"
// ceremony — the actual duplication that previously let the reasoning-merge
// bug drift in — is defined exactly once. Per-store handleChunk keeps its own
// divergent side-effect cases (done/error/interview/tool_approval).
// ---------------------------------------------------------------------------

/** Context for assistant-placeholder creation. */
export interface AssistantCtx {
  /** Model stamped onto a newly-created assistant placeholder. */
  placeholderModel?: string | null
}

// ---------------------------------------------------------------------------
// Turn-boundary invariant — single source of truth
// ---------------------------------------------------------------------------
// The store streams one assistant message per turn, and that message is always
// the TRAILING entry in `messages`: `sendMessage` appends only the user
// message, and the assistant placeholder is created lazily on the first
// content chunk (never optimistically on send — the LiveActivityBar owns the
// pre-chunk "thinking" affordance, so an empty bubble would only duplicate it).
//
// Corollary: an assistant from a PRIOR turn is never the streaming target.
// Once a new user message is appended it is the trailing message until a fresh
// placeholder is created. EVERY site that routes a chunk to "the assistant"
// must go through `currentAssistant` — searching backward for the last
// assistant anywhere in the list silently targets the previous turn's response
// and overwrites it (the multi-turn disappearance/ordering bug).

/** The current turn's assistant placeholder, or undefined when a user message
 *  is still waiting for its first content chunk. Pure. */
function currentAssistant(messages: ChatMessage[]): ChatMessage | undefined {
  const last = messages[messages.length - 1]
  return last?.role === 'assistant' ? last : undefined
}
/**
 * Ensure the last message is an assistant message, appending an empty
 * placeholder if not. Returns the (possibly unchanged) list and the index of
 * the last assistant message. Pure.
 */
export function ensureLastAssistant(
  messages: ChatMessage[],
  ctx: AssistantCtx,
): { messages: ChatMessage[]; index: number } {
  if (currentAssistant(messages)) {
    return { messages, index: messages.length - 1 }
  }
  const placeholder: ChatMessage = {
    id: uuid(),
    role: 'assistant',
    content: '',
    timestamp: new Date().toISOString(),
    model: ctx.placeholderModel ?? undefined,
  }
  return { messages: [...messages, placeholder], index: messages.length }
}

/**
 * Append token text to the last assistant message, creating a placeholder if
 * none exists. Pure. Used by chat's RAF token-flush and quick-ask's immediate
 * token append — the transformation is identical; only the batching strategy
 * differs (kept per-store).
 */
export function appendTokenToMessages(
  messages: ChatMessage[],
  content: string,
  ctx: AssistantCtx,
): ChatMessage[] {
  if (!content) return messages
  const { messages: ensured, index } = ensureLastAssistant(messages, ctx)
  const target = ensured[index]!
  const next = ensured.slice()
  next[index] = { ...target, content: target.content + content }
  return next
}

/**
 * Append/merge an activity onto the last assistant message (creating a
 * placeholder if absent) and accumulate token counts. Pure. Delegates
 * dedup/merge to mergeOrAppendActivity. Used by both chat and quick-ask so the


/**
 * Patch the model of the last assistant message; if no assistant message
 * exists yet, return a `pendingModel` signal so the store can stash it for the
 * next placeholder (consumed via AssistantCtx.placeholderModel). Pure. Lets
 * quick-ask patch the live message too, matching chat's behaviour.
 */
export function patchAssistantModel(
  messages: ChatMessage[],
  modelId: string,
): { messages: ChatMessage[]; pendingModel?: string } {
  // Turn-aware (see lastAssistantMessageId): patch only the current turn's
  // placeholder — the trailing message. When the trailing message is a user
  // message (new turn, no assistant created yet), stash as pendingModel so
  // the next placeholder picks it up; otherwise we'd overwrite the PREVIOUS
  // turn's assistant `model` field instead of routing to the new turn.
  const cur = currentAssistant(messages)
  if (cur) {
    const next = messages.slice()
    next[next.length - 1] = { ...cur, model: modelId }
    return { messages: next }
  }
  return { messages, pendingModel: modelId }
}
/**
 * Finalize the trailing assistant message when a turn ends abnormally
 * (WS close, cancel/disconnect). A placeholder that never received any
 * chunk (no content/reasoning/toolCalls/activities) is a ghost — drop it.
 * A placeholder with partial data is kept with `generating` cleared so its
 * spinner stops. Pure. Pairs with the optimistic placeholder `sendMessage`
 * appends: without this, a cancel before the first chunk would leave an
 * empty "Thinking…" bubble stuck on screen.
 */
export function finalizeStreamingMessage(messages: ChatMessage[]): ChatMessage[] {
  const last = currentAssistant(messages)
  if (!last?.generating) return messages
  // Blocks are the single source of truth; a turn with non-empty blocks is
  // not empty even if `content` is blank.
  const isEmpty = !(last.content ?? '').trim() && !(last.blocks && last.blocks.length > 0)
  if (isEmpty) return messages.slice(0, -1)
  return messages.map((m, i) =>
    i === messages.length - 1
      ? {
          ...m,
          generating: false,
          isToolCallGenerating: false,
        }
      : m,
  )
}

function trajectoryToActivity(step: {
  tool_name: string
  tool_args: unknown
  output_summary: string
  duration_ms: number
  is_error: boolean
  tool_call_id: string
  timestamp: string
  context?: ToolCallContext
}): ChatActivity {
  return {
    id: step.tool_call_id,
    type: 'tool_call',
    timestamp: step.timestamp,
    toolName: step.tool_name,
    toolCallId: step.tool_call_id,
    toolArgs: (step.tool_args as Record<string, unknown> | undefined) ?? undefined,
    outputSummary: step.output_summary,
    durationMs: step.duration_ms,
    isError: step.is_error,
    ...(step.context ? { context: step.context } : {}),
  }
}

export function getToken(): string {
  return useAuthStore.getState().token || ''
}

/**
 * Whether the backend has authentication enabled.
 *
 * Learned from `GET /api/status`, which is reachable without a token exactly
 * when auth is disabled (`require_auth` skips when `auth_enabled=false`). The
 * result is cached: `auth_enabled` only changes across a daemon restart, so a
 * single probe per page session is sufficient.
 */
let authEnabledCached: boolean | null = null
/** Test-only: reset the cached auth-enabled probe so a fresh connect re-probes. */
export function __clearAuthCacheForTesting(): void {
  authEnabledCached = null
}

async function isAuthEnabled(): Promise<boolean> {
  if (authEnabledCached !== null) return authEnabledCached
  try {
    const res = await fetch('/api/status', { headers: { Accept: 'application/json' } })
    // 401/403 means the endpoint demands auth → auth is on.
    if (res.status === 401 || res.status === 403) {
      authEnabledCached = true
      return true
    }
    if (res.ok) {
      const data = (await res.json().catch(() => null)) as { auth_enabled?: boolean } | null
      authEnabledCached = data?.auth_enabled === true
      return authEnabledCached
    }
    // 503 = subsystems still warming up (the readiness gate returns 503 until
    // the engine/state-store reach Ready/Degraded). That is not an auth signal,
    // so default to auth-off (the common case) and leave the cache unset so the
    // next connect attempt re-probes once the server is ready.
    if (res.status === 503) return false
  } catch {
    // Network error — fall through to the conservative default below.
  }
  // Could not determine: default to auth-enabled so a protected deployment is
  // never silently bypassed. The common auth-off case resolves via the 200
  // path above; this only governs genuine request failures.
  return true
}

export async function buildWsUrl(): Promise<string> {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const base = `${protocol}//${window.location.host}/api/chat/stream`

  // When auth is disabled (the default for local single-user deployments),
  // connect without credentials. The backend skips ticket/token validation,
  // and a browser WebSocket cannot carry a Bearer header anyway — so blocking
  // the connection on a missing token only stranded deployments that have no
  // login UI to set one.
  if (!(await isAuthEnabled())) {
    return base
  }

  const token = getToken()

  // Auth is enabled — a token is mandatory.
  if (!token) {
    throw new Error('Cannot open WebSocket: not authenticated')
  }

  // Prefer a short-lived ticket so the token itself never appears in the URL
  // (URLs are logged by proxies and may leak via Referer).
  try {
    const res = await fetch('/api/chat/ticket', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
    })
    if (res.ok) {
      const data = await res.json()
      if (data.ticket) {
        return `${base}?ticket=${encodeURIComponent(data.ticket)}`
      }
    }
  } catch {
    // Ticket endpoint not available — fall through to token query param.
  }

  // F3: WS cannot use custom headers, so the token must travel as a query
  // parameter. This is strictly better than the previous behaviour which sent
  // a completely unauthenticated request when the ticket endpoint failed.
  return `${base}?token=${encodeURIComponent(token)}`
}

/** Max fast-backoff reconnect attempts before switching to long-tail retry. */
const MAX_RECONNECT_ATTEMPTS = 5
/**
 * Steady retry cadence used after the fast exponential backoff exhausts.
 *
 * Without this, a daemon restart that outlasted the ~31 s fast-backoff window
 * (5 attempts: 1+2+4+8+16 s) stranded the tab permanently at
 * `connected === false` — the chat input disabled and the reconnect banner
 * (`chat.reconnecting`) stuck open, recoverable only by a full page refresh.
 * The long-tail retry keeps probing at a gentle cadence so the client always recovers once
 * the daemon returns, with no user action and no hammering (one attempt per
 * interval; `connect()` early-returns while a socket is already open, and
 * `onopen` resets the counter to 0).
 */
const RECONNECT_LONG_TAIL_MS = 10_000

// RFC-024 SP2 (B4): client-side keepalive interval. Independent of the
// server's 20 s ping — sending our own ping every 25 s means the
// server's 60 s pong-deadline is reset on either side's traffic, so
// the connection survives NAT/proxy timeouts that fire anywhere from
// 30 s (aggressive) to 5 min (lenient).
const WS_CLIENT_PING_MS = 25_000

// RFC-024 SP2 (C3): cap on the dedup ring. 256 is generous — the
// server's replay buffer is 512 by default, so anything we've seen
// in the last 256 messages we will not apply twice.
const DEDUP_RING_MAX = 256

// RFC-024 SP2 (C2): sessionStorage keys. We deliberately use
// sessionStorage (not localStorage): the cursor is per-tab, and a
// different tab's session/project should not contaminate this one.
const SS_LAST_SEQ_KEY = 'oxios:ws:last_seq'
const SS_SEEN_IDS_KEY = 'oxios:ws:seen_ids'

function loadLastSeq(): number {
  try {
    const raw = sessionStorage.getItem(SS_LAST_SEQ_KEY)
    if (!raw) return 0
    const n = Number.parseInt(raw, 10)
    return Number.isFinite(n) && n >= 0 ? n : 0
  } catch {
    return 0
  }
}

function saveLastSeq(seq: number): void {
  try {
    sessionStorage.setItem(SS_LAST_SEQ_KEY, String(seq))
  } catch {
    // sessionStorage may be unavailable (private mode, quota); degrade silently.
  }
}

function loadSeenIds(): string[] {
  try {
    const raw = sessionStorage.getItem(SS_SEEN_IDS_KEY)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === 'string') : []
  } catch {
    return []
  }
}

function saveSeenIds(ids: string[]): void {
  try {
    sessionStorage.setItem(SS_SEEN_IDS_KEY, JSON.stringify(ids))
  } catch {
    // see saveLastSeq
  }
}

// Returns true if this is a new id (caller should apply), false if
// we have seen it before (caller should drop). Mutates `ring` in place.
function markSeen(id: string, ring: string[]): boolean {
  if (ring.includes(id)) return false
  ring.push(id)
  // Cap the ring so a burst of replayed messages does not leave the
  // dedup set carrying entries the user will never see again. We
  // drop down to DEDUP_RING_MAX (rather than splice one at a time)
  // so a flood of replays after a long offline period is O(1) total.
  if (ring.length > DEDUP_RING_MAX) ring.length = DEDUP_RING_MAX
  return true
}
// Tool-approval ids the user has already acted on (approved/denied). A WS
// reconnect replay (RFC-024 SP2 C2) can re-deliver a `tool_approval` chunk
// for an approval already resolved on the backend; without this set, the
// replay re-arms a dead card whose every click returns 404. Bounded so a
// long session cannot grow it without limit.
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
// F9: Token-streaming batching
// ---------------------------------------------------------------------------
// Each incoming token chunk previously rebuilt the entire messages array
// (O(n) per token → O(n×t) for a response of t tokens across n messages),
// triggering a Zustand subscriber re-render on every token. We instead
// accumulate token content in a module-scoped buffer and flush it at most once
// per animation frame. Any non-token chunk flushes synchronously first so
// streamed text is never lost when a tool/done/error event arrives mid-stream.
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
  useChatStore.setState((s) => {
    const msgId = lastAssistantMessageId(s.messages)
    if (!msgId) {
      // No assistant target — fall back to legacy append behavior.
      return {
        messages: appendTokenToMessages(s.messages, content, {
          placeholderModel: s.pendingModel ?? s.activeModelId,
        }),
      }
    }
    const processor = getOrCreateProcessor(msgId)
    const result = processor.handleEvent({
      kind: 'text.delta',
      messageId: msgId,
      text: content,
    })
    return {
      messages: applyProcessorResult(s.messages, msgId, result, {
        placeholderModel: s.pendingModel ?? s.activeModelId,
      }),
    }
  })
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

// ---------------------------------------------------------------------------
// StreamProcessor integration (Phase 1, 2026-07-21)
// ---------------------------------------------------------------------------
// One processor per active assistant message. Phase 1 has a single stream at
// a time, but the map keys by message id for forward-compat with concurrent
// streams (background agents, A2A).

/** Test-only: clear all StreamProcessor instances. Phase 1 has module-level
 *  processor state that would otherwise leak between tests. */
export function __clearStreamProcessorsForTesting(): void {
  streamProcessors.clear()
}
const streamProcessors = new Map<string, StreamProcessor>()

function getOrCreateProcessor(messageId: string): StreamProcessor {
  let p = streamProcessors.get(messageId)
  if (!p) {
    p = new StreamProcessor(messageId)
    streamProcessors.set(messageId, p)
  }
  return p
}

/** Id of the current turn's assistant placeholder, or null. See
 *  `currentAssistant` for the turn-boundary invariant. */
function lastAssistantMessageId(messages: ChatMessage[]): string | null {
  return currentAssistant(messages)?.id ?? null
}

/** Apply a StreamProcessor result to a specific message by id, creating an
 *  assistant placeholder if absent. Returns new messages array (immutable). */
function applyProcessorResult(
  messages: ChatMessage[],
  msgId: string,
  result: ProcessorResult,
  ctx: AssistantCtx,
): ChatMessage[] {
  let next = messages
  // Ensure target message exists — create empty assistant placeholder.
  if (!messages.some((m) => m.id === msgId)) {
    const placeholder: ChatMessage = {
      id: msgId,
      role: 'assistant',
      content: '',
      timestamp: new Date().toISOString(),
      model: ctx.placeholderModel ?? undefined,
      generating: true,
    }
    next = messages.concat(placeholder)
  }
  // Apply patch.
  if (result.patch && Object.keys(result.patch).length > 0) {
    next = next.map((m) => (m.id === msgId ? { ...m, ...result.patch } : m))
  }
  return next
}

// ---------------------------------------------------------------------------
// Pure content-chunk routing — shared with quick-ask.ts.
// Encapsulates adapt → processor → applyProcessorResult so other stores don't
// duplicate the helpers or the processor map. Returns a new messages array
// (caller wraps with setState).
// ---------------------------------------------------------------------------

/** Apply one content-streaming chunk (token, reasoning, tool_*, phase) to the
 *  messages list using the StreamProcessor. Pure. */
export function applyContentChunk(
  messages: ChatMessage[],
  chunk: StreamChunk,
  ctx: AssistantCtx,
): { messages: ChatMessage[]; finishedMsgId?: string } {
  const msgId = lastAssistantMessageId(messages)
  if (!msgId) return { messages }
  const processor = getOrCreateProcessor(msgId)
  const { events } = adaptChunk(chunk, { msgId })
  let next = messages
  let finishedMsgId: string | undefined
  for (const ev of events) {
    const result = processor.handleEvent(ev)
    next = applyProcessorResult(next, msgId, result, ctx)
    if (result.finished) {
      finishedMsgId = msgId
      streamProcessors.delete(msgId)
    }
  }
  return { messages: next, finishedMsgId }
}

/** Flush a buffered text run to the last assistant message via StreamProcessor.
 *  Falls back to appendTokenToMessages when no assistant target exists. Pure. */
export function applyTextFlush(
  messages: ChatMessage[],
  text: string,
  ctx: AssistantCtx,
): ChatMessage[] {
  if (!text) return messages
  const msgId = lastAssistantMessageId(messages)
  if (!msgId) return appendTokenToMessages(messages, text, ctx)
  const processor = getOrCreateProcessor(msgId)
  const result = processor.handleEvent({ kind: 'text.delta', messageId: msgId, text })
  return applyProcessorResult(messages, msgId, result, ctx)
}
// ---------------------------------------------------------------------------
// Store definition
// ---------------------------------------------------------------------------
export const useChatStore = create<ChatStore>()(
  persist(
    (set, get) => ({
      activeSessionId: null,
      activeProjectId: null,
      activeMountIds: null,
      activeRole: null,
      activeModelId: null,
      temperature: null,
      maxTokens: null,

      // ── Runtime ──
      messages: [],
      isStreaming: false,
      streamStartedAt: null as number | null,
      pendingModel: null as string | null,
      connected: false,
      _sendQueue: [],
      _pendingQueue: [],
      _lastDoneSessionId: null,
      _lastDoneProjectId: null,
      detectedProject: null,
      detectedMountTag: null,
      detectedMountIds: [],
      dismissedProjectIds: [],
      activeInterview: null,
      interviewRound: 0,
      interviewAmbiguity: 0,
      compression: null,
      activeToolApproval: null,
      activePathAccess: null,
      // WebSocket lifecycle
      _ws: null,
      _reconnectTimer: null,
      _reconnectAttempts: 0,
      _pingTimer: null,
      // RFC-024 SP2 (C2): restore the seq cursor from sessionStorage
      // so a hard refresh / new tab can resume the stream without
      // gaps. Both values are best-effort — if the server's buffer
      // is older than the saved cursor, the server emits a `resync`
      // chunk and the client falls back to a full state refresh.
      _lastSeq: loadLastSeq(),
      _seenMsgIds: loadSeenIds(),
      // ── Actions ──
      async connect() {
        const currentWs = get()._ws

        // Already connected — nothing to do.
        if (currentWs && currentWs.readyState === WebSocket.OPEN) return
        if (typeof window === 'undefined') return

        // Tear down any previous connection (stale or connecting).
        if (currentWs) {
          currentWs.onopen = null
          currentWs.onmessage = null
          currentWs.onclose = null
          currentWs.onerror = null
          if (
            currentWs.readyState === WebSocket.OPEN ||
            currentWs.readyState === WebSocket.CONNECTING
          ) {
            currentWs.close()
          }
        }

        // Clear any pending reconnect timer.
        const prevTimer = get()._reconnectTimer
        if (prevTimer) {
          clearTimeout(prevTimer)
          set({ _reconnectTimer: null })
        }

        let url: string
        try {
          url = await buildWsUrl()
        } catch {
          // F3: not authenticated — abort the connection attempt gracefully
          // instead of letting the rejection propagate unhandled.
          return
        }
        const ws = new WebSocket(url)

        // Store reference so stale-checks work.
        set({ _ws: ws, connected: false, isStreaming: false })

        ws.onopen = () => {
          // If another connect() replaced this ws, ignore.
          if (get()._ws !== ws) return
          set({ connected: true, _reconnectAttempts: 0 })

          // RFC-024 SP2 (C2): if we have a saved cursor, ask the server
          // to replay any messages we missed while disconnected. The
          // server either broadcasts the gapless slice or, if the
          // cursor is older than its replay buffer, sends a synthetic
          // `resync` chunk so we can pull fresh state via HTTP.
          const lastSeq = get()._lastSeq
          if (lastSeq > 0) {
            ws.send(JSON.stringify({ type: 'resume', last_seq: lastSeq }))
          }

          // RFC-024 SP2 (B4): start the client-side keepalive. We send
          // our own ping every WS_CLIENT_PING_MS so the server's
          // 60 s pong-deadline is reset by either side's traffic.
          // Browsers do not auto-pong application-level pings.
          get().stopPingTimer()
          const pingTimer = window.setInterval(() => {
            if (get()._ws === ws && ws.readyState === WebSocket.OPEN) {
              try {
                ws.send(JSON.stringify({ type: 'ping' }))
              } catch {
                // Send can throw if the socket was just closed between
                // the readyState check and the send; the close handler
                // will deal with the reconnect.
              }
            }
          }, WS_CLIENT_PING_MS)
          set({ _pingTimer: pingTimer })

          // Flush queued messages.
          const queue = get()._sendQueue
          if (queue.length > 0) {
            set({ _sendQueue: [] })
            for (const msg of queue) {
              get().sendMessage(msg)
            }
          }
        }

        ws.onmessage = (event) => {
          // Stale connection — ignore.
          if (get()._ws !== ws) return
          try {
            const raw = JSON.parse(event.data as string) as Record<string, unknown>
            // RFC-024 SP2 (C2): track the highest seq we have observed
            // so the next reconnect can resume from here. Persisted
            // eagerly so a crash mid-stream still leaves a usable
            // cursor.
            const seq = raw.seq
            if (typeof seq === 'number' && seq > get()._lastSeq) {
              set({ _lastSeq: seq })
              saveLastSeq(seq)
            }
            // RFC-024 SP2 (C3): drop replays of messages we have
            // already applied. The server's replay path can deliver
            // duplicates when the cursor is just inside the buffer
            // window; without dedup the user would see the same
            // token stream rendered twice.
            const msgId = raw.id
            if (typeof msgId === 'string') {
              const ring = get()._seenMsgIds
              if (!markSeen(msgId, ring)) return
              saveSeenIds(ring)
            }
            const chunk = parseChunk(raw)
            get().handleChunk(chunk)
          } catch {
            // Ignore malformed JSON
          }
        }

        ws.onclose = () => {
          // Another connect() already replaced this ws — do nothing.
          if (get()._ws !== ws) return

          // RFC-024 SP2 (B4): stop the client-side keepalive so the
          // orphaned timer does not keep firing after the socket is
          // gone (it would silently fail in `onopen` above, but the
          // interval itself would survive until disconnect() or a new
          // connect()).
          get().stopPingTimer()

          set((s) => ({
            connected: false,
            isStreaming: false,
            _ws: null,
            _pendingQueue: [],
            messages: finalizeStreamingMessage(s.messages),
          }))

          // Auto-reconnect. Fast exponential backoff for the first
          // MAX_RECONNECT_ATTEMPTS tries, then a steady long-tail retry so a
          // daemon restart that outlasts the backoff window still recovers
          // instead of stranding the tab at connected === false forever.
          const attempt = get()._reconnectAttempts
          const inFastBackoff = attempt < MAX_RECONNECT_ATTEMPTS
          const delay = inFastBackoff ? 1000 * 2 ** attempt : RECONNECT_LONG_TAIL_MS
          window.setTimeout(() => {
            set({ _reconnectTimer: null })
            // Only reconnect if no new connection was established in the meantime.
            if (get()._ws === null) {
              // Pin the counter at the cap during long-tail retry; onopen
              // resets it to 0 once a connection finally succeeds.
              set({ _reconnectAttempts: inFastBackoff ? attempt + 1 : attempt })
              get().connect()
            }
          }, delay)
        }

        ws.onerror = () => {
          if (get()._ws !== ws) return
          ws.close()
        }
      },

      // RFC-024 SP2 (B4): cancel the keepalive interval. Safe to
      // call from `disconnect`, the close handler, or the start of
      // a new `connect()` to avoid overlapping timers.
      stopPingTimer() {
        const t = get()._pingTimer
        if (t !== null) {
          window.clearInterval(t)
          if (get()._pingTimer === t) set({ _pingTimer: null })
        }
      },

      disconnect() {
        const { _ws, _reconnectTimer } = get()

        // Stop any pending reconnect.
        if (_reconnectTimer) clearTimeout(_reconnectTimer)

        // RFC-024 SP2 (B4): kill the keepalive timer.
        get().stopPingTimer()

        if (_ws) {
          // Detach handlers before closing to prevent onclose from
          // triggering auto-reconnect.
          _ws.onopen = null
          _ws.onmessage = null
          _ws.onclose = null
          _ws.onerror = null
          _ws.close()
        }

        // F9: flush any buffered tokens before tearing down the connection so
        // the final streamed content is committed to the message.
        flushPendingTokens()
        set((s) => ({
          connected: false,
          isStreaming: false,
          _pendingQueue: [],
          _ws: null,
          _reconnectTimer: null,
          _reconnectAttempts: 0,
          messages: finalizeStreamingMessage(s.messages),
        }))
      },

      sendMessage(content: string) {
        const {
          activeSessionId,
          activeProjectId,
          activeMountIds,
          activeRole,
          activeModelId,
          temperature,
          maxTokens,
          connected,
          connect,
          _ws,
        } = get()

        // Ensure WS is connected first
        if (!connected || !_ws || _ws.readyState !== WebSocket.OPEN) {
          connect()
          const q = get()._sendQueue
          if (!q.includes(content)) {
            set({ _sendQueue: [...q, content] })
          }
          return
        }
        // Streaming: defer this message until the in-flight turn completes.
        // The caller has already cleared the textarea; we just stash the
        // content. It is dispatched (and added to the message list) when the
        // done/error handler drains the queue — so there are no ghost
        // messages to clean up on cancel/reconnect.
        if (get().isStreaming) {
          set((s) => ({ _pendingQueue: [...s._pendingQueue, content] }))
          return
        }

        // Optimistic: add the user message immediately and mark the turn live.
        // No assistant placeholder is created here — the LiveActivityBar holder
        // (pinned above the input) owns the "what's happening" indicator for the
        // whole turn, including the assess→crystallize→execute gap before the
        // first chunk, so an empty bubble would only duplicate it. The assistant
        // message is created lazily on the first reasoning/tool/token chunk and
        // patched in place thereafter; `done`/`error` merge into it.
        const userMsg: ChatMessage = {
          id: uuid(),
          role: 'user',
          content,
          timestamp: new Date().toISOString(),
        }
        set((s) => ({
          messages: [...s.messages, userMsg],
          isStreaming: true,
          streamStartedAt: Date.now(),
        }))

        // Send via WebSocket with session context.
        // The backend WS handler reads `model` and writes it into
        // `model_override` metadata, which the orchestrator honours
        // at priority 1 (above role routing and default).
        const payload: Record<string, unknown> = {
          type: 'message',
          content,
          session_id: activeSessionId ?? '',
          // Web-C2: backend WS handler reads singular `project_id`
          project_id: activeProjectId ?? '',
          mount_ids: activeMountIds ?? '',
          // RFC-032: role hint for model routing
          role: activeRole ?? '',
          // Per-message model override (or last-picked persistent one).
          model: activeModelId ?? '',
        }
        if (temperature != null) payload.temperature = temperature
        if (maxTokens != null) payload.max_tokens = maxTokens
        _ws.send(JSON.stringify(payload))
      },

      _drainPendingQueue() {
        const { _pendingQueue } = get()
        if (_pendingQueue.length === 0) return
        // Shift the head before dispatching: sendMessage's normal path runs
        // here because isStreaming was just cleared by the done/error handler.
        const next = _pendingQueue[0]
        if (next === undefined) return
        set({ _pendingQueue: _pendingQueue.slice(1) })
        get().sendMessage(next)
      },

      async loadSession(sessionId: string) {
        if (!sessionId) return
        // F9: discard buffered tokens from any prior streaming session.
        discardPendingTokens()
        try {
          const res = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}`, {
            headers: {
              Authorization: `Bearer ${getToken()}`,
              'Content-Type': 'application/json',
            },
          })
          if (!res.ok) return

          const data = await res.json()

          // Reconstruct messages from session history
          const messages: ChatMessage[] = []
          const userMsgs: { content: string; timestamp?: string }[] = data.user_messages ?? []
          const agentMsgs: Array<{
            content: string
            timestamp?: string
            trajectory_range?: { start: number; end: number }
          }> = data.agent_responses ?? []
          const trajectorySteps: Array<{
            tool_name: string
            tool_args: unknown
            output_summary: string
            duration_ms: number
            is_error: boolean
            tool_call_id: string
            timestamp: string
            context?: ToolCallContext
          }> = data.trajectory_steps ?? []
          const trajectoryActivities = trajectorySteps.map(trajectoryToActivity)
          const reasoningRecords: Array<{
            content: string
            source: string
            timestamp: string
            segments?: Array<{ before_step: number; text: string }>
          }> = data.reasoning_records ?? []

          const maxLen = Math.max(userMsgs.length, agentMsgs.length)
          for (let i = 0; i < maxLen; i++) {
            const userMsg = userMsgs[i]
            const agentMsg = agentMsgs[i]
            if (userMsg != null) {
              messages.push({
                id: uuid(),
                role: 'user',
                content: userMsg.content,
                timestamp: userMsg.timestamp ?? data.created_at,
              })
            }
            if (agentMsg) {
              const range = agentMsg.trajectory_range
              const blocks: ChatBlock[] = []
              const toolSlice =
                range && trajectoryActivities.length > 0
                  ? trajectoryActivities.slice(range.start, range.end)
                  : i === maxLen - 1 && trajectoryActivities.length > 0
                    ? trajectoryActivities
                    : []
              // P4 (§7 persistence) + block-stream P2: restore the turn as
              // an ordered ChatBlock[] directly from the trajectory slice
              // and the reasoning segments (no ChatActivity intermediate).
              const reasoning = reasoningRecords[i]
              const segs = reasoning?.segments
              const toolBlockCount = toolSlice.filter((a) => a.type === 'tool_call').length
              // Build a { before_step -> text[] } index of reasoning segments.
              const byPos = new Map<number, string[]>()
              if (segs && segs.length > 0) {
                for (const s of segs) {
                  const arr = byPos.get(s.before_step) ?? []
                  arr.push(s.text)
                  byPos.set(s.before_step, arr)
                }
              }
              let toolIdx = 0
              for (let pos = 0; pos <= toolBlockCount; pos++) {
                const texts = byPos.get(pos)
                if (texts) {
                  for (const text of texts) {
                    blocks.push({
                      type: 'reasoning',
                      id: `reason-${i}-${pos}-${blocks.length}`,
                      text,
                      status: 'done',
                      startedAt: Date.parse(reasoning!.timestamp) || Date.now(),
                      ...(reasoning!.source
                        ? { source: reasoning!.source as 'thinking' | 'compaction' }
                        : {}),
                    })
                  }
                }
                while (toolIdx < toolSlice.length) {
                  const a = toolSlice[toolIdx]!
                  if (a.type !== 'tool_call') {
                    toolIdx++
                    continue
                  }
                  blocks.push({
                    type: 'tool',
                    id: a.toolCallId ?? a.id,
                    identifier: 'kernel',
                    apiName: a.toolName ?? 'unknown',
                    arguments: a.toolArgs ?? {},
                    status: a.isError ? 'error' : 'success',
                    ...(a.outputSummary != null ? { result: a.outputSummary } : {}),
                    ...(a.durationMs != null ? { durationMs: a.durationMs } : {}),
                  })
                  toolIdx++
                  break
                }
              }
              if (agentMsg.content) {
                blocks.push({ type: 'text', id: `t-${i}-1`, text: agentMsg.content })
              }
              const assistantMessage: ChatMessage = {
                id: uuid(),
                role: 'assistant',
                content: agentMsg.content ?? '',
                timestamp: agentMsg.timestamp ?? data.updated_at,
              }
              messages.push({
                ...assistantMessage,
                ...(blocks.length > 0 ? { blocks } : null),
              })
            }
          }

          const projectId =
            data.project_id ?? data.metadata?.project_id ?? data.metadata?.project_ids ?? null

          set({
            messages,
            activeSessionId: sessionId,
            activeProjectId: projectId,
            isStreaming: false,
            _pendingQueue: [],
            compression: (data.compression ?? null) as CompressionInfo | null,
          })
        } catch {
          // Silently fail — network issues shouldn't break the UI
        }
      },
      newSession() {
        // F9: discard any buffered tokens from the previous session so they
        // don't leak into the new session via a late rAF callback.
        discardPendingTokens()
        set(() => ({
          messages: [],
          isStreaming: false,
          _pendingQueue: [],
          pendingModel: null,
          activeSessionId: null,
          _lastDoneSessionId: null,
          _lastDoneProjectId: null,
          activeInterview: null,
          interviewRound: 0,
          interviewAmbiguity: 0,
          compression: null,
        }))
      },

      setActiveProject(projectId: string | null) {
        // F9: discard buffered tokens when switching projects (clears messages).
        discardPendingTokens()
        set({
          activeProjectId: projectId,
          activeSessionId: null,
          messages: [],
          _pendingQueue: [],
          detectedProject: null,
        })
      },

      setActiveMountIds(mountIds: string[] | null) {
        set({
          activeMountIds: mountIds ? mountIds.join(',') : null,
        })
      },

      setActiveRole(role: string | null) {
        set({ activeRole: role })
      },

      setActiveModelId(modelId: string | null) {
        set({ activeModelId: modelId })
      },

      setTemperature(temperature: number | null) {
        set({ temperature })
      },

      setMaxTokens(maxTokens: number | null) {
        set({ maxTokens })
      },

      removeMessage(id: string) {
        // F9: discard any buffered tokens so they don't leak into the next
        // streaming turn when the user retries an errored message.
        discardPendingTokens()
        set((s) => {
          const target = s.messages.find((m) => m.id === id)
          const wasStreaming = target?.role === 'assistant' && s.isStreaming
          return {
            messages: s.messages.filter((m) => m.id !== id),
            // If we just removed the streaming assistant placeholder, drop
            // isStreaming so the input is re-enabled and the user can retry.
            isStreaming: wasStreaming ? false : s.isStreaming,
          }
        })
      },

      setDetectedMountTag(tag: string | null) {
        set({ detectedMountTag: tag })
      },

      clearDetectedMount() {
        set({ detectedMountTag: null, detectedMountIds: [] })
      },

      setDetectedProject(project: Project | null) {
        set({ detectedProject: project })
      },

      dismissDetection(projectId: string) {
        set((s) => ({
          dismissedProjectIds: [...s.dismissedProjectIds, projectId],
          detectedProject: s.detectedProject?.id === projectId ? null : s.detectedProject,
        }))
      },

      clearPersist() {
        set({
          activeSessionId: null,
          activeProjectId: null,
          activeMountIds: null,
          activeRole: null,
          activeModelId: null,
          temperature: null,
          maxTokens: null,
          messages: [],
          _pendingQueue: [],
          activeInterview: null,
          interviewRound: 0,
          interviewAmbiguity: 0,
          activeToolApproval: null,
          activePathAccess: null,
          detectedMountTag: null,
          detectedMountIds: [],
        })
      },

      submitInterviewResponse(answers: InterviewAnswer[]) {
        const {
          _ws,
          activeInterview,
          activeSessionId,
          activeProjectId,
          activeRole,
          activeModelId,
          interviewRound,
        } = get()
        if (!activeInterview) return

        // Build answer summary for user message bubble
        const answerParts = answers
          .filter((a) => a.value.trim())
          .map((a) => {
            const q = activeInterview.find((q) => q.id === a.question_id)
            return q ? `${q.text}\n→ ${a.value}` : a.value
          })
        const answerText = answerParts.join('\n\n')

        // Persist interview questions as an assistant message BEFORE
        // the user's answer, so the Q&A exchange remains in chat history.
        const interviewMsg: ChatMessage = {
          id: uuid(),
          role: 'assistant',
          content: '',
          timestamp: new Date().toISOString(),
          metadata: {
            phase: 'interview',
            tool_calls: [],
          },
          _interviewQuestions: activeInterview,
          _interviewRound: interviewRound,
        }
        // Send via WebSocket as interview_response
        if (_ws && _ws.readyState === WebSocket.OPEN) {
          _ws.send(
            JSON.stringify({
              type: 'interview_response',
              session_id: activeSessionId ?? '',
              project_id: activeProjectId ?? '',
              role: activeRole ?? '',
              // Per-message model override (or last-picked persistent one).
              model: activeModelId ?? '',
              answers,
              text: answerText,
            }),
          )
        }

        // Add user message showing their answers
        const userMsg: ChatMessage = {
          id: uuid(),
          role: 'user',
          content: answerText || answers.map((a) => a.value).join(', '),
          timestamp: new Date().toISOString(),
        }

        set((s) => ({
          messages: [...s.messages, interviewMsg, userMsg],
          activeInterview: null,
          interviewRound: 0,
          interviewAmbiguity: 0,
          isStreaming: true,
        }))
      },

      async resolveToolApproval(id: string, approved: boolean, remember?: boolean) {
        const { activeToolApproval } = get()
        if (!activeToolApproval || activeToolApproval.id !== id) return
        // Record as resolved BEFORE the fetch so a concurrent WS replay of
        // the same approval cannot re-arm the card. Cleared only on a genuine
        // error below so the user can retry.
        markApprovalResolved(id)
        set({ activeToolApproval: null, isStreaming: true })
        try {
          const token = useAuthStore.getState().token
          const res = await fetch(`/api/chat/tool-approval/${encodeURIComponent(id)}/respond`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
              ...(token ? { Authorization: `Bearer ${token}` } : {}),
            },
            body: JSON.stringify({ approved, ...(remember ? { remember: true } : {}) }),
          })
          if (!res.ok && res.status !== 404) {
            // 404 "not found or already resolved" is a benign idempotent
            // outcome — the approval was handled (prior click, WS replay
            // re-arm, or the exec_tool 120 s timeout auto-denying it).
            // Treat it as success and leave the card dismissed; restoring a
            // dead-id card re-arms it and every later click 404s again (the
            // N×404 loop). Only genuine server/network errors re-arm the
            // card for a user retry.
            const err = await res.text().catch(() => 'unknown error')
            throw new Error(`HTTP ${res.status}: ${err}`)
          }
        } catch (e) {
          // Genuine error (network/5xx): forget the resolution and restore
          // the card so the user can retry. Swallowed — call sites are
          // fire-and-forget, so throwing would only surface as an unhandled
          // promise rejection in the console. The restored card is the
          // user-facing failure signal.
          _resolvedApprovalIds.delete(id)
          set({ activeToolApproval, isStreaming: false })
          console.warn('[chat] tool approval resolve failed:', e)
        }
      },
      async resolvePathAccess(id: string, action: 'mount' | 'temp' | 'deny') {
        const { activePathAccess } = get()
        if (!activePathAccess || activePathAccess.id !== id) return
        markApprovalResolved(id)
        set({ activePathAccess: null, isStreaming: true })
        try {
          const token = useAuthStore.getState().token
          const res = await fetch(`/api/chat/path-access/${encodeURIComponent(id)}/respond`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
              ...(token ? { Authorization: `Bearer ${token}` } : {}),
            },
            body: JSON.stringify({ action }),
          })
          if (!res.ok && res.status !== 404) {
            const err = await res.text().catch(() => 'unknown error')
            throw new Error(`HTTP ${res.status}: ${err}`)
          }
        } catch (e) {
          _resolvedApprovalIds.delete(id)
          set({ activePathAccess, isStreaming: false })
          console.warn('[chat] path access resolve failed:', e)
        }
      },

      handleChunk(chunk) {
        // F9: flush any buffered token content before a non-token chunk so
        // streamed text is committed to the message before a tool/done/error
        // event reads or replaces the last assistant message.
        if (chunk.type !== 'token') {
          flushPendingTokens()
        }
        switch (chunk.type) {
          // RFC-015 model mark — arrives before the first token. Patch the live
          // assistant message, or stash as pendingModel for the next placeholder
          // (consumed via AssistantCtx.placeholderModel).
          case 'model': {
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
          case 'token': {
            // F9: batch tokens into a single rAF flush instead of rebuilding
            // the messages array on every token.
            if (!chunk.content) break
            _pendingTokens += chunk.content
            scheduleTokenFlush()
            break
          }

          // ── RFC-015 chat transparency chunks ──
          // ── RFC-015 chat transparency chunks (block-stream source of truth) ──
          case 'tool_start':
          case 'tool_progress':
          case 'tool_end':
          case 'tool_call_delta':
          case 'reasoning':
          case 'grounding':
          case 'memory':
          case 'usage': {
            // Route through StreamProcessor so the single source of truth
            // (`blocks`) is built incrementally and the patch carries the
            // latest `blocks` for the message.
            // Ensure an assistant message exists. A reasoning model streams
            // reasoning BEFORE the first token, so without this the opening
            // reasoning/tool deltas would be dropped (no message to attach
            // to). The LiveActivityBar holder owns the pre-chunk gap
            // indicator, so the message is created lazily here on the first
            // real chunk — never optimistically on send (no empty bubble).
            let msgId = lastAssistantMessageId(get().messages)
            if (!msgId) {
              const ensured = ensureLastAssistant(get().messages, {
                placeholderModel: get().pendingModel ?? get().activeModelId,
              })
              if (ensured.index < 0) break
              set({ messages: ensured.messages })
              msgId = ensured.messages[ensured.index]!.id
            }
            const processor = getOrCreateProcessor(msgId)
            const { events } = adaptChunk(chunk, { msgId })
            for (const ev of events) {
              const result = processor.handleEvent(ev)
              set((s) => ({
                messages: applyProcessorResult(s.messages, msgId, result, {
                  placeholderModel: s.pendingModel ?? s.activeModelId,
                }),
              }))
              if (result.finished) streamProcessors.delete(msgId)
            }
            // Auto-open search panel on web_search/browse tool calls
            if (
              (chunk.type === 'tool_start' || chunk.type === 'tool_end') &&
              (chunk.tool_name === 'web_search' || chunk.tool_name === 'browse')
            ) {
              const portalState = usePortalStore.getState()
              const top = portalState.stack[portalState.stack.length - 1]
              if (top?.type !== 'search') {
                portalState.pushView({ type: 'search', messageId: msgId })
              }
            }
            break
          }

          case 'interview': {
            if (chunk.questions && chunk.questions.length > 0) {
              set({
                activeInterview: chunk.questions,
                interviewRound: chunk.round ?? 1,
                interviewAmbiguity: chunk.ambiguity ?? 0,
                isStreaming: false,
              })
            }
            break
          }

          case 'tool_approval': {
            const approvalId = chunk.id as string | undefined
            // Dedup: a WS reconnect replay (RFC-024 SP2 C2) can re-deliver a
            // tool_approval chunk for an approval already resolved on the
            // backend. Re-arming such a card guarantees a 404 on the next
            // click. Skip ids already acted on, or already active (idempotent
            // re-delivery of a still-pending approval).
            if (
              approvalId &&
              chunk.tool_name &&
              !_resolvedApprovalIds.has(approvalId) &&
              get().activeToolApproval?.id !== approvalId
            ) {
              set({
                activeToolApproval: {
                  id: approvalId,
                  toolName: chunk.tool_name as string,
                  reason: (chunk.reason as string) || '',
                },
                isStreaming: false,
              })
            }
            break
          }
          case 'path_access': {
            const reqId = chunk.id as string | undefined
            if (
              reqId &&
              chunk.path &&
              !_resolvedApprovalIds.has(reqId) &&
              get().activePathAccess?.id !== reqId
            ) {
              set({
                activePathAccess: {
                  id: reqId,
                  path: chunk.path as string,
                  mode: (chunk.mode as string) || 'read',
                  toolName: (chunk.tool_name as string) || '',
                  reason: (chunk.reason as string) || '',
                },
                isStreaming: false,
              })
            }
            break
          }

          case 'done': {
            // Phase 1: route done through StreamProcessor first so reasoning.end
            // and stream.stop events fire (clearing isReasoning, generating).
            set((s) => ({
              messages: applyContentChunk(s.messages, chunk, {
                placeholderModel: s.pendingModel ?? s.activeModelId,
              }).messages,
            }))
            const sid = chunk.session_id ?? null
            const vid = chunk.project_id ?? null
            // RFC-025: extract mount_tag from metadata (gateway sets it)
            const chunkExtra = chunk as unknown as Record<string, unknown>
            const mountTag = chunkExtra.mount_tag as string | undefined
            const mountIdsRaw = chunkExtra.mount_ids as string | string[] | undefined
            // Web-M4: gateway serializes mount_ids as a JSON-array string
            // (e.g. `["id1","id2"]`); splitting on comma produces garbage.
            const mountIds = Array.isArray(mountIdsRaw)
              ? mountIdsRaw
              : typeof mountIdsRaw === 'string' && mountIdsRaw.trim().startsWith('[')
                ? (() => {
                    try {
                      return JSON.parse(mountIdsRaw) as string[]
                    } catch {
                      return []
                    }
                  })()
                : mountIdsRaw
                  ? mountIdsRaw.split(',').filter(Boolean)
                  : []
            const toolCalls = chunk.tool_calls ?? []
            const phase = chunk.phase
            const evaluationPassed: boolean | undefined =
              chunk.evaluation_passed === true || chunk.evaluation_passed === 'true'
                ? true
                : chunk.evaluation_passed === false || chunk.evaluation_passed === 'false'
                  ? false
                  : undefined
            const durationMs = chunk.duration_ms

            set((s) => {
              const updated = [...s.messages]

              // currentAssistant invariant (see helper): completion metadata
              // belongs to THIS turn's trailing assistant. If nothing streamed
              // (model+done only), create a fresh placeholder rather than
              // reusing a prior turn's response.
              const target = currentAssistant(updated)
              if (!target) {
                const placeholder: ChatMessage = {
                  id: uuid(),
                  role: 'assistant',
                  content: '',
                  timestamp: new Date().toISOString(),
                  model: get().pendingModel ?? get().activeModelId ?? undefined,
                  metadata: {
                    phase,
                    evaluation_passed: evaluationPassed,
                    duration_ms: durationMs,
                    tool_calls: Array.isArray(toolCalls) ? toolCalls : [],
                  },
                }
                return {
                  messages: [...updated, placeholder],
                  isStreaming: false,
                  pendingModel: null,
                  activeToolApproval: null,
                  activePathAccess: null,
                }
              }

              updated[updated.length - 1] = {
                ...target,
                id: target.id ?? uuid(),
                metadata: {
                  phase,
                  evaluation_passed: evaluationPassed,
                  duration_ms: durationMs,
                  tool_calls: Array.isArray(toolCalls) ? toolCalls : [],
                },
              }

              return {
                messages: updated,
                isStreaming: false,
                // Clear the per-turn model stash so a later turn without a
                // model chunk does not stamp the prior turn's model onto its
                // placeholder.
                pendingModel: null,
                activeToolApproval: null,
                activePathAccess: null,
              }
            })

            if (sid) {
              set({
                _lastDoneSessionId: sid,
                activeSessionId: sid,
              })
            }
            if (vid) {
              set({ activeProjectId: vid, _lastDoneProjectId: vid })
            }
            // RFC-025: store detected mount tag + ids for the detection badge.
            if (mountTag) {
              set({ detectedMountTag: mountTag })
            }
            if (mountIds.length > 0) {
              set({ detectedMountIds: mountIds })
            }
            // Queue drain: if the user queued follow-ups while this turn
            // streamed, dispatch the next one now that the turn is idle.
            get()._drainPendingQueue()
            break
          }
          case 'error': {
            // Phase 1: route error through StreamProcessor first so stream.stop
            // fires (clearing generating state on the in-flight message).
            set((s) => ({
              messages: applyContentChunk(s.messages, chunk, {
                placeholderModel: s.pendingModel ?? s.activeModelId,
              }).messages,
            }))
            // RFC-032: create an assistant message with the error text
            // so the user sees the failure inline rather than just a
            // loading spinner that silently stops.
            const errMsg = (chunk as unknown as Record<string, unknown>).message as
              | string
              | undefined
            // RFC-032: narrow the chunk's `kind` to the errorKind union so the
            // bubble can render kind-specific copy. Anything unrecognized
            // falls back to 'unknown' rather than an unchecked cast.
            const rawKind = (chunk as unknown as Record<string, unknown>).kind
            const errKind: 'quota_exceeded' | 'auth' | 'routing' | 'unknown' =
              rawKind === 'quota_exceeded' || rawKind === 'auth' || rawKind === 'routing'
                ? rawKind
                : 'unknown'
            const errSuggestion = (chunk as unknown as Record<string, unknown>).suggestion as
              | string
              | undefined
            const errorContent = errSuggestion
              ? `${errMsg}\n\n${errSuggestion}`
              : (errMsg ?? 'An error occurred')
            set((s) => {
              const updated = [...s.messages]
              // Add an error message after the user's last message
              const errorMsg: ChatMessage = {
                id: uuid(),
                role: 'assistant',
                content: errorContent,
                timestamp: new Date().toISOString(),
                model: get().pendingModel ?? get().activeModelId ?? undefined,
                metadata: {
                  isError: true,
                  errorKind: errKind,
                },
              }
              return {
                messages: [...updated, errorMsg],
                isStreaming: false,
                pendingModel: null,
                activeToolApproval: null,
                activePathAccess: null,
              }
            })
            // Turn ended — advance the queue (same as done).
            get()._drainPendingQueue()
            break
          }
          case 'compression_delta': {
            if (!chunk.content) break
            set((s) => {
              const prev = s.compression
              return {
                compression: {
                  summary: (prev?.summary ?? '') + chunk.content!,
                  status: 'generating' as const,
                  compressed_before_index: prev?.compressed_before_index,
                },
              }
            })
            break
          }
          case 'compression_done': {
            set((s) => ({
              compression: s.compression ? { ...s.compression, status: 'done' as const } : null,
            }))
            break
          }
          case 'compression_failed': {
            set((s) => ({
              compression: s.compression
                ? { ...s.compression, status: 'failed' as const, error: chunk.error }
                : { summary: '', status: 'failed' as const, error: chunk.error },
            }))
            break
          }
        }
      },
    }),
    {
      name: PERSIST_KEY,
      partialize: (state): PersistedState => ({
        activeSessionId: state.activeSessionId,
        activeProjectId: state.activeProjectId,
        activeMountIds: state.activeMountIds,
        activeRole: state.activeRole,
        activeModelId: state.activeModelId,
        temperature: state.temperature,
        maxTokens: state.maxTokens,
      }),
      onRehydrateStorage: () => (state) => {
        if (!state) return
        // After rehydration, if there's an active session, load its history.
        // WS auto-connect is handled by the ChatPage component — only
        // connect when the chat route is active.
        if (state.activeSessionId) {
          state.loadSession(state.activeSessionId)
        }
      },
    },
  ),
)
