// Task templates — recurring automation templates (ported from LobeHub)
// Pre-built templates for common scheduled agent tasks:
// - Weekly design inspiration (fonts + colors)
// - Daily creator tracking
// - Weekly YouTube summary
// - Daily news digest
// - Weekly project review

// ── Types ──

export type TaskTemplateCategory =
  | 'content-creation'
  | 'research'
  | 'monitoring'
  | 'productivity'
  | 'creative'
  | 'analytics'

export type TaskTemplateSchedule = 'daily' | 'weekly' | 'hourly' | 'custom'

export interface TaskTemplate {
  id: string
  identifier: string
  title: string
  description: string
  category: TaskTemplateCategory
  /** Cron pattern (e.g. "0 10 * * 3" = every Wednesday 10am). */
  cronPattern: string
  /** Human-readable schedule label. */
  scheduleLabel: string
  /** Agent instruction / system prompt. */
  instruction: string
  /** Required tools/skills. */
  requiredTools?: string[]
  /** Icon name (lucide-react). */
  icon: string
  /** Color accent. */
  color: string
}

// ── Templates ──

export const TASK_TEMPLATES: TaskTemplate[] = [
  {
    id: 'weekly-design-inspiration',
    identifier: 'weekly-design-inspiration',
    title: 'Weekly Design Inspiration',
    description:
      'Every Wednesday, get 3 font pairings and 3 color palettes worth saving to your inspiration library.',
    category: 'creative',
    cronPattern: '0 10 * * 3', // Wed 10am
    scheduleLabel: 'Every Wednesday at 10:00',
    instruction: `You are a design curator. Every week, provide:
1. **3 font pairings** — a heading font + body font combination, with use-case suggestions
2. **3 color palettes** — 5 colors each, with hex codes and mood description

For each, explain why it works and when to use it. Save the best ones to the inspiration library.`,
    requiredTools: ['web_search', 'write'],
    icon: 'Palette',
    color: 'text-hue-purple',
  },
  {
    id: 'daily-creator-tracking',
    identifier: 'daily-creator-tracking',
    title: 'Daily Creator Tracking',
    description: 'Every morning, track what 3-5 creators you follow posted and how they performed.',
    category: 'monitoring',
    cronPattern: '0 9 * * *', // Daily 9am
    scheduleLabel: 'Every day at 09:00',
    instruction: `You are a creator performance analyst. Track the creators the user follows:
1. Check their latest posts across platforms (YouTube, Twitter/X, blog)
2. Note engagement metrics (views, likes, comments)
3. Highlight any viral or breakthrough content
4. Summarize trends in their content strategy

Present as a morning briefing with actionable insights.`,
    requiredTools: ['web_search', 'browser'],
    icon: 'Users',
    color: 'text-hue-blue',
  },
  {
    id: 'weekly-youtube-summary',
    identifier: 'weekly-youtube-summary',
    title: 'Weekly YouTube Summary',
    description:
      "Every Monday, summarize last week's channel performance: views, CTR, retention — flag follow-up topics.",
    category: 'analytics',
    cronPattern: '0 9 * * 1', // Mon 9am
    scheduleLabel: 'Every Monday at 09:00',
    instruction: `You are a YouTube channel analyst. Provide a weekly summary:
1. **Performance overview** — total views, watch time, subscriber change
2. **Top videos** — which videos performed best and why
3. **CTR & retention** — click-through rate, average view duration
4. **Follow-up topics** — 3-5 content ideas based on what worked
5. **Audience insights** — demographics, traffic sources

Format as a clear report with numbers and recommendations.`,
    requiredTools: ['web_search', 'browser'],
    icon: 'Video',
    color: 'text-hue-red',
  },
  {
    id: 'daily-news-digest',
    identifier: 'daily-news-digest',
    title: 'Daily News Digest',
    description:
      'Every morning, get a curated summary of the top 5 news stories in your areas of interest.',
    category: 'research',
    cronPattern: '0 8 * * *', // Daily 8am
    scheduleLabel: 'Every day at 08:00',
    instruction: `You are a news curator. Provide a daily digest:
1. Find the top 5 most important news stories in the user's interest areas
2. For each: headline, 2-sentence summary, source link, why it matters
3. Highlight any breaking news or developing stories
4. Note any stories that connect to previous days' news

Keep it concise — the user should be able to read this in 2 minutes.`,
    requiredTools: ['web_search'],
    icon: 'Newspaper',
    color: 'text-hue-green',
  },
  {
    id: 'weekly-project-review',
    identifier: 'weekly-project-review',
    title: 'Weekly Project Review',
    description: 'Every Friday, review what you accomplished this week and plan next week.',
    category: 'productivity',
    cronPattern: '0 17 * * 5', // Fri 5pm
    scheduleLabel: 'Every Friday at 17:00',
    instruction: `You are a project review assistant. Provide a weekly review:
1. **Accomplishments** — what was completed this week
2. **Blockers** — what's stuck and needs attention
3. **Metrics** — progress on key goals
4. **Next week plan** — top 3 priorities for next week
5. **Reflection** — one thing that went well, one to improve

Use the workspace files and git history to ground the review in reality.`,
    requiredTools: ['read', 'bash', 'grep'],
    icon: 'CheckCircle',
    color: 'text-hue-amber',
  },
  {
    id: 'hourly-social-listening',
    identifier: 'hourly-social-listening',
    title: 'Hourly Social Listening',
    description:
      'Every hour, check for mentions of your brand or keywords across social platforms.',
    category: 'monitoring',
    cronPattern: '0 * * * *', // Every hour
    scheduleLabel: 'Every hour',
    instruction: `You are a social listening monitor. Check for:
1. Brand mentions across platforms
2. Keyword alerts for configured topics
3. Sentiment analysis (positive/negative/neutral)
4. Engagement opportunities (questions, complaints worth responding to)

Alert immediately for negative sentiment or viral mentions. Otherwise, batch into a summary.`,
    requiredTools: ['web_search', 'browser'],
    icon: 'Radar',
    color: 'text-hue-teal',
  },
]

// ── Categories ──

export const TASK_TEMPLATE_CATEGORIES: { id: TaskTemplateCategory; label: string; icon: string }[] =
  [
    { id: 'creative', label: 'Creative', icon: 'Palette' },
    { id: 'monitoring', label: 'Monitoring', icon: 'Radar' },
    { id: 'analytics', label: 'Analytics', icon: 'BarChart' },
    { id: 'research', label: 'Research', icon: 'Search' },
    { id: 'productivity', label: 'Productivity', icon: 'CheckCircle' },
    { id: 'content-creation', label: 'Content', icon: 'PenTool' },
  ]
