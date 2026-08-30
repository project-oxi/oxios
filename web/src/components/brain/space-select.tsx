import { useTranslation } from 'react-i18next'
import { Select } from '@/components/ui/select'
import { useBrainSpaces } from '@/hooks/use-brain'
import { useConfig, useSaveConfig } from '@/hooks/use-config'
import type { SpaceSummary } from '@/types/brain'

/**
 * Wire value for a spaces-list row: the space name when present, else the
 * id. All scoped `/api/brain/*` calls take this value as their `space`
 * parameter (the kernel forwards it to the oxibrain ops verbatim).
 */
export function spaceParam(sp: SpaceSummary): string {
  return sp.name || sp.id
}

function useSpaceOptions(): Array<{ label: string; value: string }> {
  const { data } = useBrainSpaces()
  const spaces = Array.isArray(data) ? data : []
  // oxibrain ops address spaces by NAME (ADR-013, `space://{name}`). A
  // nameless space would be picked by its id, forwarded as `?space=` to
  // the kernel, and rejected with no actionable error — exclude it from
  // the picker options, mirroring the chat picker's `s.name === resolved`
  // filter.
  return spaces
    .filter((sp) => sp.name)
    .map((sp) => ({ label: spaceParam(sp), value: spaceParam(sp) }))
}

/**
 * Space picker for the `/brain` routes. Bound to the route's `space` search
 * param — an empty value means "no space selected", which keeps every
 * scoped brain query disabled until a row is picked.
 */
export function SpaceSelect({
  value,
  onValueChange,
  className,
}: {
  value: string
  onValueChange: (space: string) => void
  className?: string
}) {
  const { t } = useTranslation()
  const options = useSpaceOptions()
  return (
    <Select
      value={value}
      onValueChange={onValueChange}
      options={options}
      placeholder={t('brain.selectSpace')}
      aria-label={t('brain.selectSpace')}
      className={className}
    />
  )
}

// Radix select items cannot carry an empty value, so "no default" uses the
// same `'none'` sentinel as the persona genre picker.
const NONE = 'none'

/**
 * Global default-brain selector: config field `brain.default_space`, read
 * from `GET /api/config` and persisted via the deep-merge `PATCH
 * /api/config`. The sentinel option clears the default (empty string on
 * the wire = no default brain).
 */
export function DefaultBrainSelect({ className }: { className?: string }) {
  const { t } = useTranslation()
  const { data: config } = useConfig()
  const saveConfig = useSaveConfig()
  const options = useSpaceOptions()

  const brain = (config?.brain ?? {}) as Record<string, unknown>
  const current = typeof brain.default_space === 'string' ? brain.default_space : ''

  const setDefault = (value: string) =>
    saveConfig.mutate({ brain: { default_space: value === NONE ? '' : value } })

  return (
    <Select
      value={current || NONE}
      onValueChange={setDefault}
      options={[{ label: t('brain.none'), value: NONE }, ...options]}
      aria-label={t('brain.defaultSpace')}
      className={className}
    />
  )
}

/**
 * Project-level default-brain selector (`Project.default_brain_space`).
 * Controlled: `null` = no default; the project dialogs persist the choice
 * with the project (tri-state on the wire: `null` clears, string sets).
 */
export function ProjectBrainSelect({
  value,
  onValueChange,
  className,
}: {
  value: string | null
  onValueChange: (space: string | null) => void
  className?: string
}) {
  const { t } = useTranslation()
  const options = useSpaceOptions()
  return (
    <Select
      value={value ?? NONE}
      onValueChange={(v) => onValueChange(v === NONE ? null : v)}
      options={[{ label: t('brain.none'), value: NONE }, ...options]}
      aria-label={t('projects.defaultBrain')}
      className={className}
    />
  )
}
