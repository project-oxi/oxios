import { createFileRoute } from '@tanstack/react-router'
import { RunCenter } from '@/components/operate/run-center'

export const Route = createFileRoute('/operate/runs')({ component: RunCenter })
