import { Link } from '@tanstack/react-router'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { SystemHealthCard } from '@/components/dashboard/system-health-card'
import { cn } from '@/lib/utils'
import type { SystemStatus } from '@/types'

interface SystemLink {
  to: string
  labelKey: string
  /** undefined = no live status for this surface; boolean = render a health dot. */
  dot?: boolean
}

/**
 * System links (spec §6, tail): MCP, Brain, Skills, Budget, Resources and
 * Security stay reachable as compact status pills instead of four
 * equal-weight dashboard cards. The System pill expands an overflow panel
 * with the full component-health card and the version strings that moved
 * out of the header headline.
 */
export function SystemLinks({ status }: { status?: SystemStatus }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  const links: SystemLink[] = [
    { to: '/operate/system/mcp', labelKey: 'commandCenter.system.links.mcp' },
    {
      to: '/brain',
      labelKey: 'commandCenter.system.links.brain',
      dot: status?.components?.brain?.healthy,
    },
    { to: '/operate/system/skills', labelKey: 'commandCenter.system.links.skills' },
    { to: '/operate/system/resources', labelKey: 'commandCenter.system.links.budget' },
    { to: '/operate/system/resources', labelKey: 'commandCenter.system.links.resources' },
    { to: '/operate/system/security', labelKey: 'commandCenter.system.links.security' },
  ]

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="mr-1 text-2xs uppercase tracking-wide text-muted-foreground">
          {t('commandCenter.system.title')}
        </span>
        {links.map((link) => (
          <Link
            key={link.to}
            to={link.to}
            className="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            {typeof link.dot === 'boolean' && (
              <span
                aria-hidden="true"
                className={cn(
                  'h-1.5 w-1.5 rounded-full',
                  link.dot ? 'bg-status-success' : 'bg-status-error',
                )}
              />
            )}
            {t(link.labelKey)}
          </Link>
        ))}
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          className="inline-flex items-center gap-1 rounded-full border px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          {open ? (
            <ChevronUp className="h-3 w-3" aria-hidden="true" />
          ) : (
            <ChevronDown className="h-3 w-3" aria-hidden="true" />
          )}
          {open ? t('commandCenter.system.hideDetails') : t('commandCenter.system.showDetails')}
        </button>
      </div>

      {open && (
        <div className="space-y-3 rounded-lg border p-3">
          {status && <SystemHealthCard status={status} />}
          {status?.version && (
            <p className="font-mono text-2xs text-muted-foreground">
              {t('commandCenter.system.versions', {
                binary: status.version,
                web: status.web_version ?? '—',
              })}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
