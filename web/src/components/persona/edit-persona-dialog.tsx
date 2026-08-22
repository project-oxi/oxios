import { useQuery } from '@tanstack/react-query'
import { Pencil } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select } from '@/components/ui/select'
import { Textarea } from '@/components/ui/textarea'
import { useMounts } from '@/hooks/use-mounts'
import { api } from '@/lib/api-client'

/** Capability tags currently backed by a UI consumer (RFC-044 §8.4).
 * Surfaced as toggle chips in the editor. */
const KNOWN_CAPABILITIES = ['terminal', 'diff-viewer', 'worktree-fanout'] as const

export interface PersonaItem {
  id: string
  name: string
  role: string
  description: string
  enabled: boolean
  personality_traits?: string[]
  /// RFC-044 Phase 3: capability tags surfaced by the list endpoint.
  capabilities?: string[]
  category?: string
  genre?: string | null
  system_prompt?: string
  default_mount_ids?: string[]
}

export interface PersonaPatch {
  name: string
  description: string
  system_prompt: string
  capabilities?: string[]
  category?: string
  genre?: string | null
  default_mount_ids?: string[]
}

interface EditPersonaDialogProps {
  persona: PersonaItem | null
  isPending: boolean
  onOpenChange: (open: boolean) => void
  onSave: (patch: PersonaPatch) => void
}

interface PersonaDetail {
  id: string
  name: string
  role: string
  description: string
  system_prompt: string
  enabled: boolean
  personality_traits: string[]
  capabilities?: string[]
  category?: string
  genre?: string | null
  default_mount_ids?: string[]
}

/**
 * Persona 편집 다이얼로그. 백엔드 PUT /api/personas/:id 로 부분 업데이트.
 *
 * 리스트 응답에는 system_prompt 가 없으므로 (wipe 방지) 열릴 때
 * GET /api/personas/:id 로 전체를 가져와 system_prompt 까지 prefill 합니다.
 * 사용자가 system_prompt 를 수정하지 않은 경우에도 보내지만, 백엔드는
 * Some("") 와 Some(prev) 를 구분하지 못하므로 — 따라서 사용자가 textarea
 * 를 건드리지 않으면 원본을 그대로 보냅니다.
 */
