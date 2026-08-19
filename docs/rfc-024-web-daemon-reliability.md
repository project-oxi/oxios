# RFC-024: Web ↔ Daemon 연결 신뢰성

> **Status:** Accepted
> **Created:** 2026-06-15
> **Depends on:** RFC-015 (chat transparency — WS 채널 기반), RFC-016 (autonomous persistence — 세션 저장 경로)
> **설계 근거:** `docs/designs/2026-06-15-web-daemon-reliability-design.md`

## Problem

백그라운드 데몬과 웹 UI 사이에 간헐적 불안정(특히 "가끔 404, 시간 지나면 회복")이 보고된다. 코드 검증 결과 6개의 구조적 원인을 식별했다.

### 근본 원인

응답·이벤트·자산이 **"전달됐다"는 보장이 없다.** 드롭·지연·재연결·중복이 처리되지 않는다.

### 6개 원인 (코드 검증 완료)

| # | 문제 | 위치 | 증상 매칭 | 웹 UI 영향 |
|---|------|------|-----------|------------|
| 1 | **비원자적 web dist 갱신 — 자동 2경로** | `src/kernel.rs:555` (`daily_health_check`, 새벽 3시 `remove_dir_all`+증분 해제), `system.rs:425` (`handle_update_run`, 수동) | **간헐 404 후 회복 — 직접 원인** | **실제 (핵심)** |
| 2 | SSE 클라이언트가 `response.ok` 미검사 | `web/src/lib/sse-client.ts` (`doConnect`) | 이벤트 단절. 단 `auth_enabled=false` 기본값이라 401 경로 자체가 희귀 | 희귀 (auth 활성 배포에서만) |
| 3 | `send_and_wait` 타임아웃 없음 + 게이트웨이 드롭 경로 무응답 | `bridge.rs` (`send_and_wait`), `gateway.rs:300,430` | 웹 UI 채팅은 **WS 전용**(`POST /api/chat` 미사용)이라 웹 UI 무관. 프로그래매틱 API 소비자 한정 | 없음 (웹 UI) |
| 4 | WebSocket ping/pong keepalive 없음 | `chat.rs` (`handle_chat_websocket`), `chat.ts` | 유휴 단절(프록시/NAT 60s), 비행 중 메시지 유실 | **실제** |
| 5 | broadcast lag 시 이벤트 조용히 드랍 | `routes/events.rs` (`Err(_) => None`), 용량 256 | 상태 어긋남, 클라이언트 모르게 유실 | **실제** |
| 6 | readiness 게이트 없음 + daemon spawn 미검증 | `plugin.rs` (bind 즉시 수용), `daemon.rs` (`start`가 리스닝 미확인) | 시작 직후 일시 불안정 | 경미 |

**증상 기여도:** 보고된 "가끔 404, 회복"의 핵심 기여자는 **#1**(자동 `daily_health_check`)이다. #4·#5는 부차적 실시간 문제. #2·#3은 기본 설정·웹 UI에서 영향이 제한적이나, 신뢰성 오버홀 범위에 포함하여 **auth 활성 배포**와 **프로그래매틱 API 소비자**까지 커버한다.

## Design Overview

### 핵심 불변조건 (전 설계가 지켜야 할 계약)

> **C1 (응답 보장):** 게이트웨이가 메시지를 accept했으면, deadline 내에 **반드시** OutgoingMessage(정상 또는 에러)가 도착한다.
>
> **C2 (순서 + 재생):** 모든 OutgoingMessage는 단조 증가 `seq`를 갖는다. 재연결 시 클라이언트가 `last_seq`를 주면 그 다음부터 빈틈없이 재생된다. 범위 초과 시 `resync` 신호 1개로 전체 pull.
>
> **C3 (멱등):** 같은 `msg.id`를 두 번 받아도 클라이언트는 한 번만 적용한다.
>
> **C4 (자산 무결):** serving 중인 정적 자산은 절대 404를 내지 않는다.

### 아키텍처

