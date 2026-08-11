import {
  AlertCircle,
  Check,
  Eye,
  EyeOff,
  KeyRound,
  Loader2,
  Lock,
  Plus,
  ShieldCheck,
  Trash2,
  X,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { api } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import type { ProviderInfo } from '@/types/engine'
import { ProviderSelect } from './provider-select'

// ─── Category indicator dots ─────────────────────────────────

const CATEGORY_DOT: Record<string, string> = {
  major: 'bg-status-info',
  open: 'bg-status-success',
  regional: 'bg-status-warning',
  local: 'bg-hue-purple',
}

// ─── ProviderCard ────────────────────────────────────────────

interface ProviderCardProps {
  provider: ProviderInfo
  onChangeKey: (apiKey: string) => void
  onRemove: () => void
  isPending?: boolean
}

export function ProviderCard({ provider, onChangeKey, onRemove, isPending }: ProviderCardProps) {
  const { t } = useTranslation()
  const [showKeyInput, setShowKeyInput] = useState(false)
  const [keyValue, setKeyValue] = useState('')
  const [keyVisible, setKeyVisible] = useState(false)
  const [validateState, setValidateState] = useState<'idle' | 'validating' | 'valid' | 'invalid'>(
    'idle',
  )
  const [validateMsg, setValidateMsg] = useState('')

  const isEnvKey = provider.keySource === 'env'

  const handleKeySubmit = () => {
    if (keyValue.trim()) {
      onChangeKey(keyValue.trim())
      setKeyValue('')
      setShowKeyInput(false)
    }
  }

  const handleValidate = async () => {
    setValidateState('validating')
    setValidateMsg('')
    try {
      const res = await api.post<{ valid: boolean; message?: string }>('/api/engine/validate-key', {
        provider: provider.id,
      })
      setValidateState(res.valid ? 'valid' : 'invalid')
      setValidateMsg(res.message ?? '')
    } catch {
      setValidateState('invalid')
      setValidateMsg(t('common.error'))
    }
  }

  return (
    <div className="flex flex-col rounded-lg border bg-card p-4 transition-all hover:border-primary/30 hover:shadow-sm">
      {/* Header */}
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span
            className={cn(
              'h-2.5 w-2.5 rounded-full shrink-0',
              CATEGORY_DOT[provider.category] ?? 'bg-muted-foreground',
            )}
          />
          <span className="font-medium text-sm truncate">{provider.name}</span>
        </div>
      </div>

      {/* Description */}
      {provider.description && (
        <p className="text-xs text-muted-foreground mt-1.5 line-clamp-2">{provider.description}</p>
      )}

      {/* Status */}
      <div className="flex items-center gap-1.5 mt-2 text-xs">
        <Check
          className={cn(
            'h-3.5 w-3.5 shrink-0',
            isEnvKey ? 'text-status-warning' : 'text-status-success',
          )}
        />
        <span className="text-muted-foreground">
          {isEnvKey ? t('engine.envKey') : t('engine.connected')}
          {' · '}
          {provider.modelCount} {t('engine.models')}
        </span>
      </div>

      {/* Validation result */}
      {validateState !== 'idle' && (
        <div
          className={cn(
            'flex items-center gap-1.5 mt-1.5 rounded px-1.5 py-1 text-xs',
            validateState === 'validating' && 'bg-muted/50 text-muted-foreground',
            validateState === 'valid' && 'bg-status-success-subtle text-status-success-on-subtle',
            validateState === 'invalid' && 'bg-status-error-subtle text-status-error-on-subtle',
          )}
        >
          {validateState === 'validating' && <Loader2 className="h-3 w-3 shrink-0 animate-spin" />}
          {validateState === 'valid' && <Check className="h-3 w-3 shrink-0" />}
          {validateState === 'invalid' && <AlertCircle className="h-3 w-3 shrink-0" />}
          <span className="truncate">
            {validateState === 'validating' && t('engine.verifying')}
            {validateState === 'valid' && t('engine.valid')}
            {validateState === 'invalid' && (validateMsg || t('engine.invalid'))}
          </span>
        </div>
      )}

      {/* Key change (inline) */}
      {showKeyInput ? (
        <div className="mt-3 space-y-2">
          <div className="relative">
            <Input
              type={keyVisible ? 'text' : 'password'}
              value={keyValue}
              onChange={(e) => setKeyValue(e.target.value)}
              placeholder={t('engine.enterApiKey')}
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleKeySubmit()
                if (e.key === 'Escape') {
                  setShowKeyInput(false)
                  setKeyValue('')
                }
              }}
              className="h-8 pr-9 text-sm"
              autoFocus
            />
            <button
              type="button"
              onClick={() => setKeyVisible((v) => !v)}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            >
              {keyVisible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
            </button>
          </div>
          <div className="flex gap-2">
            <Button
              size="sm"
              className="h-7"
              onClick={handleKeySubmit}
              disabled={isPending || !keyValue.trim()}
            >
              {t('common.save')}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7"
              onClick={() => {
                setShowKeyInput(false)
                setKeyValue('')
              }}
            >
              {t('common.cancel')}
            </Button>
          </div>
        </div>
      ) : (
        /* Spacer + Actions — pushed to bottom for equal card heights */
        <div className="flex-1" />
      )}

      {/* Actions */}
      {!showKeyInput && (
        <div className="flex items-center gap-1 mt-3 -mr-1">
          <Button
            size="sm"
            variant="ghost"
            className={cn(
              'h-8 gap-1.5 text-xs',
              validateState === 'valid' && 'text-status-success-on-surface',
            )}
            onClick={handleValidate}
            disabled={validateState === 'validating' || isPending}
            title={t('engine.verify')}
          >
            {validateState === 'validating' ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : validateState === 'valid' ? (
              <Check className="h-3.5 w-3.5" />
            ) : (
              <ShieldCheck className="h-3.5 w-3.5" />
            )}
            <span className="hidden sm:inline">
              {validateState === 'validating' ? t('engine.verifying') : t('engine.verify')}
            </span>
          </Button>
          <Button
            size="icon"
            variant="ghost"
            className="h-8 w-8"
            onClick={() => setShowKeyInput(true)}
            disabled={isPending}
            title={t('engine.changeKey')}
          >
            <KeyRound className="h-4 w-4" />
          </Button>
          {isEnvKey ? (
            <span
              className="ml-auto flex items-center gap-1 pr-1 text-xs text-muted-foreground"
              title={t('engine.envProtected')}
            >
              <Lock className="h-3 w-3" />
            </span>
          ) : (
            <Button
              size="icon"
              variant="ghost"
              className="ml-auto h-8 w-8 text-muted-foreground hover:text-destructive"
              onClick={onRemove}
              disabled={isPending}
              title={t('common.delete')}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          )}
        </div>
      )}
    </div>
  )
}

