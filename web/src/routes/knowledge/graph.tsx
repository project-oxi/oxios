import { createFileRoute, redirect } from '@tanstack/react-router'

export const Route = createFileRoute('/knowledge/graph')({
  beforeLoad: ({ search }) => {
    throw redirect({ to: '/brain/knowledge/graph', search: search as never })
  },
})
