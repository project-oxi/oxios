import { useRouter } from '@tanstack/react-router'
import { FilePlus, FolderKanban, FolderPlus, Theater, Timer, Zap } from 'lucide-react'
import type { ReactNode } from 'react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import type { CommandProvider, PaletteItem, QueryContext } from './types'

interface NewTarget {
  key: string
  /** i18n key for the "New X" title. */
  titleKey: string
  href: string
  icon: ReactNode
}

/**
 * Creation targets — each routes to the page that owns its creation UI
 * (SkillEditorDialog, persona/project/cron create forms, knowledge "New File").
 * v1 routes to the page (resolved OQ); directly opening the create dialog from
 * the palette is a later enhancement requiring cross-component state.
 */
const TARGETS: NewTarget[] = [
  {
    key: 'skill',
    titleKey: 'commandPalette.newSkill',
    href: '/skills',
    icon: <Zap className="h-4 w-4" />,
  },
  {
    key: 'persona',
    titleKey: 'commandPalette.newPersona',
    href: '/personas',
    icon: <Theater className="h-4 w-4" />,
  },
  {
    key: 'project',
    titleKey: 'commandPalette.newProject',
    href: '/projects',
    icon: <FolderKanban className="h-4 w-4" />,
  },
  {
    key: 'cron',
    titleKey: 'commandPalette.newCron',
    href: '/cron-jobs',
    icon: <Timer className="h-4 w-4" />,
  },
  {
    key: 'note',
    titleKey: 'commandPalette.newNote',
    href: '/brain/knowledge',
    icon: <FilePlus className="h-4 w-4" />,
  },
  {
    key: 'mount',
    titleKey: 'commandPalette.newMount',
    href: '/mounts',
    icon: <FolderPlus className="h-4 w-4" />,
  },
]

/**
 * New provider — verb `new` (prefix `+`).
 *
 * Routes to the creation surface for a target. The target key comes from an
 * explicit `@type` (or `@name`), or from the bare text after `+`
 * (e.g. `+ skill`). With no target, shows the full picker.
 */
export function useNewProvider(): CommandProvider {
  const { t } = useTranslation()
  const router = useRouter()

  return useMemo(
    () => ({
      id: 'new',
      verbs: ['new'],
      resolve(ctx: QueryContext): PaletteItem[] {
        if (ctx.verb !== 'new') return []

        const targetKey = (ctx.entity?.type || ctx.entity?.name || ctx.text).toLowerCase().trim()
        const target = TARGETS.find((x) => x.key === targetKey)

        if (target) {
          return [
            {
              id: `new-${target.key}`,
              verb: 'new',
              icon: target.icon,
              title: t(target.titleKey),
              score: 100,
              onSelect: () => router.history.push(target.href),
            },
          ]
        }

        // No/unknown target → picker.
        return TARGETS.map(
          (tg): PaletteItem => ({
            id: `new-${tg.key}`,
            verb: 'new',
            icon: tg.icon,
            title: t(tg.titleKey),
            hint: <kbd className="text-[10px] text-muted-foreground">+ {tg.key}</kbd>,
            compose: `+ ${tg.key} `,
            score: 0,
            onSelect: () => router.history.push(tg.href),
          }),
        )
      },
    }),
    [t, router],
  )
}
