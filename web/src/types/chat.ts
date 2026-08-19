// ── Extended chat types (ported from LobeHub @lobechat/types) ──
// Fields that Oxios ChatMessage doesn't have yet.

// ── Error (ported from @lobechat/types message/common/base.ts) ──

export type ChatErrorAttribution = 'user' | 'provider' | 'harness' | 'system'
export type ChatErrorSeverity = 'info' | 'warning' | 'error' | 'critical'

export interface ChatError {
  attribution?: ChatErrorAttribution
  body?: unknown
  category?: string // 'auth' | 'quota' | 'capacity' | 'routing' | ...
  httpStatus?: number
  message?: string
  retryable?: boolean
  severity?: ChatErrorSeverity
  type: string // ErrorType from model-runtime
}

export interface CitationItem {
  favicon?: string
  id?: string
  title?: string
  url: string
}

export interface ImageCitationItem {
  domain?: string
  imageUri?: string
  sourceUri?: string
  title?: string
}

export interface GroundingSearch {
  citations?: CitationItem[]
  imageResults?: ImageCitationItem[]
  imageSearchQueries?: string[]
  searchQueries?: string[]
}

// ── File chunks (RAG references) ──

export interface ChatFileChunk {
  id: string
  content: string
  filename?: string
  score?: number
}

// ── Image item ──

export interface ChatImageItem {
  alt?: string
  url: string
}

// ── File item ──

export interface ChatFileItem {
  id: string
  name: string
  size: number
  type: string
  url?: string
}

/**
 * Error kinds as serialized by the gateway (`crates/oxios-gateway/src/message.rs`,
 * `ErrorKind`, `serde(rename_all = "snake_case")`). Renaming a backend variant
 * is a wire-breaking change — keep this list in lock-step. `cancelled` is a
 * user action, not a fault (RFC-049); the store renders it separately from
 * error cards.
 */
export const KNOWN_ERROR_KINDS = [
  'execution_failed',
  'api_key_missing',
  'provider_error',
  'timeout',
  'permission_denied',
  'validation_error',
  'cancelled',
  'internal',
] as const

export type ErrorKindValue = (typeof KNOWN_ERROR_KINDS)[number] | 'unknown'

// ── Tool call payload (LobeHub-aligned, replaces toolName/toolArgs/toolResult) ──

export type ChatToolStatus = 'loading' | 'success' | 'error' | 'aborted'

export interface ChatToolPayload {
  /** Tool call id — matches backend `tool_call_id`. Stable across stream lifecycle. */
  id: string
  /** Tool package/namespace (Oxios: always 'kernel' for now; reserved for future plugins). */
  identifier: string
  /** Specific tool name, e.g. 'read_file', 'bash', 'web_search'. */
  apiName: string
  /** Parsed arguments. JSON-parsed form of backend tool_args. */
  arguments: unknown
  /** Parsed result. Absent until the tool completes. */
  result?: unknown
  /** Rich error if the tool failed. */
  error?: ChatError | null
  /** Lifecycle status — drives Inspector spinner/check/x. */
  status: ChatToolStatus
  /** Epoch ms when tool started. */
  startedAt?: number
  /** Epoch ms when tool ended. */
  endedAt?: number
  /** Duration in ms (set on end). */
  durationMs?: number
  /** Human approval state (RFC-017 GatedTool). */
  intervention?: { required: boolean; resolved?: 'approved' | 'rejected' }
  /** Latest progress text from a running tool (RFC-015 v0.12+). */
  progress?: string
  /** Browser tab id when upstream tool is tab-aware (browser tools). */
  tabId?: string
}

// ── Block-stream transparency (2026-07-27) ──────────────────────────────
// A turn is an ordered array of blocks rendered as an interleaved timeline
// (reason → tool → reason → tool → answer). Single source of truth; legacy
// content/reasoning/toolCalls fields are derived from these during transition.

