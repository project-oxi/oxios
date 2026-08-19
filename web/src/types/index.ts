// LobeHub-ported chat types (named versions; replace inline anonymous types below).
import type {
  ChatBlock,
  ChatError,
  ChatFileChunk,
  ChatFileItem,
  ChatImageItem,
  ErrorKindValue,
  GroundingSearch,
} from './chat'

export type {
  ChatBlock,
  ChatError,
  ChatErrorAttribution,
  ChatErrorSeverity,
  ChatFileChunk,
  ChatFileItem,
  ChatImageItem,
  ChatItemAvatar,
  ChatItemProps,
  ChatMessageExtensions,
  ChatToolPayload,
  ChatToolStatus,
  CitationItem,
  GroundingSearch,
  ImageCitationItem,
  ReasoningBlock,
  TextBlock,
  ToolBlock,
  ToolRenderProps,
} from './chat'

// Agent
export interface Agent {
  id: string
  name: string
  status: string // Backend sends Debug format: "Running", "Idle", "Stopped", "Starting", "Error"
  created_at?: string
}

export interface AgentListResponse {
  items: Agent[]
  total: number
  page: number
  limit: number
}

// Session
export interface Session {
  id: string
  user_id?: string
  project_id?: string
  created_at: string
  updated_at?: string
  message_count?: number
  title?: string
  metadata?: Record<string, unknown>
}

// Session detail (from GET /api/sessions/:id)
export interface SessionDetail {
  id: string
  user_id: string
  project_id?: string
  user_messages: { content: string; timestamp: string }[]
  agent_responses: {
    content: string
    session_id: string
    // Web-m6: these fields are nullable on the backend (chat-only sessions
    // never run the Ouroboros pipeline), so type them optional.
    phase_reached?: string | null
    evaluation_passed?: boolean | null
    timestamp: string
  }[]
  active_persona_id?: string
  /// P4 (§7 persistence): reasoning text per turn. One record per turn
  /// (when reasoning was emitted), same indexing as `agent_responses`.
  /// Restored by `loadSession` and rendered in the ThinkingPanel above
  /// the answer.
  reasoning_records?: { content: string; source: string; timestamp: string }[]
  created_at: string
  updated_at: string
  metadata?: Record<string, unknown>
}

// Project (replaces Space)
export interface Project {
  id: string
  name: string
  description?: string
  paths?: string[]
  tags?: string[]
  emoji?: string
  source?: string
  memory_visible?: boolean
  /** RFC-025: referenced Mount IDs */
  mount_ids?: string[]
  /** RFC-025: custom instructions injected into the system prompt */
  instructions?: string
  created_at: string
  updated_at?: string
  last_active_at?: string
  metadata?: Record<string, unknown>
}

// Mount (RFC-025) — path alias
export interface MountMeta {
  languages: string[]
  stack: string[]
  markers: string[]
  summary: string
}

export interface Mount {
  id: string
  name: string
  paths: string[]
  auto_description: string
  auto_meta: MountMeta
  source: string
  enrichment_pending: boolean
  last_enriched_at?: string
  created_at: string
  updated_at?: string
  last_active_at?: string
}

// Skill (RFC-009 unified model)
export type SkillSource = 'bundled' | 'managed' | 'workspace'
export type SkillStatus = 'ready' | 'needs_setup' | 'disabled'
export type SkillFormat = 'oxios' | 'openclaw' | 'claude_code' | 'agent_skills'

export interface SkillRequirements {
  bins: string[]
  anyBins: string[]
  env: string[]
  config: string[]
  /** Integration IDs this skill hard-requires (RFC-041). Optional: the
   * skills list API omits the key entirely when the skill declares none. */
  integrations?: string[]
  /** Integration IDs of which at least one must be satisfied. Optional —
   * same API omission as `integrations`. */
  anyIntegrations?: string[]
}

export interface SkillInstallSpec {
  kind: 'brew' | 'node' | 'bun' | 'cargo' | 'pip' | 'go' | 'uv' | 'download'
  label?: string
  bins: string[]
}

/** Per-integration live status, joined from the registry + scanner + credential
 * resolver at the API layer (RFC-041 Phase 5). Keys mirror the `id`s in
 * `SkillRequirements.integrations` / `anyIntegrations`. */
