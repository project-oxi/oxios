// StreamProcessor — client state machine for one assistant message stream.
//
// LobeHub analogue: src/store/chat/agents/StreamingHandler.ts
//
// Responsibilities:
//   • Accumulate content streaming state for one message (text, reasoning,
//     tool calls, search, usage).
//   • Emit partial ChatMessage patches per ChatEvent so the store can update
//     React state incrementally without rebuilding the whole message array.
//   • Track lifecycle (generating, first-reasoning-seen, etc.).
//
// What it does NOT do:
//   • Talk to React / zustand directly. Caller applies patches.
//   • Handle Oxios-semantic chunks (model, memory, interview_question,
//     tool_approval, mount_detected) — those stay on the store's legacy arms.
//   • RAF batching. Caller batches text.delta events before calling.
//
// One StreamProcessor per assistant message. Store keeps Map<msgId, StreamProcessor>.
// See docs/designs/2026-07-21-lobehub-chat-port-design.md §6.2.

import type { ChatMessage } from '@/types'
import type {
  ChatBlock,
  ChatError,
  ChatToolPayload,
  ChatToolStatus,
  CitationItem,
  GroundingSearch,
  ReasoningBlock,
  TextBlock,
} from '@/types/chat'
import type { ChatEvent } from './ChatEvent'

/** Total reasoning text budget per turn — bounds the persisted trace. On
 *  overflow, further reasoning deltas are dropped (a marker is left on the
 *  last reasoning block by the caller if desired). */
const REASONING_BUDGET_BYTES = 16 * 1024

export interface ProcessorResult {
  /** Partial ChatMessage patch to merge into the stored message. */
  patch: Partial<ChatMessage>
  /** Set when the stream has terminated (stop event). Cleanup hint for store. */
  finished?: boolean
}

/**
 * State machine for one streaming assistant message.
 *
 * Construct with the message id; feed ChatEvents via handleEvent; apply
 * returned patch to the store; on `finished: true`, call materialize() for
 * a clean final state and discard the processor.
 */
export class StreamProcessor {
  readonly messageId: string

  private text = ''
  private tools = new Map<string, ChatToolPayload>()

  private error: ChatError | null = null
  private stopped = false
  private usageSeq = 0
  private memorySeq = 0
  /** Accumulated web-search grounding for the turn (citations deduped by url). */
  private search: GroundingSearch | null = null

  // ── Block-stream transparency (single source of truth for the timeline) ──
  private blocks: ChatBlock[] = []
  private reasoningSeq = 0
  private textSeq = 0
  private reasoningBytes = 0

  constructor(messageId: string) {
    this.messageId = messageId
  }

  /** Feed one ChatEvent. Returns incremental patch + lifecycle signals.
   *  Always attaches the block-stream timeline (`blocks`) to the patch —
   *  the single source of truth rendered by BlockStream. */
  handleEvent(ev: ChatEvent): ProcessorResult {
    const result = this.handleEventInner(ev)
    return { ...result, patch: { ...result.patch, blocks: [...this.blocks] } }
  }

