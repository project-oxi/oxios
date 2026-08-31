import { createFileRoute } from '@tanstack/react-router'
import { CommandCenterPage } from '@/components/dashboard/command-center/command-center-page'

export const Route = createFileRoute('/')({ component: CommandCenterPage })
