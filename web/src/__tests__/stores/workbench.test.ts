// Task 7 (project-roots persona workbench): workbench store — stage
// emergence, pin/dismiss/auto-dismiss rule, reviewed set.
//
// The store is the single source of truth for what the workbench shows:
//   activeRail         — which rail is highlighted (UI affordance tie-in)
//   stageOpen          — whether a stage panel is visible
//   stagePinned        — whether user pinned the stage (auto-dismiss skips)
//   quickOpenOpen      — ⌘P quick-open dialog
//   editingPath        — file editor modal path
//   reviewedChangeIds  — set of change ids the user marked reviewed
//
// Design §7.2 emergence rules verified here:
//   openStage('changes', pinned: changesPending > 0)
//     → stageOpen: true, stagePinned derived from the pending count
//   dismissStage while unpinned + a new artifact would replace it
//     → stageOpen stays false; pin keeps it open
//   markReviewed removes from the pending count
//
// This file intentionally lives under __tests__/stores (created in this
// task) — prior tests under __tests__/components/* use Vitest's per-file
// discovery and there is no global registry.

import { beforeEach, describe, expect, it } from 'vitest'
import { useWorkbenchStore } from '@/stores/workbench'

beforeEach(() => {
  // Each test starts from a fully reset store so the (not persisted) state
  // from the previous test never leaks across cases.
  useWorkbenchStore.getState().reset()
})

describe('openStage / dismissStage transitions', () => {
  it('openStage sets stageOpen=true and default-pins when pending changes exist', () => {
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: true, changesPending: 2 })
    const s = useWorkbenchStore.getState()
    expect(s.stageOpen).toBe(true)
    expect(s.activeRail).toBe('changes')
    expect(s.stagePinned).toBe(true)
  })

  it('openStage with no pending changes does not pin by default', () => {
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: true, changesPending: 0 })
    expect(useWorkbenchStore.getState().stagePinned).toBe(false)
  })

  it('dismissStage clears the stage but preserves the reviewed set', () => {
    useWorkbenchStore.getState().markReviewed('msg::blockA')
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: false })
    useWorkbenchStore.getState().dismissStage()
    const s = useWorkbenchStore.getState()
    expect(s.stageOpen).toBe(false)
    expect(s.reviewedChangeIds.has('msg::blockA')).toBe(true)
  })

  it('auto-dismiss on a new artifact is a no-op when the stage is pinned', () => {
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: true, changesPending: 1 })
    useWorkbenchStore.getState().openStage('preview', { pinIfChanges: false })
    // Pinned stage stays open even when a fresh artifact would normally
    // take the slot.
    expect(useWorkbenchStore.getState().stageOpen).toBe(true)
    expect(useWorkbenchStore.getState().activeRail).toBe('changes')
  })

  it('auto-dismiss on a new artifact closes the prior stage when unpinned', () => {
    useWorkbenchStore.getState().openStage('preview', { pinIfChanges: false })
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: false })
    const s = useWorkbenchStore.getState()
    expect(s.stageOpen).toBe(true)
    expect(s.activeRail).toBe('changes')
    expect(s.stagePinned).toBe(false)
  })

  it('togglePin flips the pinned flag without touching open state', () => {
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: false })
    expect(useWorkbenchStore.getState().stagePinned).toBe(false)
    useWorkbenchStore.getState().togglePin()
    expect(useWorkbenchStore.getState().stagePinned).toBe(true)
    expect(useWorkbenchStore.getState().stageOpen).toBe(true)
    useWorkbenchStore.getState().togglePin()
    expect(useWorkbenchStore.getState().stagePinned).toBe(false)
  })

  it('openStage with pinIfChanges=false always leaves the stage unpinned', () => {
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: false, changesPending: 99 })
    expect(useWorkbenchStore.getState().stagePinned).toBe(false)
  })

  it('setActiveRail updates the rail independently of stageOpen', () => {
    useWorkbenchStore.getState().setActiveRail('files')
    expect(useWorkbenchStore.getState().activeRail).toBe('files')
    expect(useWorkbenchStore.getState().stageOpen).toBe(false)
  })
})

