import { createFileRoute, redirect } from '@tanstack/react-router'

/** Legacy deep link — permanent redirect into the Brain tab (2026-08-19). */
export const Route = createFileRoute('/knowledge/')({
  beforeLoad: ({ search }) => {
    throw redirect({ to: '/brain/knowledge', search: search as never })
  },
})
