import { describe, expect, it } from 'vitest'
import { surfacePrimaryVerb } from './ranker'

describe('surfacePrimaryVerb', () => {
  it('operate → go', () => {
    expect(surfacePrimaryVerb('operate')).toBe('go')
  })

  it('knowledge → capture', () => {
    expect(surfacePrimaryVerb('knowledge')).toBe('capture')
  })

  it('studio → run', () => {
    expect(surfacePrimaryVerb('studio')).toBe('run')
  })
})
