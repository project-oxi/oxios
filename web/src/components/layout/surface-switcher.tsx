import { Link, useRouterState } from '@tanstack/react-router'
import { BookOpen, LayoutDashboard, MessageSquare } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { deriveSurface, SURFACE_HREFS, type SurfaceId } from '@/types/surfaces'

/**
 * The three top-level surfaces shared by the desktop `SurfaceSwitcher` and
 * the mobile `BottomNav` so the surface set stays in sync.
 */
export const SURFACE_TABS: {
  key: SurfaceId
  icon: typeof LayoutDashboard
  labelKey: string
}[] = [
  { key: 'operate', icon: LayoutDashboard, labelKey: 'sidebar.operate' },
  { key: 'knowledge', icon: BookOpen, labelKey: 'sidebar.knowledge' },
  { key: 'studio', icon: MessageSquare, labelKey: 'sidebar.studio' },
]

/**
 * The one shared surface switcher — lives immediately beneath the Oxios
 * identity in the `Sidebar` and is the single source of truth for switching
 * Operate / Knowledge / Studio on desktop. On mobile, the `BottomNav` mirrors
 * the same `SURFACE_TABS`.
 *
 * - Expanded: horizontal icon + label tabs.
 * - Collapsed (icon rail): vertical icon-only stack with right-side tooltips,
 *   mirroring `NavItemLink` (VS Code Activity Bar pattern).
 */
export function SurfaceSwitcher({ collapsed = false }: { collapsed?: boolean }) {
  const { t } = useTranslation()
  const router = useRouterState()
  const active = deriveSurface(router.location.pathname)

  if (collapsed) {
    return (
      <nav aria-label={t('common.surfaceNavigation')} className="flex flex-col items-center gap-1">
        {SURFACE_TABS.map(({ key, icon: Icon, labelKey }, idx) => {
          const isActive = active === key
          const link = (
            <Link
              to={SURFACE_HREFS[key]}
              aria-current={isActive ? 'page' : undefined}
              aria-label={t(labelKey)}
              className={cn(
                'flex items-center justify-center rounded-md p-2 select-none transition-all',
                'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
                isActive
                  ? 'bg-sidebar-accent text-sidebar-accent-foreground'
                  : 'text-sidebar-foreground/50 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground',
              )}
            >
              <Icon className="h-4 w-4" />
            </Link>
          )
          return (
            <Tooltip key={key}>
              <TooltipTrigger asChild>{link}</TooltipTrigger>
              <TooltipContent side="right" sideOffset={8}>
                <span>{t(labelKey)}</span>
                <kbd className="ml-1.5 rounded border border-border/50 bg-muted/50 px-1 font-mono text-[10px] text-muted-foreground">
                  ⌃{idx + 1}
                </kbd>
              </TooltipContent>
            </Tooltip>
          )
        })}
      </nav>
    )
  }

  return (
    <nav aria-label={t('common.surfaceNavigation')} className="flex items-center gap-0.5">
      {SURFACE_TABS.map(({ key, icon: Icon, labelKey }, idx) => {
        const isActive = active === key
        return (
          <Link
            key={key}
            to={SURFACE_HREFS[key]}
            aria-current={isActive ? 'page' : undefined}
            title={`${t(labelKey)} (⌃${idx + 1})`}
            className={cn(
              'flex min-w-0 flex-1 items-center justify-center gap-1 rounded-md px-2 py-1.5 text-sm font-medium select-none transition-all',
              'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
              isActive
                ? 'bg-sidebar-accent text-sidebar-accent-foreground'
                : 'text-sidebar-foreground/50 hover:bg-sidebar-accent/50',
            )}
          >
            <Icon className="h-3.5 w-3.5 shrink-0" />
            <span className="whitespace-nowrap">{t(labelKey)}</span>
          </Link>
        )
      })}
    </nav>
  )
}
