import { describe, expect, it } from 'vitest'
import { deriveSurface, SURFACE_HREFS, SURFACES } from './surfaces'

describe('deriveSurface', () => {
  it('maps the surface roots', () => {
    expect(deriveSurface('/operate')).toBe('operate')
    expect(deriveSurface('/knowledge')).toBe('knowledge')
    expect(deriveSurface('/studio')).toBe('studio')
  })

  it('maps nested surface routes', () => {
    expect(deriveSurface('/operate/projects/p1/issues')).toBe('operate')
    expect(deriveSurface('/knowledge/library/journal')).toBe('knowledge')
    expect(deriveSurface('/studio/sessions/s1')).toBe('studio')
    expect(deriveSurface('/studio/automations/a1')).toBe('studio')
  })

  it('maps trailing slashes', () => {
    expect(deriveSurface('/operate/')).toBe('operate')
    expect(deriveSurface('/knowledge/')).toBe('knowledge')
    expect(deriveSurface('/studio/')).toBe('studio')
  })

  it('returns operate for unknown paths and the root', () => {
    expect(deriveSurface('/')).toBe('operate')
    expect(deriveSurface('/agents/a1/trace')).toBe('operate')
    expect(deriveSurface('/settings')).toBe('operate')
    expect(deriveSurface('')).toBe('operate')
  })

  it('never returns a legacy surface name', () => {
    const samples = [
      '/',
      '/operate',
      '/operate/runs',
      '/knowledge',
      '/knowledge/memory',
      '/studio',
      '/studio/sessions/x',
      '/unknown',
    ]
    for (const p of samples) {
      expect(SURFACES).toContain(deriveSurface(p))
      expect(deriveSurface(p)).not.toBe('console')
      expect(deriveSurface(p)).not.toBe('brain')
      expect(deriveSurface(p)).not.toBe('chat')
    }
  })
})

describe('SURFACE_HREFS', () => {
  it('covers every surface with a rooted href', () => {
    for (const s of SURFACES) {
      expect(SURFACE_HREFS[s]).toMatch(/^\//)
    }
  })
})
