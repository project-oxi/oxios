import { createFileRoute } from '@tanstack/react-router'
import { CapabilityHub } from '@/components/operate/capability-hub'

export const Route = createFileRoute('/operate/capabilities')({ component: CapabilityHub })