```
┌─────────────────────────────────────────────────────────────────┐
│                         Frontend (브라우저)                       │
│  SSE Client ──┐                              ┌── WS Client       │
│  (Last-Seq)   │                              │  (resume + ping)  │
│               ▼                              ▼                   │
└─────────────────────────────────────────────────────────────────┘
                  │                              │
                  │  HTTP/WS                     │
                  ▼                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  axum 서버 (surface/oxios-web)                                    │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────┐  │
│  │ ActiveWebDist│  │ ReadinessGate│  │ WebBridge              │  │
│  │ (atomic swap)│  │ 미들웨어 게이트│  │ incoming_tx / subscribe│  │
│  └─────────────┘  └──────────────┘  └───────────┬────────────┘  │
│         ▲                                         │              │
│         │                           send_and_wait(deadline)     │
└─────────┼─────────────────────────────────────────┼──────────────┘
          │                                         ▼
┌─────────┴─────────────────────────────────────────────────────────┐
│  Gateway (crates/oxios-gateway)                                    │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │  ReliabilityLayer  ★ 코어                                       │ │
│  │  ├─ SequenceCounter  (per-channel 단조 seq)                   │ │
│  │  ├─ ReplayBuffer     (인메모리 ring, 용량 N, TTL T)            │ │
│  │  └─ ResyncSignal     (커서 범위 초과 시 pull 유도)             │ │
│  └──────────────────────────────────────────────────────────────┘ │
│         │ assign_seq + buffer push → Channel::send()              │
│         ▼                                                          │
│   Orchestrator / Supervisor / ...                                  │
└────────────────────────────────────────────────────────────────────┘
```

### Key Decisions

| 결정 | 선택 | 근거 |
|------|------|------|
| **범위** | 전체 6개 안정성 오버홀 | 문제들이 서로 다른 신뢰성 축에 걸쳐 있어 부분 수정 시 빈틈 남 |
| **접근법** | **전달 프로토콜 재설계** (시퀀스 + 커서 재연결 + idempotency) | 근본 원인(전달 보장 부재) 치유. 메커니즘 개별 패치는 증상 치료에 그침 |
| **재생 저장소** | **하이브리드** (인메모리 ring + 커서 범위 초과 시 resync 신호) | 성능(인메모리) + 견고함(resync) 절충. 영속은 future scope |

### 서브프로젝트 분해

| # | 서브프로젝트 | 해결 문제 | 빌드 순서 |
|---|--------------|-----------|-----------|
| **SP1** | 전달 프로토콜 코어 (`ReliabilityLayer`) | 3, 4, 5 기반 | 1순위 (기반) |
| **SP2** | 채널 신뢰성 (SSE/WS) | 2, 4, 5 | SP1 다음 |
| **SP3** | 정적 자산 신뢰성 (atomic swap + immutable 캐시) | 1 | 독립 (병렬) |
| **SP4** | 라이프사이클 신뢰성 (readiness 게이트 + daemon 검증) | 6 | 독립 (병렬) |

---

## Part A: ReliabilityLayer (SP1)

### A1. 위치와 책임

`crates/oxios-gateway/src/reliability.rs` (신규). 게이트웨이가 채널에 메시지를 보내는 **모든 경로**를 래핑한다. `Channel` trait은 그대로 두고, Gateway가 `channel.send()` 직전에 레이어를 통과한다.

### A2. 데이터 모델 확장

`oxios_gateway::message::OutgoingMessage`에 필드 추가:

```rust
pub struct OutgoingMessage {
    pub id: Uuid,                 // 기존 — idempotency 키로 사용
    pub seq: Option<u64>,         // 신규 — ReliabilityLayer가 부여
    // ... 기존 필드
}
```

`seq`는 `Option`이므로 **구버전/테스트 메시지는 그대로 동작**한다 (C2 약화 없이 점진 적용).

### A3. 컴포넌트

```rust
pub struct ReliabilityLayer {
    /// per-channel 단조 시퀀스 (원자적)
    seq: AtomicU64,
    /// 인메모리 재생 버퍼 (하이브리드의 인메모리 절반)
    buffer: RwLock<RingBuffer<OutgoingMessage>>,
}

pub struct ReplayConfig {
    pub buffer_size: usize,   // 기본 512
    pub ttl_secs: u64,        // 기본 60
}

pub enum ReplayResult {
    /// last_seq 이후 메시지들 — 빈틈없이 재생
    Replay(Vec<OutgoingMessage>),
    /// last_seq가 버퍼 범위를 벗어남 — 클라이언트가 pull로 전체 리프레치
    Resync,
}
```