export interface SkillIntegrationStatus {
  id: string
  installed: boolean
  configured: boolean
  satisfied: boolean
}

export interface Skill {
  name: string
  description: string
  author?: string
  version?: string
  emoji?: string
  homepage?: string
  source: SkillSource
  bundled: boolean
  status: SkillStatus
  eligible: boolean
  always: boolean
  user_invocable: boolean
  file_path: string
  requirements: SkillRequirements
  missing: SkillRequirements
  os: string[]
  install: SkillInstallSpec[]
  config_checks: Array<{ path: string; satisfied: boolean }>
  /** Live status for each required/any integration (RFC-041 Phase 5). */
  integration_status: SkillIntegrationStatus[]
  format: SkillFormat
}

// Memory
export interface MemoryEntry {
  name: string
  category?: string
  content?: string
  created_at?: string
  updated_at?: string
}

// Config
export interface OxiosConfig {
  general?: {
    default_model?: string
    max_concurrent_agents?: number
    workspace_path?: string
    [key: string]: unknown
  }
  engine?: {
    provider?: string
    model?: string
    api_key?: string
    base_url?: string
    [key: string]: unknown
  }
  [key: string]: unknown
}

// Chat
export interface ChatMessage {
  id: string
  role: 'user' | 'assistant' | 'system' | 'tool'
  content: string
  /** Model that produced this assistant turn (`provider/model`). Absent for
   *  user/tool messages and for history reloaded before backend threading. */
  model?: string
  timestamp?: string
  // Tool call fields (role === 'tool')
  toolName?: string
  toolArgs?: Record<string, unknown>
  toolResult?: unknown
  toolDurationMs?: number
  // Completion metadata (last assistant message)
  metadata?: {
    phase?: string
    evaluation_passed?: boolean
    duration_ms?: number
    tool_calls?: ToolCallSummary[]
    /// RFC-032: rendered as an inline error card with a retry action.
    isError?: boolean
    /// Optional error category.
    errorKind?: ErrorKindValue
    /// User cancelled the turn mid-stream; the partial answer is kept and
    /// rendered with an InterruptedNotice instead of an error card.
    cancelled?: boolean
    /// Socket dropped mid-stream on this turn; the partial answer is kept
    /// and rendered with the InterruptedNotice `reason="interrupted"` copy.
    interrupted?: boolean
    /// User's rating of the answer: 1 (good) / -1 (bad) (Task 22).
    rating?: 1 | -1
  }
  totalInputTokens?: number
  totalOutputTokens?: number
  _interviewQuestions?: InterviewQuestion[]
  _interviewRound?: number
  // ── LobeHub-aligned fields (Phase 1 streaming foundation, 2026-07-21) ──
  // Named types imported from './chat'. Old inline anonymous types removed.
  /** Web search grounding with citation cards. */

  search?: GroundingSearch | null
  /** RAG reference chunks from knowledge base. */
  chunksList?: ChatFileChunk[]
  /** Generated or attached images. */
  imageList?: ChatImageItem[]
  /** Attached files. */
  fileList?: ChatFileItem[]

  /** Rich error with classification (LobeHub-ported). Renamed from chatError (which was unused). */
  error?: ChatError | null
  /** Whether this message is currently generating (streaming). */
  generating?: boolean
  /** Whether tool calls are being generated. */

  isToolCallGenerating?: boolean
  /** Whether this message is collapsed. */
  isCollapsed?: boolean
  /** Block-stream transparency (2026-07-27): ordered reasoning/tool/text
   *  blocks rendered as an interleaved timeline. Single source of truth;
   *  legacy content/reasoning/toolCalls are derived from this during
   *  transition. Absent on very-old messages (hydrated on load). */
  blocks?: ChatBlock[]
}

// RFC-015: a single transparency activity entry shown in the chat timeline.
export type ChatActivityType = 'phase' | 'tool_call' | 'memory' | 'reasoning' | 'usage'