export function EditPersonaDialog({
  persona,
  isPending,
  onOpenChange,
  onSave,
}: EditPersonaDialogProps) {
  const { t } = useTranslation()
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [systemPrompt, setSystemPrompt] = useState('')
  const [capabilities, setCapabilities] = useState<string[]>([])
  const { data: mountsData } = useMounts()
  const [category, setCategory] = useState('general')
  const [genre, setGenre] = useState<string>('')
  const [defaultMountIds, setDefaultMountIds] = useState<string[]>([])

  const detail = useQuery({
    queryKey: ['persona', persona?.id],
    queryFn: () => api.get<PersonaDetail>(`/api/personas/${persona!.id}`),
    enabled: persona !== null,
  })

  // 대상 페르소나가 바뀌거나 상세가 로딩되면 로컬 필드 동기화.
  useEffect(() => {
    if (!persona) return
    if (detail.data) {
      setName(detail.data.name)
      setDescription(detail.data.description)
      setSystemPrompt(detail.data.system_prompt)
      setCapabilities(detail.data.capabilities ?? [])
      setCategory(detail.data.category ?? 'general')
      setGenre(detail.data.genre ?? '')
      setDefaultMountIds(detail.data.default_mount_ids ?? [])
    } else {
      setName(persona.name)
      setDescription(persona.description)
    }
  }, [persona, detail.data])

  const close = () => onOpenChange(false)

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!persona) return
    const n = name.trim()
    if (!n) return
    onSave({
      name: n,
      description: description.trim(),
      system_prompt: systemPrompt,
      capabilities,
      category,
      genre: category === 'writing' && genre ? genre : null,
      default_mount_ids: defaultMountIds,
    })
  }

  return (
    <Dialog open={persona !== null} onOpenChange={(o) => !o && close()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Pencil className="h-5 w-5" />
            {t('personas.edit')}
          </DialogTitle>
          <DialogDescription>{t('personas.editDescription')}</DialogDescription>
        </DialogHeader>
        {detail.isLoading ? (
          <div className="text-sm text-muted-foreground p-4 text-center">{t('common.loading')}</div>
        ) : detail.isError ? (
          <div className="space-y-3 p-2">
            <p className="text-sm text-destructive">{t('personas.loadFailed')}</p>
            <Button
              type="button"
              variant="outline"
              onClick={() => detail.refetch()}
              disabled={detail.isFetching}
            >
              {t('common.retry')}
            </Button>
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="persona-edit-name">{t('personas.personaName')}</Label>
              <Input
                id="persona-edit-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                autoFocus
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="persona-edit-desc">{t('common.description')}</Label>
              <Input
                id="persona-edit-desc"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="persona-edit-prompt">{t('personas.systemPrompt')}</Label>
              <Textarea
                id="persona-edit-prompt"
                value={systemPrompt}
                onChange={(e) => setSystemPrompt(e.target.value)}
                rows={6}
              />
            </div>
            <div className="space-y-2">
              <Label>{t('personas.capabilities')}</Label>
              <div className="flex flex-wrap gap-1.5">
                {KNOWN_CAPABILITIES.map((cap) => {
                  const active = capabilities.includes(cap)
                  return (
                    <Button
                      key={cap}
                      type="button"
                      variant={active ? 'default' : 'outline'}
                      size="sm"
                      className="h-7 gap-1 px-2 text-2xs font-normal"
                      aria-pressed={active}
                      onClick={() =>
                        setCapabilities((prev) =>
                          active ? prev.filter((c) => c !== cap) : [...prev, cap],
                        )
                      }
                    >
                      {cap}
                    </Button>
                  )
                })}
              </div>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-2">
                <Label>{t('personas.category')}</Label>
                <Select
                  value={category}
                  onValueChange={setCategory}
                  options={['normal', 'coding', 'writing', 'research', 'operations', 'general'].map(
                    (c) => ({ label: t(`chat.persona.categories.${c}`), value: c }),
                  )}
                />
              </div>
              <div className="space-y-2">
                <Label>{t('personas.genre')}</Label>
                <Select
                  value={genre || 'none'}
                  onValueChange={(v) => setGenre(v === 'none' ? '' : v)}
                  disabled={category !== 'writing'}
                  options={[
                    { label: t('personas.genreNone'), value: 'none' },
                    ...['novel', 'scenario', 'essay', 'blog'].map((g) => ({
                      label: t(`chat.persona.genres.${g}`),
                      value: g,
                    })),
                  ]}
                />
              </div>
            </div>
            <div className="space-y-2">
              <Label>{t('personas.defaultMounts')}</Label>
              <p className="text-xs text-muted-foreground">{t('personas.defaultMountsHint')}</p>
              <div className="flex flex-wrap gap-1.5">
                {(mountsData?.items ?? []).map((m) => {
                  const active = defaultMountIds.includes(m.id)
                  return (
                    <Button
                      key={m.id}
                      type="button"
                      variant={active ? 'default' : 'outline'}
                      size="sm"
                      className="h-7 gap-1 px-2 text-2xs font-normal"
                      aria-pressed={active}
                      onClick={() =>
                        setDefaultMountIds((prev) =>
                          active ? prev.filter((id) => id !== m.id) : [...prev, m.id],
                        )
                      }
                    >
                      {m.name}
                    </Button>
                  )
                })}
                {(mountsData?.items ?? []).length === 0 && (
                  <span className="text-xs text-muted-foreground">{t('personas.genreNone')}</span>
                )}
              </div>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={close}>
                {t('common.cancel')}
              </Button>
              <Button type="submit" disabled={!name.trim() || isPending}>
                {isPending ? t('common.saving') : t('common.save')}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}