  private handleEventInner(ev: ChatEvent): ProcessorResult {
    if (this.stopped && ev.kind !== 'stream.stop') {
      return { patch: {} }
    }

    switch (ev.kind) {
      case 'text.delta':
        this.text += ev.text
        this.appendText(ev.text)
        return { patch: { content: this.text, generating: true } }

      case 'reasoning.start': {
        const i = this.blocks.length - 1
        const last = this.blocks[i]
        if (last && last.type === 'reasoning') {
          // Reopen the existing span: the runtime emitted another start
          // marker for the same uninterrupted thinking run (no tool/text
          // arrived between — those close + displace it, see
          // closeReasoningBlock / upsertToolBlock / appendText). Spawning a
          // sibling here is what produced adjacent duplicate "Thought" cards.
          this.blocks[i] = { ...last, status: 'streaming' }
        } else {
          this.openReasoning()
        }
        return { patch: { generating: true } }
      }

      case 'reasoning.delta': {
        this.appendReasoning(ev.text, ev.source)
        return { patch: { generating: true } }
      }

      case 'reasoning.end': {
        this.closeReasoningBlock()
        return { patch: { generating: true } }
      }

      case 'tool.args_delta': {
        // oxi 0.58+: partial tool-call args streamed by the LLM before
        // ToolExecutionStart. Create a placeholder if this tool_call_id is
        // unseen; otherwise accumulate the raw JSON fragment. When tool.start
        // arrives it replaces the placeholder with the parsed args + real name.
        const cur = this.tools.get(ev.toolCallId)
        if (!cur) {
          this.tools.set(ev.toolCallId, {
            id: ev.toolCallId,
            identifier: 'kernel',
            apiName: '(constructing…)',
            arguments: ev.argsDelta,
            status: 'loading' satisfies ChatToolStatus,
            startedAt: Date.now(),
          })
        } else {
          this.tools.set(ev.toolCallId, {
            ...cur,
            arguments: (typeof cur.arguments === 'string' ? cur.arguments : '') + ev.argsDelta,
          })
        }
        this.closeReasoningBlock()
        this.upsertToolBlock(this.tools.get(ev.toolCallId)!)
        return { patch: {} }
      }

      case 'tool.start': {
        const tool: ChatToolPayload = {
          id: ev.toolCallId,
          identifier: 'kernel',
          apiName: ev.toolName,
          arguments: ev.args,
          status: 'loading' satisfies ChatToolStatus,
          startedAt: Date.now(),
          ...(ev.tabId !== undefined ? { tabId: ev.tabId } : {}),
        }
        this.tools.set(ev.toolCallId, tool)
        this.closeReasoningBlock()
        this.upsertToolBlock(tool)
        return {
          patch: {
            isToolCallGenerating: true,
            generating: true,
          },
        }
      }

      case 'tool.progress': {
        const cur = this.tools.get(ev.toolCallId)
        if (!cur) return { patch: {} }
        const next: ChatToolPayload = {
          ...cur,
          progress: ev.progress,
          ...(ev.tabId !== undefined ? { tabId: ev.tabId } : {}),
        }
        this.tools.set(ev.toolCallId, next)
        this.upsertToolBlock(next)
        return { patch: {} }
      }

      case 'tool.end': {
        const cur = this.tools.get(ev.toolCallId)
        if (!cur) return { patch: {} }
        const status: ChatToolStatus = ev.error ? 'error' : 'success'
        const endedAt = Date.now()
        const durationMs = ev.durationMs ?? (cur.startedAt ? endedAt - cur.startedAt : undefined)
        const next: ChatToolPayload = {
          ...cur,
          result: ev.result,
          error: ev.error ?? null,
          status,
          endedAt,
          durationMs,
        }
        this.tools.set(ev.toolCallId, next)
        this.upsertToolBlock(next)
        const allSettled = [...this.tools.values()].every(
          (t) => t.status === 'success' || t.status === 'error' || t.status === 'aborted',
        )
        return {
          patch: {
            isToolCallGenerating: !allSettled,
          },
        }
      }

      case 'usage': {
        // Append/replace a UsageBlock in the block stream. The cumulative
        // token totals are also mirrored to the patch for legacy consumers.
        const last = this.blocks[this.blocks.length - 1]
        if (last && last.type === 'usage') {
          this.blocks[this.blocks.length - 1] = {
            ...last,
            inputTokens: ev.usage.inputTokens,
            outputTokens: ev.usage.outputTokens,
          }
        } else {
          this.usageSeq++
          this.blocks.push({
            type: 'usage',
            id: `u-${this.messageId}-${this.usageSeq}`,
            inputTokens: ev.usage.inputTokens,
            outputTokens: ev.usage.outputTokens,
          })
        }
        return {
          patch: {
            totalInputTokens: ev.usage.inputTokens,
            totalOutputTokens: ev.usage.outputTokens,
          },
        }
      }

      case 'memory': {
        this.memorySeq++
        this.blocks.push({
          type: 'memory',
          id: `m-${this.messageId}-${this.memorySeq}`,
          action: ev.action,
          ...(ev.query !== undefined ? { query: ev.query } : {}),
          ...(ev.count !== undefined ? { count: ev.count } : {}),
          ...(ev.source !== undefined ? { source: ev.source } : {}),
          timestamp: ev.timestamp,
        })
        return { patch: {} }
      }
      // A turn can search several times; each chunk carries only that call's
      // hits. Accumulate and dedupe by url so later searches never erase
      // earlier citations.
      case 'grounding': {
        const seen = new Set<string>()
        const citations: CitationItem[] = []
        for (const c of [...(this.search?.citations ?? []), ...(ev.search.citations ?? [])]) {
          if (seen.has(c.url)) continue
          seen.add(c.url)
          citations.push(c)
        }
        this.search = {
          ...this.search,
          ...ev.search,
          ...(citations.length > 0 ? { citations } : {}),
        }
        return { patch: { search: this.search } }
      }

      case 'file_chunks':
        return { patch: { chunksList: ev.chunks } }

      case 'phase':
        return { patch: {} }

      case 'stream.stop':
        this.stopped = true
        this.error = ev.error ?? null
        this.closeAllBlocks()
        return {
          patch: {
            generating: false,
            isToolCallGenerating: false,
            error: ev.error ?? undefined,
          },
          finished: true,
        }

      default: {
        const _exhaustive: never = ev
        void _exhaustive
        return { patch: {} }
      }
    }
  }

