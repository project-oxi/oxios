# RFC-039 — 페르소나 시스템 완성

| | |
|---|---|
| **Status** | Implemented (2026-08, v1.38.0 audit) |
| **Author** | Persona system analysis (kernel team) |
| **Scope** | `crates/oxios-kernel/src/persona/`, `src/api/persona_routes.rs`, `src/kernel.rs:880`, `web/src/components/command-palette/switch.tsx`, `web/src/routes/personas.tsx` |

## 1. 문제

페르소나 시스템은 도메인은 정직하게 동작하지만 외피는 약속을 과하게 합니다. 다음 8가지 결함이 코드 검증으로 확정되었습니다.

| # | 결함 | 영향 | 출처 |
|---|---|---|---|
| 1 | store 메모리 전용, docstring 거짓 | 사용자/에이전트 페르소나 재시작 후 소실 | `persona/store.rs:1-4`, `persona/manager.rs:21-29` |
| 2 | `max_concurrent_personas` 데드 | docstring 과 코드 불일치, dead config | `config.rs:1600-1607` |
| 3 | `default_persona_id` 무시 | dead config | `config.rs:1599`, `persona/manager.rs:92` |
| 4 | 페르소나 role 이 모델 해석 경로에 닿지 않음 | `engine.role_routing[role]` 매핑이 페르소나에 의해 트리거되지 않음 | `agent_runtime.rs:311` (`env.role` = WS 힌트), `:351-354` (페르소나 role 별도 추출), `:495-498` (모델 해석) |
| 5 | HTTP↔Web UI 라우트 불일치 | `POST /api/personas/:id/activate` 가 404 | `web/.../switch.tsx:53`, `web/.../personas.tsx:88`, `src/api/routes/mod.rs:432` |
| 6 | HTTP 라우트는 보안 리뷰 미수행, `PersonaTool` 은 수행 | 위협 모델 비대칭, 일관성 결여 | `src/api/persona_routes.rs:86-158` vs `tools/builtin/persona_tool.rs:252-272` |
| 7 | `Session.active_persona_id` 필드는 있지만 write 호출자 없음 | 글로벌/세션 동기화 누락 | `state_store.rs:265-268`, 검색 결과 호출자 0 |
| 8 | 부팅 시 `set_persona_prompt` 1회, `set_active` 시 재시드 없음 | 활성 페르소나 변경 후 intent engine 미반영 | `src/kernel.rs:881-883` |

각 항목은 2026-07-08 분석에서 직접 grep/read 로 검증된 사실입니다.

## 2. 비-목표

- **다중 동시 활성 페르소나**. 별도 RFC.
- **`Persona::model` 필드 삭제**. 사용자 데이터 손실. deprecation only.
- **HTTP 경로 보안 리뷰**. 사용자 직접 작성은 자기 책임, 에이전트 자동 생성만 위협 모델링 대상 (불대칭 유지).
- **세션별 페르소나 override**. v2. 글로벌만 동기화.
- **`max_concurrent_personas` 호환 shim**. dead config 라서 config warning 한 줄로 충분.

## 3. 설계

### 3.1 영속성 — `StateStore` 재사용

**결정**: `personas.toml` 같은 새 파일 + bespoke atomic-tmp-rename 을 만들지 **않는다**. `StateStore::save_json` 이 이미 `durable_write` (atomic fsync+rename+dir fsync, `state_store.rs:357-383`) 를 제공하므로 이를 따른다.

- **경로**: `~/.oxios/state/personas/index.json`
- **스키마**: `{ "schema_version": 1, "active_persona_id": "dev", "personas": [Persona, ...] }`
- **이유**:
  1. cron (`cron.rs:527-534`), budget (`budget.rs:381-395`), token-maxing (`maxer.rs:379-381`), supervisor (`supervisor.rs:567-570`), knowledge-saves (`persistence_hook.rs:286-288`), email (`kernel_handle/email_api.rs:155-157`) — 모두 같은 패턴. 페르소나만 다르면 일관성이 깨짐.
  2. `engine_api.rs:1254-1258` 의 plain `fs::write` 는 anti-pattern 이지만, `StateStore::durable_write` 는 그 *수정된* 표준 패턴. 같은 코드베이스 안에서 best practice 가 두 개가 되면 안 됨.
  3. 수동 편집 친화성을 위해 TOML 을 살리고 싶다면 `serde` 가 양쪽 다 지원하므로 `index.toml` 로 바꾸는 것은 같은 변경 면적. **이번 RFC 는 JSON 으로 시작하고, hand-edit 필요성이 드러나면 TOML 마이그레이션은 별도 RFC.**

**예외**: 사용자가 수동으로 `personas.toml` 을 손볼 일이 있는가? — 조사 결과 **그 어디에도 그 의도가 발견되지 않음**. UI/CLI/agent 모두 `PersonaApi` 경유.