**핵심 메서드:**

```rust
impl ReliabilityLayer {
    /// 송신 경로: seq 부여 → 버퍼 push → 채널 전송
    pub async fn deliver(&self, channel: &dyn Channel, mut msg: OutgoingMessage) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        msg.seq = Some(seq);
        self.buffer.write().purge_expired(now);   // TTL 만료 정리
        self.buffer.write().push(msg.clone());     // 용량 초과 시 가장 오래된 것 eviction
        channel.send(msg).await;                   // 실제 전송
    }

    /// 재연결 경로: 하이브리드 정책
    pub fn replay(&self, last_seq: u64) -> ReplayResult {
        let buf = self.buffer.read();
        let oldest = buf.oldest_seq().unwrap_or(u64::MAX);
        if last_seq + 1 < oldest {
            ReplayResult::Resync                  // 커서가 너무 옛날 → pull
        } else {
            ReplayResult::Replay(buf.range_after(last_seq))
        }
    }
}
```

### A4. 응답 보장 (C1 이행, 문제 3 해결)

**`bridge.rs::send_and_wait`** — deadline 추가:

```rust
pub async fn send_and_wait(&self, msg: IncomingMessage) -> Result<OutgoingMessage> {
    // ... oneshot 등록 (기존)
    match tokio::time::timeout(timeout_duration(), rx).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(_)) => Err(anyhow!("response channel dropped")),
        Err(_) => {
            // 만료 시 correlation map에서 자신 제거 (누수 방지)
            self.responses.write().await.remove(&msg_id);
            Err(anyhow!("gateway response timeout"))
        }
    }
}
```

타임아웃 값은 config `gateway.response_timeout_secs` (기본 120초). HTTP 라우트는 이 에러를 **504 Gateway Timeout**으로 매핑 (`AppError`에 변종 추가).

**`gateway.rs` 드롭 경로 수정** — C1 계약 이행:

| 현재 코드 | 수정 |
|----------|------|
| `permit.acquire()` 실패 → `return` (무응답) | `ReliabilityLayer::deliver(channel, error_resp)` 후 return |
| `(_, None)` 채널 없음 → `warn!`만 | 이 경우는 거의 발생 안 함(자기 채널이므로)이나, 발생 시 correlation map에서 deadline이 잡음. 추가로 `warn!`에 request_id 로깅 강화 |

### A5. 인터페이스 요약

```rust
// crates/oxios-gateway/src/reliability.rs
pub struct ReliabilityLayer { /* ... */ }
impl ReliabilityLayer {
    pub fn new(config: ReplayConfig) -> Self;
    pub async fn deliver(&self, channel: &dyn Channel, msg: OutgoingMessage);
    pub fn replay(&self, last_seq: u64) -> ReplayResult;
    pub fn next_seq(&self) -> u64;  // 테스트/진단용
}
```

---

## Part B: 채널 신뢰성 — SSE/WS (SP2)

### B1. SSE 서버 (`routes/events.rs`)

- **재연결 핸드셰이크:** 표준 `Last-Event-ID` 헤더 파싱 → `ReliabilityLayer::replay()`:
  - `Replay(msgs)` → 연결 직후 각 메시지를 SSE `id: <seq>` 이벤트로 플러시 후 live 스트림으로 전환
  - `Resync` → `{type:"resync"}` 이벤트 1개 전송 → 클라이언트가 `/api/status` 등 pull 후 정상 재개
- **lag 처리 (문제 5):** `BroadcastStream::Lagged`를 조용히 무시(`None`)하는 대신 `Resync` 신호로 변환. 클라이언트는 빈틈을 안다.
- **이벤트 식별:** 모든 SSE 이벤트에 `id: <seq>\n` 라인 추가.
- keepalive ping 30초는 기존 유지.

