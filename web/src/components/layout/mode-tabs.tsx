import { Link, useRouterState } from '@tanstack/react-router'
import { Brain, LayoutDashboard, MessageSquare } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { deriveSidebarMode, type SidebarMode } from '@/stores/sidebar'

/**
 * The three top-level surfaces of Oxios: Console / Brain / Chat. Shared by
 * the Sidebar `ModeTabs` (desktop) and the mobile `BottomNav` so the mode set
 * stays in sync. `knowledge` lives nested under `/brain/knowledge`; the top
 * tab still highlights `brain` so a single tap drops the user back into the
 * Brain surface and the KnowledgeNav sub-section stays accessible.
 */
export const SIDEBAR_MODES: {
  key: SidebarMode
  icon: typeof LayoutDashboard
  labelKey: string
  href: string
}[] = [
  { key: 'console', icon: LayoutDashboard, labelKey: 'sidebar.console', href: '/' },
  { key: 'brain', icon: Brain, labelKey: 'sidebar.brain', href: '/brain' },
  { key: 'chat', icon: MessageSquare, labelKey: 'sidebar.chat', href: '/chat' },
]

/**
 * Desktop mode switcher — lives at the top of the Sidebar and is the single
 * source of truth for switching Console / Brain / Chat on desktop.
 *
 * The header no longer carries mode tabs (removed to eliminate the
 * sidebar↔header duplication); on mobile, mode switching lives in the
 * `BottomNav` bar instead.
 *
 * - Expanded: horizontal icon + label tabs.
 * - Collapsed (icon rail): vertical icon-only stack with right-side tooltips,
 *   mirroring `NavItemLink` so the collapsed sidebar stays consistent
 *   (VS Code Activity Bar pattern).
 *
 * `knowledge` is a sub-mode: routes nested under `/brain/knowledge` highlight
 * the Brain tab so a single tap drops the user back into the Brain surface.
 */
export function ModeTabs({ collapsed = false }: { collapsed?: boolean }) {
  const { t } = useTranslation()
  const router = useRouterState()
  const currentMode = deriveSidebarMode(router.location.pathname)
  const activeKey = currentMode === 'knowledge' ? 'brain' : currentMode

  if (collapsed) {
    return (
      <nav aria-label={t('common.modeNavigation')} className="flex flex-col items-center gap-1">
        {SIDEBAR_MODES.map(({ key, icon: Icon, labelKey, href }, idx) => {
          const isActive = activeKey === key
          const link = (
            <Link
              to={href}
              aria-current={isActive ? 'page' : undefined}
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
    <nav aria-label={t('common.modeNavigation')} className="flex items-center gap-0.5">
      {SIDEBAR_MODES.map(({ key, icon: Icon, labelKey, href }, idx) => {
        const isActive = activeKey === key
        return (
          <Link
            key={key}
            to={href}
            aria-current={isActive ? 'page' : undefined}
            title={`${t(labelKey)} (⌃${idx + 1})`}
            className={cn(
              'flex flex-1 items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium select-none transition-all',
              'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
              isActive
                ? 'bg-sidebar-accent text-sidebar-accent-foreground'
                : 'text-sidebar-foreground/50 hover:bg-sidebar-accent/50',
            )}
          >
            <Icon className="h-3.5 w-3.5" />
            <span>{t(labelKey)}</span>
          </Link>
        )
      })}
    </nav>
  )
}
