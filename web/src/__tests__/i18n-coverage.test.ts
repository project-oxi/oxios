// i18n regression guard: en and ko must define the same key set. A key in
// one locale but not the other silently renders as the raw key string for
// bilingual users (or falls back to English for Korean-only keys).

import { describe, expect, it } from 'vitest'
import en from '@/i18n/locales/en.json'
import ko from '@/i18n/locales/ko.json'

const flatten = (o: Record<string, unknown>, p = ''): string[] =>
  Object.entries(o).flatMap(([k, v]) =>
    v && typeof v === 'object' ? flatten(v as Record<string, unknown>, `${p}${k}.`) : [`${p}${k}`],
  )

describe('i18n locale parity', () => {
  it('en and ko define the same key set', () => {
    expect(flatten(en).sort()).toEqual(flatten(ko).sort())
  })
})
