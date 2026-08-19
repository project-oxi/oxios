// Artifact store + identity tests — pure store-level. The component-level
// ArtifactCard ↔ panel wiring lives in artifact.test.tsx (toggled as a JSX
// file because it renders JSX). These tests target the data model: collision-
// free keys and version history on rewrite.

import { describe, expect, it } from 'vitest'
import { artifactKey, usePortalStore } from '@/stores/portal'
import { ArtifactType } from '@/types/artifact'

describe('artifactKey', () => {
  it('does not collide two untitled artifacts of the same type in one message', () => {
    const a = artifactKey({
      messageId: 'm1',
      type: ArtifactType.Html,
      source: 'language',
      ordinal: 0,
    })
    const b = artifactKey({
      messageId: 'm1',
      type: ArtifactType.Html,
      source: 'language',
      ordinal: 1,
    })
    expect(a).not.toBe(b)
  })

  it('scopes identity by blockId so same-type untitled artifacts across blocks do not collide', () => {
    // BlockStream renders one MarkdownMessage per text block, all sharing the
    // owning messageId — each block's document-order ordinal restarts at 0.
    const blockA = artifactKey({
      messageId: 'm1',
      blockId: 'blk-1',
      type: ArtifactType.Html,
      source: 'language',
      ordinal: 0,
    })
    const blockB = artifactKey({
      messageId: 'm1',
      blockId: 'blk-2',
      type: ArtifactType.Html,
      source: 'language',
      ordinal: 0,
    })
    expect(blockA).not.toBe(blockB)
  })
})

describe('usePortalStore artifact version history', () => {
  beforeEach(() => {
    usePortalStore.setState({ stack: [] })
  })

  it('keeps prior artifact versions when the agent rewrites one', () => {
    const meta = {
      messageId: 'm1',
      type: ArtifactType.Html,
      source: 'language' as const,
      ordinal: 0,
    }
    usePortalStore.getState().toggleArtifact(meta, '<p>v1</p>')
    usePortalStore.getState().pushArtifactVersion(artifactKey(meta), '<p>v2</p>')
    const view = usePortalStore.getState().stack.at(-1)
    if (view?.type !== 'artifact') throw new Error('expected artifact view on top')
    expect(view.versions).toEqual(['<p>v1</p>', '<p>v2</p>'])
    expect(view.activeVersion).toBe(1)
    expect(view.content).toBe('<p>v2</p>')
  })

  it('does not clobber a stepped-back version slot with live stream updates', () => {
    const meta = {
      messageId: 'm1',
      type: ArtifactType.Html,
      source: 'language' as const,
      ordinal: 0,
    }
    const store = usePortalStore
    store.getState().toggleArtifact(meta, '<p>v1</p>')
    store.getState().pushArtifactVersion(artifactKey(meta), '<p>v2</p>')
    // Step back to v1, then a live stream patch arrives for the same key.
    store.getState().setActiveVersion(artifactKey(meta), 0)
    store.getState().updateArtifactContent(artifactKey(meta), '<p>v2-streamed</p>')
    const view = store.getState().stack.at(-1)
    if (view?.type !== 'artifact') throw new Error('expected artifact view on top')
    // The historical slot the user is viewing stays intact.
    expect(view.versions[0]).toBe('<p>v1</p>')
    // The live patch landed on the live-tip slot (last), not on the viewed one.
    expect(view.versions[1]).toBe('<p>v2-streamed</p>')
    // The visible content still shows what the user stepped back to.
    expect(view.content).toBe('<p>v1</p>')
  })
})