### 3.2 Manager 시그니처 — `&self` 인터페이스 유지

`PersonaManager` 는 항상 `Arc` 뒤에 있다 (`persona_api.rs:9`, `agent_runtime.rs:200`, `persona_tool.rs:49`). 기존 `set_active_persona(&self, ...)` (`manager.rs:53`) 가 interior mutability 로 `&self` 를 쓰므로 신규 메서드도 동일.

```rust
impl PersonaManager {
    /// 빈 매니저. 디스크 로드는 `load_from_state_store` 로 별도.
    pub fn new() -> Self { ... }

    /// StateStore 에서 로드. 없거나 손상되면 기본 페르소나 생성.
    pub async fn load_from_state_store(&self, store: &StateStore) -> Result<()> { ... }

    /// Config.default_persona_id 적용. 디스크 meta 가 이미 덮어썼다면 그것 우선.
    pub fn apply_config(&self, cfg: &PersonaConfig) { ... }

    /// 디스크 저장. 실패는 Result 로 propagate, 메모리 상태는 유지.
    pub async fn persist(&self, store: &StateStore) -> Result<()> { ... }

    /// 글로벌 활성. 성공 시 intent engine 에도 재시드 (3.6).
    pub async fn set_active(&self, id: &str, intent_engine: &IntentEngine) -> Result<()> { ... }
}
```

### 3.3 부팅 호출 지점 — `src/kernel.rs:880`

**확정 변경**: `PersonaManager::new()` 단독 호출을 다음으로 교체.

```rust
// src/kernel.rs:880 (현재)
let persona_manager = PersonaManager::new();
if let Some(p) = persona_manager.first_enabled() {
    intent_engine.set_persona_prompt(Some(p.system_prompt));
    tracing::info!(persona = %p.name, "Active persona set on engines");
}
```

**변경 후**:

```rust
let persona_manager = PersonaManager::new();
if let Err(e) = persona_manager.load_from_state_store(&state_store).await {
    tracing::error!(error = %e, "persona load failed; falling back to defaults");
    // 실패는 propagate 하지 않음. defaults 는 new() 안에서 이미 생성됨.
}
persona_manager.apply_config(&config.persona);
if let Some(p) = persona_manager.first_enabled() {
    intent_engine.set_persona_prompt(Some(p.system_prompt));
    tracing::info!(persona = %p.name, "Active persona set on engines");
}
```

`state_store` 는 `src/kernel.rs` 부팅 초반에 이미 생성되어 있음 (확인됨). `intent_engine` 도 line 869 에서 이미 만들어져 있어 순서 의존성 없음.

### 3.4 우선순위

활성 페르소나 결정 우선순위 — `apply_config` 와 `load_from_state_store` 양쪽 모두 다음 규칙:

1. StateStore `index.json` 의 `active_persona_id` (enabled 면 적용)
2. `PersonaConfig.default_persona_id` (enabled 면 적용)
3. store 의 첫 번째 enabled

`set_active` 성공 시 → `active_persona_id` 를 StateStore 에 즉시 flush.

### 3.5 페르소나 role → 모델 라우팅 (RFC-032 재사용) — 변경 있음

**이전 초안은 "변경 없음"으로 잘못 기술됨**. 코드 검증 결과 페르소나 role 은 모델 해석 경로에 닿지 않음:

- `execute_inner(role: Option<&str>)` 의 `role` 인자는 **WS 클라이언트가 보낸 per-message 힌트** (`execute_directive_with_session` → `env.role.as_deref()`, `agent_runtime.rs:311`) 이지 페르소나 role 이 아님.
- 페르소나 role 은 351-354 에서 *별도로* 추출되어 `resolve_cspace(cspace_hint, persona_role, default="worker")` (357-362) 로만 흐름 — 모델 해석 단계까지 닿지 않음.
- 결과: 현재 코드는 페르소나 role 을 모델 선택에 전혀 사용하지 않음. 결함 #4 가 그대로 남음.

**수정**: `execute_inner` 의 모델 해석 직전에 effective_role 계산. WS hint > 페르소나 role > 없음.

```rust
// agent_runtime.rs:495-498 부근
let persona_role = self
    .persona_manager
    .as_ref()
    .and_then(|pm| pm.get_active_persona().map(|p| p.role.clone()));

let effective_role = role.or(persona_role.as_deref());

let model_id = model_override
    .map(|s| s.to_string())
    .or_else(|| effective_role.and_then(|r| self.kernel_handle.engine.model_for_role(r)))
    .unwrap_or_else(|| engine.default_model_id().to_string());
```

**우선순위 (수정 후)**:

