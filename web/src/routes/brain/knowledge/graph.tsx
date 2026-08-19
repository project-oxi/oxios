import { createFileRoute, Link } from '@tanstack/react-router'
import { ArrowLeft, Network } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { LinkGraph } from '@/components/knowledge/link-graph'

export const Route = createFileRoute('/brain/knowledge/graph')({
  component: function GraphPage() {
    const { t } = useTranslation()
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-3 px-5 py-3.5 border-b shrink-0">
          <Link
            to="/brain/knowledge"
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeft className="h-4 w-4" />
            <span>{t('knowledge.title')}</span>
          </Link>
          <span className="text-muted-foreground">/</span>
          <h1 className="text-lg font-semibold flex items-center gap-2">
            <Network className="h-5 w-5" />
            {t('knowledge.linkGraphTitle')}
          </h1>
        </div>
        <div className="flex-1 overflow-auto p-6 flex items-start justify-center">
          <LinkGraph className="w-full max-w-2xl" />
        </div>
      </div>
    )
  },
})
