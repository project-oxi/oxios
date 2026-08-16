# RFC-003: Knowledge Base 독립 분리
> **상태:** 구현 완료 ✅
>
> **Note (RFC-048 §5):** The two knowledge systems are kept distinct:
> **agent memory** is Brain-ledger data (BrainClient via the oxibrain
> daemon, RFC-047), and **KnowledgeBase notes** are user-facing `.md`
> files (`oxios-markdown`). The LLM note-refinement feature is now
> named *knowledge curation* (was *knowledge dream*); Brain episode
> consolidation is `oxios brain consolidate`. The two operations are
> never interchanged.
> **상태:** 구현 완료 ✅  
> **날짜:** 2026-05-20  
> **이전:** rfc-003-knowledge-separation.md (초안)  
> **범위:** oxios-markdown, oxios-kernel, oxios-web

---

## 1. 동기

사용자의 삶이 마크다운으로 흐른다. 소설, 기획서, 일정, 일기, 습관 — 전부 `.md` 파일이다.
이 지식 베이스는 **에이전트 OS와 무관하게 독립적으로 존재**해야 한다.
사용자가 노트 에디터로 글을 쓸 때 에이전트 스케줄러, 예산 관리자, 수퍼바이저를 거칠 이유가 없다.

**현재 구조의 문제:**
1. `KnowledgeApi`가 kernel 내부에 있어서 마크다운 CRUD가 kernel 전체를 의존
2. oxios-web이 노트 읽기/쓰기만 할 때도 KernelHandle을 거쳐야 함
3. 지식 베이스 앱 로직과 에이전트 연동 로직이 한 struct에 혼재
4. 같은 내용이 `.md`와 JSON에 이중 저장됨
5. Space별로 knowledge 디렉토리가 분리됨
6. **이전 초안 리뷰에서 발견:** `KnowledgeBridge` 이름이 `space/knowledge_bridge.rs`와 충돌

---

## 2. 핵심 원칙

```
1. .md 파일이 유일한 원천(Source of Truth)이다
2. 마크다운 앱은 kernel 없이도 동작한다
3. 세션 메모리와 지식 베이스는 별개의 영역이다
4. 지식은 전역 하나다 (Space별 분리 안 함)
5. Semantic search와 AI 기능은 kernel 영역이다
6. 에이전트는 기본적으로 knowledge에 쓸 수 있다 (audit trail로 추적)
```

---

## 3. 용어 재정의 (이름 충돌 해결)

| 용어 | Before | After | 이유 |
|------|--------|-------|------|
| kernel 내부 knowledge API | `KnowledgeApi` | **삭제** | KnowledgeBase + KnowledgeLens로 분리 |
| kernel → knowledge 연동 | `KnowledgeBridge` | **`KnowledgeLens`** | 새 이름. 에이전트가 지식 베이스를 "들여다보는" 렌즈 |
| Space 간 메모리 흐름 | `space::KnowledgeBridge` | **`SpaceBridge`** | 기존 이름 개명. 메모리(memory) 전송이므로 |
| 메모리 흐름 타입 | `KnowledgeFlow` | **`MemoryFlow`** | enum 이름도 맞게 변경 |
| Space 가시성 플래그 | `knowledge_visible` | **`memory_visible`** | 필드명도 일관성 있게 |

---

## 4. 전체 아키텍처 (After)