1. `model_override` (호출 측 명시 override, recovery 등)
2. `env.role` (WS 클라이언트의 per-message role hint) — *명시적이므로 페르소나 role 보다 우선*
3. 페르소나 `role` (`engine.role_routing[role]` lookup)
4. `engine.default_model_id()`

`Persona::model` 필드:
- docstring 에 `/// DEPRECATED: use engine.role_routing[role] instead.` 주석.
- `agent_runtime` / `PersonaApi` / `PersonaTool` 어디서도 *읽지 않음* (현재 상태 유지).
- v2 schema_version 에서 제거 (이번 RFC 의 비-목표).

### 3.6 `set_active` 와 intent engine 재시드

`src/kernel.rs:881-883` 의 시드를 `set_active` 호출 시에도 재실행.

```rust
pub async fn set_active(&self, id: &str, intent_engine: &IntentEngine) -> Result<()> {
    let persona = self.store.get(id).ok_or_else(...)?;
    if !persona.enabled { bail!(...); }
    *self.active_persona_id.write() = Some(id.into());
    intent_engine.set_persona_prompt(Some(persona.system_prompt.clone()));
    self.persist(state_store).await?;
    Ok(())
}
```

`set_persona_prompt` 는 `IntentEngine` 메서드 (이미 존재). 호출자는 `PersonaTool::set_active` 와 `handle_persona_active_set` 양쪽.

### 3.7 Web UI ↔ 백엔드 라우트 정합

**변경 면적**:
- `web/src/components/command-palette/switch.tsx:53`:
  ```ts
  mutationFn: (id: string) =>
    api.put('/api/personas/active', { id }),
  ```
- `web/src/routes/personas.tsx:88`:
  ```ts
  mutationFn: (id: string) =>
    api.put('/api/personas/active', { id }),
  ```

백엔드 라우트 (`PUT /api/personas/active`) 와 1:1 정렬. `POST /:id/activate` 라우트는 추가하지 **않음** — 의도적 REST 단일 표면.

### 3.8 글로벌↔세션 활성 동기화

HTTP `PUT /api/personas/active` 핸들러 (`src/api/persona_routes.rs:221-237`) 안에서:

```rust
state.kernel.persona.set_active(&body.id, intent_engine.as_ref())
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

// 글로벌 active 갱신 후 새 세션의 기본값으로 흘리도록
// Session.active_persona_id 는 세션 생성 시점에 글로벌 active 로 채워짐.
// 별도 write 없음 (v1 단순화).
```

**v1 단순화 이유**: `Session.active_persona_id` 필드는 RFC-015 의 일부로 보존되어 있지만, 다중 세션 동시 override 약속은 코드/문서 어디에도 없음. 글로벌 = 세션 active 로 통일하고, 필드는 시리얼라이즈 호환을 위해 유지.

### 3.9 비대칭 위협 모델 — docstring 명시

`tools/builtin/persona_tool.rs:1-10` 의 docstring 에 다음을 추가:

```
//! ## Security review asymmetry
//!
//! Agent-authored `create`/`update` runs the LLM judge (`security_review`)
//! because the system_prompt is injected into future agent sessions. The
//! HTTP API path (`/api/personas*`) intentionally skips this review:
//! direct user edits via the Web UI / CLI are user-trusted context.
//! This split is documented in RFC-039 §3.9.
```

`src/api/persona_routes.rs:1` 에도 같은 내용.

## 4. 변경 파일 매니페스트

| 파일 | 변경 | 줄 범위 (approx) |
|---|---|---|
| `crates/oxios-kernel/src/persona/manager.rs` | 시그니처/메서드 추가/변경 | 신규 ~120 줄 |
| `crates/oxios-kernel/src/persona/store.rs` | docstring 정정 | 1-4 |
| `crates/oxios-kernel/src/persona/mod.rs` | "multiple simultaneously" 문구 제거 | 16 |
| `crates/oxios-kernel/src/config.rs` | `max_concurrent_personas` 제거 | 1600-1607, 1613 |
| `crates/oxios-kernel/src/agent_runtime.rs` | `effective_role` 계산 + 모델 해석 우선순위 (§3.5) | 350-362, 495-498 |
| `src/kernel.rs` | 부팅 시퀀스 변경 | 880-884 |
| `src/api/persona_routes.rs` | intent engine 재시드, docstring 추가 | 221-237, 1 |
| `web/src/components/command-palette/switch.tsx` | PUT 라우트 | 53 |
| `web/src/routes/personas.tsx` | PUT 라우트 | 88 |
| `docs/CHANGELOG.md` | 변경 기록 | 신규 항목 |

## 5. 통합 테스트 (Tester 위임)