### B2. SSE 클라이언트 (`web/src/lib/sse-client.ts`) — 문제 2 해결

```ts
private async doConnect(...) {
  const response = await fetch(url, {
    headers: {
      Authorization: `Bearer ${token}`,
      // fetch(≠ EventSource)이므로 브라우저가 자동으로 안 보냄 —
      //   클라이언트가 lastSeq를 추적해 수동으로 헤더 송신.
      //   EventSource는 Authorization 헤더를 못 넣어 채택 불가.
      'Last-Event-ID': String(this.lastSeq ?? 0),
    },
    signal: this.controller!.signal,
  })

  // response.ok 검사 (기존엔 없었음)
  if (!response.ok) {
    if (response.status === 401 || response.status === 403) {
      this.transitionTo('unauthorized')             // 재시도 안 함
      return
    }
    // 5xx 등은 기존 backoff 재연결
    this.scheduleReconnect()
    return
  }
  // ... 기존 스트림 읽기, 단 data 파싱 시 seq 기록 → this.lastSeq
}
```

**연결 상태 모델** (zustand store에 추가): `connecting | connected | reconnecting | unauthorized | dead`

`resync` 이벤트 수신 시 → 전역 상태 pull (`/api/status`, `/api/sessions/*`) 후 `lastSeq` 리셋.

### B3. WebSocket 서버 (`routes/chat.rs::handle_chat_websocket`) — 문제 4 해결

- **핸드셰이크 resume:** 첫 프레임 `{type:"resume", last_seq: N}` 지원. 미전송 시(구버전 클라이언트) live-only로 동작 (점진적).
- **keepalive (서버):** 20초마다 `{type:"ping"}` 전송. 60초 내 `pong` 없으면 연결 종료 → 클라이언트 재연결 트리거.
- 재연결 시 `ReliabilityLayer::replay()` 결과를 동일 커넥션에서 재생.
- 이미 per-conn 라우팅(`target_conn_id`)이 있으므로 replay는 해당 conn에만.

### B4. WebSocket 클라이언트 (`web/src/stores/chat.ts`)

- `ws.onclose` 시 `lastSeq`를 `sessionStorage`에 저장 → 재연결 시 첫 프레임 resume.
- `onmessage`에서 각 chunk의 `seq` 추적 → `lastSeq` 갱신.
- 25초마다 `{type:"ping"}` 송신 (서버 ping과 독립적 양방향).
- `onerror` 시 상태 분기: unauthorized면 재시도 중단, 그 외 backoff (기존 5회 유지).
- idempotency: 처리한 `msg.id` Set 유지(최근 N개), 중복 chunk 무시.

---

## Part C: 정적 자산 신뢰성 (SP3, 문제 1)

### C1. 활성 디렉토리 atomic 핀닝

`AppState`의 `web_dist: Option<PathBuf>`를 atomic 스왑 가능한 핸들로 교체:

```rust
use arc_swap::ArcSwapOption;  // 신규 의존성

pub struct AppState {
    pub web_dist: Arc<ArcSwapOption<PathBuf>>,  // 기존 Option<PathBuf> 대체
    // ...
}
```

`serve_file`은 매 요청 **포인터만 로드**(O(1), 디스크 I/O 없이 활성 디렉토리 확인)한 뒤 해당 디렉토리에서 파일을 읽는다. 파일 자체의 디스크 읽기는 유지되지만(로컬 파일이라 비용 미미), **브라우저 immutable 캐시로 대부분의 재요청이 클라이언트에서 해결**된다 (C3절). 서버가 매번 읽는 구조는 auto-update 호환성을 위해 그대로 두되, 404 원인(비원자 덮어쓰기)만 제거한다.

### C2. 원자적 업데이트 — 두 갱신 경로 모두

현재: serving 중인 `~/.oxios/web/dist/`에 직접 파일 덮어쓰기 → 404 윈도우.

