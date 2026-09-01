import { useRouter } from '@tanstack/react-router'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { SURFACE_TABS } from '@/components/layout/surface-switcher'
import { SURFACE_HREFS } from '@/types/surfaces'
import { matchScore } from './ranker'
import type { CommandProvider, PaletteItem, QueryContext } from './types'

/**
 * Navigation provider — verb `go`.
 *
 * Matches the surface destinations by translated label on bare-text queries,
 * and returns the full destination list on an empty query (empty-state
 * surface). Only contributes when no explicit verb prefix was typed (bare
 * text), since `go` has no prefix of its own. Deeper destinations appear as
 * their owning workers add them; the palette never links legacy routes.
 */
export function useNavProvider(): CommandProvider {
  const router = useRouter()
  const { t } = useTranslation()

  const flatNav = useMemo(
    () =>
      SURFACE_TABS.map(({ key, icon: Icon, labelKey }) => ({
        href: SURFACE_HREFS[key],
        labelKey,
        icon: <Icon className="h-4 w-4" />,
      })),
    [],
  )

  return useMemo(
    () => ({
      id: 'nav',
      verbs: ['go'],
      resolve(ctx: QueryContext): PaletteItem[] {
        // `go` is bare-text only; an explicit verb prefix belongs to another provider.
        if (ctx.verb !== null) return []
        const q = (ctx.text || ctx.raw).trim().toLowerCase()
        const src = q ? flatNav.filter((i) => t(i.labelKey).toLowerCase().includes(q)) : flatNav
        const cap = q ? 8 : flatNav.length
        return src.slice(0, cap).map(
          (i): PaletteItem => ({
            id: `nav-${i.href}`,
            verb: 'go',
            icon: i.icon,
            title: t(i.labelKey),
            score: q ? matchScore(ctx, { label: t(i.labelKey) }) : 0,
            onSelect: () => {
              router.history.push(i.href)
            },
          }),
        )
      },
    }),
    [flatNav, router, t],
  )
}