```
┌────────────────────────────────────────────────────────────────┐
│                         oxios-web                               │
│                                                                  │
│  AppState {                                                     │
│    knowledge: Arc<KnowledgeBase>,   ← 마크다운 앱 직접 접근   │
│    kernel:   Arc<KernelHandle>,       ← 에이전트 전용           │
│  }                                                              │
│                                                                  │
│  knowledge 라우트     → state.knowledge.note_read()             │
│  chat/agent 라우트    → state.kernel                            │
│  Space 라우트         → state.kernel.spaces                     │
└──────────────┬───────────────────────────┬──────────────────────┘
               │                           │
               │ (직접)                     │ (에이전트 전용)
               ▼                           ▼
┌──────────────────────────┐    ┌─────────────────────────────────┐
│   oxios-markdown        │    │   oxios-kernel                   │
│                          │    │                                  │
│  ┌──────────────────┐  │    │  ┌──────────────────────────┐  │
│  │ KnowledgeBase     │  │    │  │  KnowledgeLens          │  │
│  │ (신규)            │  │◄───┼──│  (kernel_handle/)       │  │
│  │                  │  │    │  │  semantic_search()      │  │
│  │ VirtualFs        │  │    │  │  copilot_chat()         │  │
│  │ BacklinkIndex   │  │    │  │  recall_for_context()   │  │
│  │ note CRUD       │  │    │  │  agent_write()          │  │
│  │ chat/journal/   │  │    │  │  (on_file_change hook)  │  │
│  │ habits/         │  │    │  └──────────────────────────┘  │
│  │ checklist       │  │    │                                   │
│  │ search (파일명) │  │    │  ┌──────────────────────────┐  │
│  │ worker/stats    │  │    │  │  MemoryManager          │  │
│  │ on_file_change │  │    │  │  Session / Fact /       │  │
│  └──────────────────┘  │    │  │  Episode / Conversation │  │
│                          │    │  │  Space별 격리            │  │
│  kernel 의존 없음        │    │  └──────────────────────────┘  │
│  AI 의존 없음            │    │                                   │
│  oxios_markdown 의존 없음│    │  ┌──────────────────────────┐  │
└──────────────────────────┘    │  │  SpaceBridge            │  │
                                 │  │  (space/)               │  │
                                 │  │  cross-Space memory     │  │
                                 │  │  flow management        │  │
                                 │  └──────────────────────────┘  │
                                 └─────────────────────────────────┘
```

---

## 5. 데이터 영역 분리

```
~/.oxios/
│
├── knowledge/                    ← 전역 지식 베이스 (유일 원천)
│   ├── .index/                  ← semantic search 인덱스
│   │   ├── vectors.usearch        HNSW 인덱스
│   │   └── key_map.json          파일 경로 ↔ u64 키 매핑
│   ├── brain/
│   │   ├── Rust.md
│   │   └── 아키텍처-고민.md
│   ├── dev/
│   ├── 일상/
│   ├── Chat.md
│   ├── Later.md
│   ├── Done.md
│   ├── journal/
│   ├── habits/
│   └── config.json
│
├── workspace/                    ← 에이전트 영역
│   ├── sessions/
│   ├── seeds/
│   ├── programs/
│   ├── skills/
│   └── spaces/
│       ├── {space-id}/
│       │   └── memory/           Space별 에이전트 메모리 (JSON)
│       │       ├── conversations/
│       │       ├── facts/
│       │       ├── episodes/
│       │       └── sessions/
│       └── ...
│
└── config.toml
```

**지식(knowledge)** = 사용자의 평생 마크다운. 전역 하나.  
**메모리(memory)** = 에이전트의 작업 기억. Space별로 격리.  
**SpaceBridge** = Space 간 메모리 전송만 관리.

---

## 6. 핵심 컴포넌트 상세

### 6.1 KnowledgeBase (oxios-markdown, 신규)

```rust
// crates/oxios-markdown/src/knowledge.rs

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use parking_lot::{RwLock, Mutex as ParkingMutex};

/// 마크다운 지식 베이스의 통합 앱 레이어.
///
/// VirtualFs + BacklinkIndex + 모든 앱 기능을 하나로 묶는다.
/// kernel 의존성, AI 의존성 모두 없다.
/// 웹 에디터, CLI 등 어떤 채널에서도 직접 사용 가능.
pub struct KnowledgeBase {
    fs: RwLock<VirtualFs>,
    backlinks: RwLock<BacklinkIndex>,
    /// Files written by agents (not by users).
    agent_writes: ParkingMutex<std::collections::HashSet<String>>,
    /// Callbacks invoked on file changes.
    /// Used by KnowledgeLens to keep semantic index in sync.
    on_change: RwLock<Vec<Box<dyn Fn(&str, FileChange) + Send + Sync>>>,
}

pub enum FileChange {
    Created(String),
    Updated(String),
    Deleted(String),
    Moved { old: String, new: String },
}
```

**핵심: 파일 변경 콜백 시스템**

KnowledgeBase.note_write()가 호출되면 등록된 콜백을 실행한다:

