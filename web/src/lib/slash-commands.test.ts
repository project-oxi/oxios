import { describe, expect, it } from 'vitest'
import { parseSlashInput, SLASH_COMMANDS } from './slash-commands'

describe('parseSlashInput', () => {
  it('parses client commands with and without args', () => {
    expect(parseSlashInput('/clear')).toEqual({
      cmd: expect.objectContaining({ id: 'clear', kind: 'client' }),
      args: '',
    })
    expect(parseSlashInput('/model anthropic/claude-sonnet-4')).toEqual({
      cmd: expect.objectContaining({ id: 'model', kind: 'client' }),
      args: 'anthropic/claude-sonnet-4',
    })
  })

  it('parses server commands', () => {
    expect(parseSlashInput('/compact')).toEqual({
      cmd: expect.objectContaining({ id: 'compact', kind: 'server' }),
      args: '',
    })
    expect(parseSlashInput('/persona dev')).toEqual({
      cmd: expect.objectContaining({ id: 'persona', kind: 'server' }),
      args: 'dev',
    })
  })

  it('parses turn commands with multi-word args', () => {
    const parsed = parseSlashInput('/search rust tokio cancellation safety')
    expect(parsed?.cmd.id).toBe('search')
    expect(parsed?.cmd.kind).toBe('turn')
    expect(parsed?.args).toBe('rust tokio cancellation safety')

    const skill = parseSlashInput('/skill pdf extract page 3')
    expect(skill?.cmd.id).toBe('skill')
    expect(skill?.args).toBe('extract page 3')
  })

  it('preserves newlines and inner whitespace in args', () => {
    const parsed = parseSlashInput('/skill review\ncheck the diff\nthoroughly')
    expect(parsed?.args).toBe('check the diff\nthoroughly')
  })

  it('returns null for plain messages and unknown commands', () => {
    expect(parseSlashInput('hello /compact world')).toBeNull()
    expect(parseSlashInput('/nonexistent foo')).toBeNull()
    expect(parseSlashInput('/')).toBeNull()
    expect(parseSlashInput('')).toBeNull()
  })

  it('registry ids are unique and labels carry the slash prefix', () => {
    const ids = SLASH_COMMANDS.map((c) => c.id).sort()
    for (let i = 1; i < ids.length; i++) {
      expect(ids[i]).not.toBe(ids[i - 1])
    }
    for (const cmd of SLASH_COMMANDS) {
      expect(cmd.label).toBe(`/${cmd.id}`)
      expect(['client', 'server', 'turn']).toContain(cmd.kind)
    }
  })
})
