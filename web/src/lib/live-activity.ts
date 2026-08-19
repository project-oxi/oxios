import type { ChatBlock, ToolCallContext } from '@/types'

/**
 * RFC-015 §4.3 — descriptor for the "current activity" header shown above the
 * chat timeline while an assistant turn is being built.
 *
 * The selector is intentionally separate from `LiveActivityBar` so it can be
 * unit-tested without a React render. It mirrors the design-doc rule:
 *
 *   currentActivity = activities.findLast(status==='running')
 *                  ?? currentPhase
 *                  ?? default 'Thinking'
 *
 * The store marks `isRunning === true` on tool_start / tool_progress and
 * `isRunning === false` on tool_end (see `chunkToActivity` in
 * `web/src/stores/chat.ts`). Reasoning fragments are fire-and-forget — they
 * carry no completion flag — so the latest reasoning entry is by definition
 * the one being streamed.
 */
export type LiveActivityKind = 'thinking' | 'tool_running' | 'reasoning'

export interface LiveActivityDescriptor {
  kind: LiveActivityKind
  /** Populated only for `tool_running`; the raw tool identifier. */
  toolName?: string
  /** Human-readable progress text streamed by the running tool (RFC-015 v0.12). */
  progress?: string
  /** Semantic context from a browsing tool (web_search, page_visit, …). */
  context?: ToolCallContext
  /** Raw tool arguments — used to derive a detail string (file path, command). */
  toolArgs?: Record<string, unknown>
}

/** Minimal translator signature so the module stays free of React imports. */
export type Translator = (key: string, opts?: Record<string, unknown>) => string

/** Label + optional detail produced by {@link describeLiveActivity}. */
export interface LiveActivityLabel {
  /** Main verb, e.g. "Searching the web" / "웹 검색 중". */
  label: string
  /** Concrete detail, e.g. the search query or a shortened file path. */
  detail?: string
}
/**
 * Block-stream selector for the live activity header. Walks a message's
 * `blocks[]` (the single source of truth) backwards and returns the
 * descriptor for the most recent in-flight reasoning or tool block.
 * Falls back to `thinking` when blocks are absent or all settled.
 */
export function deriveCurrentActivityFromBlocks(
  blocks: readonly ChatBlock[] | undefined,
): LiveActivityDescriptor {
  if (!blocks || blocks.length === 0) return { kind: 'thinking' }
  for (let i = blocks.length - 1; i >= 0; i--) {
    const b = blocks[i]
    if (!b) continue
    if (b.type === 'tool' && b.status === 'loading') {
      return {
        kind: 'tool_running',
        toolName: b.apiName,
        progress: b.progress,
        toolArgs: (b.arguments ?? {}) as Record<string, unknown>,
      }
    }
    if (b.type === 'reasoning' && b.status === 'streaming') {
      return { kind: 'reasoning' }
    }
  }
  return { kind: 'thinking' }
}

// ---------------------------------------------------------------------------
// Label derivation — maps a descriptor to a human-readable sentence.
//
// Label priority: context → toolName verb → generic fallback.
// Detail priority: context-derived → progress → toolArgs-derived.
// ---------------------------------------------------------------------------

/** Maps known tool names to i18n keys for a descriptive verb. */
const TOOL_VERB_KEYS: Record<string, string> = {
  read: 'chat.liveActivity.read',
  write: 'chat.liveActivity.write',
  edit: 'chat.liveActivity.edit',
  grep: 'chat.liveActivity.grep',
  find: 'chat.liveActivity.find',
  ls: 'chat.liveActivity.ls',
  exec: 'chat.liveActivity.exec',
  browser: 'chat.liveActivity.browser',
  memory_read: 'chat.liveActivity.memoryRead',
  memory_search: 'chat.liveActivity.memorySearch',
  memory_write: 'chat.liveActivity.memoryWrite',
  knowledge: 'chat.liveActivity.knowledge',
  a2a_delegate: 'chat.liveActivity.a2aDelegate',
  a2a_send: 'chat.liveActivity.a2aSend',
  a2a_query: 'chat.liveActivity.a2aQuery',
  send_email: 'chat.liveActivity.sendEmail',
  calendar: 'chat.liveActivity.calendar',
  cron: 'chat.liveActivity.cron',
  ask_user: 'chat.liveActivity.askUser',
}

