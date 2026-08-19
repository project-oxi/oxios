import { Link, useRouterState } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { deriveSidebarMode, useSidebarStore } from '@/stores/sidebar'
import { SIDEBAR_MODES } from './mode-tabs'
/**
 * BottomNav — mobile-only mode switcher (Console / Brain / Chat).
 *
 * On mobile the Sidebar lives in a slide-in drawer, which is the right place
 * for *context* navigation (chat sessions, file trees) but the wrong place for
 * switching the top-level surface — that should be one tap away and within
 * thumb reach. This bar mirrors the Discord / Slack mobile pattern: a
 * persistent bottom tab bar for surface switching, with the drawer reserved
 * for per-surface context.
 *
 * Hidden on desktop (lg+), where the Sidebar's `ModeTabs` is the single
 * source of truth. Respects the home indicator via `safe-area-inset-bottom`.
 *
 * Always visible across surfaces (including the immersive Chat
 * view): hiding it conditionally would leave that view with no mode
 * switcher, since the drawer no longer carries mode tabs. The Chat input is a
 * `shrink-0` flex sibling, so it stacks cleanly above this bar with no
 * overlap.
 */
export function BottomNav() {
  const { t } = useTranslation()
  const router = useRouterState()
  const currentMode = deriveSidebarMode(router.location.pathname)
  const activeKey = currentMode === 'knowledge' ? 'brain' : currentMode
  const setMobileOpen = useSidebarStore((s) => s.setMobileOpen)

  return (
    <nav
      aria-label={t('common.modeNavigation')}
      className={cn(
        'lg:hidden shrink-0 flex items-stretch justify-around',
        'border-t bg-background/95 backdrop-blur-sm',
        'pb-[env(safe-area-inset-bottom)]',
      )}
    >
      {SIDEBAR_MODES.map(({ key, icon: Icon, labelKey, href }) => {
        const isActive = activeKey === key
        return (
          <Link
            key={key}
            to={href}
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
