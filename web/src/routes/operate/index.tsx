import { createFileRoute } from '@tanstack/react-router'
import { LayoutDashboard } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { EmptyState } from '@/components/shared/empty-state'

/**
 * Operate landing route — the Attention shell. The source-backed Attention
 * composition is owned by the wave-2 Operate worker; this shell-safe empty
 * state only establishes the route so the surface switcher, bottom bar, and
 * palette can link it.
 */
function OperateIndex() {
  const { t } = useTranslation()
  return (
    <EmptyState
      icon={<LayoutDashboard className="h-10 w-10" />}
      title={t('sidebar.operate')}
      description={t('common.surfacePlaceholderBody')}
    />
  )
}

export const Route = createFileRoute('/operate/')({
  component: OperateIndex,
})
