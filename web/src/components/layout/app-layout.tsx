import { Outlet, useRouterState } from '@tanstack/react-router'
import React, { useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { InfoPanel } from '@/components/knowledge/info-panel'
import { MoveModal } from '@/components/knowledge/move-modal'
import { SearchModal } from '@/components/knowledge/search-modal'
import { CommandPalette } from '@/components/layout/command-palette'
import { QuickAskDialog } from '@/components/quick-ask/quick-ask-dialog'
import {
  useApprovalWatcher,
  useGlobalEvents,
  useReconnectOnVisible,
} from '@/hooks/use-global-events'
import { useKnowledgeShortcuts } from '@/hooks/use-knowledge-shortcuts'
import { useQuickAskShortcut } from '@/hooks/use-quick-ask-shortcut'
import { useTabShortcuts } from '@/hooks/use-tab-shortcuts'
import { cn } from '@/lib/utils'
import { useEventStore } from '@/stores/events'
import { useKnowledgeStore } from '@/stores/knowledge'
import { useSidebarStore } from '@/stores/sidebar'
import { BottomNav } from './bottom-nav'
import { Header } from './header'
import { NotificationCenter } from './notification-center'
import { Sidebar } from './sidebar'

/**
 * AppLayout — single sidebar, mode-adaptive.
 *
 * The Sidebar component internally switches between Console/Knowledge/Chat
 * nav content based on the current route. No sidebar replacement needed.
 */
export function AppLayout() {
  const { t } = useTranslation()
  const { mobileOpen, setMobileOpen } = useSidebarStore()

  const router = useRouterState()
  const pathname = router.location.pathname
  const isKnowledge = pathname.startsWith('/brain/knowledge')
  const isKnowledgeSubRoute =
    isKnowledge && pathname !== '/brain/knowledge' && pathname !== '/brain/knowledge/'
  const isChat = pathname === '/chat'
  const { infoPanelOpen } = useKnowledgeStore()

  useKnowledgeShortcuts()
  useGlobalEvents()
  useApprovalWatcher()
  useQuickAskShortcut()
  useTabShortcuts()
  useReconnectOnVisible()

  // Bootstrap singleton SSE connection on first mount
  const connectEvents = useEventStore((s) => s.connect)
  React.useState(() => {
    connectEvents()
  })

  // Close the mobile drawer on Escape — keyboard accessibility
  useEffect(() => {
    if (!mobileOpen) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMobileOpen(false)
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [mobileOpen, setMobileOpen])

  return (
    <div className="flex h-[100vh] h-dvh overflow-hidden">
      {/* ── Desktop sidebar — persistent, width-collapsible ── */}
      <div className="hidden lg:flex">
        <Sidebar />
      </div>

      {/* ── Mobile sidebar — slide-in drawer (animated enter + exit) ── */}
      {/* Backdrop: always mounted, opacity + pointer-events toggled */}
      <div
        role="presentation"
        aria-hidden={!mobileOpen}
        onClick={() => setMobileOpen(false)}
        className={cn(
          'fixed inset-0 z-40 bg-black/50 backdrop-blur-sm lg:hidden',
          'transition-opacity duration-300 ease-[var(--animate-in-easing)]',
          mobileOpen ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
      />
      {/* Drawer panel: always mounted, translateX toggled so closing slides out */}
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t('common.closeMenu')}
        className={cn(
          'fixed inset-y-0 left-0 z-50 flex lg:hidden',
          'transition-transform duration-300 ease-[var(--animate-in-easing)] will-change-transform',
          mobileOpen ? 'translate-x-0' : 'pointer-events-none -translate-x-full',
        )}
      >
        <Sidebar />
      </div>

      {/* ── Main content area ── */}
      <div className="flex flex-1 flex-col min-w-0 overflow-hidden">
        <Header />

        {isKnowledge ? (
          <div className="flex flex-1 min-h-0 overflow-hidden">
            <div className="flex flex-1 min-w-0 overflow-hidden">
              <Outlet />
            </div>
            {/* InfoPanel only on main knowledge route, not sub-routes */}
            {!isKnowledgeSubRoute && infoPanelOpen && <InfoPanel />}
          </div>
        ) : isChat ? (
          /* Chat: no padding, full height */
          <main className="flex-1 min-h-0 overflow-hidden">
            <Outlet />
          </main>
        ) : (
          <main className="flex-1 overflow-y-auto p-3 lg:p-4 min-h-0">
            <Outlet />
          </main>
        )}

        {/* Mobile mode switcher — Console/Knowledge/Chat, thumb-reachable */}
        <BottomNav />
      </div>

      {isKnowledge && (
        <>
          <SearchModal />
          <MoveModal />
        </>
      )}
      {/* Global Notification Center slide-over (schedule + notifications) */}
      <NotificationCenter />
      <CommandPalette />
      <QuickAskDialog />
    </div>
  )
}
