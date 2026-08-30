// Workbench store — runtime state for the conversation-first workbench
// (Task 7, design §7.1-§7.3).
//
// This store is the single source of truth for what the workbench shows:
//   activeRail         — which rail is highlighted (UI affordance tie-in)
//   stageOpen          — whether a stage panel is visible
//   stagePinned        — whether the user pinned the stage (auto-dismiss
//                        skips the next "fresh artifact would replace it"
//                        transition until they explicitly toggle)
//   quickOpenOpen      — ⌘P quick-open dialog visibility
//   editingPath        — file editor modal target (null when closed)
//   reviewedChangeIds  — Set of change ids the user marked reviewed
//   revertedChangeIds  — Set of change ids the user reverted (Task 7
//                        affordance — the diff stage keeps them visible
//                        until the user resolves them)
//
// The store is NOT persisted. A reload returns to a clean state; per-tab
// stage toggling is intentional, not durable state.
//
// Emergence contract (design §7.2), verified in
// __tests__/stores/workbench.test.ts:
//   * openStage(rail, { pinIfChanges, changesPending })
//       - always sets stageOpen=true and activeRail=rail when not pinned
//       - stagePinned defaults to `pinIfChanges && changesPending > 0`
//         so calling openStage from a renderable-artifact path does not
//         accidentally inherit a pinned diff stage
//       - if a pinned stage is already open, the call is a no-op — the
//         pinned rail wins until the user explicitly dismisses it
//   * dismissStage clears stageOpen and stagePinned (it does NOT clear
//     activeRail — that stays as the user's last selection)
//   * togglePin flips stagePinned without touching stageOpen
//   * markReviewed / revertChange are idempotent set operations on the
//     reviewed / reverted sets so consumers can safely re-render

import { create } from 'zustand'

export type Rail = 'conversation' | 'files' | 'terminal' | 'changes' | 'preview'

export interface OpenStageOptions {
  /** Whether the rail type should pin itself if it has pending work
   *  (changesPending > 0). Defaults to false. */
  pinIfChanges?: boolean
  /** Number of pending changes awaiting review. Combined with
   *  `pinIfChanges` to decide whether the stage opens pinned. */
  changesPending?: number
}

export interface WorkbenchState {
  activeRail: Rail
  stageOpen: boolean
  stagePinned: boolean
  quickOpenOpen: boolean
  editingPath: string | null
  reviewedChangeIds: Set<string>
  revertedChangeIds: Set<string>

  setActiveRail: (r: Rail) => void
  openStage: (r: Rail, opts?: OpenStageOptions) => void
  dismissStage: () => void
  togglePin: () => void
  /** Explicit user override from the ActivityRail: selects the rail and
   *  opens its stage unpinned, clearing any stale pin left by emergence
   *  (the user's click outranks the auto-pin contract). */
  userSelectRail: (r: Rail) => void
  setQuickOpen: (open: boolean) => void
  openEditor: (path: string) => void
  closeEditor: () => void
  /** Mark a change reviewed. `pendingRemaining` is the count of still-
   *  pending changes AFTER this action; when it reaches 0 the stage pin
   *  releases (the stage stays open until the user dismisses/navigates). */
  markReviewed: (id: string, pendingRemaining?: number) => void
  revertChange: (id: string, pendingRemaining?: number) => void
  reset: () => void
}

const initialState = {
  activeRail: 'conversation' as Rail,
  stageOpen: false,
  stagePinned: false,
  quickOpenOpen: false,
  editingPath: null as string | null,
  reviewedChangeIds: new Set<string>(),
  revertedChangeIds: new Set<string>(),
}

export const useWorkbenchStore = create<WorkbenchState>()((set) => ({
  ...initialState,

  setActiveRail: (r) => set({ activeRail: r }),

  openStage: (r, opts) => {
    const pinIfChanges = opts?.pinIfChanges === true
    const pending = opts?.changesPending ?? 0
    set((s) => {
      // Design §7.2: a pinned stage stays open until the user dismisses
      // it. A fresh-artifact openStage call is therefore a no-op for the
      // activeRail — the pinned rail wins. Callers wanting to force the
      // override must dismissStage() first.
      if (s.stageOpen && s.stagePinned) {
        return s
      }
      return {
        activeRail: r,
        stageOpen: true,
        stagePinned: pinIfChanges && pending > 0,
      }
    })
  },

  dismissStage: () => set({ stageOpen: false, stagePinned: false }),

  togglePin: () => set((s) => ({ stagePinned: !s.stagePinned })),

  userSelectRail: (r) =>
    set({
      activeRail: r,
      // Rail click is an explicit override: open the target stage fresh and
      // unpinned — a pin left over from emergence must not reattach here.
      stageOpen: true,
      stagePinned: false,
    }),

  setQuickOpen: (open) => set({ quickOpenOpen: open }),

  openEditor: (path) => set({ editingPath: path }),
  closeEditor: () => set({ editingPath: null }),

  markReviewed: (id, pendingRemaining) =>
    set((s) => {
      const next = new Set(s.reviewedChangeIds)
      next.add(id)
      const settled = pendingRemaining === 0
      return {
        reviewedChangeIds: next,
        // Pin releases when the last pending change is resolved; the stage
        // itself stays open until the user dismisses or navigates.
        stagePinned: settled ? false : s.stagePinned,
      }
    }),

  revertChange: (id, pendingRemaining) =>
    set((s) => {
      const nextReviewed = new Set(s.reviewedChangeIds)
      nextReviewed.delete(id)
      const nextReverted = new Set(s.revertedChangeIds)
      nextReverted.add(id)
      const settled = pendingRemaining === 0
      return {
        reviewedChangeIds: nextReviewed,
        revertedChangeIds: nextReverted,
        stagePinned: settled ? false : s.stagePinned,
      }
    }),

  reset: () =>
    set({
      ...initialState,
      reviewedChangeIds: new Set(),
      revertedChangeIds: new Set(),
    }),
}))
