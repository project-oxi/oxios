# Knowledge × Search Integration Implementation Plan
> **Status**: Shipped — 2026-07 (chat mention knowledge search)

> **For agentic workers:** REQUIRED SUB-SKILL: Use executing-plans to implement.

**Goal:** Integrate Search Panel with Knowledge Base — Knowledge tab, Save, ⌘K Web, Copilot, KnowledgeView.

**Architecture:** Frontend-only. 1 new component, ~7 modified files. All backend APIs exist.

**Tech Stack:** TypeScript/React, Zustand, Tailwind

## Global Constraints

- No Rust/backend changes
- KnowledgeSearchHit: `{ path: string; name: string; snippet?: string; score?: number }`
- PortalView union: add `{ type: 'knowledge'; path: string; title?: string }`
- Save path: `web-clippings/{domain}/{YYYY-MM-DD}-{slug}.md`

---

### Task 1: KnowledgeBrowser component

**Files:** Create `web/src/components/portal/views/knowledge-browser.tsx`

Stateful search + read-only preview for knowledge notes. Props: `initialPath?`. Renders: search input → results list → on click → MarkdownMessage preview. [Open in Knowledge] button navigates to `/knowledge/file/$path`.

### Task 2: SearchPanel store expansion

**Files:** Modify `web/src/stores/search-panel.ts`

Add: `activeTab`, `knowledgeResults/loading/error`, `selectedKnowledgePath/content/loading`, `saveModalOpen/url/title/content/path/loading/error`. Actions: `setActiveTab`, `searchKnowledge`, `selectKnowledge`, `openSaveModal`, `closeSaveModal`, `saveModalSave`.

### Task 3: SearchView tab bar + Knowledge tab + Save labeling

**Files:** Modify `web/src/components/portal/views/search-view.tsx`

Tab bar below search input: `[Web] [Knowledge]`. Web tab = existing content. Knowledge tab = `<KnowledgeBrowser />`. Existing Save button gets full label `t('search.panel.saveToKnowledge')`.

### Task 4: i18n keys

**Files:** Modify `web/src/i18n/locales/en.json`, `ko.json`

Keys: `saveToKnowledge`, `savedToKnowledge`, `viewInKnowledge`, `knowledgeTab`, `webTab`, `knowledgePlaceholder`, `knowledgeEmpty`, `knowledgeNoResults`, `openInKnowledge`.

### Task 5: SearchModal Web tab

**Files:** Modify `web/src/components/knowledge/search-modal.tsx`

Add tab bar (`[Files] [Web]`) between input and results. Web tab: debounced `/api/search`, results cards with `[Open in Search Panel]` button. Toggle between existing file search and web search UI.

### Task 6: Copilot Web toggle

**Files:** Modify `web/src/components/knowledge/copilot.tsx`

Toggle `[Include web results]` → on send, also call `/api/search` in parallel. Show web results section below copilot response.

### Task 7: PortalPanel KnowledgeView

**Files:** Modify `web/src/stores/portal.ts`, `web/src/components/portal/portal-panel.tsx`

Add `{ type: 'knowledge'; path: string; title?: string }` to PortalView. Add case in portal-panel.tsx dispatcher → `<KnowledgeBrowser initialPath={path} />`.

### Task 8: Verify

**Files:** `bun x tsc --noEmit && cargo build -p oxios && cargo test -p oxios-kernel --lib && cd web && bun run test`
