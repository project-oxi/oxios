import {
  BarChart3,
  Bell,
  Bot,
  Brain,
  Calendar,
  Clock,
  Cpu,
  Database,
  Eye,
  Gauge,
  Globe,
  Image as ImageIcon,
  KeyRound,
  ListOrdered,
  MessageSquare,
  Monitor,
  Palette,
  Send,
  Server,
  Shield,
  SlidersHorizontal,
  Sparkles,
  StickyNote,
  Terminal,
  Wallet,
  Wrench,
  Zap,
} from 'lucide-react'
import type { SectionIconKey } from './field-defs'

const ICON_MAP: Record<SectionIconKey, React.ComponentType<{ className?: string }>> = {
  engine: Bot,
  kernel: Cpu,
  exec: Terminal,
  executionRecipe: Wrench,
  security: Shield,
  orchestrator: Zap,
  context: Brain,
  gateway: Globe,
  session: Monitor,
  logging: Server,
  memory: Database,
  channels: Send,
  audit: Eye,
  update: Sparkles,
  calendar: Calendar,
  agentLog: ListOrdered,
  resourceMonitor: Gauge,
  browser: Globe,
  budget: Wallet,
  secrets: KeyRound,
  notifications: Bell,
  memo: StickyNote,
  timeline: Clock,
  systemAgents: SlidersHorizontal,
  stats: BarChart3,
  hostTools: Wrench,
  image: ImageIcon,
  appearance: Palette,
}

/** Generic fallback used when an id is unknown. */
const Fallback = MessageSquare

interface SectionIconProps {
  iconKey: SectionIconKey | string
  className?: string
}

/**
 * Renders the Lucide icon mapped to a section's `iconKey`. Centralised
 * here so the rail, the section tabs, and the section card all share
 * the same icon vocabulary.
 */
export function SectionIcon({ iconKey, className }: SectionIconProps) {
  const Icon = ICON_MAP[iconKey as SectionIconKey] ?? Fallback
  return <Icon className={className} />
}