수정:
```rust
// 1. 임시 디렉토리에 풀기 (예: ~/.oxios/web/dist.new.<rand>/)
let staging = dest_dir.with_extension(format!("new.{}", rand_suffix));
extract_zip_into(&staging, &bytes)?;

// 2. 검증: index.html 존재 + 최소 자산 존재
if !staging.join("index.html").is_file() {
    bail!("extracted dist missing index.html");
}

// 3. atomic swap: 포인터만 교체
state.web_dist.store(Some(Arc::new(staging.clone())));

// 4. 구버전 보존 (TTL 또는 2세대) — 비행 중 요청이 마무리되도록
//    백그라운드 태스크가 5분 후 이전 디렉토리 정리
tokio::spawn(cleanup_old_dist_dirs(...));
```

**C4 무결성 — 두 갱신 경로 모두 적용:**
- `handle_update_run` (수동, `system.rs:425`)
- `daily_health_check` (자동 새벽 3시, `src/kernel.rs:555`) — **이 경로가 보고된 404의 실제 원인**이므로 반드시 포함. 현재 `remove_dir_all` 후 증분 해제하는데, 동일하게 staging + swap으로 전환.

포인터가 가리키는 디렉토리는 항상 온전(풀린 후 swap). serving 중 삭제되지 않음 → **404 불가**.

### C3. 내장(embedded) fallback 상호작용 — 3-소스 404 벡터

정적 자산 출처는 **3개**다: (1) 파일시스템 활성 dist, (2) 바이너리 내장(`rust-embed`), (3) 양자의 버전 불일치. atomic-swap은 (1)의 레이스를 고치지만 **(3)을 별도로 처리**해야 한다:

- 브라우저가 활성 dist 버전의 해시를 참조하는 HTML을 캐시 → 어느 순간 내장 fallback으로 떨어지면 내장은 컴파일 시점 해시라 미스매치 → **404**.
- **해결:** `serve_file`의 fallback 체인을 단순화한다 — 활성 dist가 존재하면 **내장 fallback을 끈다**(활성 디렉토리 안에서만 해결). 활성 dist가 `None`(시작 시 다운로드 실패 등)일 때만 내장을 쓴다. 이렇게 하면 한 요청이 두 소스를 섞지 않아 해시 일관성이 보장된다.
- 부가: `index.html` 응답에 현재 활성 버전(`<dist>/version.json` 기반)을 `X-Web-Version` 헤더로 노출 → 클라이언트가 버전 전환을 감지해 캐시를 버리고 강제 리로드. 이것이 3-소스 미스매치의 마지막 빈틈을 막는다.

### C4. 캐시 정책 재설정

| 자산 | 현재 | 수정 | 근거 |
|------|------|------|------|
| `/index.html` | `no-cache` | `no-cache` (유지) | 항상 최신 포인터의 HTML |
| `/assets/*` (해시 파일명) | `no-cache` | **`public, max-age=31536000, immutable`** | content-addressed → 파일명이 바뀌므로 immutable 안전. auto-update 시 새 해시 파일명 |

이 조합이 **auto-update와 캐시를 양립**시킨다: HTML이 새 포인터를 가리키고, 자산은 영구 캐시되지만 해시가 바뀌어 자연스럽게 갱신.

### C5. 시작 시 다운로드 (`src/web_dist.rs::ensure_web_dist`)

이미 bind 전에 실행되므로 레이스 없음. 다만 동일한 `remove_dir_all` 패턴을 staging 방식으로 통일 (일관성).

---

## Part D: 라이프사이클 신뢰성 (SP4, 문제 6)

### D1. Readiness 게이트

`kernel.rs`에 게이트 추가:

```rust
pub struct ReadinessGate {
    state_store_ready: AtomicBool,
    engine_ready: AtomicBool,
}
impl ReadinessGate {
    pub fn is_ready(&self) -> bool {
        self.state_store_ready.load(SeqCst) && self.engine_ready.load(SeqCst)
    }
}
```

커널 어셈블러가 각 서브시스템 초기화 완료 후 해당 플래그 `store(true)`.

### D2. 실패/비정상 경로 (영구 블록 방지)

`engine_ready`가 세팅 안 되는 경우(API 키 없음, 엔진 초기화 실패) 게이트가 영원히 not-ready → `/api/*` 전체 영구 503이 되면 안 된다.

