/**
 * The three top-level Oxios surfaces. The only navigation roots in the
 * product: Operate (decisions), Knowledge (context), Studio (creation).
 *
 * Frozen WT-2 contract (docs/designs/2026-08-31-oxios-web-ia-implementation-plan.md):
 * legacy `console` / `brain` / `chat` surface names must not appear anywhere
 * in shipped navigation code.
 */
export const SURFACES = ['operate', 'knowledge', 'studio'] as const

export type SurfaceId = (typeof SURFACES)[number]

/** Canonical href for each surface — the switcher, bottom bar, and palette
 * all link these; there is exactly one switcher per viewport. */
export const SURFACE_HREFS: Record<SurfaceId, string> = {
  operate: '/operate',
  knowledge: '/knowledge',
  studio: '/studio',
}

/**
 * Derive the active surface from a pathname.
 *
 * - `/operate*` → `operate`
 * - every `/knowledge` route → `knowledge`
 * - every `/studio` route → `studio`
 * - anything else (unknown paths, router error rendering, root) → `operate`,
 *   the documented replacement for the removed root dashboard.
 */
export function deriveSurface(pathname: string): SurfaceId {
  if (pathname.startsWith('/knowledge')) return 'knowledge'
  if (pathname.startsWith('/studio')) return 'studio'
  return 'operate'
}