export interface ChatActivity {
  id: string
  type: ChatActivityType
  timestamp: string
  // phase
  phase?: string
  status?: 'started' | 'completed'
  summary?: string
  // tool_call
  toolName?: string
  toolCallId?: string
  toolArgs?: Record<string, unknown>
  outputSummary?: string
  durationMs?: number
  isError?: boolean
  /// Latest progress text from a running tool (RFC-015 v0.12). Replaced
  /// in place as the tool emits new updates.
  progress?: string
  /// True while a tool is still running. Drives the spinner in
  /// `ActivityCard` and is cleared on `tool_end`.
  isRunning?: boolean
  /// Browser tab id that produced this tool call (when the upstream tool
  /// is tab-aware, e.g. browser). Rendered as a short badge in
  /// `ActivityCard` so users can distinguish concurrent tab activity.
  tabId?: string
  /// Semantic context from browsing tool (PageVisit, WebSearch, etc.).
  /// Used by `BrowseContextBadge` / `BrowseContextDetail` for rich rendering.
  context?: ToolCallContext
  // memory
  memoryAction?: 'recall' | 'store'
  query?: string
  count?: number
  memorySource?: string
  // reasoning
  content?: string
  reasoningSource?: string
  // usage
  inputTokens?: number
  outputTokens?: number
}

export interface ToolCallSummary {
  /// Tool name. Backend ToolCallRecord serializes as `tool`,
  /// but TrajectoryStepRecord uses `tool_name`. Accept both.
  tool_name?: string
  tool?: string
  input: string
  output: string
  duration_ms: number
}

export interface ChatRequest {
  message: string
  session_id?: string
  project_id?: string
}

export interface ChatResponse {
  response: string
  session_id: string
  project_id?: string
  agent_id?: string
  phase_reached?: string
  evaluation_passed?: boolean
  exit_code?: number
  duration_ms?: number
}

// ── Interactive interview (chat UI redesign) ────────────────────────────

export interface InterviewOption {
  value: string
  label: string
  description?: string
}

export interface InterviewQuestion {
  id: string
  text: string
  kind: 'single_choice' | 'multi_choice' | 'free_text' | 'yes_no'
  options?: InterviewOption[]
}

export interface InterviewAnswer {
  question_id: string
  value: string
}

/** LLM-generated session compression summary (LobeHub port). */
export interface CompressionInfo {
  summary: string
  status: 'done' | 'generating' | 'failed'
  error?: string
  compressed_at?: string
  original_count?: number
  compressed_before_index?: number
  model?: string
}

export interface StreamChunk {
  type:
    | 'token'
    | 'tool_call'
    | 'tool_result'
    | 'done'
    | 'error'
    // RFC-015 chat transparency chunks
    // No standalone 'phase' frame exists: post-RFC-027 the Ouroboros phase is a
    // plain string that is only ever "execute" (orchestrator.rs:602), delivered
    // as a field on `done`. Do not re-add a streaming phase event.
    | 'tool_start'
    | 'tool_end'
    | 'tool_progress'
    | 'tool_call_delta'
    | 'memory'
    | 'reasoning'
    | 'usage'
    // Phase E: search grounding with citation cards
    | 'grounding'
    // Chat UI redesign: interactive interview
    | 'interview'
    // RFC-017: runtime tool capability escalation
    | 'tool_approval'
    | 'path_access'
    // RFC-015 model mark — one-shot announcement of the responding model.
    | 'model'
    // Task 21: sub-agent forks in the turn timeline (session-correlated).
    | 'agent_start'
    | 'agent_end'
    // Context compression: LLM session summary streaming.
    | 'compression_delta'
    | 'compression_done'
    | 'compression_failed'
  content?: string
  model?: string
  tool_name?: string
  path?: string
  mode?: string
  tool_args?: Record<string, unknown>
  tool_result?: unknown
  error?: string
  session_id?: string
  project_id?: string
  phase?: string
  evaluation_passed?: boolean | string
  duration_ms?: number
  tool_calls?: ToolCallSummary[]
  // RFC-015 chunk fields
  tool_call_id?: string
  /// Partial tool-call args fragment (oxi 0.58+ ToolCallDelta). Raw JSON
  /// fragment; the frontend accumulates per tool_call_id.
  args_delta?: string
  status?: 'started' | 'completed'
  summary?: string
  output_summary?: string
  is_error?: boolean
  /// Human-readable progress text (RFC-015 v0.12).
  progress?: string
  /// Browser tab id (when the upstream tool is tab-aware, e.g. browser).
  /// Absent on legacy oxicode-agent versions; the frontend treats absence
  /// as "no badge".
  tab_id?: string
  /// Sub-agent fork id (Task 21: agent_start/agent_end).
  agent_id?: string
  /// Sub-agent name/goal (Task 21: agent_start).
  name?: string
  /// Sub-agent terminal success (Task 21: agent_end).
  success?: boolean
  action?: 'recall' | 'store'
  query?: string
  count?: number
  source?: string
  input_tokens?: number
  output_tokens?: number
  /// Semantic context from the tool call (oxicode-agent 0.29+ BrowseProgress).
  /// Carries structured info about what a browsing tool is doing.
  /// UI consumers that understand a context kind render it richly;
  /// older consumers simply ignore the field.
  context?: ToolCallContext
  // ── Interview chunk fields (chat UI redesign) ──
  questions?: InterviewQuestion[]
  round?: number
  ambiguity?: number
  // ── Tool approval (RFC-017) ──
  id?: string
  reason?: string
  /// Phase B: reasoning lifecycle subtype ('start' | 'end'). Absent on
  /// regular reasoning deltas. Backend emits these as explicit markers
  /// so the frontend can auto-expand/collapse the Thinking block.
  subtype?: 'start' | 'end'
  /// Phase E: grounding chunk fields (web search citations).
  citations?: Array<{ url: string; title?: string; favicon?: string }>
}

