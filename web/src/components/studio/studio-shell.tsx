// StudioShell — the Studio conversation surface (IA design §8).
//
// Ported verbatim from the legacy chat route (streaming, cancellation,
// scroll anchoring, compression, interview, approvals, path access, and
// block ordering are unchanged) and recomposed as the Studio shell:
//
//   StudioContextBar  — the ONE persistent selector row (§8.3)
//   transcript        — the same stable DOM node (never re-keyed)
//   StudioInspector   — on-demand, data-driven tabs (§8.5)
//   StudioComposer    — 'studio'-variant input, no duplicated selectors
//
// Content lanes (§12.4): the row chooses its lane — prose stays on the
// 760px reading lane; code, tables, tool output, diffs, images, and
// artifacts take the bounded 1240px work lane. No persona-wide width.

import { ArrowDown, GitPullRequest, PanelRight, RefreshCw, Search } from 'lucide-react'
import { type UIEvent, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useShallow } from 'zustand/react/shallow'
import { AgentFanoutCardGrid } from '@/components/chat/AgentFanoutCard'
import type { AttachedFile, ContextAttachment } from '@/components/chat/chat-input'
import { ChatMiniMap } from '@/components/chat/chat-minimap'
import { CompressedGroup } from '@/components/chat/compressed-group'
import { EmptyChatState } from '@/components/chat/empty-chat-state'
import { InterviewWizard } from '@/components/chat/interview-wizard'
import { MessageBubble } from '@/components/chat/message-bubble'
import { PathAccessCard } from '@/components/chat/path-access-card'
import { SessionSkeleton } from '@/components/chat/session-skeleton'
import { TerminalToggle } from '@/components/chat/TerminalToggle'
import { TextSelectionBar } from '@/components/chat/text-selection-bar'
import { ToolApprovalCard } from '@/components/chat/tool-approval-card'
import { WorktreeComparePanel } from '@/components/chat/WorktreeComparePanel'
import { PortalPanel } from '@/components/portal/portal-panel'
import { StudioComposer } from '@/components/studio/studio-composer'
import { StudioContextBar } from '@/components/studio/studio-context-bar'
import {
  StudioInspector,
  type StudioInspectorTab,
  useInspectorAvailability,
} from '@/components/studio/studio-inspector'
import { StudioLaunchSheet, useStudioLaunchStore } from '@/components/studio/studio-launch-sheet'
import { Button } from '@/components/ui/button'
import { WorkbenchShell } from '@/components/workbench/WorkbenchShell'
import { useBrainBindingSync } from '@/hooks/use-brain-binding'
import { useDraftPersistence } from '@/hooks/use-draft-persistence'
import { useEffectiveProfile } from '@/hooks/use-effective-profile'
import { useRoles } from '@/hooks/use-engine'
import { has, presentationLens } from '@/lib/affordances'
import type { ChatRow } from '@/lib/chat-rows'
import { buildChatRows } from '@/lib/chat-rows'
import { exportTranscriptMarkdown } from '@/lib/export-transcript'
import { addInputHistory } from '@/lib/input-history-storage'
import { parseSlashInput, SLASH_COMMANDS, type SlashCommandDef } from '@/lib/slash-commands'
import { getToken, useChatStore } from '@/stores/chat'
import { useFanoutStore } from '@/stores/fanout'
import { usePortalStore } from '@/stores/portal'
import type { ChatMessage } from '@/types'

// ---------------------------------------------------------------------------
// Content lanes (IA design §12.4)
// ---------------------------------------------------------------------------

/** Which lane a transcript row renders in. The BLOCK chooses the lane:
 *  tool output, code, tables, images, and artifacts go wide; prose stays
 *  at reading measure. */
