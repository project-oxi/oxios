import { createFileRoute } from '@tanstack/react-router'
import { OperateAttention } from '@/components/operate/operate-attention'

export const Route = createFileRoute('/operate/')({ component: OperateAttention })
