import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createFileRoute } from '@tanstack/react-router'
import { Pencil, Plus, Star, Trash2, Users } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { groupByCategory } from '@/components/chat/persona-picker'
import {
  EditPersonaDialog,
  type PersonaItem,
  type PersonaPatch,
} from '@/components/persona/edit-persona-dialog'
import { EmptyState } from '@/components/shared/empty-state'
import { ErrorState } from '@/components/shared/error-state'
import { LoadingCards } from '@/components/shared/loading'
import { PageHeader } from '@/components/shared/page-header'
import { RefreshButton } from '@/components/shared/refresh-button'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
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
import { api } from '@/lib/api-client'

export const Route = createFileRoute('/personas')({ component: PersonasPage })

function PersonasPage() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [showCreate, setShowCreate] = useState(false)
  const [editing, setEditing] = useState<PersonaItem | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<PersonaItem | null>(null)
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [systemPrompt, setSystemPrompt] = useState('')
  const [category, setCategory] = useState('general')
  const [genre, setGenre] = useState('')

  const {
    data: personas,
    isLoading,
    isError,
    refetch,
    isFetching,
  } = useQuery({
    queryKey: ['personas'],
    queryFn: async () => {
      const res = await api.get<PersonaItem[]>('/api/personas')
      // Backend returns raw array
      return Array.isArray(res) ? res : []
    },
    refetchInterval: 30000,
  })

  const createMutation = useMutation({
    mutationFn: (p: {
      name: string
      description: string
      system_prompt: string
      category: string
      genre: string | null
    }) => api.post('/api/personas', p),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['personas'] })
      setShowCreate(false)
      setName('')
      setDescription('')
      setSystemPrompt('')
      setCategory('general')
      setGenre('')
    },
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/api/personas/${id}`),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['personas'] }),
  })

  const activateMutation = useMutation({
    // RFC-039: PUT /api/personas/active {id} (was POST /:id/activate — 404)
    mutationFn: (id: string) => api.put('/api/personas/active', { id }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['personas'] }),
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, patch }: { id: string; patch: PersonaPatch }) =>
      api.put(`/api/personas/${id}`, patch),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['personas'] })
      setEditing(null)
      toast.success(t('personas.saved'))
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : t('personas.saveFailed'))
    },
  })

  if (isLoading) return <LoadingCards count={4} />
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const items = Array.isArray(personas) ? personas : []

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('personas.title')}
        subtitle={`${t('personas.subtitle')} · ${t('personas.singleActiveHint')}`}
        actions={
          <div className="flex gap-2">
            <RefreshButton onClick={() => refetch()} isFetching={isFetching} />
            <Button size="sm" onClick={() => setShowCreate(true)}>
              <Plus className="h-4 w-4" /> {t('common.create')}
            </Button>
          </div>
        }
      />

      <Dialog
        open={showCreate}
        onOpenChange={(open) => {
          setShowCreate(open)
          if (!open) {
            setName('')
            setDescription('')
            setSystemPrompt('')
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('personas.createPersona')}</DialogTitle>
            <DialogDescription>{t('personas.createPersonaDescription')}</DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div className="space-y-1">
              <Label htmlFor="persona-name">{t('personas.nameLabel')}</Label>
              <Input
                id="persona-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t('personas.personaNamePlaceholder')}
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="persona-description">{t('common.description')}</Label>
              <Textarea
                id="persona-description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder={t('common.description')}
                rows={2}
              />
            </div>
            <div className="space-y-1">
              <Label htmlFor="persona-prompt">{t('personas.systemPromptLabel')}</Label>
              <Textarea
                id="persona-prompt"
                value={systemPrompt}
                onChange={(e) => setSystemPrompt(e.target.value)}
                placeholder={t('personas.systemPromptPlaceholder')}
                rows={4}
              />
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1">
                <Label>{t('personas.category')}</Label>
                <Select
                  value={category}
                  onValueChange={setCategory}
                  options={['normal', 'coding', 'writing', 'research', 'operations', 'general'].map(
                    (c) => ({ label: t(`chat.persona.categories.${c}`), value: c }),
                  )}
                />
              </div>
              <div className="space-y-1">
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
          </div>
          <DialogFooter>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setShowCreate(false)}
              disabled={createMutation.isPending}
            >
              {t('common.cancel')}
            </Button>
            <Button
              size="sm"
              onClick={() =>
                createMutation.mutate({
                  name,
                  description,
                  system_prompt: systemPrompt,
                  category,
                  genre: category === 'writing' && genre ? genre : null,
                })
              }
            >
              {t('common.create')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {items.length === 0 && !showCreate ? (
        <EmptyState
          icon={<Users className="h-10 w-10" />}
          title={t('personas.noPersonas')}
          description={t('personas.descriptionHint')}
        />
      ) : (
        <div className="space-y-6">
          {groupByCategory(items).map((group) => (
            <section key={group.category} className="space-y-3">
              <h2 className="text-sm font-semibold text-muted-foreground">
                {group.labelKey ? t(group.labelKey) : t('chat.persona.categories.other')}
                <span className="ml-1.5 text-xs font-normal">{group.items.length.toString()}</span>
              </h2>
              <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
                {group.items.map((persona) => (
                  <Card key={persona.id}>
                    <CardHeader className="flex flex-row items-start justify-between pb-2">
                      <div>
                        <CardTitle className="text-base flex items-center gap-2">
                          <Users className="h-4 w-4" /> {persona.name}
                          {persona.enabled && (
                            <Star className="h-3 w-3 text-warning fill-warning" />
                          )}
                        </CardTitle>
                        {persona.description && (
                          <p className="text-xs text-muted-foreground mt-1">
                            {persona.description}
                          </p>
                        )}
                      </div>
                      <div className="flex gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setEditing(persona)}
                          aria-label={t('common.edit')}
                        >
                          <Pencil className="h-4 w-4" />
                        </Button>
                        {!persona.enabled && (
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() => activateMutation.mutate(persona.id)}
                            aria-label={t('personas.activatePersona')}
                          >
                            <Star className="h-4 w-4" />
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setDeleteTarget(persona)}
                          aria-label={t('personas.deletePersona')}
                        >
                          <Trash2 className="h-4 w-4 text-destructive" />
                        </Button>
                      </div>
                    </CardHeader>
                    <CardContent>
                      <p className="text-xs text-muted-foreground">
                        {t('personas.role')}: {persona.role}
                      </p>
                      {persona.system_prompt && (
                        <p className="text-xs text-muted-foreground line-clamp-2 mt-1">
                          {persona.system_prompt}
                        </p>
                      )}
                      {(persona.genre || (persona.capabilities?.length ?? 0) > 0) && (
                        <div className="mt-2 flex flex-wrap gap-1">
                          {persona.genre && (
                            <span className="rounded bg-primary/10 px-1.5 py-0.5 text-2xs font-medium text-primary">
                              {t(`chat.persona.genres.${persona.genre}`, {
                                defaultValue: persona.genre ?? '',
                              })}
                            </span>
                          )}
                          {(persona.capabilities ?? []).map((cap) => (
                            <span
                              key={cap}
                              className="rounded bg-info-muted px-1.5 py-0.5 text-2xs font-medium text-info"
                            >
                              {cap}
                            </span>
                          ))}
                        </div>
                      )}
                    </CardContent>
                  </Card>
                ))}
              </div>
            </section>
          ))}
        </div>
      )}

      <EditPersonaDialog
        persona={editing}
        isPending={updateMutation.isPending}
        onOpenChange={(open) => !open && setEditing(null)}
        onSave={(patch) => {
          if (!editing) return
          updateMutation.mutate({ id: editing.id, patch })
        }}
      />

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('personas.deletePersonaConfirmTitle')}</DialogTitle>
            <DialogDescription>{t('personas.deletePersonaConfirmDescription')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setDeleteTarget(null)}
              disabled={deleteMutation.isPending}
            >
              {t('common.cancel')}
            </Button>
            <Button
              size="sm"
              variant="destructive"
              onClick={() => {
                if (!deleteTarget) return
                deleteMutation.mutate(deleteTarget.id, {
                  onSettled: () => setDeleteTarget(null),
                })
              }}
              disabled={deleteMutation.isPending}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