// ─── AddProviderCard ─────────────────────────────────────────

interface AddProviderCardProps {
  availableProviders: ProviderInfo[]
  onAdd: (provider: string, apiKey: string) => void
  isPending?: boolean
}

export function AddProviderCard({ availableProviders, onAdd, isPending }: AddProviderCardProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null)
  const [apiKey, setApiKey] = useState('')
  const [keyVisible, setKeyVisible] = useState(false)

  const handleConnect = () => {
    if (selectedProvider && apiKey.trim()) {
      onAdd(selectedProvider, apiKey.trim())
      setApiKey('')
      setSelectedProvider(null)
      setExpanded(false)
      setKeyVisible(false)
    }
  }

  const handleCancel = () => {
    setApiKey('')
    setSelectedProvider(null)
    setExpanded(false)
    setKeyVisible(false)
  }

  if (!expanded) {
    return (
      <button
        type="button"
        onClick={() => setExpanded(true)}
        disabled={availableProviders.length === 0}
        className={cn(
          'flex flex-col items-center justify-center gap-2 rounded-lg border bg-card p-4 min-h-[140px] transition-all',
          availableProviders.length === 0
            ? 'border-dashed border-border/50 opacity-40 cursor-not-allowed'
            : 'border-dashed border-border hover:border-primary/40 hover:bg-primary/5 cursor-pointer',
        )}
      >
        <div className="flex h-9 w-9 items-center justify-center rounded-full border border-dashed border-border">
          <Plus className="h-4 w-4 text-muted-foreground" />
        </div>
        <span className="text-xs text-muted-foreground">
          {availableProviders.length === 0 ? t('engine.allConnected') : t('engine.addProvider')}
        </span>
      </button>
    )
  }

  return (
    <div className="flex flex-col rounded-lg border border-primary/40 bg-primary/5 p-4 ring-1 ring-primary/10">
      <div className="flex items-center justify-between mb-3">
        <span className="text-sm font-medium">{t('engine.addProvider')}</span>
        <Button size="icon" variant="ghost" className="h-7 w-7" onClick={handleCancel}>
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className="space-y-2.5">
        <ProviderSelect
          providers={availableProviders}
          value={selectedProvider}
          onValueChange={setSelectedProvider}
        />
        <div className="relative">
          <Input
            type={keyVisible ? 'text' : 'password'}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={t('engine.enterApiKey')}
            onKeyDown={(e) => {
              if (e.key === 'Enter') handleConnect()
              if (e.key === 'Escape') handleCancel()
            }}
            className="h-8 pr-9 text-sm"
            autoFocus
          />
          <button
            type="button"
            onClick={() => setKeyVisible((v) => !v)}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          >
            {keyVisible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
          </button>
        </div>
        <Button
          className="w-full"
          size="sm"
          onClick={handleConnect}
          disabled={isPending || !selectedProvider || !apiKey.trim()}
        >
          {t('engine.connect')}
        </Button>
      </div>
    </div>
  )
}
