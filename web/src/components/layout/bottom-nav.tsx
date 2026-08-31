import { Link, useRouterState } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { useSidebarStore } from '@/stores/sidebar'
import { deriveSurface, SURFACE_HREFS } from '@/types/surfaces'
import { SURFACE_TABS } from './surface-switcher'

/**
 * BottomNav — mobile-only surface switcher (Operate / Knowledge / Studio).
 *
 * On mobile the Sidebar lives in a slide-in drawer, which is the right place
 * for *context* navigation (sessions, local nav) but the wrong place for
 * switching the top-level surface — that should be one tap away and within
 * thumb reach. This bar mirrors the Discord / Slack mobile pattern: a
 * persistent bottom tab bar for surface switching, with the drawer reserved
 * for the active surface's local tree. There is no second switcher inside
 * the drawer.
 *
 * Hidden on desktop (lg+), where the Sidebar's `SurfaceSwitcher` is the
 * single source of truth. Respects the home indicator via
 * `safe-area-inset-bottom`.
 */
export function BottomNav() {
  const { t } = useTranslation()
  const router = useRouterState()
  const active = deriveSurface(router.location.pathname)
  const setMobileOpen = useSidebarStore((s) => s.setMobileOpen)

  return (
    <nav
      aria-label={t('common.surfaceNavigation')}
      className={cn(
        'lg:hidden shrink-0 flex items-stretch justify-around',
        'border-t bg-background/95 backdrop-blur-sm',
        'pb-[env(safe-area-inset-bottom)]',
      )}
    >
      {SURFACE_TABS.map(({ key, icon: Icon, labelKey }) => {
        const isActive = active === key
        return (
          <Link
            key={key}
            to={SURFACE_HREFS[key]}
            aria-current={isActive ? 'page' : undefined}
            // Close the drawer if a tab is tapped while it's open.
            onClick={() => setMobileOpen(false)}
            className={cn(
              'flex flex-1 flex-col items-center justify-center gap-0.5 py-2 select-none transition-colors',
              'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring',
              isActive ? 'text-primary' : 'text-muted-foreground hover:text-foreground',
            )}
          >
            <Icon className={cn('h-5 w-5 transition-transform', isActive && 'scale-110')} />
            <span className="text-[10px] font-medium">{t(labelKey)}</span>
          </Link>
        )
      })}
    </nav>
  )
}