function toolVerb(toolName: string | undefined, t: Translator): string {
  if (!toolName) return t('chat.liveActivity.toolDefault')
  const key = TOOL_VERB_KEYS[toolName]
  return key ? t(key) : t('chat.liveActivity.toolRunning', { name: toolName })
}

/** Extract a short detail string from tool arguments (file path, command, …). */
function extractArgDetail(toolArgs?: Record<string, unknown>): string | undefined {
  if (!toolArgs) return undefined
  // File tools: path / file_path
  const path = strArg(toolArgs, 'path') ?? strArg(toolArgs, 'file_path')
  if (path) return shortenPath(path)
  // Exec / shell: command
  const command = strArg(toolArgs, 'command') ?? strArg(toolArgs, 'cmd')
  if (command) return truncate(command, 60)
  // Grep: pattern
  const pattern = strArg(toolArgs, 'pattern')
  if (pattern) return truncate(pattern, 40)
  // Browser: url / query
  const url = strArg(toolArgs, 'url')
  if (url) return shortenUrl(url)
  const query = strArg(toolArgs, 'query')
  if (query) return truncate(query, 40)
  return undefined
}

function strArg(args: Record<string, unknown>, key: string): string | undefined {
  const v = args[key]
  return typeof v === 'string' ? v : undefined
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n)}…` : s
}

function shortenPath(p: string): string {
  // Keep last 2 segments — enough context without overflowing the bar.
  const parts = p.split('/')
  if (parts.length <= 2) return p
  return `…/${parts.slice(-2).join('/')}`
}

function shortenUrl(url: string): string {
  try {
    const u = new URL(url)
    const path = u.pathname === '/' ? '' : truncate(u.pathname, 30)
    return path ? `${u.host}${path}` : u.host
  } catch {
    return truncate(url, 40)
  }
}

/**
 * Map a {@link LiveActivityDescriptor} to a human-readable label + detail.
 *
 * The bar shows `<label> · <detail>` while a tool is running, giving the
 * user a sentence-level description of what is happening right now —
 * "Searching the web · rust async runtime" rather than a generic
 * "Running browser".
 */
export function describeLiveActivity(d: LiveActivityDescriptor, t: Translator): LiveActivityLabel {
  if (d.kind === 'reasoning') {
    return { label: t('chat.liveActivity.reasoning') }
  }
  if (d.kind === 'thinking') {
    return { label: t('chat.liveActivity.thinking') }
  }

  // --- tool_running ---

  // 1. Structured context (browser tool) — most reliable label + detail.
  if (d.context) {
    switch (d.context.kind) {
      case 'web_search':
        return {
          label: t('chat.liveActivity.webSearch'),
          detail: d.context.query || undefined,
        }
      case 'page_visit':
        return {
          label: t('chat.liveActivity.pageVisit'),
          detail: shortenUrl(d.context.url),
        }
      case 'data_extraction':
        return {
          label: t('chat.liveActivity.dataExtraction'),
          detail: d.context.target || undefined,
        }
      case 'script_step':
        return {
          label: d.context.step || t('chat.liveActivity.browserAction'),
          detail: `${d.context.current}/${d.context.total}`,
        }
      case 'session_action':
        return {
          label: t('chat.liveActivity.browserAction'),
          detail: d.context.action || undefined,
        }
    }
  }

  // 2. Progress text (free-form, streamed by the tool itself).
  if (d.progress) {
    return {
      label: toolVerb(d.toolName, t),
      detail: truncate(d.progress, 60),
    }
  }

  // 3. Tool args detail + tool name verb.
  return {
    label: toolVerb(d.toolName, t),
    detail: extractArgDetail(d.toolArgs),
  }
}
