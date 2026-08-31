import { Skeleton } from '@/components/ui/skeleton'

export function LoadingCards({ count = 3 }: { count?: number }) {
  return (
    <div className="grid gap-4 animate-stagger">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="rounded-xl border p-6">
          <Skeleton className="h-4 w-1/3 mb-4" />
          <Skeleton className="h-3 w-2/3 mb-2" />
          <Skeleton className="h-3 w-1/2" />
        </div>
      ))}
    </div>
  )
}

export function LoadingTable({ rows = 5 }: { rows?: number }) {
  return (
    <div className="rounded-xl border animate-fade-in-up">
      <div className="border-b p-4 bg-muted/30">
        <Skeleton className="h-4 w-1/4" />
      </div>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex items-center gap-4 border-b p-4 last:border-0">
          <Skeleton className="h-4 w-1/4" />
          <Skeleton className="h-4 w-1/3" />
          <Skeleton className="h-4 w-1/6" />
        </div>
      ))}
    </div>
  )
}

/** Agent list skeleton — icon + name + id */
export function LoadingAgentList({ count = 4 }: { count?: number }) {
  return (
    <div className="space-y-2 animate-stagger">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="flex items-center gap-3 rounded-md border px-3 py-2">
          <Skeleton className="h-3.5 w-3.5 rounded-sm" />
          <Skeleton className="h-4 w-1/3" />
          <Skeleton className="h-3 w-12 ml-auto" />
        </div>
      ))}
    </div>
  )
}

/** Dashboard system health skeleton */
export function LoadingSystemHealth() {
  return (
    <div className="rounded-xl border p-4 space-y-3 animate-fade-in-up">
      <div className="flex items-center gap-2">
        <Skeleton className="h-4 w-4 rounded" />
        <Skeleton className="h-5 w-32" />
      </div>
      {Array.from({ length: 3 }, (_, i) => (
        <div key={i} className="flex items-center gap-2">
          <Skeleton className="h-2 w-2 rounded-full" />
          <Skeleton className="h-3 w-20" />
          <Skeleton className="h-3 w-12 ml-auto" />
        </div>
      ))}
    </div>
  )
}