describe('quickOpen / editor modal', () => {
  it('setQuickOpen(true) opens the dialog without affecting the stage', () => {
    useWorkbenchStore.getState().openStage('changes', { pinIfChanges: true, changesPending: 1 })
    useWorkbenchStore.getState().setQuickOpen(true)
    const s = useWorkbenchStore.getState()
    expect(s.quickOpenOpen).toBe(true)
    expect(s.stageOpen).toBe(true)
    expect(s.activeRail).toBe('changes')
  })

  it('openEditor(path) sets editingPath and leaves stage state intact', () => {
    useWorkbenchStore.getState().openStage('preview', { pinIfChanges: false })
    useWorkbenchStore.getState().openEditor('/repo/src/foo.ts')
    const s = useWorkbenchStore.getState()
    expect(s.editingPath).toBe('/repo/src/foo.ts')
    expect(s.stageOpen).toBe(true)
  })

  it('closeEditor clears the editing path', () => {
    useWorkbenchStore.getState().openEditor('/repo/src/foo.ts')
    useWorkbenchStore.getState().closeEditor()
    expect(useWorkbenchStore.getState().editingPath).toBeNull()
  })
})

describe('reviewedChangeIds set semantics', () => {
  it('markReviewed adds the id to the set (idempotent)', () => {
    const store = useWorkbenchStore.getState()
    store.markReviewed('m1::b1')
    store.markReviewed('m1::b1')
    expect(useWorkbenchStore.getState().reviewedChangeIds.has('m1::b1')).toBe(true)
    expect(useWorkbenchStore.getState().reviewedChangeIds.size).toBe(1)
  })

  it('markReviewed tracks multiple distinct ids', () => {
    const store = useWorkbenchStore.getState()
    store.markReviewed('a')
    store.markReviewed('b')
    store.markReviewed('c')
    const set = useWorkbenchStore.getState().reviewedChangeIds
    expect([...set].sort()).toEqual(['a', 'b', 'c'])
  })

  it('revertChange removes the id from reviewed (so it returns to pending on next pass)', () => {
    const store = useWorkbenchStore.getState()
    store.markReviewed('a')
    store.revertChange('a')
    expect(useWorkbenchStore.getState().reviewedChangeIds.has('a')).toBe(false)
  })

  it('reset clears every transient field including the reviewed set', () => {
    const store = useWorkbenchStore.getState()
    store.openStage('changes', { pinIfChanges: true, changesPending: 3 })
    store.markReviewed('x')
    store.openEditor('/foo')
    store.setQuickOpen(true)
    store.reset()
    const s = useWorkbenchStore.getState()
    expect(s.stageOpen).toBe(false)
    expect(s.stagePinned).toBe(false)
    expect(s.editingPath).toBeNull()
    expect(s.quickOpenOpen).toBe(false)
    expect(s.reviewedChangeIds.size).toBe(0)
    expect(s.activeRail).toBe('conversation')
  })
})

describe('userSelectRail — explicit rail override (fix F5)', () => {
  it('opens the target rail unpinned, clearing a stale emergence pin', () => {
    const store = useWorkbenchStore.getState()
    // Emergence left the changes stage open AND pinned.
    store.openStage('changes', { pinIfChanges: true, changesPending: 2 })
    expect(useWorkbenchStore.getState().stagePinned).toBe(true)
    // User clicks the preview rail → fresh unpinned stage.
    store.userSelectRail('preview')
    const s = useWorkbenchStore.getState()
    expect(s.activeRail).toBe('preview')
    expect(s.stageOpen).toBe(true)
    expect(s.stagePinned).toBe(false)
  })
})

describe('pin release when pending hits zero (fix F6)', () => {
  it('markReviewed with pendingRemaining=0 releases the pin (stage stays open)', () => {
    const store = useWorkbenchStore.getState()
    store.openStage('changes', { pinIfChanges: true, changesPending: 1 })
    expect(useWorkbenchStore.getState().stagePinned).toBe(true)
    store.markReviewed('a', 0)
    const s = useWorkbenchStore.getState()
    expect(s.stagePinned).toBe(false)
    expect(s.stageOpen).toBe(true)
    expect(s.reviewedChangeIds.has('a')).toBe(true)
  })

  it('markReviewed with pendingRemaining>0 keeps the pin', () => {
    const store = useWorkbenchStore.getState()
    store.openStage('changes', { pinIfChanges: true, changesPending: 2 })
    store.markReviewed('a', 1)
    expect(useWorkbenchStore.getState().stagePinned).toBe(true)
  })

  it('revertChange with pendingRemaining=0 releases the pin', () => {
    const store = useWorkbenchStore.getState()
    store.openStage('changes', { pinIfChanges: true, changesPending: 1 })
    store.revertChange('a', 0)
    const s = useWorkbenchStore.getState()
    expect(s.stagePinned).toBe(false)
    expect(s.revertedChangeIds.has('a')).toBe(true)
  })
})
