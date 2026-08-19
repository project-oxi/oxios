// Shimmer placeholder while a session's history is fetched. Three rows in the
// alternating user/assistant rhythm so the layout does not jump on arrival.
export function SessionSkeleton() {
  return (
    <div className="mx-auto max-w-3xl space-y-4 px-4 py-6" aria-hidden="true">
      {[0, 1, 2].map((i) => (
        <div key={i} className={i % 2 === 0 ? 'flex justify-end' : ''}>
          <div className="w-2/3 space-y-2">
            <div className="h-3 w-1/3 animate-pulse rounded bg-muted" />
            <div className="h-3 w-full animate-pulse rounded bg-muted" />
            <div className="h-3 w-4/5 animate-pulse rounded bg-muted" />
          </div>
        </div>
      ))}
    </div>
  )
}