// ── Browser observability (RFC-015 Phase G, oxicode-agent 0.29.1+) ─────────

/** Reason for visiting a page. Mirrors oxicode-agent's `VisitReason` enum. */
export type VisitReason =
  | 'direct_navigation'
  | { search_result: { position: number } }
  | { link_followed: { from_url: string } }

/** Screenshot metadata. Mirrors oxicode-agent's `ScreenshotMeta` struct. */
export interface ScreenshotMeta {
  /** PNG payload size in bytes. */
  bytes: number
  /** Viewport width. */
  width: number
  /** Capture duration in milliseconds. */
  duration_ms: number
}

/** Semantic context for a browsing tool execution event. */
export type ToolCallContext =
  | { kind: 'web_search'; query: string; engine?: string }
  | {
      kind: 'page_visit'
      url: string
      reason?: VisitReason
      page_title?: string
      page_status?: number
      page_bytes?: number
      page_duration_ms?: number
      /** Navigation error (from BrowseProgress::NavigationFailed, oxicode-agent 0.29.1+). */
      navigation_error?: string
      /** Screenshot metadata (from BrowseProgress::ScreenshotCaptured, oxicode-agent 0.29.1+). */
      screenshot?: ScreenshotMeta
    }
  | {
      kind: 'data_extraction'
      target: string
      url?: string
      result_count?: number
      page_status?: number
      page_duration_ms?: number
    }
  | { kind: 'session_action'; action: string; url?: string }
  | { kind: 'script_step'; current: number; total: number; step: string }

// Event (SSE)
export interface OxiosEvent {
  id?: string
  type: string
  agent_id?: string
  session_id?: string
  timestamp?: string
  data?: Record<string, unknown>
  // SSE events may also carry ad-hoc fields
  [key: string]: unknown
}

// Approval
export interface Approval {
  id: string
  subject: string
  action: string
  resource: string
  reason: string
  created_at: string
  status: string
}

// Cron Job
export interface CronJob {
  id: string
  name: string
  schedule: string
  goal: string
  enabled: boolean
  last_run?: string
  next_run?: string
  run_count: number
  last_result?: string
  last_success?: boolean
  source: 'config' | 'api'
}

// Budget — moved to types/budget.ts

// Resource
export interface ResourceSnapshot {
  timestamp: string
  cpu_percent: number
  memory_percent: number
  disk_percent: number
}

// Audit
export interface AuditEntry {
  id?: string
  agent_id?: string
  agent_name?: string
  action: string
  resource?: string
  allowed?: boolean
  reason?: string | null
  timestamp: string
  details?: Record<string, unknown>
  hash?: string
}

// Git
export interface GitCommit {
  hash: string
  message: string
  author: string
  timestamp: string
}

