import { useNavigate } from '@tanstack/react-router'
import { Check, ChevronsUpDown, Key, Loader2, Route, Settings } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Separator } from '@/components/ui/separator'
import { useEngineConfig, useModels, useProviders, useSetModel } from '@/hooks/use-engine'
import type { ModelInfo, ProviderInfo } from '@/types/engine'

/**
 * Interactive providers section for SystemHealthCard.
 *
 * - Shows configured providers as badges.
 * - Click a provider → expands its model list.
 * - Click a model → switches immediately via PUT /api/engine/model.
 * - Shows routing configuration below.
 */
export function ProvidersSection() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { data: engineConfig } = useEngineConfig()
  const { data: providers } = useProviders()
  const setModel = useSetModel()

  const currentModel = engineConfig?.default_model ?? ''
  const routing = engineConfig?.routing
  const routingEnabled = routing?.routingEnabled ?? false
  const currentProviderId = currentModel.includes('/') ? currentModel.split('/')[0] : null
  const currentModelId = currentModel.includes('/')
    ? currentModel.split('/').slice(1).join('/')
    : currentModel

  const configuredProviders = (providers ?? []).filter((p) => p.hasKey)

  const [expandedProvider, setExpandedProvider] = useState<string | null>(null)

  return (
    <div className="space-y-2">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
          {t('dashboard.providers')}
        </h3>
        <button
          type="button"
          onClick={() =>
            navigate({ to: '/operate/system/settings', search: { section: 'engine' } })
          }
          className="text-muted-foreground hover:text-foreground transition-colors"
          title={t('dashboard.engineSettings')}
        >
          <Settings className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Current model */}
      <div className="flex items-center gap-2 text-xs">
        <span className="text-muted-foreground">{t('dashboard.currentModel')}:</span>
        <span className="font-semibold truncate">
          {currentModelId || t('dashboard.modelNotSet')}
        </span>
        {currentProviderId && (
          <span className="inline-flex items-center rounded-full bg-primary/10 px-1.5 py-0.5 text-2xs font-medium text-primary whitespace-nowrap">
            {providers?.find((p) => p.id === currentProviderId)?.name ?? currentProviderId}
          </span>
        )}
      </div>

      {/* Configured providers */}
      {configuredProviders.length > 0 && (
        <div className="space-y-1.5">
          <div className="flex flex-wrap gap-1.5">
            {configuredProviders.map((p) => (
              <ProviderBadge
                key={p.id}
                provider={p}
                isCurrent={p.id === currentProviderId}
                expanded={expandedProvider === p.id}
                switching={setModel.isPending}
                onToggle={() => setExpandedProvider((prev) => (prev === p.id ? null : p.id))}
              />
            ))}
          </div>

          {/* Model picker for expanded provider */}
          {expandedProvider && (
            <ModelPicker
              providerId={expandedProvider}
              currentModel={currentModel}
              onSelect={(fullModelId) => setModel.mutate(fullModelId)}
              isPending={setModel.isPending}
            />
          )}
        </div>
      )}

      {/* Routing status */}
      {routing && (
        <>
          <Separator />
          <div className="space-y-1">
            <div className="flex items-center gap-2 text-xs">
              <Route
                className={`h-3.5 w-3.5 ${routingEnabled ? 'text-success' : 'text-muted-foreground'}`}
              />
              <span className="text-muted-foreground">{t('dashboard.routing')}:</span>
              <span
                className={routingEnabled ? 'text-success font-medium' : 'text-muted-foreground'}
              >
                {routingEnabled ? t('dashboard.routingEnabled') : t('dashboard.routingDisabled')}
              </span>
            </div>
            {routingEnabled && (
              <div className="pl-5 space-y-0.5">
                {routing.preferCostEfficient && (
                  <p className="text-2xs text-muted-foreground">{t('dashboard.costOptimized')}</p>
                )}
                {routing.fallbackModels.length > 0 && (
                  <p className="text-2xs text-muted-foreground">
                    {t('dashboard.fallbackModels', { count: routing.fallbackModels.length })}
                    {': '}
                    <span className="font-mono">
                      {routing.fallbackModels
                        .slice(0, 3)
                        .map((m) => m.split('/').pop())
                        .join(', ')}
                      {routing.fallbackModels.length > 3 &&
                        ` +${routing.fallbackModels.length - 3}`}
                    </span>
                  </p>
                )}
                {routing.excludedModels.length > 0 && (
                  <p className="text-2xs text-muted-foreground">
                    {t('dashboard.excludedModels', { count: routing.excludedModels.length })}
                  </p>
                )}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  )
}

// ─── Provider Badge ───────────────────────────────────────────

function ProviderBadge({
  provider,
  isCurrent,
  expanded,
  switching,
  onToggle,
}: {
  provider: ProviderInfo
  isCurrent: boolean
  expanded: boolean
  switching: boolean
  onToggle: () => void
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      disabled={switching}
      className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-2xs font-medium whitespace-nowrap transition-colors ${
        expanded
          ? 'bg-primary/15 text-primary ring-1 ring-primary/30'
          : isCurrent
            ? 'bg-primary/10 text-primary'
            : 'bg-muted text-muted-foreground hover:bg-accent'
      }`}
      title={`${provider.name}${provider.hasKey ? ' (API key configured)' : ''}`}
    >
      <Key
        className={`h-2.5 w-2.5 ${isCurrent || expanded ? 'text-primary' : 'text-muted-foreground/60'}`}
      />
      {provider.name}
      {expanded ? (
        <ChevronsUpDown className="h-2.5 w-2.5 ml-0.5" />
      ) : provider.modelCount > 0 ? (
        <span className="text-muted-foreground/60">({provider.modelCount})</span>
      ) : null}
    </button>
  )
}

// ─── Model Picker ─────────────────────────────────────────────

function ModelPicker({
  providerId,
  currentModel,
  onSelect,
  isPending,
}: {
  providerId: string
  currentModel: string
  onSelect: (fullModelId: string) => void
  isPending: boolean
}) {
  const { t } = useTranslation()
  const { data: models, isLoading } = useModels(providerId)

  if (isLoading) {
    return (
      <div className="flex items-center gap-1.5 px-2 py-1.5 text-2xs text-muted-foreground">
        <Loader2 className="h-3 w-3 animate-spin" />
        {t('common.loading')}
      </div>
    )
  }

  const list = Array.isArray(models) ? models : []
  if (list.length === 0) {
    return <p className="px-2 text-2xs text-muted-foreground">{t('dashboard.noModels')}</p>
  }

  return (
    <div className="max-h-[160px] overflow-y-auto rounded-md border bg-background p-1 space-y-0.5">
      {list.map((m: ModelInfo) => {
        const fullId = `${providerId}/${m.id}`
        const isActive = fullId === currentModel
        return (
          <button
            key={m.id}
            type="button"
            disabled={isPending}
            onClick={() => onSelect(fullId)}
            className={`flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs transition-colors ${
              isActive
                ? 'bg-primary/10 text-primary font-medium'
                : 'hover:bg-accent text-foreground'
            } ${isPending ? 'opacity-50' : ''}`}
          >
            <span className={`shrink-0 ${isActive ? 'text-primary' : 'text-transparent'}`}>
              <Check className="h-3 w-3" />
            </span>
            <span className="flex-1 truncate">{m.name}</span>
            {m.reasoning && (
              <span className="shrink-0 rounded-full bg-info/10 px-1.5 py-0 text-2xs text-info font-medium">
                reasoning
              </span>
            )}
            {(m.costInput > 0 || m.costOutput > 0) && (
              <span className="shrink-0 text-2xs text-muted-foreground tabular-nums">
                ${m.costInput}/${m.costOutput}
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}