- **초기화 결과 모델:** 각 서브시스템은 `ready | degraded(reason) | failed(reason)` 세 상태를 갖는다.
- `is_ready()` = (state_store.ready) && (engine ∈ {ready, degraded}). 즉 **엔진 degraded(예: 키 없지만 폴백 모델 사용)는 ready로 간주** — 채팅 불가능 상태가 ready를 막지 않음.
- 엔진 `failed`(초기화 자체 실패)일 때만 not-ready. 단 `/api/status`·`/api/engine/*`는 **예외 허용**하여 사용자가 진단·수정 가능.
- ready 전환에 **데드라인**(기본 30s) — 데드라인 내 ready/degraded 못 하면 degraded로 강제 전환 + 로그 경고. 영구 멈춤 방지.

### D3. readiness 미들웨어

`routes/mod.rs`의 보호된 API 그룹에 레이어 추가:

```rust
.layer(axum::middleware::from_fn_with_state(
    state.readiness.clone(),
    require_ready,  // ready 전 → 503 + Retry-After: 2
))
```

제외: `/health`, `/health/ready`, `/metrics`, 정적 자산, SPA. (이들은 ready 전에도 접근 가능해야 함 — `/health/ready` 자체가 검사 도구이므로.)

### D4. 데몬 시작 검증 (`daemon.rs::start`)

현재: 자식 spawn → PID 기록 → 즉시 "started" 출력.

수정:
```rust
let pid = child.id();
self.write_pid(pid)?;

// 리스닝 검증: /health가 200(또는 /health/ready가 응답)할 때까지 폴링
let ready = self.wait_until_listening(port, Duration::from_secs(15));
match ready {
    Ok(()) => println!("⬡ oxios started (PID {pid}) — ready"),
    Err(_) => {
        println!("⬡ oxios started (PID {pid}) — still warming up");
        println!("  Dashboard: http://127.0.0.1:4200 (may take a few seconds)");
    }
}
```

`wait_until_listening`은 TCP connect 시도(또는 `/health` HTTP GET)를 200ms 간격으로 폴링. 포트 바인드 실패(소켓 TIME_WAIT 등)를 즉시 감지.

---

## Build Order

```
  ┌─────────────────────────────────┐
  │ SP1: ReliabilityLayer 코어       │  ← 모든 실시간 신뢰성의 기반
  │  (gateway crate, OutgoingMessage │
  │   확장, deliver/replay)          │
  └────────────┬────────────────────┘
               │ 의존
               ▼
  ┌─────────────────────────────────┐
  │ SP2: 채널 신뢰성 (SSE/WS)        │  ← SP1 위에 올림
  │  (서버 핸드셰이크 + 클라이언트   │
  │   response.ok/keepalive/resume)  │
  └─────────────────────────────────┘

  ┌──────────────────┐   ┌──────────────────────┐
  │ SP3: 정적 자산    │   │ SP4: 라이프사이클     │  ← 독립, SP1/2와 병렬
  │ (atomic swap +   │   │ (readiness 게이트 +  │
  │  immutable 캐시) │   │  daemon 검증)         │
  └──────────────────┘   └──────────────────────┘
```

**권장 순서:** SP3, SP4 먼저(독립·빠른 승리·가시적) → SP1 → SP2.

각 SP는 별도 PR. SP1은 gateway crate에 private API 추가이므로 게이트웨이 테스트에 영향 최소.

> **참고 (SP1 우선순위):** 보고된 증상(404)의 대부분은 SP3(C2의 `daily_health_check` 수정) 단독으로 해결된다. SP1(replay 버퍼)은 "재연결 중 누락 없음"이라는 요구에 대한 근본 대응이지만 증상 대비 오버킬일 수 있다. 구현 시 SP3·SP4·SP2-lite를 먼저 끝내고, 실제 수요(재연결 중 누락 불만 등)가 확인되면 SP1을 도입하는 것도 유효한 경로다. 본 RFC는 전체 범위를 규정하되, 도입 순서는 운영 판단에 맡긴다.

---

## Configuration (`config.toml`)