  /** Produce final ChatMessage (snapshot of accumulated state). */
  materialize(base: ChatMessage): ChatMessage {
    // Derive legacy fields from the single source of truth (`blocks`) so
    // downstream consumers (FollowUpChips, error card, etc.) keep working
    // during the transition to a block-only model.
    const text = this.blocks
      .filter((b): b is TextBlock => b.type === 'text')
      .map((b) => b.text)
      .join('')
    // Sum all usage blocks (each one is a cumulative snapshot from the
    // provider) so chat.ts/UI see the final total without re-aggregating.
    let totalInputTokens = 0
    let totalOutputTokens = 0
    for (const b of this.blocks) {
      if (b.type === 'usage') {
        totalInputTokens = b.inputTokens
        totalOutputTokens = b.outputTokens
      }
    }

    return {
      ...base,
      id: this.messageId,
      blocks: [...this.blocks],
      content: text || base.content,
      search: this.search ?? base.search,

      totalInputTokens: totalInputTokens || base.totalInputTokens,
      totalOutputTokens: totalOutputTokens || base.totalOutputTokens,
      error: this.error ?? base.error ?? null,
      generating: false,
      isToolCallGenerating: false,
    }
  }

  // ── Internals ──

  // ── Block-stream helpers (2026-07-27) ───────────────────────────────
  // Mutate `this.blocks` immutably (replace objects, never in-place) so the
  // shallow clone `[...this.blocks]` returned in each patch yields new block
  // refs and React detects per-block changes. Block ids are counter-assigned
  // at open (stable across re-renders / rAF batches); tool blocks reuse
  // tool_call_id.

  private openReasoning(source?: ReasoningBlock['source']): void {
    this.reasoningSeq++
    this.blocks.push({
      type: 'reasoning',
      id: `r-${this.messageId}-${this.reasoningSeq}`,
      text: '',
      status: 'streaming',
      startedAt: Date.now(),
      ...(source ? { source } : {}),
    })
  }

  private appendReasoning(text: string, source?: ReasoningBlock['source']): void {
    if (this.reasoningBytes >= REASONING_BUDGET_BYTES) return
    const slice = text.slice(0, REASONING_BUDGET_BYTES - this.reasoningBytes)
    if (!slice) return
    this.reasoningBytes += slice.length
    const i = this.blocks.length - 1
    const last = this.blocks[i]
    if (last && last.type === 'reasoning') {
      // Append to the trailing reasoning span, REOPENING it if a prior
      // reasoning.end closed it. A tool/text block would have displaced it
      // (so `last` wouldn't be reasoning), which means we only ever merge a
      // single uninterrupted thinking run — never across a tool. Mirrors the
      // backend's per-position segment coalescing (agent_runtime.rs).
      const nextSource = last.source ?? source
      this.blocks[i] = {
        ...last,
        status: 'streaming',
        text: last.text + slice,
        ...(nextSource ? { source: nextSource } : {}),
      }
    } else {
      this.openReasoning(source)
      const j = this.blocks.length - 1
      const cur = this.blocks[j]
      if (cur && cur.type === 'reasoning') this.blocks[j] = { ...cur, text: slice }
    }
  }

  /** Close the trailing reasoning block if it is still streaming. */
  private closeReasoningBlock(): void {
    const i = this.blocks.length - 1
    const last = this.blocks[i]
    if (last && last.type === 'reasoning' && last.status === 'streaming') {
      this.blocks[i] = { ...last, status: 'done', durationMs: Date.now() - last.startedAt }
    }
  }

  /** Append text to the trailing text block, opening a new one (and closing
   *  any open reasoning span) when the previous block isn't text. */
  private appendText(text: string): void {
    this.closeReasoningBlock()
    const i = this.blocks.length - 1
    const last = this.blocks[i]
    if (last && last.type === 'text') {
      this.blocks[i] = { ...last, text: last.text + text, streaming: true }
    } else {
      this.textSeq++
      this.blocks.push({
        type: 'text',
        id: `t-${this.messageId}-${this.textSeq}`,
        text,
        streaming: true,
      })
    }
  }

  /** Insert or replace the tool block for `payload.id`, preserving stream position. */
  private upsertToolBlock(payload: ChatToolPayload): void {
    const i = this.blocks.findIndex((b) => b.type === 'tool' && b.id === payload.id)
    const block: ChatBlock = { type: 'tool', ...payload }
    if (i >= 0) this.blocks[i] = block
    else this.blocks.push(block)
  }

  /** Mark every open block as done (called on stream.stop). */
  private closeAllBlocks(): void {
    for (let i = 0; i < this.blocks.length; i++) {
      const b = this.blocks[i]!
      if (b.type === 'text') {
        this.blocks[i] = { ...b, streaming: false }
      } else if (b.type === 'reasoning' && b.status === 'streaming') {
        this.blocks[i] = { ...b, status: 'done', durationMs: Date.now() - b.startedAt }
      }
    }
  }
}
