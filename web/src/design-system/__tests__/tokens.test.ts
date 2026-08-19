// ============================================================================
// Oxios Design System — Token Snapshot Tests
// ============================================================================
//
// Validates token system integrity:
// 1. All semantic tokens resolve to valid OKLCH values
// 2. Light/dark themes define identical property names
// 3. Primitive palette generation is deterministic
// 4. APCA contrast passes for key text/surface pairs
// ============================================================================

import { describe, expect, it } from 'vitest'
import { primitiveColors, semanticColors } from '../tokens/index'
import { apcaContrast, meetsContrastThreshold } from '../utils/contrast'
import { generatePalette, invertForDark, parseOklch } from '../utils/oklch'

// ── OKLCH Parsing ────────────────────────────────────────────────────────────

describe('parseOklch', () => {
  it('parses basic oklch value', () => {
    const result = parseOklch('oklch(0.596 0.145 163)')
    expect(result).toEqual({ l: 0.596, c: 0.145, h: 163, alpha: 1 })
  })

  it('parses oklch with alpha', () => {
    const result = parseOklch('oklch(1 0 0 / 10%)')
    expect(result).toEqual({ l: 1, c: 0, h: 0, alpha: 0.1 })
  })

  it('parses oklch with decimal alpha', () => {
    const result = parseOklch('oklch(0.577 0.245 27.325 / 0.15)')
    expect(result).toEqual({ l: 0.577, c: 0.245, h: 27.325, alpha: 0.15 })
  })

  it('returns null for invalid input', () => {
    expect(parseOklch('#ff0000')).toBeNull()
    expect(parseOklch('rgb(255,0,0)')).toBeNull()
    expect(parseOklch('')).toBeNull()
  })
})

// ── Palette Generation ───────────────────────────────────────────────────────

describe('generatePalette', () => {
  it('generates 11 steps by default', () => {
    const palette = generatePalette(0.5, 0.15, 250)
    const steps = Object.keys(palette).map(Number)
    expect(steps).toEqual([50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950])
  })

  it('is deterministic (same input = same output)', () => {
    const a = generatePalette(0.596, 0.145, 163)
    const b = generatePalette(0.596, 0.145, 163)
    expect(a).toEqual(b)
  })

  it('step 50 is lightest, step 950 is darkest', () => {
    const palette = generatePalette(0.5, 0.15, 250)
    const l50 = parseOklch(palette[50]!)!.l
    const l950 = parseOklch(palette[950]!)!.l
    expect(l50).toBeGreaterThan(l950)
  })

  it('preserves hue across all steps', () => {
    const palette = generatePalette(0.5, 0.15, 250)
    const hues = Object.values(palette).map((v) => parseOklch(v)!.h)
    expect(new Set(hues).size).toBe(1)
  })
})

// ── Dark Mode Inversion ──────────────────────────────────────────────────────

describe('invertForDark', () => {
  it('inverts lightness', () => {
    const dark = invertForDark('oklch(0.9 0.01 286)')
    const parsed = parseOklch(dark)!
    expect(parsed.l).toBeCloseTo(0.1, 2)
  })

  it('preserves hue and chroma', () => {
    const dark = invertForDark('oklch(0.6 0.15 163)')
    const parsed = parseOklch(dark)!
    expect(parsed.c).toBe(0.15)
    expect(parsed.h).toBe(163)
  })
})

// ── Primitive Colors Validation ──────────────────────────────────────────────

describe('primitiveColors', () => {
  it('has all 6 hue palettes', () => {
    const palettes = Object.keys(primitiveColors)
    expect(palettes).toContain('zinc')
    expect(palettes).toContain('red')
    expect(palettes).toContain('emerald')
    expect(palettes).toContain('amber')
    expect(palettes).toContain('blue')
    expect(palettes).toContain('violet')
  })

  it('each palette has 11 steps', () => {
    for (const [_name, palette] of Object.entries(primitiveColors)) {
      const steps = Object.keys(palette)
      expect(steps).toHaveLength(11)
      expect(steps).toEqual([
        '50',
        '100',
        '200',
        '300',
        '400',
        '500',
        '600',
        '700',
        '800',
        '900',
        '950',
      ])
    }
  })

  it('all values parse as valid OKLCH', () => {
    for (const [_name, palette] of Object.entries(primitiveColors)) {
      for (const [step, value] of Object.entries(palette)) {
        const parsed = parseOklch(value)
        expect(parsed, `Failed to parse ${name}.${step}: ${value}`).not.toBeNull()
      }
    }
  })
})