```rust
impl KnowledgeBase {
    /// Register a callback to be invoked on every file change.
    pub fn on_file_change<F>(&self, f: F)
    where
        F: Fn(&str, FileChange) + Send + Sync + 'static,
    {
        self.on_change.write().push(Box::new(f));
    }

    fn notify_change(&self, path: &str, change: FileChange) {
        for cb in self.on_change.read().iter() {
            cb(path, change.clone());
        }
    }

    pub fn note_write(&self, path: &str, content: &str) -> Result<()> {
        self.fs.read().write_path(path, content)?;
        self.backlinks.write().index_file(path, content);
        self.notify_change(path, FileChange::Updated(path.to_string()));
        Ok(())
    }
}
```

KnowledgeBase 자체는 kernel 의존이 없으므로, 이 콜랩은 함수 포인터만 전달한다.
KnowledgeLens가 자기만의 인덱스를 관리하면서 콜백을 등록한다.

**제공 메서드:**

| 카테고리 | 메서드 |
|----------|--------|
| Note CRUD | `note_read`, `note_write`, `note_delete`, `note_move`, `note_tree` |
| Search (파일명) | `search` |
| Backlinks | `backlinks_for`, `link_graph`, `index_all` |
| Chat | `chat_append`, `chat_messages`, `chat_delete`, `chat_rename`, `chat_move_to` |
| Journal | `journal_add_record`, `journal_add_emoji`, `journal_today_path` |
| Habits | `habits`, `habits_write`, `habits_last_week` |
| Checklist | `checklist_items`, `checklist_add`, `checklist_complete`, `checklist_remove` |
| Config | `config`, `set_config` |
| Worker | `run_nightly_cleanup`, `run_scheduled_tasks` |
| Stats | `today_report`, `done_today` |
| Utilities | `markdown_to_html`, `auto_emoji` |
| Agent tracking | `mark_agent_write`, `is_agent_write`, `clear_agent_write` |

**제공하지 않는 것:**
- `copilot_chat` (AI — KnowledgeLens가 담당)
- `semantic_search` (AI — KnowledgeLens가 담당)
- `index_to_memory` (이중 저장 — 제거됨)

---

### 6.2 KnowledgeLens (oxios-kernel, 신규)

```rust
// crates/oxios-kernel/src/kernel_handle/knowledge_lens.rs

use std::sync::{Arc, RwLock};
use oxios_markdown::KnowledgeBase;

/// 에이전트가 지식 베이스를 참조하는 thin bridge.
///
/// KnowledgeBase의 .md 파일을 읽어 에이전트 컨텍스트에 주입하고,
/// TF-IDF/HNSW semantic search, AI copilot chat 기능을 제공한다.
pub struct KnowledgeLens {
    /// Shared knowledge base (owned by AppState, cloned here).
    kb: Arc<KnowledgeBase>,
    /// Embedding provider for semantic search.
    embedding: Arc<dyn EmbeddingProvider>,
    /// AI engine provider for copilot.
    engine: Arc<dyn EngineProvider>,
    /// Default model ID.
    default_model: String,
    /// HNSW index for fast ANN search.
    hnsw_index: RwLock<Option<Arc<HnswMemoryIndex>>>,
    /// Last index update timestamp (for stale detection).
    index_version: RwLock<u64>,
}
```

**semantic index 동기화 — 콜백 기반:**

```rust
impl KnowledgeLens {
    pub fn new(
        kb: Arc<KnowledgeBase>,
        embedding: Arc<dyn EmbeddingProvider>,
        engine: Arc<dyn EngineProvider>,
        default_model: String,
    ) -> Self {
        let lens = Self {
            kb,
            embedding,
            engine,
            default_model,
            hnsw_index: RwLock::new(None),
            index_version: RwLock::new(0),
        };

        // Register callback: KnowledgeBase notifies us on file changes
        lens.kb.on_file_change({
            let hnsw = lens.hnsw_index.read().clone();
            let idx_ver = lens.index_version.clone();
            move |path, change| {
                if let Some(ref idx) = hnsw {
                    match change {
                        FileChange::Created(p) | FileChange::Updated(p) => {
                            let content = kb.note_read(p).ok().flatten();
                            if let Some(text) = content {
                                if let Some(vec) = embedding.embed(&text).ok() {
                                    if let Some(f32_vec) = vec.to_f32_dense() {
                                        let _ = idx.add_entry(p, &f32_vec);
                                    }
                                }
                            }
                        }
                        FileChange::Deleted(p) => { idx.remove_entry(p).ok(); }
                        FileChange::Moved { old, new } => {
                            idx.remove_entry(old).ok();
                            // Re-index new path
                            if let Some(content) = kb.note_read(new).ok().flatten() {
                                if let Some(vec) = embedding.embed(&content).ok() {
                                    if let Some(f32_vec) = vec.to_f32_dense() {
                                        let _ = idx.add_entry(new, &f32_vec);
                                    }
                                }
                            }
                        }
                    }
                    *idx_ver.write() += 1;
                }
            }
        });

        lens
    }

    /// Semantic search over all .md files using HNSW index.
    pub fn semantic_search(&self, query: &str, limit: usize) -> Result<Vec<SemanticHit>>;
    
    /// Return (path, content) pairs for relevant notes.
    pub fn recall_for_context(&self, query: &str, limit: usize) -> Result<Vec<(String, String)>>;
    
    /// AI copilot chat about the knowledge base.
    pub fn copilot_chat(&self, question: &str, context_path: Option<&str>) -> Result<CopilotResponse>;
    
    /// Agent writes to the knowledge base (marks + audit).
    pub fn agent_write(&self, path: &str, content: &str) -> Result<()> {
        self.kb.note_write(path, content)?;
        self.kb.mark_agent_write(path);
        // Audit trail logged by the caller (agent_runtime or KnowledgeTool)
        Ok(())
    }
}
```

