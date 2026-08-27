import { Menu, Search, Settings2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { useCommandPaletteStore } from '@/stores/command-palette'
import { useOxiosCopilotStore } from '@/stores/oxios-copilot'
import { useSidebarStore } from '@/stores/sidebar'
import { MenuClock } from './menu-clock'

export function Header() {
  const { t } = useTranslation()
  const { setMobileOpen } = useSidebarStore()
  const openPalette = useCommandPaletteStore((s) => s.openPalette)
  const openCopilot = useOxiosCopilotStore((s) => s.openCopilot)

  return (
    <header className="flex h-14 items-center gap-2 border-b bg-background px-3 pt-[env(safe-area-inset-top)] lg:gap-3 lg:px-4">
      {/* Mobile hamburger */}
      <button
        type="button"
        className="shrink-0 rounded-md p-1.5 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring lg:hidden"
        onClick={() => setMobileOpen(true)}
        aria-label={t('common.openNav')}
      >
        <Menu className="h-5 w-5" />
      </button>

      {/*
        Command palette trigger — promoted to a primary, left-aligned search
        affordance (Notion / Linear / Raycast pattern). ⌘K is the app's most
        powerful feature; this gives it visible prominence instead of burying it
        as a tiny right-aligned button. Fills the void left by removing the
        header mode tabs. Fills available width on mobile, fixed width desktop.
      */}
      <button
        type="button"
        onClick={() => openPalette()}
        className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg border border-border bg-muted/40 px-3 text-muted-foreground transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring lg:w-64 lg:flex-none"
        aria-label={t('commandPalette.openAria')}
        title={`${t('commandPalette.openAria')} (⌘K)`}
      >
        <Search className="h-4 w-4 shrink-0" />
        <span className="flex-1 truncate text-left text-sm">{t('commandPalette.placeholder')}</span>
        <kbd className="hidden h-5 shrink-0 items-center rounded border border-border bg-background px-1.5 font-mono text-[10px] sm:inline-flex">
          ⌘K
        </kbd>
      </button>

      {/* Desktop spacer — pushes quick actions + clock to the right edge */}
      <div className="hidden flex-1 lg:block" />

      {/* Oxios Copilot (⌘J) — one-shot control/Q&A overlay, no session persisted */}
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={() => openCopilot()}
        className="h-8 shrink-0 gap-1 px-2.5 text-muted-foreground"
        aria-label={t('oxiosCopilot.openAria')}
        title={`${t('oxiosCopilot.openAria')} (⌘J)`}
      >
        <Settings2 className="h-3.5 w-3.5" />
        <kbd className="hidden h-5 items-center rounded border border-border bg-muted/50 px-1.5 font-mono text-[10px] sm:inline-flex">
          ⌘J
        </kbd>
      </Button>

      {/* Menu-bar clock — single trigger for Notification Center */}
      <MenuClock />
    </header>
  )
}