function studioLaneFor(m: ChatMessage): 'reading' | 'work' {
  if (m.role === 'tool') return 'work'
  if (m.role !== 'assistant') return 'reading'
  const blocks = m.blocks ?? []
  if (blocks.some((b) => b.type === 'tool' || b.type === 'subagent')) return 'work'
  if ((m.imageList?.length ?? 0) > 0) return 'work'
  if (m.content.includes('```')) return 'work'
  if (m.content.split('\n').some((line) => line.trimStart().startsWith('|'))) return 'work'
  return 'reading'
}

const LANE_CLASS: Record<'reading' | 'work', string> = {
  reading: 'mx-auto max-w-[760px] px-4',
  work: 'mx-auto max-w-[1240px] px-4',
}

/** Wrapper class for a transcript row: lane width + the row's padding. */
function rowClass(row: ChatRow, pad: string): string {
  if (row.kind !== 'message') return `mx-auto max-w-[760px] px-4 ${pad}`
  return `${LANE_CLASS[studioLaneFor(row.message)]} ${pad}`
}

// ---------------------------------------------------------------------------
// Studio shell
// ---------------------------------------------------------------------------

export function StudioShell() {
  useBrainBindingSync()
  const { t } = useTranslation()
  // Scope the subscription to exactly the fields the page renders — the
  // whole-store destructure re-rendered the page on every store write (e.g.
  // tool state, approval bookkeeping) regardless of relevance. Actions are
  // stable references, so useShallow keeps them without extra re-renders.
  const {
    messages,
    isStreaming,
    connected,
    activeSessionId,
    activeInterview,
    interviewRound,
    interviewAmbiguity,
    activeModelId,
    sendMessage,
    setActiveRole,
    setActiveModelId,
    submitInterviewResponse,
    activeToolApproval,
    resolveToolApproval,
    activePathAccess,
    resolvePathAccess,
    compression,
    disconnect,
    isLoadingSession,
    sessionLoadError,
    retryLoadSession,
    connect,
    newSession,
    sendCommandMessage,
    pushSystemNotice,
  } = useChatStore(
    useShallow((s) => ({
      messages: s.messages,
      isStreaming: s.isStreaming,
      connected: s.connected,
      activeSessionId: s.activeSessionId,
      activeInterview: s.activeInterview,
      interviewRound: s.interviewRound,
      interviewAmbiguity: s.interviewAmbiguity,
      activeModelId: s.activeModelId,
      sendMessage: s.sendMessage,
      setActiveRole: s.setActiveRole,
      setActiveModelId: s.setActiveModelId,
      submitInterviewResponse: s.submitInterviewResponse,
      activeToolApproval: s.activeToolApproval,
      resolveToolApproval: s.resolveToolApproval,
      activePathAccess: s.activePathAccess,
      resolvePathAccess: s.resolvePathAccess,
      compression: s.compression,
      disconnect: s.disconnect,
      isLoadingSession: s.isLoadingSession,
      sessionLoadError: s.sessionLoadError,
      retryLoadSession: s.retryLoadSession,
      connect: s.connect,
      newSession: s.newSession,
      sendCommandMessage: s.sendCommandMessage,
      pushSystemNotice: s.pushSystemNotice,
    })),
  )
  const queuedCount = useChatStore((s) => s._pendingQueue.length)
  const stackOpen = usePortalStore((s) => s.stack.length > 0)
  // Server-derived persona affordances (design §6.1) drive the header's
  // conditional controls; fan-out agent tracking is unrelated store state.
  const profile = useEffectiveProfile()
  const lens = presentationLens(profile)
  const fanoutGroups = useFanoutStore((s) => s.groups)
  const [compareGroupId, setCompareGroupId] = useState<string | null>(null)
  const { data: rolesData } = useRoles()
  const roles = Object.entries(rolesData?.roles ?? {}).map(([name, model]) => ({ name, model }))

  const [input, setInput] = useState('')
  useDraftPersistence(activeSessionId, input, setInput)
  const [userScrolledUp, setUserScrolledUp] = useState(false)
  const messagesContainerRef = useRef<HTMLDivElement>(null)
  const atBottomRef = useRef(true)
  /** Session key that has been anchored to the bottom at least once. The
   *  session-switch effect fires before the async session fetch resolves, so
   *  `rows` is still empty and an anchor scroll is a no-op. The initial list
   *  layout then emits a scroll event at `scrollTop: 0`, which flips
   *  `atBottomRef` to false — permanently disarming the auto-scroll above.
   *  The first non-empty row render for a session force-anchors once. */
  const anchoredSessionRef = useRef<string | null>(null)
  const [expanded, setExpanded] = useState(false)

  // Studio inspector: closed by default (IA design §8.5). It may open by
  // itself ONLY during a live Code/Operations run with real activity, and
  // it always closes explicitly with focus return.
  const [inspectorOpen, setInspectorOpen] = useState(false)
  const [inspectorTab, setInspectorTab] = useState<StudioInspectorTab | null>(null)
  const inspectorToggleRef = useRef<HTMLButtonElement>(null)
  const availability = useInspectorAvailability(messages)
  const wasStreaming = useRef(false)
  useEffect(() => {
    const liveCodeOrOpsRun = lens === 'code' || lens === 'operations'
    if (isStreaming && !wasStreaming.current && liveCodeOrOpsRun && availability.activity) {
      setInspectorOpen(true)
      setInspectorTab('activity')
    }
    wasStreaming.current = isStreaming
  }, [isStreaming, lens, availability.activity])

  // Cross-surface launch (IA design §9): a confirmed intent seeds the
  // composer prompt once.
  const pendingPrompt = useStudioLaunchStore((s) => s.pendingPrompt)
  const setPendingPrompt = useStudioLaunchStore((s) => s.setPendingPrompt)
  useEffect(() => {
    if (pendingPrompt) {
      setInput(pendingPrompt)
      setPendingPrompt(null)
    }
  }, [pendingPrompt, setPendingPrompt])

  // Compressed groups: collapse older messages when a conversation is long.
  const COLLAPSE_THRESHOLD = 40
  const VISIBLE_TAIL = 20

  // Anchor the message list to its bottom (used on session switch, streaming
  // growth, and the scroll-to-bottom affordance). The list is a plain
  // scrolling div — rows are always ≤ collapseThreshold + cards, so the
  // previous virtua <VList> virtualization bought nothing and its initial
  // layout could leave the pane empty (rows rendered `visibility: hidden`
  // until a forced remeasure).
  const scrollToBottom = () => {
    const el = messagesContainerRef.current
    if (!el) return
    el.scrollTop = el.scrollHeight
  }

  // Virtualized row model (LobeHub borrow): the list renders this flat array.
  const rows = useMemo(
    () =>
      buildChatRows({
        messages,
        expanded,
        collapseThreshold: COLLAPSE_THRESHOLD,
        visibleTail: VISIBLE_TAIL,
        hasInterview: !!activeInterview && activeInterview.length > 0,
        hasToolApproval: !!activeToolApproval,
        hasPathAccess: !!activePathAccess,
        compression,
        isLoadingSession,
      }),
    [
      messages,
      expanded,
      activeInterview,
      activeToolApproval,
      activePathAccess,
      compression,
      isLoadingSession,
    ],
  )

  // Signature of the trailing message: content length + block count + streaming
  // flag. Any of these changing means the last row grew (text, a tool/reasoning
  // block, or a state flip) and the view must re-anchor to the bottom.
  const lastMsg = messages.at(-1)
  const lastSig = `${lastMsg?.content?.length ?? 0}:${lastMsg?.blocks?.length ?? 0}:${
    lastMsg?.generating ? 1 : 0
  }`

  // Auto-scroll to the last row while the user is at (or near) the bottom.
  // Re-anchors as the streaming message grows.
  useEffect(() => {
    if (rows.length === 0) return
    const sessionKey = activeSessionId ?? '_new'
    if (anchoredSessionRef.current !== sessionKey) {
      // First non-empty render for this session — anchor once, regardless of
      // the poisoned at-bottom flag, and re-arm auto-scrolling.
      anchoredSessionRef.current = sessionKey
      atBottomRef.current = true
      scrollToBottom()
      return
    }
    if (atBottomRef.current) {
      scrollToBottom()
    }
  }, [rows.length, lastSig, activeSessionId])

  // Session switch: always jump to the bottom of the freshly loaded session
  // (the original behavior scrolled on every messages change; keep that for
  // loadSession, independent of the current at-bottom state).
  useEffect(() => {
    anchoredSessionRef.current = null
    scrollToBottom()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSessionId])

  // Auto-connect WebSocket on mount
  useEffect(() => {
    connect()
  }, [connect])

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (mod && e.shiftKey && e.key.toLowerCase() === 'n') {
        e.preventDefault()
        newSession()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [newSession])

  // Auto-trigger LLM compression for long sessions.
  const compressTriggered = useRef(false)
  useEffect(() => {
    if (
      activeSessionId &&
      messages.length >= COLLAPSE_THRESHOLD &&
      compression === null &&
      !compressTriggered.current
    ) {
      compressTriggered.current = true
      const sid = activeSessionId
      const token = getToken()
      fetch(`/api/sessions/${encodeURIComponent(sid)}/compress`, {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${token}`,
          'Content-Type': 'application/json',
        },
      }).catch(() => {
        compressTriggered.current = false
      })
    }
    // Reset the guard when the session changes.
    return () => {
      compressTriggered.current = false
    }
  }, [activeSessionId, messages.length, compression])

  const handleListScroll = (e: UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80
    atBottomRef.current = atBottom
    // Only commit when the boolean actually flips — the raw handler fires per
    // scroll frame and each setState re-rendered the whole page.
    setUserScrolledUp((prev) => (prev === !atBottom ? prev : !atBottom))
  }

  // Client-local slash commands (2026-08-31 design): executed entirely in
  // the page — no WS traffic, no turn. Feedback goes through the same
  // notice rows the server command path uses.
  const runClientCommand = (cmd: SlashCommandDef, args: string) => {
    switch (cmd.id) {
      case 'clear':
        newSession()
        pushSystemNotice(t('command.conversationReset'), { command: 'clear', status: 'ok' })
        break
      case 'model': {
        const modelId = args.trim()
        if (!modelId) {
          pushSystemNotice(t('command.modelUsage'), { command: 'model', status: 'error' })
          break
        }
        setActiveModelId(modelId)
        pushSystemNotice(t('command.modelSet', { model: modelId }), {
          command: 'model',
          status: 'ok',
        })
        break
      }
      case 'export': {
        if (!activeSessionId) {
          pushSystemNotice(t('command.exportNoSession'), { command: 'export', status: 'error' })
          break
        }
        exportTranscriptMarkdown(activeSessionId)
          .then((filename) =>
            pushSystemNotice(t('command.exported', { filename }), {
              command: 'export',
              status: 'ok',
            }),
          )
          .catch(() =>
            pushSystemNotice(t('command.exportFailed'), { command: 'export', status: 'error' }),
          )
        break
      }
      case 'help': {
        const list = SLASH_COMMANDS.map(
          (c) =>
            `${c.label}${c.argHint ? ` ${c.argHint}` : ''} — ${t(`slash.${c.id}.description`)}`,
        ).join('\n')
        pushSystemNotice(list, { command: 'help', status: 'ok' })
        break
      }
      default:
        break
    }
  }

  const handleMiniMapJump = (index: number) => {
    const el = messagesContainerRef.current?.querySelector(`[data-msg-index="${index}"]`)
    el?.scrollIntoView({ block: 'center' })
  }

  const handleSend = (
    content: string,
    contextItems: ContextAttachment[],
    files: AttachedFile[],
  ) => {
    if (!content.trim()) return

    // Slash commands (2026-08-31 design): dispatched by execution class.
    // client → local action; server → command frame (no turn); turn → sent
    // verbatim, the WS layer strips the prefix and attaches the directive.
    // Commands bypass context enrichment and input history.
    const slash = parseSlashInput(content)
    if (slash) {
      setInput('')
      if (slash.cmd.kind === 'client') {
        runClientCommand(slash.cmd, slash.args)
        return
      }
      if (slash.cmd.kind === 'server') {
        sendCommandMessage(content)
        setUserScrolledUp(false)
        return
      }
    }

    let enrichedContent = content

    // Append context references
    if (contextItems.length > 0) {
      const contextRefs = contextItems
        .map((ctx) => {
          if (ctx.type === 'knowledge') return `[context:knowledge:${ctx.id}]`
          if (ctx.type === 'file') return `[context:file:${ctx.id}]`
          if (ctx.type === 'artifact') return `[context:artifact:${ctx.id}]`
          if (ctx.type === 'citation') return `[context:citation:${ctx.id}]`
          return `[context:memory:${ctx.id}]`
        })
        .join(' ')
      enrichedContent = `${content}\n${contextRefs}`
    }

    // Append file contents
    if (files.length > 0) {
      const fileContents = files
        .map((f) => {
          if (f.content) {
            return `[file:${f.name}]\n${f.content}\n[/file]`
          }
          if (f.dataUrl) {
            return `[image:${f.name}](${f.dataUrl})`
          }
          return `[file:${f.name}]`
        })
        .join('\n')
      enrichedContent = `${enrichedContent}\n${fileContents}`
    }

    addInputHistory(content)
    sendMessage(enrichedContent)
    setInput('')
    setUserScrolledUp(false)
  }

  // RFC-049: real cancellation. The previous implementation dropped the
  // socket and reconnected, which left the backend turn running and then
  // replayed its terminal message — the "cancelled" answer came back.
  const handleCancel = () => {
    useChatStore.getState().cancelTurn()
  }

  // RFC-032: retry the message that produced an error card. Pop the error
  // bubble AND the user message that preceded it (the store will append a
  // fresh user message when we resend, so leaving the original in place
  // would duplicate it on screen). After removal, scroll the user back to
  // the bottom and re-fire the same send pipeline as their original tap.
  const handleRetry = (errorMessageId: string) => {
    const errIdx = messages.findIndex((m) => m.id === errorMessageId)
    if (errIdx < 0) return
    const precedingUser = [...messages.slice(0, errIdx)].reverse().find((m) => m.role === 'user')
    if (!precedingUser) return
    const { removeMessage } = useChatStore.getState()
    removeMessage?.(errorMessageId)
    removeMessage?.(precedingUser.id)
    handleSend(precedingUser.content, [], [])
    setUserScrolledUp(false)
  }

  return (
    <div data-testid="studio-shell" className="flex h-full min-h-0 flex-col">
      <StudioContextBar />
      <WorkbenchShell>
        <div className="flex h-full">
          <div className="flex flex-1 flex-col min-w-0">
            {/* Reconnect warning banner */}
            {!connected && (
              <div className="flex items-center gap-2 px-4 py-2 bg-warning/10 text-warning text-xs border-b">
                <span className="h-2 w-2 rounded-full bg-warning animate-pulse shrink-0" />
                <span className="flex-1">{t('chat.reconnecting')}</span>
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 px-2 text-warning hover:text-warning"
                  onClick={() => {
                    disconnect()
                    connect()
                  }}
                >
                  <RefreshCw className="h-3 w-3 mr-1" />
                  {t('chat.retry')}
                </Button>
              </div>
            )}

            {/* Surfaced load failure with retry (sits beside the
              reconnect banner so the user can see WHY the pane is empty). */}
            {sessionLoadError && (
              <div className="flex items-center gap-2 border-b bg-destructive/10 px-4 py-2 text-xs text-destructive">
                <span className="flex-1">{t('chat.sessionLoadFailed')}</span>
                <Button size="sm" variant="ghost" className="h-6 px-2" onClick={retryLoadSession}>
                  <RefreshCw className="mr-1 h-3 w-3" />
                  {t('chat.retry')}
                </Button>
              </div>
            )}

            {/* Top-right controls: profile-gated terminal, global search,
                and the inspector toggle (explicit open — focus return
                target when the inspector closes). */}
            <div className="fixed top-4 right-4 z-50 flex items-center gap-2">
              {has(profile, 'terminal') && (
                <TerminalToggle className="h-8 gap-1 rounded-lg border bg-background px-3 py-1.5 text-xs font-normal shadow-sm" />
              )}
              <button
                type="button"
                className="flex items-center gap-1 rounded-lg border bg-background px-3 py-1.5 text-xs text-muted-foreground hover:text-primary hover:border-primary/50 transition-colors shadow-sm"
                onClick={() => usePortalStore.getState().pushView({ type: 'search' })}
              >
                <Search className="w-3.5 h-3.5" />
                {t('common.search')}
              </button>
              <button
                ref={inspectorToggleRef}
                type="button"
                data-testid="studio-inspector-toggle"
                aria-label={t('studio.inspector.label')}
                aria-expanded={inspectorOpen}
                className={`flex items-center gap-1 rounded-lg border px-3 py-1.5 text-xs shadow-sm transition-colors ${
                  inspectorOpen
                    ? 'border-primary/50 text-primary'
                    : 'bg-background text-muted-foreground hover:text-primary hover:border-primary/50'
                }`}
                onClick={() => {
                  if (inspectorOpen) {
                    setInspectorOpen(false)
                    setInspectorTab(null)
                  } else {
                    setInspectorOpen(true)
                    setInspectorTab((cur) => cur ?? 'context')
                  }
                }}
              >
                <PanelRight className="w-3.5 h-3.5" />
                {t('studio.inspector.label')}
              </button>
            </div>

            {/* ── Messages area ── */}
            <div
              ref={messagesContainerRef}
              onScroll={handleListScroll}
              className="relative flex-1 min-h-0 overflow-y-auto"
              role="log"
              aria-label={t('common.messages')}
            >
              {rows.map((row) => {
                if (row.kind === 'empty') {
                  return (
                    <div key="empty" className={rowClass(row, 'py-6')}>
                      <EmptyChatState />
                    </div>
                  )
                }
                if (row.kind === 'collapse-bar') {
                  return (
                    <div key="collapse-bar" className={rowClass(row, 'pt-6')}>
                      <CompressedGroup
                        count={row.count}
                        expanded={expanded}
                        onToggle={() => setExpanded((v) => !v)}
                        foldedMessages={row.foldedMessages}
                        compression={row.compression}
                      />
                    </div>
                  )
                }
                if (row.kind === 'message') {
                  const m = row.message
                  const assistantIndex =
                    m.role === 'assistant'
                      ? messages.slice(0, row.index).filter((x) => x.role === 'assistant').length
                      : undefined
                  return (
                    <div key={m.id} data-msg-index={row.index} className={rowClass(row, 'py-0.5')}>
                      <MessageBubble
                        message={m}
                        sessionId={activeSessionId ?? undefined}
                        assistantIndex={assistantIndex}
                        onRetry={m.metadata?.isError ? () => handleRetry(m.id) : undefined}
                      />
                    </div>
                  )
                }
                if (row.kind === 'interview') {
                  return (
                    <div key="interview" className={rowClass(row, 'py-2')}>
                      <InterviewWizard
                        questions={activeInterview!}
                        round={interviewRound}
                        ambiguity={interviewAmbiguity}
                        onSubmit={submitInterviewResponse}
                        disabled={isStreaming}
                      />
                    </div>
                  )
                }
                if (row.kind === 'tool-approval') {
                  return (
                    <div key="tool-approval" className={rowClass(row, 'py-2')}>
                      <ToolApprovalCard
                        toolName={activeToolApproval!.toolName}
                        reason={activeToolApproval!.reason}
                        onApprove={(remember) =>
                          resolveToolApproval(activeToolApproval!.id, true, remember)
                        }
                        onDeny={() => resolveToolApproval(activeToolApproval!.id, false)}
                        disabled={isStreaming}
                      />
                    </div>
                  )
                }
                // Loading shimmer placeholder.
                if (row.kind === 'skeleton') {
                  return (
                    <div key="skeleton">
                      <SessionSkeleton />
                    </div>
                  )
                }
                // path-access
                return (
                  <div key="path-access" className={rowClass(row, 'py-2')}>
                    <PathAccessCard
                      path={activePathAccess!.path}
                      mode={activePathAccess!.mode}
                      toolName={activePathAccess!.toolName}
                      reason={activePathAccess!.reason}
                      onTempAllow={() => resolvePathAccess(activePathAccess!.id, 'temp')}
                      onDeny={() => resolvePathAccess(activePathAccess!.id, 'deny')}
                      disabled={isStreaming}
                    />
                  </div>
                )
              })}
            </div>
            {userScrolledUp && (
              <button
                type="button"
                onClick={() => {
                  scrollToBottom()
                  setUserScrolledUp(false)
                }}
                className="absolute bottom-4 left-1/2 z-10 flex h-9 w-9 -translate-x-1/2 items-center justify-center rounded-full border bg-background shadow-lg transition-all hover:bg-accent"
                aria-label={t('chat.scrollToBottom')}
              >
                <ArrowDown className="h-4 w-4" />
              </button>
            )}
            <ChatMiniMap messages={messages} onJump={handleMiniMapJump} />
            <TextSelectionBar containerRef={messagesContainerRef} />
            {/* RFC-044 Phase 4: fan-out agent status grid + compare/merge */}
            {fanoutGroups.length > 0 && (
              <div className="shrink-0 border-t bg-background/95 px-4 py-2">
                {fanoutGroups.map((group) => {
                  const allSettled =
                    group.agents.length > 0 && group.agents.every((a) => a.status !== 'working')
                  return (
                    <div key={group.groupId} className="space-y-1.5">
                      {allSettled && (
                        <div className="flex items-center justify-between gap-2">
                          <span className="truncate text-2xs text-muted-foreground">
                            {group.prompt}
                          </span>
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="h-6 shrink-0 gap-1 px-2 text-2xs"
                            onClick={() => setCompareGroupId(group.groupId)}
                          >
                            <GitPullRequest className="h-3 w-3" />
                            {t('chat.fanout.compare', { defaultValue: 'Compare' })}
                          </Button>
                        </div>
                      )}
                      <AgentFanoutCardGrid agents={group.agents} />
                    </div>
                  )
                })}
              </div>
            )}
            {!activeInterview && (
              <StudioComposer
                messages={messages}
                input={input}
                setInput={setInput}
                onSend={handleSend}
                onCancel={handleCancel}
                isStreaming={isStreaming}
                connected={connected}
                queuedCount={queuedCount}
                roles={roles}
                setActiveRole={setActiveRole}
                activeModelId={activeModelId}
                setActiveModelId={setActiveModelId}
                onRevealTodo={scrollToBottom}
              />
            )}
          </div>
          <StudioInspector
            open={inspectorOpen}
            tab={inspectorTab}
            onTabChange={setInspectorTab}
            onClose={() => {
              setInspectorOpen(false)
              setInspectorTab(null)
            }}
            toggleRef={inspectorToggleRef}
          />
          {stackOpen && <PortalPanel className="shrink-0" />}
          {/* RFC-044 Phase 4: compare/merge panel */}
          {compareGroupId &&
            (() => {
              const group = fanoutGroups.find((g) => g.groupId === compareGroupId)
              if (!group) return null
              return (
                <WorktreeComparePanel
                  group={group}
                  open={!!compareGroupId}
                  onOpenChange={(v) => !v && setCompareGroupId(null)}
                />
              )
            })()}
        </div>
      </WorkbenchShell>
      <StudioLaunchSheet />
    </div>
  )
}