1. **round-trip**: `load_from_state_store` → create → `persist` → 새 매니저 `load_from_state_store` → 동일 페르소나 + 동일 active_persona_id.
2. **첫 부팅**: StateStore 파일 부재 → 기본 3개 + "dev" 활성.
3. **schema_version=99**: 에러 propagate, silent fallback 없음.
4. **`apply_config` 우선순위 4가지**: (디스크+config, 디스크만, config만, 둘 다 없음).
5. **`set_active` 실패 케이스**: enabled=false → `Err`, 슬롯 변경 없음, intent engine 미변경, persist 호출 안 됨.
6. **intent engine 재시드**: `set_active` → `intent_engine.set_persona_prompt` 가 새 system_prompt 로 호출됨.
7. **effective_role 우선순위**: (a) 페르소나 role="qa" + `engine.role_routing.roles["qa"] = opus`, `env.role=None` → opus 해석. (b) `env.role="developer"` (WS 명시) + 페르소나 role="qa" → `engine.role_routing.roles["developer"]` 우선 (없으면 persona fallback). (c) 둘 다 None → default_model. (d) 페르소나 role 변경 직후 다음 `agent_runtime.execute` 가 새 role 의 모델을 해석하는지 확인.
8. **HTTP ↔ PersonaTool 비대칭**: HTTP create 는 judge LLM 호출 없음, PersonaTool create 는 호출함 — 메트릭/로그로 구분.
9. **Web UI 라우트 정합**: `PUT /api/personas/active` 본문 `{id}` 가 200, `POST /api/personas/:id/activate` 는 405.
10. **부팅 호출 시퀀스**: `kernel.rs:880` 변경 후 defaults→load→apply_config 순서로 active 가 결정됨.
11. **TOML 손상**: StateStore 가 손상된 JSON → `Result::Err`, 새 매니저는 기본 페르소나로 시작 (단, 사용자에게 에러 propagate).

## 6. 위험과 그 완화

| 위험 | 빈도 | 완화 |
|---|---|---|
| StateStore IO 실패 | 낮음 | durable_write 가 atomic. 실패 시 메모리는 유지, audit log 에 실패 기록 |
| `apply_config` 와 `load_from_state_store` 호출 순서 오류 | 중간 | 부팅 코드에서 명확한 순서 박제, 테스트 #10 |
| 다중 컴포넌트가 `set_active` 동시 호출 | 낮음 | RwLock<Option<String>>. 활성 슬롯만 보호, intent engine.set_persona_prompt 는 last-write-wins (이미 그 동작) |
| TOML 손상 시 silent fallback | 중간 | `Result::Err` 로 propagate, 기본 페르소나는 new() 가 이미 만들어 두어 다음 명령은 동작. 단 audit log 에 명시적 에러. |
| HTTP ↔ PersonaTool 비대칭이 보안 사고로 보일 수 있음 | 정보 | §3.9 docstring 명시. 사용자가 직접 만든 페르소나는 본인이 책임 |

## 7. 단계별 작업 (구현 PR 순서)

1. `PersonaConfig.max_concurrent_personas` 제거 + docstring 정정 (단일 PR, 하위 호환 — `serde(default)`).
2. `PersonaManager::load_from_state_store` / `persist` / `apply_config` 추가. `src/kernel.rs:880` 시퀀스 교체.
3. `set_active(&self, id, intent_engine)` 로 시그니처 변경, intent engine 재시드.
4. `agent_runtime.rs:495-498` 에서 `effective_role = role.or(persona_role.as_deref())` 로 변경 (§3.5 — 결함 #4 해결).
5. Web UI 라우트 정정 (`POST → PUT`).
6. `PersonaTool::create/update` 와 HTTP 라우트 비대칭 docstring.
7. 통합 테스트 11 케이스.
8. CHANGELOG.

## 8. 한 문단 요약

이 RFC 는 페르소나 시스템의 8개 결함을 한 번에 정리합니다. 영속성은 `StateStore::save_json` 의 `durable_write` 패턴을 재사용하여 코드베이스의 기존 표준과 정렬하고, `default_persona_id` 는 부팅 시퀀스에서 실제로 적용되며, `max_concurrent_personas` 와 "multiple active simultaneously" 약속은 같은 PR 에서 제거해 표면과 코드를 일치시킵니다. 페르소나 `role` 은 `execute_inner` 의 `effective_role` 계산(`env.role.or(persona_role)`)을 통해 RFC-032 의 `EngineApi::model_for_role` 로 흘러 모델이 결정되며, Web UI 의 깨진 라우트는 백엔드의 PUT 한 곳으로 정합됩니다. `set_active` 호출 시 intent engine 의 system_prompt 도 재시드되어 활성 페르소나 변경이 즉시 반영되며, HTTP↔PersonaTool 의 비대칭 위협 모델은 docstring 으로 명시합니다. 다중 동시 활성, 세션별 override, `Persona::model` 필드 제거는 의도적으로 v2/RFC 로 미룹니다.