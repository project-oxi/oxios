// AutomationTemplateGallery — gallery of pre-built recurring automation templates.
// Ported from LobeHub's RecommendAutomationTemplates pattern.
// Shows cards for each template with "Add Automation" button.

import {
  BarChart,
  CheckCircle,
  Clock,
  type LucideIcon,
  Newspaper,
  Palette,
  PenTool,
  Plus,
  Radar,
  Search,
  Users,
  Video,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import {
  AUTOMATION_TEMPLATE_CATEGORIES,
  AUTOMATION_TEMPLATES,
  type AutomationTemplate,
  type AutomationTemplateCategory,
} from '@/types/automation-templates'

// ── Icon map ──

const ICONS: Record<string, LucideIcon> = {
  Palette,
  Users,
  Video,
  Newspaper,
  CheckCircle,
  Radar,
  Search,
  BarChart,
  PenTool,
}

// ── Props ──

interface AutomationTemplateGalleryProps {
  onSelectTemplate?: (template: AutomationTemplate) => void
  className?: string
}

// ── Component ──

export function AutomationTemplateGallery({
  onSelectTemplate,
  className,
}: AutomationTemplateGalleryProps) {
  const [activeCategory, setActiveCategory] = useState<AutomationTemplateCategory | 'all'>('all')
  const { t } = useTranslation()

  const filtered = useMemo(() => {
    if (activeCategory === 'all') return AUTOMATION_TEMPLATES
    return AUTOMATION_TEMPLATES.filter((t) => t.category === activeCategory)
  }, [activeCategory])

  return (
    <div className={cn('flex flex-col', className)} data-testid="automation-template-gallery">
      {/* Category filter */}
      <div className="flex items-center gap-1 mb-4 overflow-x-auto pb-1">
        <CategoryChip
          label={t('cronJobs.templates.all')}
          count={AUTOMATION_TEMPLATES.length}
          active={activeCategory === 'all'}
          onClick={() => setActiveCategory('all')}
        />
        {AUTOMATION_TEMPLATE_CATEGORIES.map((cat) => {
          const count = AUTOMATION_TEMPLATES.filter((t) => t.category === cat.id).length
          if (count === 0) return null
          const Icon = ICONS[cat.icon] ?? Palette
          return (
            <CategoryChip
              key={cat.id}
              label={cat.label}
              count={count}
              active={activeCategory === cat.id}
              onClick={() => setActiveCategory(cat.id)}
              icon={<Icon className="w-3.5 h-3.5" />}
            />
          )
        })}
      </div>

      {/* Template grid */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
        {filtered.map((template) => (
          <AutomationTemplateCard
            key={template.id}
            template={template}
            onSelect={() => onSelectTemplate?.(template)}
          />
        ))}
      </div>
    </div>
  )
}

// ── Category chip ──

function CategoryChip({
  label,
  count,
  active,
  onClick,
  icon,
}: {
  label: string
  count: number
  active: boolean
  onClick: () => void
  icon?: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex items-center gap-1.5 px-3 py-1.5 rounded-full text-sm whitespace-nowrap transition-colors',
        active
          ? 'bg-primary text-primary-foreground'
          : 'bg-muted text-muted-foreground hover:bg-muted/80',
      )}
    >
      {icon}
      <span>{label}</span>
      <span
        className={cn(
          'text-xs px-1.5 py-0.5 rounded-full',
          active ? 'bg-primary-foreground/20' : 'bg-background/50',
        )}
      >
        {count}
      </span>
    </button>
  )
}

// ── Template card ──

function AutomationTemplateCard({
  template,
  onSelect,
}: {
  template: AutomationTemplate
  onSelect: () => void
}) {
  const { t } = useTranslation()
  const Icon = ICONS[template.icon] ?? CheckCircle

  return (
    <div className="flex flex-col rounded-xl border bg-card p-4 hover:border-primary/30 hover:shadow-sm transition-all">
      {/* Header */}
      <div className="flex items-start gap-3 mb-2">
        <div className="shrink-0">
          <div className="w-10 h-10 rounded-lg bg-muted flex items-center justify-center">
            <Icon className={cn('w-5 h-5', template.color)} />
          </div>
        </div>
        <div className="flex-1 min-w-0">
          <h3 className="text-sm font-semibold truncate">{template.title}</h3>
          <div className="flex items-center gap-1 mt-0.5 text-xs text-muted-foreground">
            <Clock className="w-3 h-3" />
            <span>{template.scheduleLabel}</span>
          </div>
        </div>
      </div>

      {/* Description */}
      <p className="text-xs text-muted-foreground line-clamp-2 mb-3 flex-1">
        {template.description}
      </p>

      {/* Tools */}
      {template.requiredTools && template.requiredTools.length > 0 && (
        <div className="flex flex-wrap gap-1 mb-3">
          {template.requiredTools.map((tool) => (
            <span
              key={tool}
              className="text-2xs px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground font-mono"
            >
              {tool}
            </span>
          ))}
        </div>
      )}

      {/* Action */}
      <button
        type="button"
        onClick={onSelect}
        className="flex items-center justify-center gap-1.5 w-full py-2 rounded-lg bg-primary/10 text-primary text-sm font-medium hover:bg-primary/20 transition-colors"
      >
        <Plus className="w-3.5 h-3.5" />
        {t('automations.addFromTemplate')}
      </button>
    </div>
  )
}
