// Portal panel store — stack-based navigation for the right-side panel.
//
// LobeHub analogue: store/chat/slices/portal (portalStack[] + pushView/popView/
// clearPortalStack). Oxios keeps this in a dedicated store so the large chat
// store stays untouched, and so future view types (thread, document, taskDetail,
// filePreview) can be added as new PortalView variants without touching chat.ts.
//
// Navigation model: a stack of views. The panel shows the TOP view. Pushing a
// view stacks it on top (back reveals the previous). popView goes back;
// clearStack closes the panel. This replaces the single-artifact model in the
// former stores/artifact.ts — an artifact is now one view type in the stack.

import { create } from 'zustand'
import type { ArtifactDisplayMode, ArtifactMeta } from '@/types/artifact'

/** A single view pushed onto the portal navigation stack. */
  | {
    type: 'artifact'
    /** Stable identity: `${messageId}::${type}::${ordinal}::${title}`. */
    key: string
    meta: ArtifactMeta
    content: string
    displayMode: ArtifactDisplayMode
    /** Full revision history. Index 0 is the first version; `activeVersion`
     *  points at the one currently shown in the panel. Streaming updates
     *  mutate `activeVersion` in place; a completed rewrite pushes a new
     *  entry so the user can diff against what the agent replaced. */
    versions: string[]
    activeVersion: number
  }
  | {
      type: 'filePreview'
      /** Absolute path of the file being previewed. */
      path: string
      /** File body. Undefined while the consumer is still loading it. */
      content?: string
    }
  // Future view types extend this union:
  // | { type: 'thread'; sessionId: string | null; parentId: string }
  // | { type: 'taskDetail'; taskId: string }
  | {
      type: 'document'
      /** KB file path (relative to knowledge root). */
      path: string
    }
  | {
      type: 'thread'
      /** Thread session ID. `null` = the thread is being created (loading). */
      sessionId: string | null
      /** Parent session ID this thread was spawned from. */
      parentId: string
    }
  | {
      type: 'search'
      /** Search query (auto-set on agent-driven, entered in panel on manual). */
      query?: string
      /** Chat message ID that triggered this view (agent-driven only). */
      messageId?: string
    }
  | {
      type: 'knowledge'
      /** Knowledge base file path (relative to knowledge root). */
      path: string
      /** Optional display title. */
      title?: string
    }
export interface PortalState {
  /** Navigation stack; top (last) element is the visible view. */
  stack: PortalView[]

  // ── Stack navigation ──────────────────────────────────────────────

  /** Push a view onto the stack (becomes the visible view). */
  pushView: (view: PortalView) => void
  /** Pop the top view (go back). No-op on an empty stack. */
  popView: () => void
  /** Clear the entire stack (close the panel). */
  clearStack: () => void

  // ── Artifact convenience ──────────────────────────────────────────

  /** Toggle an artifact: if it is the current top view, pop it; else push. */
  toggleArtifact: (meta: ArtifactMeta, content: string) => void
  /** Update the content of the artifact view with `key` (live streaming push). */
  updateArtifactContent: (key: string, content: string) => void
  /** Update the display mode of the artifact view with `key`. */
  setArtifactDisplayMode: (key: string, mode: ArtifactDisplayMode) => void

  // ── File preview convenience ──────────────────────────────────────

  /** Push a file preview view onto the stack. Toggles off if the same path
   *  is already on top. Callers can pass `content` once loaded, or omit it
   *  to push a loading shell and update later. */
  pushFilePreview: (path: string, content?: string) => void

  /** Update the content of the topmost `filePreview` view (live streaming
   *  push while the file is being read). No-op if the top view isn't a
   *  file preview or its path doesn't match. */
  updateFilePreviewContent: (path: string, content: string) => void

  // ── Document convenience ──────────────────────────────────────────

  /** Push a KB document view onto the stack. Toggles off if the same path
   *  is already on top. */
  pushDocument: (path: string) => void
}

/** Stable identity key for an artifact within a message. */
export function artifactKey(meta: ArtifactMeta): string {
  return `${meta.messageId}::${meta.type}::${meta.title ?? ''}`
}

export const usePortalStore = create<PortalState>((set, get) => ({
  stack: [],

  pushView: (view) => set((s) => ({ stack: [...s.stack, view] })),

  popView: () => set((s) => (s.stack.length > 0 ? { stack: s.stack.slice(0, -1) } : s)),

  clearStack: () => set({ stack: [] }),

  toggleArtifact: (meta, content) => {
    const { stack } = get()
    const key = artifactKey(meta)
    const top = stack[stack.length - 1]
    if (top?.type === 'artifact' && top.key === key) {
      // Same artifact is on top → pop it (back / close if last).
      set({ stack: stack.slice(0, -1) })
    } else {
      // Check if this artifact already exists deeper in the stack.
      const existing = stack.findIndex((v) => v.type === 'artifact' && v.key === key)
      if (existing >= 0) {
        // Truncate to that view (re-surface it).
        set({ stack: stack.slice(0, existing + 1) })
      } else {
        set({
          stack: [...stack, { type: 'artifact', key, meta, content, displayMode: 'preview' }],
        })
      }
    }
  },

  updateArtifactContent: (key, content) =>
    set((s) => ({
      stack: s.stack.map((v) => (v.type === 'artifact' && v.key === key ? { ...v, content } : v)),
    })),

  setArtifactDisplayMode: (key, mode) =>
    set((s) => ({
      stack: s.stack.map((v) =>
        v.type === 'artifact' && v.key === key ? { ...v, displayMode: mode } : v,
      ),
    })),

  pushFilePreview: (path, content) => {
    const { stack } = get()
    const top = stack[stack.length - 1]
    // If the same file is already on top, pop the stack (toggle off) so
    // repeated clicks behave like a peek. Refreshing content on an existing
    // deeper view is not handled here — caller can pop and re-push.
    if (top?.type === 'filePreview' && top.path === path) {
      set({ stack: stack.slice(0, -1) })
      return
    }
    set({
      stack: [...stack, { type: 'filePreview', path, content }],
    })
  },

  updateFilePreviewContent: (path, content) =>
    set((s) => {
      // Only patch the topmost matching view; avoids clobbering an unrelated
      // preview deeper in the stack.
      const idx = s.stack.length - 1
      if (idx < 0) return s
      const top = s.stack[idx]
      if (top?.type !== 'filePreview' || top.path !== path) return s
      const next = s.stack.slice()
      next[idx] = { ...top, content }
      return { stack: next }
    }),

  pushDocument: (path) => {
    const { stack } = get()
    const top = stack[stack.length - 1]
    // Same doc on top → toggle off (peek).
    if (top?.type === 'document' && top.path === path) {
      set({ stack: stack.slice(0, -1) })
      return
    }
    // Same doc deeper in the stack → re-surface it (truncate to that view)
    // instead of pushing a duplicate. Matches toggleArtifact's dedup policy.
    const existing = stack.findIndex((v) => v.type === 'document' && v.path === path)
    if (existing >= 0) {
      set({ stack: stack.slice(0, existing + 1) })
      return
    }
    set({
      stack: [...stack, { type: 'document', path }],
    })
  },
}))
