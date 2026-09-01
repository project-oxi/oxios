import { Link } from '@tanstack/react-router'
import { Languages, Monitor, Moon, Settings, Sun } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import { useThemeStore } from '@/stores/theme'

/**
 * Sidebar footer — global preferences (theme, language, settings).
 *
 * Replaces the old header-based controls. Moves language selection and
 * settings out of the header so the Notification Center panel can focus
 * purely on schedule + notifications, like macOS.
 *
 * Collapsed state: icon-only vertical stack. Expanded: icon + label rows.
 */
export function SidebarFooter({ collapsed }: { collapsed: boolean }) {
  const { i18n, t } = useTranslation()
  const { theme, resolved, setTheme } = useThemeStore()

  const cycleTheme = () => {
    const next = theme === 'dark' ? 'light' : theme === 'light' ? 'system' : 'dark'
    setTheme(next)
  }

  const cycleLang = () => {
    // P3: startsWith handles BCP 47 tags like `ko-KR`.
    const next = i18n.resolvedLanguage?.startsWith('ko') ? 'en' : 'ko'
    i18n.changeLanguage(next)
    // No manual localStorage write — LanguageDetector with `caches: ['localStorage']`
    // already persists the new language under the `i18nextLng` key (see i18n/index.ts).
  }

  const themeIcon =
    theme === 'system' ? (
      <Monitor className="h-4 w-4" />
    ) : resolved === 'dark' ? (
      <Sun className="h-4 w-4" />
    ) : (
      <Moon className="h-4 w-4" />
    )

  const themeLabel =
    theme === 'dark' ? t('common.dark') : theme === 'light' ? t('common.light') : t('common.system')

  const langLabel = i18n.language === 'ko' ? '한국어' : 'English'

  if (collapsed) {
    return (
      <div className="flex flex-col items-center gap-1 py-2">
        <button
          type="button"
          onClick={cycleTheme}
          className="rounded-lg p-2 text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          aria-label={`${t('common.toggleTheme')}: ${themeLabel}`}
          title={themeLabel}
        >
          {themeIcon}
        </button>
        <button
          type="button"
          onClick={cycleLang}
          className="rounded-lg p-2 text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          aria-label={`${t('common.language')}: ${langLabel}`}
          title={langLabel}
        >
          <Languages className="h-4 w-4" />
        </button>
        <Link
          to="/operate/system/settings"
          search={{ section: undefined }}
          className="rounded-lg p-2 text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent hover:text-sidebar-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          aria-label={t('common.settings')}
          title={t('common.settings')}
        >
          <Settings className="h-4 w-4" />
        </Link>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-0.5 p-2">
      <button
        type="button"
        onClick={cycleTheme}
        className={cn(
          'flex items-center gap-3 rounded-lg px-2.5 py-2 text-sm text-left select-none transition-all',
          'text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-foreground',
          'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
        )}
      >
        {themeIcon}
        <span>{themeLabel}</span>
      </button>
      <button
        type="button"
        onClick={cycleLang}
        className={cn(
          'flex items-center gap-3 rounded-lg px-2.5 py-2 text-sm text-left select-none transition-all',
          'text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-foreground',
          'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
        )}
      >
        <Languages className="h-4 w-4" />
        <span>{langLabel}</span>
      </button>
      <Link
        to="/operate/system/settings"
        search={{ section: undefined }}
        className={cn(
          'flex items-center gap-3 rounded-lg px-2.5 py-2 text-sm text-left select-none transition-all',
          'text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-foreground',
          'focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
        )}
      >
        <Settings className="h-4 w-4" />
        <span>{t('common.settings')}</span>
      </Link>
    </div>
  )
}
