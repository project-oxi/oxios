// Task 6 (project-roots persona workbench): affordance helper matrix.
//
// The expectations below MIRROR the kernel derivation rule in
// crates/oxios-kernel/src/persona/mod.rs::derive_affordances (locked by
// crates/oxios-kernel/tests/execution_scope_contract.rs). The mirror is
// deliberately duplicated here: if the web-side helpers drift from the
// kernel rule, this matrix fails.
//
// Rule (design §6.1): Base/Code → ['diff'] always, plus files/terminal/
// worktree-fanout iff the project has filesystem roots. Minimal/Control → [].
// The UI never infers affordances from persona names or categories — only

import { describe, expect, it } from 'vitest'
import { has, isCodingFamily, presentationLens } from '@/lib/affordances'
import type { Affordance, EffectiveProfile, PresentationLens, ToolProfileName } from '@/types'

const ALL_AFFORDANCES: Affordance[] = ['files', 'terminal', 'diff', 'worktree-fanout']

/** Mirror of the kernel's `derive_affordances`. */
function kernelDerive(toolProfile: ToolProfileName, hasRoots: boolean): Affordance[] {
  if (toolProfile === 'base' || toolProfile === 'code') {
    return hasRoots ? ['diff', 'files', 'terminal', 'worktree-fanout'] : ['diff']
  }
  return []
}

function makeProfile(
  toolProfile: ToolProfileName,
  affordances: Affordance[],
  presentation_lens: PresentationLens = 'general',
): EffectiveProfile {
  return { persona_id: 'p1', tool_profile: toolProfile, affordances, presentation_lens }
}

describe('affordance matrix (mirrors kernel derive_affordances)', () => {
  for (const toolProfile of ['base', 'code', 'minimal', 'control'] as const) {
    for (const hasRoots of [true, false]) {
      it(`${toolProfile} × roots=${hasRoots}`, () => {
        const expected = kernelDerive(toolProfile, hasRoots)
        const p = makeProfile(toolProfile, expected)
        expect([...p.affordances].sort()).toEqual([...expected].sort())
        for (const a of ALL_AFFORDANCES) {
          expect(has(p, a)).toBe(expected.includes(a))
        }
      })
    }
  }

  it('folderless code keeps diff but hides terminal, files, and fanout', () => {
    // Kernel: unknown/unresolvable project ⇒ has_roots=false ⇒ [diff] only.
    const folderlessCode = makeProfile('code', kernelDerive('code', false))
    expect(has(folderlessCode, 'diff')).toBe(true)
    expect(has(folderlessCode, 'terminal')).toBe(false)
    expect(has(folderlessCode, 'files')).toBe(false)
    expect(has(folderlessCode, 'worktree-fanout')).toBe(false)
  })

  it('minimal and control expose nothing even with roots', () => {
    for (const toolProfile of ['minimal', 'control'] as const) {
      const p = makeProfile(toolProfile, kernelDerive(toolProfile, true))
      for (const a of ALL_AFFORDANCES) {
        expect(has(p, a)).toBe(false)
      }
    }
  })
})

describe('presentationLens accessor (IA design §8.5)', () => {
  it('returns the server-derived lens when present', () => {
    const p = makeProfile('code', [], 'research')
    expect(presentationLens(p)).toBe('research')
  })

  it('defaults a missing field to general (rolling-deploy boundary only)', () => {
    const missing = structuredClone(makeProfile('minimal', [])) as Partial<EffectiveProfile>
    delete missing.presentation_lens
    expect(presentationLens(missing as EffectiveProfile | null)).toBe('general')
    expect(presentationLens(null)).toBe('general')
  })

  it('null profile never carries an inferred lens', () => {
    expect(presentationLens(null)).toBe('general')
  })
})

describe('has', () => {
  it('null profile ⇒ no affordance (controls hidden)', () => {
    for (const a of ALL_AFFORDANCES) {
      expect(has(null, a)).toBe(false)
    }
  })
})

describe('isCodingFamily', () => {
  it('base and code are the coding family', () => {
    expect(isCodingFamily(makeProfile('base', []))).toBe(true)
    expect(isCodingFamily(makeProfile('code', []))).toBe(true)
  })

  it('minimal, control, and null are not', () => {
    expect(isCodingFamily(makeProfile('minimal', []))).toBe(false)
    expect(isCodingFamily(makeProfile('control', []))).toBe(false)
    expect(isCodingFamily(null)).toBe(false)
  })
})