```toml
[gateway]
# SP1: 응답 보장 타임아웃 (send_and_wait deadline)
response_timeout_secs = 120

[gateway.reliability]
# SP1: 재생 버퍼 (하이브리드의 인메모리 절반)
replay_buffer_size = 512      # 채널당 보관 메시지 수
replay_ttl_secs = 60          # 버퍼 보관 시간

[gateway.web]
# SP3: 정적 자산
asset_cache_max_age_secs = 31536000   # immutable 자산
keep_old_dist_gens = 1                # 이전 dist 보존 세대수
```

기존 `event_bus_capacity = 256`은 유지 (SSE lag은 이제 resync로 처리되므로).

---

## Testing Strategy

### 단위 테스트

- **ReliabilityLayer**: seq 단조성, ring eviction, TTL 만료, `replay()` 범위 내/외 임계(resync 전환점) 정확성, 동시 deliver 스트레스(원자성).
- **ReadinessGate**: 플래그 토글에 따른 `is_ready()` 전이.

### 통합 테스트 (workspace `tests/`)

- **C1 응답 보장:** permit 고갈 상태에서 POST `/api/chat` → 503 도달(hang 아님). 오케스트레이터 panic → 에러 응답 도달. deadline 초과 → 504.
- **C2 재생:** SSE 구독 → 연결 끊기 → 중간 이벤트 발생 → `Last-Event-ID`로 재연결 → 누락 없이 수신. 커서가 TTL 초과 → `resync` 이벤트 수신.
- **C3 멱등:** 같은 `msg.id` 두 번 전송 → 클라이언트 적용 1회.
- **C4 자산 무결:** 업데이트 도중 100개 동시 `/assets/*` 요청 → 0건 404 (기존엔 404 발생).
- **SP4:** 커널 ready 전 `/api/status` → 503, ready 후 200. daemon `start` 출력에 "ready"/"warming up" 반영.

### 카오스/스트레스

- WS 연결 60초 유휴 → keepalive 단절 없이 유지(기존엔 60초 컷).
- 네트워크 500ms 지연 주입 → resync 발생 후 상태 일치.
- 백그라운드 탭(SSE throttle)에서 5분 방치 → 복귀 시 resync → 상태 일치.

---

## Observability (메트릭)

"불안정하다"를 측정 가능하게. `/metrics` (Prometheus)에 추가:

| 메트릭 | 유형 | 의미 |
|--------|------|------|
| `gateway_messages_total{result}` | counter | delivered / dropped / resynced / timed_out |
| `gateway_response_duration_seconds` | histogram | `send_and_wait` 지속시간 |
| `gateway_replay_requests_total{outcome}` | counter | replay / resync |
| `sse_reconnects_total{reason}` | counter | ok / lag / error / unauthorized |
| `ws_reconnects_total{reason}` | counter | 동일 |
| `web_dist_swaps_total` | counter | atomic swap 발생 횟수 |
| `readiness_state` | gauge | 0=warming, 1=ready |

이 메트릭이 있어야 개선 효과를 객관적으로 검증한다.

---

## Non-Goals (명시적 범위 제한)

- **정확히 한 번(exactly-once)은 아니다.** at-least-once(재생) + idempotent(클라이언트 dedup) = **effectively-once**. 네트워크 분할 중 중복은 허용, 클라이언트가 걸러낸다.
- **영속 재생은 안 한다.** 데몬 재시작 후 커서는 무효(resync). 영속 큐(Kafka-style)는 future scope.
- **메시지 압축/배치는 안 한다.** 지금은 신뢰성 우선.
- **인증 토큰 갱신 플로우는 안 다룬다.** 만료 토큰 → unauthorized 상태 전이만. 재발급은 별도.
- **브라우저 호환성 타겟:** `Last-Event-ID`, `AbortController`, `sessionStorage` 지원하는 최신 브라우저. (현재 타겟과 동일.)

---

## Risks & Mitigations