// Persona
export interface Persona {
  id: string
  name: string
  role?: string
  description?: string
  enabled: boolean
  personality_traits?: string[]
  /// RFC-044 Phase 3: capability tags that enable UI affordances.
  /// Examples: 'terminal', 'diff-viewer', 'approval-cards',
  /// 'worktree-fanout', 'exec', 'web-search', 'longform-editor', 'outline'.
  /// Empty / undefined = no capabilities (all affordances disabled).
  capabilities?: string[]
}

// Workspace — matches backend TreeEntry from /api/workspace/tree
export interface TreeEntry {
  name: string
  is_dir: boolean
  size: number
}

// Paginated response
export interface PaginatedResponse<T> {
  items: T[]
  total: number
  page: number
  limit: number
}

// Status — matches backend StatusResponse
export interface SystemStatus {
  service: string
  status: string
  version: string
  web_version?: string
  /** Whether the backend requires auth. When false, the WebSocket connects without a token. */
  auth_enabled?: boolean
  channels: string[]
  uptime: string // formatted "1h 30m 5s"
  components?: {
    state_store?: { healthy: boolean; detail?: string }
    event_bus?: { healthy: boolean; detail?: string }
    git?: { healthy: boolean }
    brain?: { healthy: boolean }
    agents?: {
      active_count: number
      total_forked: number
      total_completed: number
      total_failed: number
    }
    spaces_active?: number
    projects_active?: number
  }
}

// ClawHub / Marketplace (RFC-010)

export interface ClawHubSearchResult {
  score: number
  slug: string
  displayName: string
  summary?: string
  version?: string
  updatedAt?: number
}

export interface ClawHubSkillDetail {
  skill: {
    slug: string
    displayName: string
    summary?: string
    tags?: Record<string, string>
    createdAt: number
    updatedAt: number
  } | null
  latestVersion?: {
    version: string
    createdAt: number
    changelog?: string
  }
  metadata?: {
    os?: string[]
    systems?: string[]
  }
  owner?: {
    handle?: string
    displayName?: string
    image?: string
  }
}

// Skills.sh (Vercel Labs ecosystem)

export interface SkillsShSkill {
  id: string
  slug: string
  name: string
  source: string
  installs: number
  sourceType: string
  installUrl?: string
  url: string
  isDuplicate?: boolean
}

export interface SkillsShSearchResponse {
  data: SkillsShSkill[]
  query: string
  searchType: string
  count: number
  durationMs?: number
}

export interface SkillsShListResponse {
  data: SkillsShSkill[]
  pagination: {
    page: number
    perPage: number
    total: number
    hasMore: boolean
  }
}

export interface SkillsShSkillDetail {
  id: string
  source: string
  slug: string
  installs: number
  hash?: string
  files?: Array<{
    path: string
    contents: string
  }>
}

export interface SkillsShAuditEntry {
  provider: string
  slug: string
  status: string
  summary: string
  auditedAt: string
  riskLevel?: string
}

export interface SkillsShAuditResponse {
  id: string
  source: string
  slug: string
  audits: SkillsShAuditEntry[]
}

// System Update
export interface UpdateCheckResponse {
  current_version: string
  latest_version: string
  update_available: boolean
  tag_name: string
  html_url: string
  release_notes: string
  published_at: string
  assets: { name: string; size: number; download_url: string }[]
}

export interface UpdateRunResponse {
  success: boolean
  updated_to: string
  binary_updated: boolean
  web_updated: boolean
  message: string
}

export interface ChangelogResponse {
  tag_name: string
  version: string
  published_at: string
  body: string
  html_url: string
}

// System Tools
export interface DoctorCheck {
  name: string
  status: 'pass' | 'warn' | 'fail'
  message: string
}

export interface DoctorResponse {
  checks: number
  issues: number
  results: DoctorCheck[]
  action_items: string[]
}

export interface AuditVerifyResponse {
  valid: boolean
  entries_checked: number
  message: string
}

export interface BackupResponse {
  success: boolean
  path: string
  size_bytes: number
  message: string
}

export interface LogResponse {
  lines: string[]
  total: number
}

export * from './calendar'
export * from './chat'
