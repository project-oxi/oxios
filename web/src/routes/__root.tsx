import type { QueryClient } from '@tanstack/react-query'
import { QueryClientProvider } from '@tanstack/react-query'
import { createRootRouteWithContext, Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'
import { AppLayout } from '@/components/layout/app-layout'
import { SettingsSearch } from '@/components/layout/settings-search'
import { ErrorBoundary } from '@/components/shared/error-boundary'
import { Button } from '@/components/ui/button'
import { Toaster } from '@/components/ui/sonner'
import { TooltipProvider } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

interface RouterContext {
  queryClient: QueryClient
}

export const Route = createRootRouteWithContext<RouterContext>()({
  notFoundComponent: NotFound,
  component: function RootComponent() {
    const { queryClient } = Route.useRouteContext()
    return (
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <ErrorBoundary>
            <AppLayout />
          </ErrorBoundary>
          <SettingsSearch />
          <Toaster />
        </TooltipProvider>
      </QueryClientProvider>
    )
  },
})

/**
 * Not-found state (design §10.2) — unmatched URLs land here. No redirect:
 * a centered bilingual notice with the documented way back to /operate.
 */
function NotFound() {
  const { t } = useTranslation()
  return (
    <div
      className={cn(
        'flex min-h-[60dvh] flex-col items-center justify-center gap-2 rounded-lg border border-border/40 bg-card/50 px-6 py-10 text-center',
      )}
    >
      <p className="font-mono text-2xs tracking-widest text-muted-foreground">404</p>
      <h1 className="text-lg font-semibold text-foreground">{t('notFound.title')}</h1>
      <p className="max-w-sm text-sm text-muted-foreground">{t('notFound.description')}</p>
      <Button asChild size="sm" variant="outline" className="mt-3">
        <Link to="/operate">{t('notFound.backToOperate')}</Link>
      </Button>
    </div>
  )
}