**인덱스 저장 위치:** `~/.oxios/knowledge/.index/`

```
knowledge/
├── .index/
│   ├── vectors.usearch       ← HNSW 인덱스 (usearch)
│   └── key_map.json          ← {path: String} ↔ {key: u64}
├── brain/
│   └── Rust.md
└── ...
```

index_version은 stale detection용: KnowledgeBase의 파일 변경 시각과
hnsw_index의 마지막 빌드 시각을 비교해서 차이가 크면 rebuild 트리거.

---

### 6.3 SpaceBridge (oxios-kernel, 기존 knowledge_bridge.rs 개명)

```rust
// crates/oxios-kernel/src/space/space_bridge.rs
// (기존 knowledge_bridge.rs → 파일명 + 내용 모두 변경)

/// Type of memory flow between Spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryFlow {
    /// Read-only access to another Space's memory.
    Reference,
    /// Copy entries from one Space to another.
    Transfer,
    /// Synthesize insights from multiple Spaces.
    Synthesis,
}

/// Manages memory flow between Spaces.
///
/// Knows nothing about .md files or the knowledge base.
/// Only handles cross-Space MemoryManager transfers.
pub struct SpaceBridge {
    space_manager: Arc<SpaceManager>,
    audit_trail: Option<Arc<AuditTrail>>,
    recent_refs: RwLock<Vec<CrossRefEntry>>,
}
```

**변경 사항:**
- `pub use knowledge_bridge::{CrossRefEntry, KnowledgeBridge, KnowledgeFlow}`  
  → `pub use space_bridge::{CrossRefEntry, SpaceBridge, MemoryFlow}`
- `Space::knowledge_visible` → `Space::memory_visible`
- `KernelEvent::SpaceCreated { knowledge_flow: ... }` → `memory_flow`
- `SpaceApi::knowledge_flow()` → `SpaceApi::memory_flow()`
- `space_routes.rs`의 API response field `knowledge_visible` → `memory_visible`

---

## 7. agent_runtime에 knowledge recall 연동

**현재 흐름:**

```rust
// agent_runtime.rs:240-250 (기존)
let memories = memory_manager.recall(&seed.goal).await;
system_prompt = memory_manager.blend_into_prompt(&memories, &system_prompt);
```

**새 흐름:**

```rust
// agent_runtime.rs (수정)
let memory_manager = self.kernel_handle.agents.memory_manager();

// 1. 세션 메모리 recall (기존)
match memory_manager.recall(&seed.goal).await {
    Ok(memories) if !memories.is_empty() => {
        system_prompt = memory_manager.blend_into_prompt(&memories, &system_prompt);
    }
    Ok(_) => tracing::debug!("No memories recalled"),
    Err(e) => tracing::warn!(error = %e, "Failed to recall memories"),
}

// 2. 지식 베이스 recall (추가)
let knowledge_lens = &self.kernel_handle.knowledge;
match knowledge_lens.recall_for_context(&seed.goal, 5) {
    Ok(notes) if !notes.is_empty() => {
        let block = notes.iter()
            .map(|(path, content)| format!("### {}\n{}", path, content))
            .collect::<Vec<_>>()
            .join("\n\n");
        system_prompt = format!("{}\n\n## Relevant Notes\n\n{}", system_prompt, block);
    }
    Ok(_) => tracing::debug!("No relevant notes found"),
    Err(e) => tracing::warn!(error = %e, "Failed to recall knowledge"),
}
```

