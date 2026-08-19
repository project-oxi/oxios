import { createFileRoute } from '@tanstack/react-router'
import { KnowledgeLayout } from '@/components/knowledge/knowledge-layout'

export const Route = createFileRoute('/brain/knowledge/')({
  component: KnowledgeLayout,
})
