import { BookOpen, Clock, Inbox, Newspaper, Plus, ShoppingCart, Tv } from 'lucide-react'
import type { ReactNode } from 'react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { useChatAppend, useChecklistAdd, useJournalAdd } from '@/hooks/use-knowledge'
import { matchScore, surfacePrimaryVerb } from './ranker'
import type { CommandProvider, PaletteItem, QueryContext } from './types'

/** A capture destination (checklist file or journal). */
interface CaptureRoute {
  key: string
  labelKey: string
  icon: ReactNode
  /** Checklist file path; absent for Journal (uses journal_add). */
  path?: string
}

/**
 * v1 capture destinations — ported verbatim from the legacy `CAPTURE_ROUTES`
 * for behavior parity. Data-driven detection of checklist files (design §6,
 * mechanism (a): sniff `- [ ]` headers) is a contained follow-up enhancement;
 * the structure here is the single place to swap in a tree-derived list.
 */
const ROUTES: CaptureRoute[] = [
  {
    key: 'Later',
    labelKey: 'knowledge.later',
    icon: <Clock className="h-4 w-4 text-info" />,
    path: 'Later.md',
  },
  {
    key: 'Read',
    labelKey: 'knowledge.read',
    icon: <Newspaper className="h-4 w-4 text-warning" />,
    path: 'Read.md',
  },
  {
    key: 'Shop',
    labelKey: 'knowledge.shop',
    icon: <ShoppingCart className="h-4 w-4 text-destructive" />,
    path: 'Shop.md',
  },
  {
    key: 'Watch',
    labelKey: 'knowledge.watch',
    icon: <Tv className="h-4 w-4 text-chart-4" />,
    path: 'Watch.md',
  },
  {
    key: 'Journal',
    labelKey: 'knowledge.toJournal',
    icon: <BookOpen className="h-4 w-4 text-success" />,
  },
]

/** kbd hint for a route shortcut, e.g. `/later`. */
function routeKbd(key: string) {
  return <kbd className="text-[10px] text-muted-foreground">/{key.toLowerCase()}</kbd>
}

/**
 * Capture provider — verb `capture`.
 *
 * Owns the `/route text` slash convention (a capture-specific parse of the
 * first word after `/`) and the knowledge-mode bare-text memo default. The
 * generic lexer only identifies the `/` verb; route-keyword matching lives here.
 */
export function useCaptureProvider(): CommandProvider {
  const { t } = useTranslation()
  const chatAppend = useChatAppend()
  const journalAdd = useJournalAdd()
  const checklistAdd = useChecklistAdd()

  const runCapture = useMemo(
    () => async (text: string, routeKey: string | null) => {
      const body = text.trim()
      if (!body) return
      try {
        if (routeKey === 'Journal') {
          await journalAdd.mutateAsync(body)
          toast.success(t('commandPalette.capturedJournal'))
        } else if (routeKey) {
          const target = ROUTES.find((r) => r.key === routeKey)
          if (target?.path) {
            await checklistAdd.mutateAsync({ path: target.path, item: body })
            toast.success(t('commandPalette.capturedRoute', { route: t(target.labelKey) }))
          }
        } else {
          await chatAppend.mutateAsync(body)
          toast.success(t('commandPalette.capturedInbox'))
        }
      } catch {
        toast.error(t('commandPalette.captureFailed'))
      }
    },
    [t, chatAppend, journalAdd, checklistAdd],
  )

  return useMemo(
    () => ({
      id: 'capture',
      verbs: ['capture'],
      resolve(ctx: QueryContext): PaletteItem[] {
        // `/` explicit capture verb: slash-route convention.
        if (ctx.verb === 'capture') {
          const m = ctx.text.match(/^(\w*)\s*(.*)$/s)
          const tok = (m?.[1] ?? '').toLowerCase()
          const remainder = m?.[2] ?? ''
          const route = ROUTES.find((r) => r.key.toLowerCase() === tok)
          if (route) {
            const tail = remainder.trim()
            return [
              {
                id: `capture-${route.key}`,
                verb: 'capture',
                icon: route.icon,
                title: t('commandPalette.captureTo', { route: t(route.labelKey) }),
                subtitle: tail || t('commandPalette.typeText'),
                score: 100,
                onSelect: () => runCapture(remainder, route.key),
              },
            ]
          }
          // No/unknown route → destination picker (compose sets the query).
          return ROUTES.map(
            (r): PaletteItem => ({
              id: `route-${r.key}`,
              verb: 'capture',
              icon: r.icon,
              title: t(r.labelKey),
              hint: routeKbd(r.key),
              compose: `/${r.key} `,
              score: 0,
              onSelect: () => {},
            }),
          )
        }

        // Bare text → surface-primary. CaptureProvider owns Knowledge-surface memos.
        const trimmed = ctx.text.trim()
        if (ctx.verb === null && surfacePrimaryVerb(ctx.surface) === 'capture' && trimmed) {
          return [
            {
              id: 'capture-inbox',
              verb: 'capture',
              icon: <Plus className="h-4 w-4" />,
              title: t('commandPalette.captureMemo'),
              subtitle: trimmed,
              hint: <kbd className="text-[10px] text-muted-foreground">⏎</kbd>,
              score: matchScore(ctx, { label: trimmed }),
              onSelect: () => runCapture(trimmed, null),
            },
          ]
        }

        // Empty-state: capture route shortcuts + inbox hint.
        if (ctx.verb === null && !ctx.raw.trim()) {
          return [
            ...ROUTES.map(
              (r): PaletteItem => ({
                id: `route-${r.key}`,
                verb: 'capture',
                icon: r.icon,
                title: t('commandPalette.captureTo', { route: t(r.labelKey) }),
                hint: routeKbd(r.key),
                compose: `/${r.key} `,
                score: 0,
                onSelect: () => {},
              }),
            ),
            {
              id: 'capture-inbox-hint',
              verb: 'capture',
              icon: <Inbox className="h-4 w-4 text-muted-foreground" />,
              title: t('commandPalette.inboxHint'),
              score: 0,
              onSelect: () => {},
            },
          ]
        }

        return []
      },
    }),
    [t, runCapture],
  )
}