`AgentRuntime`은 `KernelHandle`을 통해 `KnowledgeLens`에 접근한다.
`KernelHandle::knowledge` 필드가 기존 `KnowledgeApi`에서 `KnowledgeLens`로 변경되었으므로
이 접근 경로는 그대로 유지된다.

---

## 8. auto_memory_bridge 재설계

**Before:** MEMORY.md ↔ MemoryManager (JSON) 동기화

**After:** MEMORY.md ↔ KnowledgeBase (`.md` 파일) 동기화

```
MEMORY.md (외부 도구 형식)
       │
       ├── from-auto: MEMORY.md 파싱 → KnowledgeBase.note_write()
       │                              knowledge/imported/claude-code/{패턴}.md
       │
       └── to-auto: KnowledgeBase에서 .md 순회 → MEMORY.md 포맷으로 작성
                              knowledge/ 디렉토리 → MEMORY.md 변환
```

**핵심 변경:**
- `AutoMemoryBridge`에서 `MemoryManager` 대신 `Arc<KnowledgeBase>`를 받도록 변경
- `MemoryType::Knowledge` 의존 제거
- 대신 `KnowledgeBase.note_write()` / `note_tree()` 사용

이 구조 변경은 Phase 3에서 함께 처리.

---

## 9. 구체적 변경 사항 (파일별)

### 9.1 oxios-markdown

| 파일 | 액션 | 내용 |
|------|------|------|
| `src/knowledge.rs` | **신규** | KnowledgeBase (~400 LOC) |
| `src/lib.rs` | 수정 | `pub mod knowledge;` + re-export 추가 |
| `Cargo.toml` | 수정 | 새 파일 추가 (의존성 변경 없음) |

### 9.2 oxios-kernel

| 파일 | 액션 | 내용 |
|------|------|------|
| `src/kernel_handle/knowledge_lens.rs` | **신규** | KnowledgeLens (~350 LOC) |
| `src/kernel_handle/knowledge_api.rs` | **삭제** | 878 LOC 제거 |
| `src/kernel_handle/mod.rs` | 수정 | `knowledge` 필드: `KnowledgeApi` → `KnowledgeLens` |
| `src/lib.rs` | 수정 | `KnowledgeBridge` → `SpaceBridge`, `KnowledgeApi` 제거 |
| `src/space/knowledge_bridge.rs` | **개명+변경** | `space_bridge.rs`로 이름 변경. `KnowledgeBridge` → `SpaceBridge`. `KnowledgeFlow` → `MemoryFlow`. |
| `src/space.rs` | 수정 | 모듈명/ re-export 변경 |
| `src/space/manager.rs` | 수정 | `set_knowledge_bridge` → `set_memory_bridge` |
| `src/space/knowledge_bridge.rs` 삭제 | **삭제** | 파일 자체 삭제, 새 파일로 교체 |
| `src/memory/mod.rs` | 수정 | `MemoryType::Knowledge` 제거 |
| `src/memory/store.rs` | 수정 | `MemoryType::Knowledge` 관련 모든 분기/카테고리 제거 |
| `src/memory/auto_memory_bridge.rs` | 수정 | `Arc<MemoryManager>` → `Arc<KnowledgeBase>` + 메모리 동기화 로직 재설계 |
| `src/memory/migrate.rs` | 수정 | `MemoryType::Knowledge` 관련 마이그레이션 로직 제거 |
| `src/agent_runtime.rs` | 수정 | knowledge recall 추가 |
| `src/tools/kernel/knowledge_tool.rs` | 수정 | `KnowledgeBase` + `KnowledgeLens` 사용하도록 변경 |
| `src/tools/kernel/mod.rs` | 수정 | knowledge_tool re-export 유지 (signature만 변경) |
| `src/tools/kernel_bridge.rs` | 수정 | 테스트 헬퍼의 `KnowledgeApi::new()` → `KnowledgeLens` 사용 |
| `src/supervisor.rs` | 수정 | 테스트 헬퍼의 `KnowledgeApi::new()` → `KnowledgeLens` 사용 |
| `src/kernel_handle/space_api.rs` | 수정 | `knowledge_flow` → `memory_flow`. `knowledge_visible` → `memory_visible` |