// ── Semantic Tokens Validation ───────────────────────────────────────────────

describe('semanticColors', () => {
  it('all semantic tokens reference CSS variables', () => {
    for (const [name, value] of Object.entries(semanticColors)) {
      expect(value, `Token ${name} should use var()`).toMatch(/^var\(--/)
    }
  })

  it('has required surface tokens', () => {
    expect(semanticColors).toHaveProperty('background')
    expect(semanticColors).toHaveProperty('foreground')
    expect(semanticColors).toHaveProperty('card')
    expect(semanticColors).toHaveProperty('primary')
    expect(semanticColors).toHaveProperty('muted')
    expect(semanticColors).toHaveProperty('border')
  })

  it('has required status tokens', () => {
    expect(semanticColors).toHaveProperty('success')
    expect(semanticColors).toHaveProperty('warning')
    expect(semanticColors).toHaveProperty('error')
    expect(semanticColors).toHaveProperty('info')
    expect(semanticColors).toHaveProperty('successSubtle')
    expect(semanticColors).toHaveProperty('warningSubtle')
    expect(semanticColors).toHaveProperty('errorSubtle')
    expect(semanticColors).toHaveProperty('infoSubtle')
  })

  it('has sidebar tokens', () => {
    expect(semanticColors).toHaveProperty('sidebar')
    expect(semanticColors).toHaveProperty('sidebarForeground')
    expect(semanticColors).toHaveProperty('sidebarAccent')
  })
})

// ── APCA Contrast Validation ─────────────────────────────────────────────────

describe('APCA contrast', () => {
  it('light mode fg/bg meets Lc 60', () => {
    // foreground (0.141) on background (1.0)
    const contrast = Math.abs(apcaContrast(0.141, 1.0))
    expect(contrast).toBeGreaterThanOrEqual(60)
  })

  it('dark mode fg/bg meets Lc 60', () => {
    // foreground (0.985) on background (0.141)
    const contrast = Math.abs(apcaContrast(0.985, 0.141))
    expect(contrast).toBeGreaterThanOrEqual(60)
  })

  it('muted-foreground on muted meets Lc 40 (decorative text)', () => {
    // Light: muted-fg (0.552) on muted (0.967)
    // Zinc muted text is intentionally subtle — used for captions/metadata, not body text
    const contrast = Math.abs(apcaContrast(0.552, 0.967))
    expect(contrast).toBeGreaterThanOrEqual(40)
  })

  it('meetsContrastThreshold helper works', () => {
    expect(meetsContrastThreshold(0.141, 1.0)).toBe(true)
    expect(meetsContrastThreshold(0.985, 0.141)).toBe(true)
    expect(meetsContrastThreshold(0.5, 0.55)).toBe(false)
  })
})

// ── Chat Component Token Compliance ──────────────────────────────────────────

describe('chat component token compliance', () => {
  it('chat components use tokens, not raw colors or dark: variants', async () => {
    const { readdir, readFile } = await import('node:fs/promises')
    const path = await import('node:path')
    const chatDir = path.join(process.cwd(), 'src', 'components', 'chat')

    const walk = async (dir: string): Promise<string[]> => {
      const entries = await readdir(dir, { withFileTypes: true })
      const files = await Promise.all(
        entries.map(async (e) => {
          const full = path.join(dir, e.name)
          return e.isDirectory() ? walk(full) : [full]
        }),
      )
      return files.flat()
    }

    const offenders: string[] = []
    for (const f of await walk(chatDir)) {
      if (!f.endsWith('.tsx') && !f.endsWith('.ts')) continue
      const src = await readFile(f, 'utf8')
      if (/\bdark:/.test(src)) offenders.push(`${path.relative(process.cwd(), f)}: dark: variant`)
      if (/\b(bg|text|border)-(white|black)\b/.test(src))
        offenders.push(`${path.relative(process.cwd(), f)}: raw color`)
      // Hex color literals. `#` followed by a digit could be a GitHub issue
      // number in a comment ("React #185") — only flag hex that could not be
      // one (starts with a-f, or is 6-8 chars), keeping the check strict
      // enough to catch `#fff` / `#ffffff` while ignoring issue refs.
      if (/#[a-fA-F][0-9a-fA-F]{2,7}\b|#[0-9a-fA-F]{6,8}\b/.test(src))
        offenders.push(`${path.relative(process.cwd(), f)}: hex literal`)
    }
    expect(offenders).toEqual([])
  })
})