| 리스크 | 영향 | 완화 |
|--------|------|------|
| ReliabilityLayer가 모든 send 경로에 끼어듦 → 오버헤드 | 성능 | ring + atomic이라 낮음. 벤치마크로 회귀 확인. |
| WS 프로토콜 변경(resume 핸드셰이크) → 구버전 클라이언트 호환 | 프론트엔드 | resume 미전송 시 live-only fallback. 점진적. |
| `arc_swap` 신규 의존성 | 빌드 | Rust 생태계 표준, 가벼움. workspace deps 추가. |
| SSE 커서 헤더 | 호환성 | SSE 표준 `id:` 라인에 seq를 쓰고 재연결은 `Last-Event-ID`로 받음(커스텀 헤더 사용 안 함). 단 fetch 기반이라 클라이언트가 수동 송신. |
| readiness 미들웨어가 `/api/*` 전체 차단 → 과도 | 가용성 | `/health*`, 정적, SPA 제외. ready는 통상 1초 미만. 데드라인 강제 전환으로 영구 블록 방지. |
| TTL 만료 vs eviction 경계 조건 | 정확성 | 단위 테스트로 임계값 검증(resync 전환점). |
| 엔진 `failed` 상태에서 영구 not-ready | 가용성 | `/api/status`·`/api/engine/*` 예외 허용 + 데드라인 degraded 강제 전환. |

---

## Future Work (본 RFC 제외)

- 영속 재생 버퍼 (재시작 후 커서 유효) — StateStore 기반.
- 정확히 한 번 전달 (트랜잭션적 ack).
- 다중 데몬 인스턴스 시 게이트웨이 간 메시지 라우팅.
- 웹소켓 백프레셔 (느린 클라이언트 처리 정책).

---

## Implementation Checklist

- [ ] **SP1:** `OutgoingMessage.seq` 필드 + `ReliabilityLayer` (gateway crate)
- [ ] **SP1:** `send_and_wait` deadline + 드롭 경로 응답 보장
- [ ] **SP1:** `AppError` 504 변종 + 라우트 매핑
- [ ] **SP2:** SSE 서버 `Last-Event-ID`/replay/resync + lag → resync
- [ ] **SP2:** SSE 클라이언트 `response.ok` + 상태 모델 + Last-Seq
- [ ] **SP2:** WS 서버 resume 핸드셰이크 + ping/pong
- [ ] **SP2:** WS 클라이언트 lastSeq 저장/resume + ping + idempotency Set
- [ ] **SP3:** `ArcSwapOption<PathBuf>` 교체 + staging/swap 업데이트 **(2경로: `handle_update_run` + `daily_health_check`)**
- [ ] **SP3:** `serve_file` fallback 단순화(활성 dist 있으면 내장 끄기) + `X-Web-Version` 헤더
- [ ] **SP3:** 자산 immutable 캐시 헤더 + 구버전 정리
- [ ] **SP4:** `ReadinessGate` (ready/degraded/failed) + readiness 미들웨어 + 데드라인 강제 전환
- [ ] **SP4:** `daemon::start` 리스닝 검증
- [ ] 메트릭 7종 추가
- [ ] 통합 테스트 (C1~C4)
- [ ] config 스키마 + 마이그레이션

---

## Addendum (2026-08-19): The partial-replay gap is intentional

During the chat-UI defect remediation (design doc D5) the question "why are
streaming partials missing from the replay buffer?" was audited. The answer is
a deliberate boundary, now pinned so no future change re-breaks it:

**Partials (`partial: true` — per-delta `token` fragments, `reasoning`
deltas, `reasoning.start`/`reasoning.end` markers) skip `assign_seq`
(`gateway.rs` collector) and are therefore absent from the 512-entry replay
buffer.** A mid-stream reconnect recovers the terminal full-text message
(`done`/`error`, seq'd, terminal `token`) rather than resuming token flow.

This is correct because:

- the terminal carries complete text — no data is lost on reconnect;
- buffering partials would multiply the buffer's memory by the token count;
- FIFO order during live streaming comes from the mpsc channel, and each
  partial carries a unique `Uuid` for the frontend's seen-id ring, so live
  ordering needs no seq.

**Do not** assign seq to partials "for robustness" — the frontend re-merges
replayed deltas into the assistant message, which would duplicate/garbled
streamed text.