### 9.3 oxios-web

| 파일 | 액션 | 내용 |
|------|------|------|
| `src/server.rs` | 수정 | AppState에 `knowledge: Arc<KnowledgeBase>` 추가 |
| `src/routes/knowledge_routes.rs` | 수정 | `state.kernel.knowledge` → `state.knowledge`. kernel 거치지 않음. |
| `src/routes/space_routes.rs` | 수정 | `knowledge_flow` → `memory_flow`, `knowledge_visible` → `memory_visible` |
| `Cargo.toml` | 수정 | `oxios-markdown` 직접 의존 추가 |

### 9.4 src/ (바이너리)

| 파일 | 액션 | 내용 |
|------|------|------|
| `kernel.rs` | 수정 | KnowledgeBase 생성 로직 추가. KnowledgeLens를 KernelHandle에注入. |

---

## 10. 마이그레이션 경로

### Phase 1: KnowledgeBase + SpaceBridge 개명
- oxios-markdown에 `knowledge.rs` 신규 작성
- `space/knowledge_bridge.rs` → `space_bridge.rs` 개명 (내용은 최소 수정)
- 기존 코드는 그대로 동작 (KnowledgeApi 아직 사용 중)
- **이득:** 이름 충돌 해소, kernel에 새 의존성 없음

### Phase 2: oxios-web 전환 + SpaceBridge 완전 변경
- AppState에 `knowledge: Arc<KnowledgeBase>` 추가
- knowledge_routes의 handler를 `state.knowledge`로 변경
- SpaceBridge 완전 구현 (MemoryFlow, memory_visible)
- **기존 KnowledgeApi는 아직 삭제 안 함** (KnowledgeTool이 참조 중)

### Phase 3: KnowledgeLens + KnowledgeApi 삭제
- `knowledge_lens.rs` 신규 작성
- `agent_runtime`에 knowledge recall 연동
- KnowledgeTool이 `KnowledgeBase` + `KnowledgeLens` 사용하도록 수정
- `auto_memory_bridge` 재설계 (KnowledgeBase 사용)
- `index_to_memory` 제거
- `MemoryType::Knowledge` 제거
- 기존 `knowledge_api.rs` 삭제

### Phase 4: 정리
- `activate_space`에서 knowledge.switch_space 제거
- 전역 knowledge 경로 고정: `~/.oxios/knowledge/`
- 기존 Space별 knowledge 디렉토리 마이그레이션 스크립트 (선택)

---

## 11. LOC 변화 요약

| 변경 | LOC |
|------|-----|
| `KnowledgeApi` 삭제 | -878 |
| `KnowledgeBase` 추가 | +~400 |
| `KnowledgeLens` 추가 | +~350 |
| `space_bridge.rs` 개명+변경 | ±50 |
| `MemoryType::Knowledge` 제거 관련 | -~100 |
| **순변화** | **≈ -178** |

kernel에서 878 LOC가 700 LOC로 줄어든다. 핵심 기능은 동일, 구조만 정리.

---

## 12. 최종 의존성 그래프

```
oxios-web
├── oxios-markdown                   ← 가벼운 마크다운 앱 (의존성 없음)
│     └── serde, chrono, walkdir
└── oxios-kernel
      ├── oxios-markdown             ← KnowledgeLens가 KnowledgeBase 참조
      ├── oxi-sdk, oxi-ai
      └── (heavy deps)

CLI 사용자: oxios-markdown으로 노트 CRUD (kernel 없이)
에이전트: oxios-kernel으로 모든 기능 (기존대로)
```

---

## 13. 테스트 업데이트 요약

| 위치 | 변경 |
|------|------|
| `supervisor.rs:423` | `KnowledgeApi::new()` → `KnowledgeLens::new()` |
| `tools/kernel_bridge.rs:168` | `KnowledgeApi::new()` → `KnowledgeLens::new()` |
| `knowledge_api.rs` 내 테스트 | 파일 삭제 시 함께 제거 |
| `knowledge_routes.rs` 내 테스트 | `AppState`에 `knowledge` 추가 후 `state.knowledge` 사용 |
| `space/knowledge_bridge.rs` → `space_bridge.rs` | `KnowledgeBridge` → `SpaceBridge`, `KnowledgeFlow` → `MemoryFlow` 리네임 |