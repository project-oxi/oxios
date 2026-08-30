// use-stage-emergence — the §7.2 producer that turns live transcript events
// into stage transitions (Task 7 fix round 1, F1).
//
// Three rules, all derived from chat-store state already in memory (no new
// WS plumbing):
//   (a) A NEW file-change tool call appears (edit/write/patch/… aliases,
//       same matcher as use-session-changes) while the stage is closed and
//       not pinned → openStage('changes', pinned: pendingCount > 0).
//   (b) A terminal-alias tool call has been running >5s → openStage('terminal').
//   (c) A renderable artifact completes → openStage('preview'); the NEXT
//       completed artifact auto-dismisses the stage unless it is pinned.
// Event identity is tracked per session in refs so a dismissal stays
// dismissed: an id fires exactly once. Switching sessions resets the seen
// sets. The hook is a no-op for non-coding profiles — conditional hooks are
// illegal, so the gates live inside the effects.

import { useEffect, useRef } from 'react'
import { isTerminalToolName } from '@/components/workbench/terminal-aliases'
import { useSessionChanges } from '@/hooks/use-session-changes'
import { isCodingFamily } from '@/lib/affordances'
import { findLatestSessionArtifact } from '@/lib/session-artifacts'
import { useChatStore } from '@/stores/chat'
import { useWorkbenchStore } from '@/stores/workbench'
import type { ChatBlock, ChatMessage, EffectiveProfile } from '@/types'

/** How long a terminal-alias tool call may run before the stage emerges. */
const TERMINAL_EMERGE_MS = 5_000
/** Poll cadence for the running-terminal check. */
const TERMINAL_POLL_MS = 1_000

/** Running terminal tool-call ids with their start times. Pure. */
function runningTerminalCalls(messages: ChatMessage[]) {
  const out: { id: string; startedAt: number }[] = []
  for (const m of messages) {
    if (m?.role !== 'assistant') continue
    const blocks = (m.blocks ?? []) as ChatBlock[]
    for (const b of blocks) {
      if (b.type !== 'tool') continue
      const tool = b as unknown as {
        id: string
        apiName?: string
        status?: string
        startedAt?: number
      }
      if (!isTerminalToolName(tool.apiName)) continue
      if (tool.status !== 'loading') continue
      if (typeof tool.startedAt !== 'number') continue
      out.push({ id: tool.id, startedAt: tool.startedAt })
    }
  }
  return out
}

export function useStageEmergence(profile: EffectiveProfile | null): void {
  const isCoding = isCodingFamily(profile)
  const activeSessionId = useChatStore((s) => s.activeSessionId)
  const messages = useChatStore((s) => s.messages)
  const changes = useSessionChanges()

  // Event-identity refs — a seen id never fires twice, so a user dismissal
  // is final until a genuinely new event arrives.
  const seenChangeIds = useRef<Set<string>>(new Set())
  const firedTerminalIds = useRef<Set<string>>(new Set())
  const seenArtifactKeys = useRef<Set<string>>(new Set())

  // Session switch: reset identity so a freshly loaded transcript can
  // re-emerge (its pending changes deserve the stage).
  useEffect(() => {
    seenChangeIds.current = new Set()
    firedTerminalIds.current = new Set()
    seenArtifactKeys.current = new Set()
  }, [activeSessionId])

  // (a) + (c): transcript-derived emergence in one pass — changes take
  // priority over artifacts so a loaded session opens the review stage,
  // not a preview, when both kinds of events are present.
  useEffect(() => {
    if (!isCoding) return
    const store = useWorkbenchStore.getState()
    let changesFired = false

    for (const change of changes) {
      if (seenChangeIds.current.has(change.id)) continue
      seenChangeIds.current.add(change.id)
      if (changesFired) continue
      // §7.2: open only while the stage is closed and not pinned.
      if (store.stageOpen || store.stagePinned) continue
      const pendingCount = changes.filter((c) => c.status === 'pending').length
      store.openStage('changes', { pinIfChanges: true, changesPending: pendingCount })
      changesFired = true
    }

    if (changesFired) return
    const latest = findLatestSessionArtifact(messages)
    if (!latest) return
    if (seenArtifactKeys.current.has(latest.key)) return
    // First sighting in THIS session view opens the preview stage — even
    // for a pre-existing artifact surfaced on session rehydrate. Identity
    // is per session: the seen-key set resets on session switch (above),
    // so each session's first artifact re-emerges exactly once.
    const firstSighting = seenArtifactKeys.current.size === 0
    seenArtifactKeys.current.add(latest.key)
    const s = useWorkbenchStore.getState()
    if (firstSighting) {
      // §7.2: a completed (non-review) artifact opens the preview stage.
      s.openStage('preview')
      return
    }
    // Subsequent artifact: auto-dismiss the previous preview unless pinned
    // (openStage's pinned no-op keeps a pinned stage untouched).
    if (s.stageOpen && s.activeRail === 'preview' && !s.stagePinned) {
      s.dismissStage()
    } else {
      s.openStage('preview')
    }
  }, [isCoding, changes, messages])

  // (b): a terminal-alias call running >5s opens the terminal stage.
  useEffect(() => {
    if (!isCoding) return
    const tick = () => {
      const now = Date.now()
      for (const call of runningTerminalCalls(useChatStore.getState().messages)) {
        if (firedTerminalIds.current.has(call.id)) continue
        if (now - call.startedAt < TERMINAL_EMERGE_MS) continue
        firedTerminalIds.current.add(call.id)
        useWorkbenchStore.getState().openStage('terminal')
      }
    }
    const id = window.setInterval(tick, TERMINAL_POLL_MS)
    return () => window.clearInterval(id)
  }, [isCoding])
}