/** A reasoning span, positioned at the point it occurred in the turn. */
export interface ReasoningBlock {
  type: 'reasoning'
  /** Stable id: `r-${msgId}-${seq}` (seq = count of reasoning spans so far). */
  id: string
  text: string
  status: 'streaming' | 'done'
  /** 'compaction' for context-compaction summaries surfaced as reasoning. */
  source?: 'thinking' | 'compaction'
  startedAt: number
  /** Consumed by the per-segment "· Xs" timer. */
  durationMs?: number
}

/** A tool call positioned in the turn's execution order. Reuses ChatToolPayload. */
export type ToolBlock = { type: 'tool' } & ChatToolPayload

/** A text emission (preamble or terminal answer). */
export interface TextBlock {
  type: 'text'
  /** Stable id: `t-${msgId}-${seq}`. */
  id: string
  text: string
  streaming?: boolean
}

/** A memory recall/store event (RFC-015 transparency). */
export interface MemoryBlock {
  type: 'memory'
  id: string
  action: 'recall' | 'store'
  query?: string
  count?: number
  source?: string
  timestamp: string
}

/** Cumulative token usage for a turn (RFC-015 transparency). */
export interface UsageBlock {
  type: 'usage'
  id: string
  inputTokens: number
  outputTokens: number
}

/** A sub-agent fork surfaced in the turn timeline (RFC-015 transparency). */
export interface SubAgentBlockData {
  type: 'subagent'
  /** Stable id: `a-${agentId}` — one block per agent, reopened on start. */
  id: string
  /** The forked agent's id (correlates `agent_start`/`agent_end` frames). */
  agentId: string
  /** The agent's name/goal. */
  name: string
  status: 'running' | 'done' | 'failed'
}

export type ChatBlock =
  | ReasoningBlock
  | ToolBlock
  | TextBlock
  | MemoryBlock
  | UsageBlock
  | SubAgentBlockData

// ── Tool render types ──

export interface ToolRenderProps {
  toolName: string
  args: Record<string, unknown>
  result: unknown
  isRunning: boolean
  durationMs?: number
}

// ── Extended ChatMessage (additions to existing ChatMessage) ──
// These fields will be merged into the main ChatMessage type.
// Existing fields: id, role, content, model, timestamp,
//   toolName, toolArgs, toolResult, toolDurationMs,
//   metadata (phase, evaluation_passed, duration_ms, tool_calls, isError, errorKind),
//   activities, totalInputTokens, totalOutputTokens,
//   _interviewQuestions, _interviewRound

/** LobeHub-ported fields to add to ChatMessage. */
export interface ChatMessageExtensions {
  /** Web search grounding with citation cards. */

  search?: GroundingSearch | null
  /** RAG reference chunks from knowledge base. */
  chunksList?: ChatFileChunk[]
  /** Generated or attached images. */
  imageList?: ChatImageItem[]
  /** Attached files. */
  fileList?: ChatFileItem[]
  /** Rich error with classification. */
  error?: ChatError | null
  /** Whether this message is currently generating. */
  generating?: boolean
  /** Whether tool calls are being generated. */

  isToolCallGenerating?: boolean
  /** Whether this message is collapsed (compressed context). */
  isCollapsed?: boolean
}

// ── Chat item props (ported from LobeHub ChatItem) ──

export interface ChatItemAvatar {
  /** Display name shown in the hover title row (assistant model name). */
  name?: string
}

export interface ChatItemProps {
  id?: string
  /** Optional name for the hover TitleRow. No avatar is rendered — user vs
   *  agent is distinguished by alignment (placement) + a faint user tint. */
  avatar?: ChatItemAvatar
  placement?: 'left' | 'right'
  loading?: boolean
  error?: ChatError | null
  time?: number // unix ms
  durationMs?: number // assistant turn duration, shown subtly in the title row
  showTitle?: boolean
  actions?: React.ReactNode
  messageExtra?: React.ReactNode
  children: React.ReactNode
  className?: string
}
