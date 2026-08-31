import { Link } from '@tanstack/react-router'
import React from 'react'
import { useTranslation } from 'react-i18next'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

// ── Shared sidebar design primitives (consumed by surface sidebars and
//    ChatSessionNav) ──

export const itemBase =
  'flex items-center gap-3 rounded-lg px-2.5 py-2 text-sm w-full text-left select-none transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-sidebar'

export const itemDense =
  'flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-xs w-full text-left select-none transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'

export const itemActive = 'bg-sidebar-accent text-sidebar-accent-foreground font-medium'
export const itemInactive =
  'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'
export const itemCollapsedBase =
  'flex items-center justify-center rounded-lg p-2 select-none transition-all focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'

export const sectionHeader =
  'px-2 mb-1 text-xs font-medium text-muted-foreground uppercase tracking-wider select-none'

export const sectionGap = 'mb-3'

export const sectionSeparator = 'border-t border-sidebar-border my-2'

// ── Nav items ──────────────────────────────────────────────────

export interface NavItem {
  labelKey: string
  href: string
  icon: React.ReactNode
  external?: boolean
  badge?: number
}

export function NavItemLink({
  item,
  currentPath,
  collapsed,
}: {
  item: NavItem
  currentPath: string
  collapsed: boolean
}) {
  const { t } = useTranslation()
  const isActive =
    currentPath === item.href || (item.href !== '/' && currentPath.startsWith(item.href))
  const showBadge = item.badge != null && item.badge > 0

  const link = (
    <Link
      to={item.href}
      className={cn(itemBase, isActive ? itemActive : itemInactive, collapsed && 'justify-center')}
    >
      {item.icon}
      {!collapsed && <span>{t(item.labelKey)}</span>}
      {!collapsed && showBadge && (
        <span className="ml-auto flex h-4 min-w-4 items-center justify-center rounded-full bg-warning px-1 text-2xs font-bold text-white animate-scale-in">
          {item.badge}
        </span>
      )}
    </Link>
  )

  return collapsed ? (
    <Tooltip key={item.href}>
      <TooltipTrigger asChild>{link}</TooltipTrigger>
      <TooltipContent side="right">
        {`${t(item.labelKey)}${item.badge ? ` (${item.badge})` : ''}`}
      </TooltipContent>
    </Tooltip>
  ) : (
    <React.Fragment key={item.href}>{link}</React.Fragment>
  )
}

/** One sidebar navigation group. Empty groups render nothing so wave-2
 * workers can populate items without leaving orphan headers. */
export function NavGroup({
  labelKey,
  items,
  currentPath,
  collapsed,
}: {
  labelKey: string
  items: NavItem[]
  currentPath: string
  collapsed: boolean
}) {
  const { t } = useTranslation()
  if (items.length === 0) return null
  return (
    <div className={sectionGap}>
      {!collapsed && <p className={sectionHeader}>{t(labelKey)}</p>}
      {items.map((item) => (
        <NavItemLink key={item.href} item={item} currentPath={currentPath} collapsed={collapsed} />
      ))}
    </div>
  )
}
