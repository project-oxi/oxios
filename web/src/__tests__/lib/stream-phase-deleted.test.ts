// Regression: the `phase` WS chunk has no standalone frame — the Ouroboros
// phase is a plain string that is only ever "execute" and is only delivered
// as a field on `done` (backend orchestrator.rs:602). A standalone `phase`
// frame must fall through to the unknown-chunk default (events: []) rather
// than manufacturing a `phase` ChatEvent for a frame that never arrives.
import { describe, expect, it } from 'vitest'
import { adaptChunk } from '@/lib/stream/adapter'
import { KNOWN_CHUNK_TYPES } from '@/stores/chat'

describe('phase streaming path', () => {
  it('has no phase streaming path — phase is done-metadata only', () => {
    // A standalone phase frame is not part of the contract; the adapter must
    // treat it as unknown rather than manufacturing an event for it.
    expect(adaptChunk({ type: 'phase', phase: 'execute' } as never, { msgId: 'm' }).events).toEqual(
      [],
    )
    expect(KNOWN_CHUNK_TYPES).not.toContain('phase')
  })
})
