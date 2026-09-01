import { createFileRoute } from '@tanstack/react-router'
import { KnowledgeLayout } from '@/components/knowledge/knowledge-layout'

export const Route = createFileRoute('/knowledge/library/')({
  component: KnowledgeLayout,
})
