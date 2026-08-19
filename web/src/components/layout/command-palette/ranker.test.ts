import { describe, expect, it } from 'vitest'
import { modePrimaryVerb } from './ranker'

describe('modePrimaryVerb', () => {
  it('console → go', () => {
    expect(modePrimaryVerb('console')).toBe('go')
  })

  it('knowledge → capture', () => {
    expect(modePrimaryVerb('knowledge')).toBe('capture')
  })

  it('brain → search', () => {
    expect(modePrimaryVerb('brain')).toBe('search')
  })

  it('chat → run', () => {
    expect(modePrimaryVerb('chat')).toBe('run')
  })
})
