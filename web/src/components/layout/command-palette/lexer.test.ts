import { describe, expect, it } from 'vitest'
import { buildContext, lex } from './lexer'

describe('palette lexer', () => {
  it('parses verb prefixes', () => {
    expect(lex('> refactor this')).toMatchObject({ verb: 'run', text: 'refactor this' })
    expect(lex('! kill it')).toMatchObject({ verb: 'control', text: 'kill it' })
    expect(lex('~ something')).toMatchObject({ verb: 'switch', text: 'something' })
    expect(lex('+ project')).toMatchObject({ verb: 'new', text: 'project' })
    expect(lex('/later 빨래')).toMatchObject({ verb: 'capture', text: 'later 빨래' })
  })

  it('treats non-prefix text as bare (verb null)', () => {
    expect(lex('just text')).toMatchObject({ verb: null, text: 'just text' })
    expect(lex('')).toMatchObject({ verb: null, text: '' })
    expect(lex('>')).toMatchObject({ verb: 'run', text: '' })
  })

  it('parses typed @entity and strips it from text', () => {
    expect(lex('> @skill:code-audit do thing')).toMatchObject({
      verb: 'run',
      entity: { type: 'skill', name: 'code-audit' },
      text: 'do thing',
    })
    expect(lex('! @agent:abc123')).toMatchObject({
      verb: 'control',
      entity: { type: 'agent', name: 'abc123' },
      text: '',
    })
    expect(lex('~ @surface:knowledge')).toMatchObject({
      verb: 'switch',
      entity: { type: 'surface', name: 'knowledge' },
    })
  })

  it('parses bare @entity (no type namespace)', () => {
    expect(lex('@bare')).toMatchObject({ verb: null, entity: { name: 'bare' } })
    expect(lex('+ @skill')).toMatchObject({ verb: 'new', entity: { name: 'skill' } })
  })

  it('peels an inline action token only for the ! control verb', () => {
    expect(lex('! @skill:legacy disable')).toMatchObject({
      verb: 'control',
      entity: { type: 'skill', name: 'legacy' },
      action: 'disable',
      text: '',
    })
    expect(lex('! @maxing start')).toMatchObject({
      verb: 'control',
      entity: { name: 'maxing' },
      action: 'start',
    })
    // `enable` after a non-control verb is plain text, not an action.
    expect(lex('> enable something')).toMatchObject({
      verb: 'run',
      action: undefined,
      text: 'enable something',
    })
  })

  it('only recognises the four valid action tokens', () => {
    expect(lex('! @skill:x foo')).toMatchObject({ action: undefined, text: 'foo' })
    expect(lex('! @skill:x ENABLE')).toMatchObject({ action: 'enable' }) // case-insensitive
  })

  it('buildContext attaches the surface', () => {
    const ctx = buildContext('> @skill:a b', 'knowledge')
    expect(ctx).toMatchObject({
      verb: 'run',
      entity: { type: 'skill', name: 'a' },
      text: 'b',
      surface: 'knowledge',
    })
  })
})
