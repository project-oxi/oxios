# Oxios Architecture Reference

> **Version:** 1.37.0 · **Stack:** Rust 2024, tokio, serde (JSON+TOML), oxicode-sdk · **License:** MIT

This document is a standalone reference for every subsystem in the Oxios Agent OS.
Read it before modifying kernel structure, adding modules, or onboarding onto the project.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Layer Architecture](#2-layer-architecture)
3. [Kernel Subsystems](#3-kernel-subsystems)
4. [KernelHandle Facade](#4-kernelhandle-facade)
5. [Data Flow](#5-data-flow)
6. [Dependency Graph](#6-dependency-graph)
7. [Security Model](#7-security-model)
8. [Unix Philosophy Mapping](#8-unix-philosophy-mapping)
9. [Dependency Rules](#9-dependency-rules)
10. [Kernel Crate Architecture](#10-kernel-crate-architecture)

---

## 1. Overview

### What is Oxios?

Oxios is an **Agent Operating System** — an OS where AI agents execute real work on behalf of users. Agents fork, exec, wait, and kill just like Unix processes. The user talks, the OS handles the rest. Users never see how many agents are running.

### Design Philosophy

Oxios is built on two foundational metaphors:

| Metaphor | Realization |
|----------|-------------|
| **Unix** | Process-style agent lifecycle (fork/exec/wait/kill, Supervisor as init). The metaphor is structural for lifecycle and analogical elsewhere — see §8. No containers, direct host execution. |
| **Ouroboros** | Clarify before acting. Unified intent flow: assess → crystallize → execute → review. Depth adapts to the task (RFC-027); the former 5-phase protocol (Interview → Seed → Execute → Evaluate → Evolve) is retired. |

### Key Principles

| Principle | Meaning |
|-----------|---------|
| **Intent-first** | Every message is assessed; substantial tasks crystallize into a directive before execution, then review against criteria |
| **No reimplementation** | Reuse `oxicode-sdk` (crates.io). Never reimplement what oxi already provides |
| **Channel agnostic** | Gateway doesn't care where messages come from (Web, CLI, Telegram) |
| **User invisible** | Users don't know how many agents are running |
| **Least privilege** | Agents start with minimal permissions, must be explicitly granted more |
| **Tamper-evident** | Every security-relevant action is recorded in a cryptographically-chained audit trail |

---

## 2. Layer Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         CHANNELS (User-facing)                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐              │
│  │   Web     │  │   CLI    │  │ Telegram │  │  (more)   │              │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └─────┬─────┘              │
│       │              │              │              │                    │
├───────┴──────────────┴──────────────┴──────────────┴────────────────────┤
│                         GATEWAY (oxios-gateway)                         │
│              Channel-agnostic message hub. Routes user messages         │
│              from any channel to the Orchestrator.                      │
│       ┌──────────────────┴───────────────────┐                         │
├───────┴──────────────────────────────────────┴─────────────────────────┤
│                         KERNEL (oxios-kernel)                           │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                    Orchestrator ("The Brain")                    │   │
│  │  Runs Ouroboros protocol end-to-end: Interview → Seed → Exec →  │   │
│  │  Evaluate → Evolve. Manages multi-agent delegation via A2A.     │   │
│  └───────────────────┬──────────────────────────────────────────────┘   │
│                      │                                                  │
│  ┌───────────────────┴──────────────────────────────────────────────┐   │
│  │                 Supervisor ("The Init")                          │   │
│  │  Agent lifecycle: fork / exec / wait / kill. Manages agent       │   │
│  │  task handles, cooperative cancellation, status tracking.        │   │
│  └───────────────────┬──────────────────────────────────────────────┘   │
│                      │                                                  │
│  ┌───────────┐  ┌────┴────┐  ┌──────────┐  ┌─────────────┐            │
│  │ Scheduler │  │Runtime  │  │EventBus  │  │ StateStore  │            │
│  └───────────┘  └─────────┘  └──────────┘  └─────────────┘            │
│  ┌────────────┐ ┌──────────┐ ┌──────────┐  ┌───────────────┐          │
│  │AccessMgr   │ │AuditTrail│ │BudgetMgr │  │ResourceMon    │          │
│  └────────────┘ └──────────┘ └──────────┘  └───────────────┘          │
│  ┌────────────┐ ┌──────────┐ ┌──────────┐  ┌───────────────┐          │
│  │MemoryMgr   │ │SpaceMgr  │ │McpBridge │  │GitLayer       │          │
│  └────────────┘ └──────────┘ └──────────┘  └───────────────┘          │
│  ┌────────────┐ ┌──────────┐ ┌──────────┐  ┌───────────────┐          │
│  │PersonaMgr  │ │SkillMgr  │ │CronScheduler        │   │
│  └────────────┘ └──────────┘ └────────────────────┘   │
│  ┌────────────┐ ┌──────────┐ ┌────────────────────────────┐           │
│  │AuthManager │ │CircuitBkr│ │A2AProtocol + CardRegistry  │           │
│  └────────────┘ └──────────┘ └────────────────────────────┘           │
│                                                                        │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                  KernelHandle (Facade / Syscall Table)           │   │
│  │  22 typed APIs (State · Agent · Security · Exec · Browser ·      │   │
│  │  MCP · A2A · Memory · Engine · …  — full list in §4)             │   │
│  └──────────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────┘
       │
├──────┴─────────────────────────────────────────────────────────────────┤
│                         RUNTIME (oxios-ouroboros)                       │
│  Spec-first protocol engine. Interview → Seed generation → Execution   │
│  result → Evaluation scoring → Evolution of failing seeds.             │
│  Defines Seed, Phase, ExecutionResult, EvaluationResult types.         │
└────────────────────────────────────────────────────────────────────────┘
       │
├──────┴─────────────────────────────────────────────────────────────────┤
│                         ENGINE (oxicode-sdk / oxicode-ai)                       │
│  Thin wrapper around oxicode_sdk::Oxicode. Provider/model resolution via       │
│  OxicodeBuilder. Uses oxicode-ai for provider construction (Anthropic,         │
│  OpenAI, etc.). AgentLoop provides multi-turn tool-calling loop.       │
└────────────────────────────────────────────────────────────────────────┘
```

### Layer Summary

```
  Channel (Web / CLI / Telegram)
        │
        ▼
  Gateway ─── routes messages, channel-agnostic
        │
        ▼
  Kernel ─── Orchestrator + Supervisor + all subsystems
        │
        ├── KernelHandle ─── typed facade (22 APIs)
        │       │
        │       └── AgentRuntime ─── wraps oxicode-sdk AgentLoop
        │
        ├── Ouroboros ─── spec-first protocol engine
        │
        └── Engine (oxicode-sdk) ─── LLM provider abstraction
```

---

## 3. Kernel Subsystems

### 3.1 Supervisor — Agent Process Management

The Supervisor is the "init" of Oxios. It manages agent lifecycles using Unix-like primitives.

```
                    ┌─────────────────────┐
                    │     Supervisor      │
                    │  (dyn trait: Send+Sync) │
                    └──────┬──────────────┘
                           │
          ┌────────────────┼────────────────┐
          │                │                │
          ▼                ▼                ▼
    ┌──────────┐    ┌──────────┐    ┌──────────┐
    │  fork()  │    │  exec()  │    │  kill()  │
    │ Create   │    │ Start    │    │ Cancel + │
    │ AgentId  │    │ running  │    │ abort    │
    └──────────┘    └──────────┘    └──────────┘
          │                │                │
          ▼                ▼                ▼
    ┌──────────────────────────────────────────┐
    │          AgentStatus Enum                │
    │  Starting → Running → Idle | Failed      │
    │                  → Stopped               │
    └──────────────────────────────────────────┘

    fork_directive(directive, env)         → AgentId
    exec(id: AgentId)                      → ()
    run_with_directive(id, directive, env) → ExecutionResult
    wait(id: AgentId)                      → AgentStatus
    kill(id: AgentId)                      → ()
    list()                                 → Vec<AgentInfo>
```

**Implementation (`BasicSupervisor`):**
- Maintains `HashMap<AgentId, AgentInfo>` for agent metadata
- Spawns each execution as a `tokio::task::JoinHandle` with cooperative cancellation via `AtomicBool`
- Kill sets the cancellation flag and aborts the tokio task
- Emits `AgentCreated`, `AgentStarted`, `AgentStopped`, `AgentFailed` events on the EventBus
- `NoOpSupervisor` placeholder breaks the `KernelHandle → AgentRuntime → Supervisor` circular dependency during build

**Source:** `crates/oxios-kernel/src/supervisor.rs`

---

### 3.2 Orchestrator — The Brain

The Orchestrator coordinates the full Ouroboros lifecycle for user messages. It is the top-level coordinator that does NOT know about channels or HTTP.

> **Note:** The diagram below describes the legacy 5-phase Ouroboros protocol
> (Interview → Seed → Execute → Evaluate → Evolve). The current implementation
> (RFC-027) uses **unified intent handling** (assess → crystallize → execute →
> review) with adaptive depth. The core concepts (Seed, Space detection,
> multi-agent delegation, evaluation) remain, but the phase names and flow have
> changed. This section will be updated to reflect the current protocol.

```
  User Message
       │
       ▼
  ┌────────────────────────────────────────────────────────┐
  │                    Orchestrator                         │
  │                                                        │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  1. Space Detection (3-layer)                    │  │
  │  │     Path → Keyword → Topic classification        │  │
  │  └──────────────────────────────────────────────────┘  │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  2. Chat Bypass                                  │  │
  │  │     Simple messages get direct LLM response       │  │
  │  └──────────────────────────────────────────────────┘  │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  3. Interview Phase                              │  │
  │  │     Clarify ambiguity → Q&A accumulation          │  │
  │  │     If ambiguity > 0.2: return questions          │  │
  │  └──────────────────────────────────────────────────┘  │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  4. Seed Generation                              │  │
  │  │     Create spec with goal, constraints, criteria  │  │
  │  └──────────────────────────────────────────────────┘  │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  5. Multi-Agent Split (if ≥3 acceptance criteria) │  │
  │  │     Split → delegate via A2A or lifecycle         │  │
  │  └──────────────────────────────────────────────────┘  │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  6. Execute Phase                               │  │
  │  │     spawn_and_run via AgentLifecycleManager       │  │
  │  └──────────────────────────────────────────────────┘  │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  7. Evaluate Phase                              │  │
  │  │     Score result against acceptance criteria      │  │
  │  └──────────────────────────────────────────────────┘  │
  │  ┌──────────────────────────────────────────────────┐  │
  │  │  8. Evolve Loop (max 3 iterations)              │  │
  │  │     If score < 0.8: evolve seed → re-execute     │  │
  │  └──────────────────────────────────────────────────┘  │
  └────────────────────────────────────────────────────────┘
       │
       ▼
  OrchestrationResult { response, session_id, agent_id,
                        phase_reached, evaluation_passed, primary_project_id }
```

**Multi-Agent Delegation:**
- When a seed has 3+ acceptance criteria, each becomes a subtask
- Subtasks run in parallel via `JoinSet`
- A2A protocol used when available (queries `AgentCardRegistry` for capable agents)
- Falls back to direct lifecycle execution without A2A

**Source:** `crates/oxios-kernel/src/orchestrator.rs`

---

### 3.3 AgentLifecycleManager — Full Orchestrated Lifecycle

Extracted from the Orchestrator to reduce scope. Handles the complete lifecycle of a single agent from fork to cleanup.

```
  Seed + Priority
       │
       ▼
  ┌─────────────────────────────────────────────────┐
  │          AgentLifecycleManager                   │
  │                                                  │
  │  1. fork()           → AgentId                   │
  │  2. register A2A     → AgentCard                 │
  │  3. deliver pending   → A2A messages             │
  │  4. ensure perms      → AccessManager            │
  │  5. submit+start task → Scheduler                │
  │  6. run_with_seed     → Supervisor → AgentRuntime│
  │  7. cleanup           → unregister A2A, complete │
  │                                                  │
  │  Timeout: max_execution_time_secs (configurable) │
  │  Zombie reaping via scheduler.reap_zombies()     │
  └─────────────────────────────────────────────────┘
       │
       ▼
  ExecutionResult { output, steps_completed, success }
```

**Key behaviors:**
- Wraps execution in `tokio::time::timeout` when `max_execution_time_secs > 0`
- Infers A2A capabilities from seed goal (e.g., "review" → "code-review")
- Always cleans up (A2A unregister + scheduler complete/fail) even on failure

**Source:** `crates/oxios-kernel/src/agent_lifecycle.rs`

---

### 3.4 AgentRuntime — Tool-Calling Loop

Wraps `oxicode_sdk::AgentLoop` for executing Seeds through the multi-turn LLM tool-calling loop.

```
  ┌──────────────────────────────────────────────────────┐
  │                    AgentRuntime                       │
  │                                                      │
  │  execute(agent_id, seed) → ExecutionResult           │
  │                                                      │
  │  ┌──────────────┐  ┌──────────────┐  ┌────────────┐ │
  │  │ Persona Mgr  │  │Tool Retriever│  │Memory Mgr  │ │
  │  │ (system      │  │(semantic     │  │(recall +   │ │
  │  │  prompt)     │  │ capability)  │  │ blend)     │ │
  │  └──────┬───────┘  └──────┬───────┘  └─────┬──────┘ │
  │         │                 │                │         │
  │         ▼                 ▼                ▼         │
  │  ┌──────────────────────────────────────────────────┐│
  │  │          System Prompt Assembly                  ││
  │  │  Goal + Constraints + Criteria + Persona +       ││
  │  │  Capability Index (XML) + Kernel Manifest +      ││
  │  │  Recalled Memories                               ││
  │  └──────────────────────┬───────────────────────────┘│
  │                         │                             │
  │  ┌──────────────────────┴───────────────────────────┐│
  │  │         CSpace Resolution                        ││
  │  │  persona role → seed hint → default "worker"     ││
  │  └──────────────────────┬───────────────────────────┘│
  │                         │                             │
  │  ┌──────────────────────┴───────────────────────────┐│
  │  │    Tool Registration (from CSpace)               ││
  │  │  Tier 1: Always-on (read, write, edit, grep,     ││
  │  │          find, ls, web_search)                    ││
  │  │  Tier 2: CSpace-driven (exec, memory, space,     ││
  │  │          agent, a2a, persona, cron, security,    ││
  │  │          budget, resource)                        ││
  │  │  Tier 3: Skill tools + MCP tools                  ││
  │  └──────────────────────┬───────────────────────────┘│
  │                         │                             │
  │  ┌──────────────────────┴───────────────────────────┐│
  │  │         oxicode_sdk::AgentLoop::run()                ││
  │  │  Multi-turn LLM tool-calling loop                ││
  │  │  Callbacks: ToolEnd, AgentEnd, Error, Compaction ││
  │  │  Runs inside spawn_blocking (AgentLoop !Send)    ││
  │  └──────────────────────────────────────────────────┘│
  └──────────────────────────────────────────────────────┘
```

**Circuit Breaker:** Global `CircuitBreaker` protects against cascading LLM provider failures.
Records success/failure after each agent execution.

**Source:** `crates/oxios-kernel/src/agent_runtime.rs`

---

### 3.5 Scheduler — Priority Task Queue

AIOS/AgentRM-inspired priority-based task queue.

```
  ┌───────────────────────────────────────────────────┐
  │                 AgentScheduler                     │
  │                                                   │
  │  ┌─────────────────────────────────────────────┐  │
  │  │            Priority Queue                    │  │
  │  │  BinaryHeap<ScheduledTask>                   │  │
  │  │                                              │  │
  │  │  Critical (3) > High (2) > Normal (1) > Low │  │
  │  │  Within same priority: newest first (LIFO)  │  │
  │  └─────────────────────────────────────────────┘  │
  │                                                   │
  │  ┌──────────────┐  ┌──────────────────────────┐   │
  │  │Rate Limiter  │  │  Max Concurrent Enforcer │   │
  │  │requests/min  │  │  (default: 5)            │   │
  │  └──────────────┘  └──────────────────────────┘   │
  │                                                   │
  │  ┌──────────────────────────────────────────────┐ │
  │  │  Zombie Detection                            │ │
  │  │  Tasks running > zombie_timeout_secs → reaped│ │
  │  │  default: 300s                               │ │
  │  └──────────────────────────────────────────────┘ │
  │                                                   │
  │  ┌──────────────────────────────────────────────┐ │
  │  │  Budget Manager Integration                  │ │
  │  │  Checks can_schedule() before starting task  │ │
  │  │  Skips tasks for exhausted agents            │ │
  │  └──────────────────────────────────────────────┘ │
  └───────────────────────────────────────────────────┘

  Task lifecycle:
  Queued → Running → Completed | Failed | Cancelled
```

**Source:** `crates/oxios-kernel/src/scheduler.rs`

---

### 3.6 StateStore — File-Based Persistence

JSON file storage for all kernel state.

```
  ┌──────────────────────────────────────────────┐
  │              StateStore                       │
  │                                              │
  │  Base path: ~/.oxios/workspace/              │
  │  ├── seeds/{id}.json          Seed specs     │
  │  ├── evals/{id}-eval.json     Evaluations    │
  │  ├── kernel.db                # Mount/project tables (KernelDatabase, SQLite WAL)
  │  ├── audit/trail.json         Audit entries  │
  │  ├── agent_groups/{id}.json   Group state    │
  │  └── {category}/{name}.json   Arbitrary data │
  │                                              │
  │  Operations:                                  │
  │  save_json(category, name, T)                 │
  │  load_json(category, name) → Option<T>        │
  │  save_markdown(category, name, content)       │
  │  delete(category, name) → bool                │
  │  save_audit_entries(entries)                  │
  │  load_audit_entries() → Vec<AuditEntry>       │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/state_store.rs`

---

### 3.7 EventBus — Broadcast Pipe

The "pipe" of Oxios. All agents and subsystems communicate through kernel events on the bus.

```
  ┌──────────────────────────────────────────────────────┐
  │                      EventBus                        │
  │            tokio::sync::broadcast channel             │
  │                                                      │
  │  Publishers:                                         │
  │  ┌────────────┐ ┌────────────┐ ┌─────────────────┐  │
  │  │ Supervisor │ │Orchestrator│ │ AgentLifecycle   │  │
  │  └─────┬──────┘ └─────┬──────┘ └────────┬────────┘  │
  │        │              │                  │            │
  │        └──────────────┼──────────────────┘            │
  │                       ▼                               │
  │              ┌────────────────┐                       │
  │              │  broadcast::   │                       │
  │              │    Sender      │                       │
  │              └───────┬────────┘                       │
  │                      │                                │
  │       ┌──────────────┼──────────────┐                 │
  │       ▼              ▼              ▼                 │
  │  ┌─────────┐  ┌───────────┐  ┌──────────────┐       │
  │  │Subscriber│  │Subscriber │  │AuditTrail    │       │
  │  │(monitor) │  │(channel)  │  │(auto-attach) │       │
  │  └─────────┘  └───────────┘  └──────────────┘       │
  └──────────────────────────────────────────────────────┘

  KernelEvent variants:
  AgentCreated · AgentStarted · AgentStopped · AgentFailed
  MessageReceived · SeedCreated · EvaluationComplete
  PhaseStarted · PhaseCompleted · AgentOutput
  ApprovalRequested · ApprovalResolved
  MemoryStored · MemoryRecalled
  AgentGroupCreated · AgentGroupMemberCompleted
  SpaceCreated · SpaceActivated · SpaceArchived
  SpacesMerged · KnowledgeCrossReferenced
```

**Auto-audit:** When `attach_audit_trail()` is called, a background task forwards all events to the AuditTrail as `AuditAction` entries.

**Source:** `crates/oxios-kernel/src/event_bus.rs`

---

### 3.8 AccessManager — RBAC + Path Sandboxing

OWASP-inspired least-privilege security for agents.

```
  ┌──────────────────────────────────────────────────────────┐
  │                    AccessManager                          │
  │                                                          │
  │  ┌────────────────────────────────────────────────────┐  │
  │  │              AgentPermissions                      │  │
  │  │  allowed_tools: HashSet<String>                    │  │
  │  │  allowed_paths: Vec<String> (glob patterns)        │  │
  │  │  denied_paths: Vec<String>  (takes precedence)     │  │
  │  │  network_access: bool                              │  │
  │  │  can_fork: bool                                    │  │
  │  │  max_execution_time_secs: u64                      │  │
  │  │  max_memory_mb: u64                                │  │
  │  └────────────────────────────────────────────────────┘  │
  │                                                          │
  │  ┌────────────────────────────────────────────────────┐  │
  │  │              RbacManager                           │  │
  │  │  Role-based access control with HitL approvals     │  │
  │  │  Subjects: Agent(UUID), User(String), Role(String) │  │
  │  │  Actions: ExecuteTool, AccessPath, Network, Fork   │  │
  │  │  Pending approvals with approve/reject workflow    │  │
  │  └────────────────────────────────────────────────────┘  │
  │                                                          │
  │  ┌────────────────────────────────────────────────────┐  │
  │  │           Workspace Sandboxing                     │  │
  │  │  workspace_paths: name → PathBuf                   │  │
  │  │  agent_workspaces: agent_name → workspace_name     │  │
  │  │  Path boundary enforcement via canonicalize()      │  │
  │  └────────────────────────────────────────────────────┘  │
  │                                                          │
  │  Full sandbox check:                                     │
  │  can_access_path_in_workspace(agent_id, name, path, ws) │
  │    → RBAC check                                          │
  │    → Path allow/deny check                               │
  │    → Workspace boundary check                            │
  │    → All three must pass                                 │
  │                                                          │
  │  Audit log: every access decision recorded               │
  │  Async file persistence via bounded mpsc channel (1000)  │
  └──────────────────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/access_manager/mod.rs`

---

### 3.9 AuditTrail — Tamper-Evident Hash Chain

Merkle-chain style cryptographic audit log. Each entry is linked to the previous via blake3 hashing.

```
  Genesis ──► Entry 1 ──► Entry 2 ──► Entry 3 ──► ...
              hash=abc    hash=def    hash=ghi
              prev=       prev=       prev=
              "genesis"   abc         def

  ┌─────────────────────────────────────────────────────┐
  │                   AuditEntry                         │
  │  seq: u64                                           │
  │  timestamp: DateTime<Utc>                           │
  │  actor: AgentId                                     │
  │  action: AuditAction (tagged enum)                  │
  │  resource: String                                   │
  │  prev_hash: blake3 hex digest                       │
  │  hash: blake3 hex digest                            │
  │  metadata: Option<JSON>                             │
  └─────────────────────────────────────────────────────┘

  AuditAction variants:
  AgentSpawn · AgentExit · ToolCall · ToolResult
  MemoryWrite · MemoryRead · ConfigChange · SkillInstall
  CronTrigger · GitCommit · AccessDenied · Other

  Integrity:
  verify() → checks:
    1. prev_hash chain linked correctly
    2. No future timestamps
    3. Hash recomputation matches stored hash
    4. First entry: prev_hash = "genesis" | "pruned"

  Auto-pruning:
  - When entries > max_entries: oldest pruned
  - Pruned root marked with prev_hash = "pruned"
  - No hash recomputation cascade (O(1))
```

**Source:** `crates/oxios-kernel/src/audit_trail.rs`

---

### 3.10 BudgetManager — Token/Cost Enforcement

Per-agent budget tracking for LLM API calls.

```
  ┌──────────────────────────────────────────────┐
  │             BudgetManager                     │
  │                                              │
  │  BudgetLimit per agent:                      │
  │  { agent_id, token_budget, calls_budget,     │
  │    window_secs }                             │
  │                                              │
  │  can_schedule(agent_id) → bool               │
  │  track_call(agent_id) → Result               │
  │  set_budget(BudgetLimit)                     │
  └──────────────────────────────────────────────┘
```

**Integration:** The Scheduler checks `can_schedule()` before popping tasks. Agents with exhausted budgets are skipped.

**Source:** `crates/oxios-kernel/src/budget.rs`

---

### 3.11 ResourceMonitor — System Tracking

```
  ┌──────────────────────────────────────────────┐
  │            ResourceMonitor                    │
  │                                              │
  │  Tracks: CPU%, memory MB, active agents      │
  │  Configurable interval + history ring buffer  │
  │  is_overloaded() → bool                      │
  │  resource_snapshot() → Snapshot               │
  │  set_active_agents(count)                     │
  └──────────────────────────────────────────────┘
```

**Integration:** The Guardian daemon checks `is_overloaded()` every 5 minutes and logs warnings to the audit trail.

**Source:** `crates/oxios-kernel/src/resource_monitor.rs`

---

### 3.12 GitLayer — In-Process Version Control

```
  ┌──────────────────────────────────────────────┐
  │               GitLayer                        │
  │          (powered by gix crate)               │
  │                                              │
  │  Workspace: ~/.oxios/workspace/              │
  │  Auto-commit on state saves                  │
  │                                              │
  │  commit_file(rel_path, message)              │
  │  remove_file(rel_path, message)              │
  │  log(limit) → Vec<CommitInfo>                │
  │  verify() → bool                             │
  │  tag(name, message)                          │
  │  restore(tag)                                │
  └──────────────────────────────────────────────┘
```

**Integration:** Called by Orchestrator after saving seeds/evaluations, by the brain connector after writes, and by the Guardian daemon for periodic verification.

**Source:** `crates/oxios-kernel/src/git_layer.rs`

---

### 3.13 CronScheduler — Scheduled Job Execution

```
  ┌──────────────────────────────────────────────┐
  │            CronScheduler                      │
  │                                              │
  │  CronJob { id, cron_expr, task, last_run }   │
  │  Tick-based evaluation (configurable interval)│
  │  Persistent state via StateStore              │
  │  GitLayer integration for auto-commits        │
  │                                              │
  │  add_cron(job) → Uuid                        │
  │  remove_cron(id) → Result                    │
  │  list_crons() → Vec<CronJob>                 │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/cron.rs`

---

### 3.14 Brain Connector — External `oxibrain` Daemon (RFC-047)

Agent memory lives in a **standalone daemon** (`oxibrain`, separate repo) reached over a Unix-domain socket. The kernel holds a thin `BrainConnection` client; it owns no index, no embeddings store, and no SQLite memory tables.

```
  ┌──────────────────────────────────────────────────────────┐
  │                  BrainConnection                          │
  │  (crates/oxios-kernel/src/brain/mod.rs)                  │
  │                                                          │
  │  Transport:  Unix-domain socket (JSON-RPC over length-    │
  │              framed lines), lazy reconnect on call fail.  │
  │  Liveness:   AtomicBool `is_available` — set on connect/  │
  │              reconnect, cleared on a failed call.         │
  │                                                          │
  │  Surface (KernelHandle::brain: BrainApi):                 │
  │    recall(space, text?, budget) → context string          │
  │    ingest(space, text)            → episode id            │
  │    search(space, query, limit)    → Vec<SearchHit>        │
  │    get_entity(id)                 → entity detail         │
  │    stats(space)                   → SpaceStats            │
  │                                                          │
  │  Degradation contract:                                   │
  │    daemon unreachable → every op returns empty/None,      │
  │    the kernel logs a warning, agent turns complete; the   │
  │    next call retries the connection. Fail-open, never     │
  │    blocks a turn.                                        │
  └──────────────────────────────────────────────────────────┘
```

**In-kernel survivors:** SONA (trajectory pattern engine, `memory_agent::sona`) and the embedding traits (`embedding` — `TextVector`, `cosine_similarity_f32`) were rehomed from the retired `oxios-memory` crate. `KernelDatabase` (SQLite WAL) now owns the mount/project tables in `~/.oxios/workspace/kernel.db`.

**Config:** `[brain]` section — `enabled` / `socket_path` (default `~/.oxi/brain/oxibrain.sock`) / `space` (default `personal`) / `auto_manage` (default `true`) / `binary_path` (override install root).

**Metrics:** `oxibrain_available` (gauge, set at boot + on reconnect + on every supervisor state transition), `oxibrain_recall_total` (counter, incremented on successful recall).

#### First-party supervision (2026-08-19)

oxibrain is a **first-party managed dependency** (not just an external daemon). The kernel's `BrainSupervisor` (`crates/oxios-kernel/src/brain/supervisor.rs`) installs, starts, watches, and stops the daemon.

```
  BrainSupervisor (kernel) ── install │ ensure │ respawn_if_needed
        │                                                    
        ├─ GithubInstaller  → GitHub Releases (a7garden/oxibrain):
        │    `oxibrain-aarch64-apple-darwin.tar.gz` + `.sha256`,
        │    sha256-verified, atomic tmp+rename 0o755 to
        │    ~/.oxi/bin/oxibrain.
        │
        ├─ launchd keeper (macOS) → plist ~/Library/LaunchAgents/
        │    com.oxi.oxibrain.plist, RunAtLoad + KeepAlive; fallback
        │    detached spawn (setsid) when launchctl is unavailable
        │    (OXIOS_BRAIN_NO_LAUNCHD=1 test escape). Respawn
        │    rate-limited 30 s; readiness timeout 30 s.
        │
        └─ Status: SupervisorState { Disabled, NotInstalled, Installing,
             Starting, Online, Failed } + ManagedBy { None, Launchd,
             Spawn, External }.
```

- **Boot** (`src/kernel.rs`): when `brain.enabled && brain.auto_manage`, `ensure()` installs/launches the daemon before the connection is made. A disabled surface never starts a daemon.
- **Respawn hook:** `BrainConnection::with_on_unavailable` fires once per failed lazy-reconnect, asking the supervisor to `respawn_if_needed()` (rate-limited); the hook is attached only when the surface is enabled.
- **`managed_by = external` no-op:** a supervisor that finds a live socket at entry does not touch that daemon (`stop()`/`uninstall()` only act on daemons it started).
- **Degradation contract holds:** every supervisor failure logs, records state (`SupervisorStatus.last_error`), and returns — agent turns always complete.
- **CLI:** `oxios brain install|start|stop|uninstall|status`; `/api/brain/status` returns `supervisor: SupervisorStatus`.


### 3.14.1 Oxi Foundation — Versioned Profile/Package Contract (RFC-048)

The Foundation layer lives at `~/.oxi/foundation/v1`. It owns:

- `profiles.json` — schema-versioned, non-secret provider profiles with
  OS Keychain `{ service, account }` locators. Profiles declare a role
  allow-list (`memory.extract`, `memory.consolidate`, `coding.primary`,
  `assistant.general`) and never receive a CSpace on their own.
- `packages.lock` — immutable shared package registry; entries include
  source, blake3 digest, trust label, target list (`oxios` is required),
  and abstract requirements (`workspace.read`, `shell.execute`,
  `brain.query`, `schedule.manage`, …).

Foundation is **not** a provider proxy and never spawns an external
worker. The executor stays the in-process `oxicode_sdk::Oxicode`. The
kernel boots with an idempotent bootstrap (`oxios foundation
bootstrap`) that creates the directory, hands the Brain socket a
version handshake, and refuses to write a lockfile from an agent turn.
The CLI exposes `oxios foundation status / bootstrap / register /
migrate` and renders reports as JSON — secrets are redacted in every
log line and receipt.

**Source:** `crates/oxios-kernel/src/foundation/{mod,bootstrap,profile,packages,migrate,resolver}.rs`

---

### 3.15 ProjectManager — Context Partitioning

Projects partition conversations and resources into isolated contexts, auto-detected from messages. (Renamed from "Spaces": `SpaceManager` → `ProjectManager`, `SpaceId` → `ProjectId`. The old JSON-file persistence and merge/archive operations were removed.)

```
  ┌──────────────────────────────────────────────────────────┐
  │                    ProjectManager                         │
  │                                                          │
  │  ┌────────────────────────────────────────────────────┐  │
  │  │       3-Layer Detection Strategy                   │  │
  │  │                                                    │  │
  │  │  Layer 1: Filesystem Path (fast, free)             │  │
  │  │    "/projects/oxios/src/main.rs" → oxios Project   │  │
  │  │    → PathMatcher: glob-based path → ProjectId map  │  │
  │  │                                                    │  │
  │  │  Layer 2: Keyword/Tag (fast, free)                 │  │
  │  │    Message contains Project tags → match           │  │
  │  │                                                    │  │
  │  │  Layer 3: Topic Classification (LLM, slow)         │  │
  │  │    Topic shift detection via ConversationBuffer    │  │
  │  └────────────────────────────────────────────────────┘  │
  │                                                          │
  │  Operations (ProjectManager):                            │
  │  detect(message) → DetectionResult                       │
  │  create_project / update_project / remove_project        │
  │  list_projects / get_project / get_project_by_name       │
  │  link_memory / unlink_memory (project ↔ memory)          │
  │  touch(id) — bump last-active                            │
  │                                                          │
  │  Persistence:                                            │
  │  SQLite `projects` table (via ProjectDb)                 │
  │  Project { id, name, description, tags, emoji, source,   │
  │            mount_ids (RFC-025), instructions (RFC-025) }  │
  └──────────────────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/project/manager.rs` (detection in `project/detection.rs`)

---

### 3.16 McpBridge — Model Context Protocol

```
  ┌──────────────────────────────────────────────────────────┐
  │                    McpBridge                              │
  │                                                          │
  │  Model Context Protocol client for external tool servers │
  │                                                          │
  │  Server sources:                                         │
  │  1. config.toml [mcp.servers]                            │
  │  2. Environment variables (OXIOS_MCP_{NAME}_COMMAND)     │
  │  3. Skill MCP server configs                             │
  │                                                          │
  │  Operations:                                             │
  │  register_server(McpServer)                              │
  │  initialize_all() → starts all servers                   │
  │  list_tools() → enumerate all server tools               │
  │  cached_tools(server_name) → cached tool definitions     │
  │  call_tool(server, tool, args) → result                  │
  │                                                          │
  │  McpServer { name, command, args, env, enabled }         │
  └──────────────────────────────────────────────────────────┘
```

**Integration:** Skill-level MCP servers are registered during `SkillManager.init()`. Tools are surfaced to agents via `McpToolWrapper` in the tool registry.

**Source:** `crates/oxios-kernel/src/mcp/`

---

### 3.17 A2AProtocol — Agent-to-Agent Communication

Google's A2A protocol for horizontal agent↔agent communication. Unlike MCP (vertical, agent→tool), A2A enables agents to discover, delegate, and share results.

```
  ┌──────────────────────────────────────────────────────────┐
  │                    A2AProtocol                            │
  │                                                          │
  │  ┌────────────────────────────────────────────────────┐  │
  │  │              AgentCardRegistry                      │  │
  │  │  HashMap<AgentId, AgentCard>                        │  │
  │  │                                                    │  │
  │  │  AgentCard { agent_id, name, description,          │  │
  │  │    capabilities[], skills[], endpoint, status }    │  │
  │  │                                                    │  │
  │  │  register_agent(card)                              │  │
  │  │  find_agents_by_capability(cap) → Vec<AgentCard>   │  │
  │  │  find_agents_by_skill(skill) → Vec<AgentCard>      │  │
  │  └────────────────────────────────────────────────────┘  │
  │                                                          │
  │  ┌────────────────────────────────────────────────────┐  │
  │  │            Per-Agent Message Queues                 │  │
  │  │  HashMap<AgentId, AgentQueue>                       │  │
  │  │  AgentQueue: messages + tokio::sync::Notify         │  │
  │  │                                                    │  │
  │  │  send_message(from, to, message) → request_id      │  │
  │  │  receive_messages(agent_id) → Vec<A2ARequest>       │  │
  │  │  send_and_wait(from, to, msg, timeout) → Response  │  │
  │  └────────────────────────────────────────────────────┘  │
  │                                                          │
  │  Message Types:                                          │
  │  TaskDelegation { task_id, description, payload, priority}│
  │  StatusUpdate { task_id, progress, message }             │
  │  ResultSharing { task_id, result, summary }              │
  │  CapabilityQuery { query, required_capabilities }         │
  │  Handshake { agent_id, name, capabilities }              │
  │                                                          │
  │  Delegation Handler:                                     │
  │  set_delegation_handler(callback)                        │
  │  execute_delegation(from, to, task) → Option<Result>     │
  │  Default handler: spawns agent via AgentLifecycleManager │
  └──────────────────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/a2a.rs`

---

### 3.18 CircuitBreaker — LLM Provider Protection

3-state circuit breaker preventing cascading LLM provider failures.

```
          ┌─────────┐  threshold    ┌──────┐
          │ CLOSED  │  failures     │ OPEN │
          │ (normal)│──────────────►│(fail)│
          └────┬────┘               └──┬───┘
               ▲                       │
               │ success               │ timeout
               │                       ▼
          ┌────┴────┐            ┌──────────┐
          │ CLOSED  │◄───────────│HALF-OPEN │
          │         │  success   │ (probe)  │
          └─────────┘            └────┬─────┘
                                      │ failure
                                      ▼
                                 ┌────────┐
                                 │  OPEN  │
                                 └────────┘

  Default: threshold=5, timeout=30s
  Single probe in half-open state (atomic CAS gate)
  Global singleton via OnceLock
```

**Source:** `crates/oxios-kernel/src/circuit_breaker.rs`

---

### 3.19 PersonaManager — Multi-Persona System

```
  ┌──────────────────────────────────────────────┐
  │           PersonaManager                      │
  │                                              │
  │  Personas define agent behavior:             │
  │  - name, system_prompt, role                 │
  │  - enabled/disabled state                    │
  │                                              │
  │  first_enabled() → Option<&Persona>          │
  │  get_active_persona() → Option<&Persona>     │
  │  active_system_prompt() → String             │
  │                                              │
  │  Integration:                                │
  │  - OuroborosEngine: set_persona_prompt()     │
  │  - AgentRuntime: injects into system prompt  │
  │  - CSpace resolution: role → capability set  │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/persona_manager.rs`

---

### 3.20 SkillManager — Unified Skill System (RFC-009)

```
  ┌──────────────────────────────────────────────────────┐
  │                  SkillManager                         │
  │                                                      │
  │  Skills are unified capabilities:                     │
  │  ~/.oxios/workspace/skills/{name}/                   │
  │    └── SKILL.md  (YAML frontmatter + instructions)   │
  │                                                      │
  │  SkillSource hierarchy (priority):                    │
  │  1. workspace/.agents/<id>/skills/ (agent-specific)  │
  │  2. workspace/skills/             (project)         │
  │  3. ~/.oxios/workspace/skills/    (global user)       │
  │  4. share/default-skills/          (bundled, lowest)  │
  │                                                      │
  │  Requirements (4-dimensional):                        │
  │  bins: ["git", "gh"]     — required binaries         │
  │  anyBins: ["ffmpeg"]     — one must be present       │
  │  env: ["GITHUB_TOKEN"]  — required env vars          │
  │  config: []               — required config paths    │
  │                                                      │
  │  Install specs (automatic dependency installation):   │
  │  - kind: brew, formula: git                           │
  │  - kind: download, url: https://...                   │
  │                                                      │
  │  Operations:                                         │
  │  init() → load + validate + watch                    │
  │  list_skills() → Vec<SkillEntry>                     │
  │  get_skill(name) → Option<SkillEntry>                │
  │  build_snapshot() → SkillSnapshot (for agent prompt) │
  │  set_enabled(name, bool)                             │
  └──────────────────────────────────────────────────────┘
```

**Built-in skills** (in `share/default-skills/`): code-review, debug, refactor. Memory and Programs have been unified into Skills per RFC-009.

**Source:** `crates/oxios-kernel/src/skill.rs` — `SkillManager`

---

### 3.21 SkillStore — Skill Definitions

```
  ┌──────────────────────────────────────────────┐
  │              SkillStore                       │
  │                                              │
  │  Skills are reusable agent capabilities      │
  │  Stored in: ~/.oxios/workspace/skills/       │
  │                                              │
  │  init_defaults(defaults_dir) → populate      │
  │  Default skills from share/default-skills/   │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/skill.rs`

---

### 3.22 HostToolValidator — Binary Allowlist

### 3.19 Skill Requirements — Unified Requirements Checking (RFC-009)

```
  ┌──────────────────────────────────────────────┐
  │          Skill Requirements Check              │
  │                                              │
  │  4-dimensional requirements per skill:        │
  │  bins:     all must be present                │
  │  anyBins:  at least one must be present       │
  │  env:      required environment variables      │
  │  config:   required config paths               │
  │                                              │
  │  Replaces former: HostToolValidator,          │
  │  SkillManager host_requirements,                 │
  │  ExecConfig.required/optional_host_tools       │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/skill.rs` — `SkillManager::check_requirements()`

---

### 3.23 AuthManager — Identity Verification

```
  ┌──────────────────────────────────────────────┐
  │            AuthManager                        │
  │                                              │
  │  Authentication for kernel operations.       │
  │  API keys resolved via:                      │
  │  1. engine.api_key (config)                  │
  │  2. ~/.oxicode/auth.json (oxi credentials)       │
  │  3. Environment variables                    │
  │                                              │
  │  Used by KernelHandle's SecurityApi          │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/auth.rs`

---

### 3.24 CredentialStore — Multi-Source Credential Resolution

```
  ┌──────────────────────────────────────────────┐
  │          CredentialStore                      │
  │                                              │
  │  Resolution chain:                           │
  │  config.toml → oxi auth.json → env vars      │
  │                                              │
  │  Used for LLM provider API keys              │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/credential.rs`

---

### 3.25 WasmSandbox — Untrusted Code Execution

```
  ┌──────────────────────────────────────────────┐
  │           WasmSandbox                         │
  │                                              │
  │  WASM-based sandbox for executing            │
  │  untrusted code in isolation                 │
  └──────────────────────────────────────────────┘
```

**Source:** `crates/oxios-kernel/src/wasm_sandbox.rs`

---

### 3.26 ContextManager — Context Window Management

```
  ┌──────────────────────────────────────────────┐
  │          ContextManager                       │
  │                                              │
  │  Manages LLM context window:                 │
  │  - Token counting and budgeting              │
  │  - Compaction strategy (threshold: 0.8)      │
  │  - Memory blending into prompts              │
  └──────────────────────────────────────────────┘
```

**Source:** Integrated into `AgentRuntime` via `oxicode_sdk::AgentLoop` compaction callbacks.

---

## 4. KernelHandle Facade

The KernelHandle is the **syscall table** of the Agent OS. It is a facade composed of **22 typed APIs** that provide the single path for all kernel operations — all tools go through it, never reaching subsystems directly (full list below).

| Domain | Typed APIs |
|--------|-----------|
| Lifecycle & execution | `AgentApi` · `ExecApi` · `ExtensionApi` |
| State & persistence | `StateApi` · `BrainApi` · `MemoApi` · `TimelineApi` · `CompressionApi` |
| Knowledge | `KnowledgeLens` |
| Security | `SecurityApi` |
| Identity & persona | `PersonaApi` |
| Communication | `A2aApi` · `McpApi` |
| Infrastructure | `InfraApi` · `EngineApi` · `BrowserApi` · `MountApi` |
| Integrations | `CalendarApi` · `EmailApi` |
| Scheduling & quota | `TokenMaxingApi` |
| Marketplace | `MarketplaceApi` |
| Projects (was Spaces) | `ProjectApi` |

> `ProjectApi` replaces the former `SpaceApi` (conversation spaces → projects). `KnowledgeLens` is the knowledge surface — there is no separate `KnowledgeBaseApi`.

**Cross-facade convenience methods** compose across APIs: `save_and_commit()`, `delete_and_commit()`, `commit_all()` (State + Git); `flush_audit()` (Security + Git); `schedule()` (Cron). The handle is cached via `OnceLock`.

### Construction Order

The KernelHandle is created **twice** during `KernelBuilder::build()`:

1. **Placeholder Handle** — Created with `NoOpSupervisor` to break the circular dependency:
   `KernelHandle → AgentRuntime → Supervisor → KernelHandle`

2. **Final Handle** — Cached in `Kernel.handle_cache` (via `OnceLock`) with the real `BasicSupervisor`.

The first handle is discarded after the real supervisor is constructed.

**Source:** `crates/oxios-kernel/src/kernel_handle/mod.rs`

---

## 5. Data Flow

### 5.1 User Message Flow

Complete path of a user message through the system:

```
  User: "Refactor the authentication module to use JWT tokens"
       │
       ▼
  ┌─────────────────┐
  │   CLI / Web /   │  Channel receives raw message
  │   Telegram      │
  └────────┬────────┘
           │
           ▼
  ┌─────────────────┐
  │    Gateway      │  Routes to Orchestrator.handle_message()
  │ (oxios-gateway) │  (user_id, message, session_id)
  └────────┬────────┘
           │
           ▼
  ┌─────────────────────────────────────────────────────┐
  │              Space Detection (3-layer)               │
  │  Path? → Keyword? → Topic classification            │
  │  Creates/activates appropriate Space                 │
  └────────┬────────────────────────────────────────────┘
           │
           ▼
  ┌─────────────────────────────────────────────────────┐
  │              Chat Bypass Check                       │
  │  Is this a simple greeting/question?                │
  │  YES → Direct LLM response → return to user         │
  │  NO  → Continue to Ouroboros pipeline                │
  └────────┬────────────────────────────────────────────┘
           │
           ▼
  ╔═══════════════════════════════════════════════════════╗
  ║         OUROBOROS PROTOCOL (Phase 1: Interview)      ║
  ║  OuroborosEngine.interview(message)                  ║
  ║  → InterviewResult { questions, ambiguity, ready }   ║
  ║  If ambiguity > 0.2: return questions to user        ║
  ║  User answers → follow-up interview                  ║
  ╚══════════════╤════════════════════════════════════════╝
                 │ (ambiguity ≤ 0.2, ready for seed)
                 ▼
  ╔═══════════════════════════════════════════════════════╗
  ║         OUROBOROS PROTOCOL (Phase 2: Seed)           ║
  ║  OuroborosEngine.generate_seed(interview)            ║
  ║  → Seed { id, goal, constraints, criteria, ontology }║
  ║  Save to StateStore → emit SeedCreated event         ║
  ╚══════════════╤════════════════════════════════════════╝
                 │
                 ▼
  ┌─────────────────────────────────────────────────────┐
  │         Multi-Agent Split Check                      │
  │  acceptance_criteria.len() ≥ 3?                      │
  │  YES → split into subtasks → delegate in parallel    │
  │  NO  → single agent execution                        │
  └────────┬────────────────────────────────────────────┘
           │
           ▼
  ╔═══════════════════════════════════════════════════════╗
  ║    AGENT LIFECYCLE (Phase 3: Execute)                ║
  ║  AgentLifecycleManager.spawn_and_run(seed, priority) ║
  ║                                                      ║
  ║  1. fork() → AgentId                                 ║
  ║  2. Register A2A AgentCard                           ║
  ║  3. Ensure AccessManager permissions                 ║
  ║  4. Submit to Scheduler → start_task()               ║
  ║  5. Supervisor.run_with_seed(agent_id, seed)         ║
  ║     └── AgentRuntime.execute(agent_id, seed)         ║
  ║         ├── Resolve CSpace                           ║
  ║         ├── Register tools (always-on + CSpace)      ║
  ║         ├── Recall memories → blend into prompt      ║
  ║         ├── Semantic capability retrieval             ║
  ║         └── oxicode_sdk::AgentLoop::run()                ║
  ║             └── Multi-turn LLM tool-calling loop     ║
  ║                 ├── Tool calls: exec, read, write... ║
  ║                 ├── Circuit breaker on LLM errors    ║
  ║                 └── Compaction → save memory          ║
  ║  6. Cleanup: unregister A2A, complete/fail task      ║
  ╚══════════════╤════════════════════════════════════════╝
                 │ ExecutionResult
                 ▼
  ╔═══════════════════════════════════════════════════════╗
  ║    OUROBOROS PROTOCOL (Phase 4: Evaluate)            ║
  ║  OuroborosEngine.evaluate(seed, exec_result)         ║
  ║  → EvaluationResult { score, criteria_results, notes }║
  ╚══════════════╤════════════════════════════════════════╝
                 │
                 ▼
  ┌─────────────────────────────────────────────────────┐
  │         Score Check                                   │
  │  score ≥ 0.8 and all_passed?                         │
  │  YES → Return success result                         │
  │  NO  → Enter evolve loop (max 3 iterations)          │
  └────────┬────────────────────────────────────────────┘
           │ (score < 0.8)
           ▼
  ╔═══════════════════════════════════════════════════════╗
  ║    OUROBOROS PROTOCOL (Phase 5: Evolve)              ║
  ║  OuroborosEngine.evolve(seed, evaluation)            ║
  ║  → Option<Seed> (evolved version)                    ║
  ║  → Re-execute with Priority::High                    ║
  ║  → Re-evaluate                                       ║
  ║  → Repeat until pass or max iterations reached       ║
  ╚═══════════════════════════════════════════════════════╝
           │
           ▼
  OrchestrationResult {
    session_id, primary_project_id, project_tag, response,
    agent_id, phase_reached, evaluation_passed, output
  }
```

---

## 6. Dependency Graph

### Crate Dependencies

```
  ┌─────────────────────────────────────────────────────────┐
  │                    oxios (binary)                        │
  │  src/main.rs, src/kernel.rs, src/cmd_run.rs             │
  │                                                         │
  │  Depends on:                                            │
  │  ├── oxios-kernel     (core subsystems)                 │
  │  ├── oxios-ouroboros  (protocol engine)                 │
  │  ├── oxios-gateway    (message hub)                     │
  │  ├── oxios-web        (web channel, feature-gated)      │
  │  ├── oxios-cli        (CLI channel, feature-gated)      │
  │  └── oxios-telegram   (Telegram channel, feature-gated) │
  └───────────────────┬─────────────────────────────────────┘
                      │
          ┌───────────┼───────────────────┐
          │           │                   │
          ▼           ▼                   ▼
  ┌──────────────┐ ┌───────────────┐ ┌──────────────┐
  │ oxios-kernel │ │oxios-ouroboros│ │ oxios-gateway│
  │              │ │               │ │              │
  │ Core:        │ │ Protocol:     │ │ Channel-     │
  │ Supervisor   │ │ Interview     │ │ agnostic     │
  │ Scheduler    │ │ Seed gen      │ │ message hub  │
  │ EventBus     │ │ Execute       │ │              │
  │ StateStore   │ │ Evaluate      │ │ Depends on:  │
  │ AccessMgr    │ │ Evolve        │ │ oxios-kernel │
  │ AuditTrail   │ │               │ │ (Orchestrator│
  │ Memory       │ │ Depends on:   │ │  reference)  │
  │ Space        │ │ oxicode-sdk       │ └──────────────┘
  │ MCP Bridge   │ │ (Provider)    │
  │ A2A          │ └───────────────┘
  │ PersonaMgr   │
  │ SkillMgr     │   ┌──────────────────────┐
  │ GitLayer     │   │  oxicode-sdk (crates.io) │
  │ CronSched    │   │  NOT a path dep!     │
  │ BudgetMgr    │   │  AgentLoop, Provider,│
  │ ResourceMon  │   │  ToolRegistry, etc.  │
  │ CircuitBkr   │   └──────────────────────┘
  │ KernelHandle │          ▲
  │              │          │
  │ Depends on:  │──────────┘
  │ oxicode-sdk      │   + oxicode-ai (provider construction)
  │ oxicode-ai       │   + oxios-ouroboros
  │ oxios-ourobo │
  └──────────────┘
```

### Feature Gates

```
  [features]
  web       → oxios-web channel
  cli       → oxios-cli channel
  telegram  → oxios-telegram channel
  browser   → BrowserTool + BrowserApi (headless browser)
  telemetry → OpenTelemetry integration (compile-time toggle)
```

### Directory Structure

```
  oxios/
  ├── src/                        # Binary crate
  │   ├── main.rs                 # Entry point, daemon mode, CLI dispatch
  │   ├── kernel.rs               # Kernel builder + assembly
  │   ├── supervisor.rs           # Binary-level supervisor wiring
  │   ├── surface.rs              # Channel surface dispatch
  │   ├── api/                    # HTTP API server (was channels/oxios-web)
  │   │   ├── routes/             # REST route handlers
  │   │   ├── plugin.rs           # API plugin / extension
  │   │   ├── bridge.rs           # API ↔ kernel bridge
  │   │   └── quota.rs            # Rate-limit / quota
  │   ├── channels/               # In-process channels
  │   │   ├── cli/                # CLI channel
  │   │   └── telegram/           # Telegram channel
  │   ├── commands/               # CLI subcommands (run, update)
  │   ├── remote/                 # Remote RPC / pairing / transport
  │   └── web_dist.rs             # Embedded SPA assets
  ├── crates/
  │   ├── oxios-kernel/           # Core library (intentionally monolithic — see §10)
  │   │   └── src/
  │   │       ├── supervisor.rs        orchestrator.rs        agent_lifecycle.rs
  │   │       ├── agent_runtime.rs     engine.rs              event_bus.rs
  │   │       ├── state_store.rs       config.rs              types.rs
  │   │       ├── access_manager/      # RBAC + path sandbox + audit trail
  │   │       ├── brain/              # oxibrain daemon client (BrainConnection) + SONA + embeddings (RFC-047)
  │   │       ├── kernel_handle/       # Facade — 22 typed APIs (see §4)
  │   │       ├── tools/               # builtin/, kernel_bridge.rs, exec_tool.rs
  │   │       ├── skill/               # Unified skill system (RFC-009): clawhub, skills_sh
  │   │       ├── persona/             a2a/                   resilience/
  │   │       ├── token_maxing/        host_tools/            mount/
  │   │       ├── project/             image_gen/             capability/
  │   │       ├── cron.rs              git_layer.rs           budget.rs
  │   │       ├── resource_monitor.rs  auth.rs                credential.rs
  │   │       ├── wasm_sandbox.rs      mcp.rs                 agent_group.rs
  │   │       └── metrics.rs           onboarding.rs          daemon.rs
  │   ├── oxios-ouroboros/        # Unified intent engine (assess → crystallize → execute → review)
  │   ├── oxios-gateway/          # Channel-agnostic message hub
  │   ├── oxios-markdown/         # Knowledge base (VirtualFs, BacklinkIndex)
  │   ├── oxios-mcp/              # MCP client (JSON-RPC 2.0 over stdio)
  │   └── oxios-calendar/         # .ics-based calendar event management
  ├── web/                        # React frontend (SPA)
  ├── share/                      # Default skills, config
  └── docs/                       # Architecture docs, RFCs
```

---

## 7. Security Model

### 7.1 Overview

```
  ┌─────────────────────────────────────────────────────────┐
  │                   Security Layers                        │
  │                                                         │
  │  Layer 1: Authentication (AuthManager)                  │
  │    ↓ API key resolution: config → oxi auth → env        │
  │                                                         │
  │  Layer 2: Authorization (AccessManager + RBAC)          │
  │    ↓ Least privilege, tool/path/network controls        │
  │                                                         │
  │  Layer 3: Sandbox (Workspace isolation)                 │
  │    ↓ Path boundary enforcement per workspace            │
  │                                                         │
  │  Layer 4: Execution Security (ExecTool)                 │
  │    ↓ Shell mode: RBAC-enforced bash -c                  │
  │    ↓ Structured mode: binary allowlist + meta blocking  │
  │                                                         │
  │  Layer 5: Audit (AuditTrail)                            │
  │    ↓ Tamper-evident blake3 hash chain                   │
  │    ↓ Every access decision logged                       │
  │                                                         │
  │  Layer 6: Guardian Daemon                               │
  │    ↓ Periodic integrity checks (audit chain, git, load) │
  └─────────────────────────────────────────────────────────┘
```

### 7.2 RBAC (Role-Based Access Control)

```
  ┌──────────────────────────────────────────────────┐
  │                 RbacManager                       │
  │                                                  │
  │  Subjects:  Agent(UUID) | User(String) | Role    │
  │  Actions:   ExecuteTool | AccessPath | Network   │
  │             Fork | ManageAgents | ConfigChange    │
  │  Policies:  Subject + Action + Resource → Allow   │
  │                                                  │
  │  Human-in-the-Loop (HitL):                       │
  │  PendingApproval { id, subject, action, resource }│
  │  approve(id) / reject(id)                        │
  │  Approval flow: submit → queue → approve/reject  │
  └──────────────────────────────────────────────────┘
```

### 7.3 Path Sandboxing

```
  Agent requests: /workspace/secret/key.pem
       │
       ▼
  ┌──────────────────────────────────────┐
  │  1. RBAC Check                       │
  │     Subject.Agent → Action.AccessPath│
  │     → allowed?                       │
  ├──────────────────────────────────────┤
  │  2. Path Permission Check            │
  │     denied_paths matches? → BLOCK    │
  │     allowed_paths matches? → ALLOW   │
  ├──────────────────────────────────────┤
  │  3. Workspace Boundary Check         │
  │     canonicalize(path)               │
  │     starts_with(workspace_path)?     │
  │     NO → "sandbox violation" logged  │
  └──────────────────────────────────────┘
  All three must pass for access to be granted
```

### 7.4 ExecTool Security

```
  ┌──────────────────────────────────────────────────┐
  │              ExecTool                            │
  │                                                  │
  │  Shell Mode:                                     │
  │    bash -c "{command}"                           │
  │    RBAC-enforced: agent must have bash allowed    │
  │    Working directory: workspace                  │
  │                                                  │
  │  Structured Mode:                                │
  │    Binary allowlist only                         │
  │    Metacharacter blocking (|, ;, &, etc.)        │
  │    Safer for production use                      │
  └──────────────────────────────────────────────────┘
```

### 7.5 Audit Trail Integrity

```
  Every security-relevant action:
    ↓
  AuditEntry { seq, timestamp, actor, action, resource, prev_hash, hash }
    ↓
  blake3 hash links entry to predecessor
    ↓
  verify() detects any tampering:
    - Chain broken (prev_hash mismatch)
    - Hash recomputation fails
    - Future timestamps
    ↓
  Guardian daemon checks every 5 minutes:
    - Audit chain validity
    - Git repository integrity
    - System overload
```

---

## 8. Unix Philosophy Mapping

> **Where the metaphor is structural vs analogical.** The lifecycle rows (`init` / `fork` / `exec` / `wait` / `kill` / `ps`) are load-bearing — they name real `Supervisor` operations. The rest are analogies that aid intuition, not literal reimplementations: `EventBus` is broadcast pub/sub (not byte-stream pipes), `KernelHandle` is a facade (not a pipe-composed shell), and the kernel is deliberately monolithic (§10), so "compose small pieces" applies at the *module* boundary, not the process boundary. "Shell" maps to `ExecTool` (a host-command executor), while the human-facing surface is the Gateway — Unix conflates both under one shell, Oxios separates them.

| Unix Concept       | Oxios Equivalent                           | Description                                    |
|--------------------|--------------------------------------------|------------------------------------------------|
| `init` (PID 1)    | `Supervisor`                               | Manages all agent lifecycles                   |
| `fork()`          | `Supervisor::fork_directive(directive, env)` | Create new agent from a unified-intent Directive |
| `exec()`          | `Supervisor::exec(id)` / `run_with_directive()` | Start executing an agent                       |
| `wait()`          | `Supervisor::wait(id)`                     | Wait for agent completion                      |
| `kill()`          | `Supervisor::kill(id)`                     | Terminate an agent (cooperative + abort)       |
| `ps`              | `Supervisor::list()`                       | List all known agents                          |
| Pipes             | `EventBus` (broadcast channel)             | Inter-agent communication                      |
| Signals           | `KernelEvent` variants                     | AgentCreated, AgentStopped, etc.               |
| `/proc`           | `StateStore` + `Scheduler::stats()`        | Agent state inspection                         |
| `nice` / priority | `Priority` enum (Low→Critical)             | Task scheduling priority                       |
| `ulimit`          | `BudgetManager` + `AccessManager`          | Resource limits per agent                      |
| `chroot`          | Workspace sandboxing                       | Path boundary enforcement                      |
| `auditd`          | `AuditTrail`                               | Tamper-evident cryptographic audit log         |
| `cron`            | `CronScheduler`                            | Scheduled job execution                        |
| `git`             | `GitLayer`                                 | In-process version control via gix             |
| `/etc`            | `OxiosConfig` (config.toml)                | System configuration                           |
| Skills           | `SkillManager` + `skill/`                | Unified skill system (RFC-009)               |
| `man` pages       | `SKILL.md` per skill                      | Usage documentation for capabilities           |
| Shell             | `ExecTool` (shell/structured modes)        | Command execution with RBAC                    |
| Sudo / polkit     | HitL Approval (RbacManager)                | Human-in-the-loop approval for dangerous ops   |

---

## 9. Dependency Rules

### Invariant Rules

| Rule | Description |
|------|-------------|
| **oxicode-sdk is crates.io only** | `oxicode-sdk` is a published crate, NOT a path dependency. Never reimplement what oxi provides. |
| **Kernel binary assembles, library provides** | `src/kernel.rs` (binary) assembles components. `oxios-kernel` (library) provides them. |
| **KernelHandle is the syscall table** | All tool access goes through `KernelHandle`. Never bypass it with direct subsystem references in tools. |
| **Supervisor ≠ AgentLifecycleManager** | `Supervisor` = low-level process management. `AgentLifecycleManager` = full orchestrated lifecycle (A2A, scheduling, permissions). Never add lifecycle logic to Orchestrator directly. |
| **Lifecycle split** | Don't add lifecycle logic to `Orchestrator` — use `AgentLifecycleManager`. |
| **No containers** | Direct host execution. Security via `AccessManager` (RBAC + path sandboxing). |
| **Channel agnostic** | Gateway doesn't care where messages come from. Channels are feature-gated plugins. |
| **Feature gates** | Web, CLI, Telegram, browser, telemetry are all feature-gated. Check `cargo build -p oxios --features <feature>`. |

### Circular Dependency Break

The construction order solves the `KernelHandle → AgentRuntime → Supervisor → KernelHandle` cycle:

```
  1. Create placeholder KernelHandle with NoOpSupervisor
  2. Create AgentRuntime with placeholder handle
  3. Create BasicSupervisor with AgentRuntime
  4. Build real Kernel
  5. Kernel::handle() returns cached handle via OnceLock
     (real supervisor, not NoOp)
```

### Testing Conventions

| Convention | Description |
|------------|-------------|
| Unit tests | `#[cfg(test)] mod tests` in each file |
| Integration tests | `tests/` per crate |
| Mock providers | `MockProvider` implements `oxicode_sdk::Provider` with empty streams |
| Temp directories | Use `tempfile::tempdir()` for state store tests |
| Must pass | `cargo test --workspace` at every commit |

---

## 10. Kernel Crate Architecture

### Why `oxios-kernel` Is a Single Crate

`oxios-kernel` is ~50K lines across 137 source files. This is **intentionally monolithic** — not a code smell that needs fixing.

#### The dependency topology is a star, not a web

Real-world code review shows that kernel subsystems depend on each other in a clean star pattern:

```
                     types::AgentId
                    ╱      │       ╲
      EventBus   StateStore   Config
          │          │          │
     Supervisor ── Scheduler ── BudgetManager
          │
     AgentRuntime ── Engine (→ oxicode-sdk)
          │
     KernelHandle (facade: references all subsystems)
```

There are **zero circular dependencies** between modules. The only module that references everything is `KernelHandle` — and that is its explicit purpose as a Facade.

#### Internal boundaries are already clean

The kernel uses directory-level modules with `pub(crate)` encapsulation:

| Directory | Lines | External deps from kernel | Self-contained? |
|-----------|-------|---------------------------|----------------|
| `brain/` | ~340 | `oxibrain-client` (daemon socket) | ✅ Thin client — store is external (RFC-047) |
| `tools/` | 7,047 | `KernelHandle` (facade pattern) | ✅ Goes through facade |
| `access_manager/` | 3,681 | `types`, `capability`, `config` (3) | ✅ Nearly isolated |
| `kernel_handle/` | 3,063 | All subsystems (by design) | — This IS the facade |
| `capability/` | 958 | `types` (1) | ✅ Fully isolated |
| `skill/` | 1,580 | None | ✅ Fully isolated |

Each directory already has its own `mod.rs` controlling visibility. The `lib.rs` organizes modules into logical sections with explicit Korean comments:

- Lifecycle: `supervisor`, `agent_lifecycle`, `agent_runtime`, `daemon`
- Orchestration: `scheduler`, `budget`, `cron`, `orchestrator`
- Security: `access_manager`, `audit_trail`, `auth`, `capability`, `credential`
- Communication: `a2a`, `event_bus`, `mcp`, `coordination`
- Intelligence: `brain`, `embedding`, `persona`, `onboarding`
- Tools & Skills: `tools`, `skill`, `clawhub`, `skills_sh`
- State & Config: `config`, `state_store`, `git_layer`, `project`, `backup`
- Infrastructure: `engine`, `error`, `types`, `metrics`, `telemetry`
- API Surface: `kernel_handle`

#### Why not split into separate crates?

Every past attempt to extract a subsystem was cancelled because:

| Cost of extraction | Impact |
|-------------------|--------|
| `pub(crate)` → `pub` everywhere | Leaks internal types, widens API surface |
| Circular dependency risk | Extracted crates need kernel types → kernel needs extracted crate |
| Version synchronization | N crates × version bumps = coordination overhead |
| Build time regression | More crate boundaries = more compilation units = worse cache |
| No reuse benefit | Nobody consumes `oxios-kernel-memory` or `oxios-kernel-security` independently |
| Trait abstraction overhead | To break cycles, you'd need traits → dynamic dispatch → complexity |

The Linux kernel is 30M+ lines in a single tree. It uses directories (`mm/`, `fs/`, `net/`, `drivers/`) for organization, not separate libraries. Same principle applies here at a different scale.

#### When WOULD splitting make sense?

Splitting would be justified if and only if:

1. **Independent reuse** — Another project wants to use a subsystem without the whole kernel
2. **Independent versioning** — A subsystem changes at a fundamentally different cadence
3. **Build isolation** — A subsystem has dramatically different compile-time dependencies (e.g., heavy C FFI)
4. **Team boundary** — Different teams own different subsystems with different release cycles

None of these conditions hold today.

#### How to manage the kernel's size

Instead of crate splitting, the kernel manages complexity through:

- **Feature gates**: `sqlite-memory`, `embedding-gguf`, `wasm-sandbox`, `browser` — only compile what you need
- **`pub(crate)` encapsulation**: Module internals stay internal
- **Directory-level mod.rs**: Each subsystem controls its own visibility
- **lib.rs section organization**: Clear logical grouping with labeled sections
- **KernelHandle facade**: Single entry point for all 22 APIs, preventing direct subsystem coupling in consumers

---

## 11. Chat Streaming Contract

The chat WebSocket (RFC-015 transparency over the RFC-024 reliability
channel) is the single live path for turn progress. This section pins the
boundaries that implementers keep tripping over. See
`docs/rfc-049-turn-cancellation.md` for the cancellation half.

### Turn identity (D1)

One turn key — `session_id`, or `request_id` for a session's first message —
is shared by `StreamingSinkRegistry`, `TurnRegistry`, and
`ExecEnv.session_id`. Never introduce a second identifier for a turn.

### WS chunk table

| type | seq | partial | content | source |
|---|---|---|---|---|
| `token` (delta) | — | yes | text fragment | `StreamDelta::Text` collector |
| `token` (terminal) | yes | no | full response text (suppressed when partials delivered this turn) | terminal `OutgoingMessage` |
| `reasoning.start` / `reasoning.end` | — | yes | lifecycle markers | collector |
| `reasoning` (delta) | — | yes | thinking fragment | `StreamDelta::ThinkingDelta` |
| `model` | — | no | one-shot model announcement | `StreamDelta::Model` |
| `tool_start` / `tool_end` / `tool_progress` / `tool_call_delta` | — | no | tool call lifecycle | `KernelEvent` → chat.rs |
| `memory` | — | no | recall/store transparency | `KernelEvent` → chat.rs |
| `usage` | — | no | cumulative token usage | `KernelEvent` → chat.rs |
| `grounding` | — | no | web-search citations | `KernelEvent` → chat.rs |
| `agent_start` / `agent_end` | — | no | sub-agent fork timeline | `KernelEvent` → chat.rs |
| `compression_delta` / `compression_done` / `compression_failed` | — | no | context compaction | `KernelEvent` → chat.rs |
| `interview` / `tool_approval` / `path_access` | — | no | interactive features | passthrough raw chunk |
| `done` / `error` | yes | no | terminal — seq'd, replayed on reconnect | terminal `OutgoingMessage` |

Partials carry no `seq` and are absent from the replay buffer: FIFO order
comes from the mpsc, each frame has a unique `Uuid` for the frontend's
seen-id ring, and reconnect recovery uses the terminal instead.

### The phase decision (D3)

There is no multi-phase lifecycle to visualize. Post-RFC-027 the Ouroboros
phase is a plain `String` and the only literal ever assigned is `"execute"`.
`phase` and `evaluation_passed` live purely as fields on the `done` chunk,
rendered by `chat-metadata.tsx`. Do not re-add a standalone phase streaming
path — it would be UI for state that does not exist.

### The replay-gap decision (D5)

Partial deltas skip `assign_seq` so they are absent from the 512-entry replay
buffer. A mid-stream reconnect therefore recovers the terminal full-text
message rather than resuming token flow. This is intentional: the terminal
carries complete text, and buffering partials would multiply the buffer's
memory by the token count. **Do not** assign seq to partials "for
robustness" — it re-breaks the reconnect merge.

### The SSE/WS overlap (D6)

Both `/api/events` (SSE) and the chat WS carry `KernelEvent`s. SSE is the
global dashboard feed (every agent, every session); the WS is per-turn chat
transparency filtered to the active session. Consumers dedupe by context —
SSE never drives the chat timeline and the WS never drives the dashboard.
This overlap is legitimate; do not merge the two surfaces.

---

## Appendix: Quick Reference

### Key File Locations

| Path | Purpose |
|------|---------|
| `~/.oxios/config.toml` | Main configuration |
| `~/.oxios/workspace/` | Agent working directory |
| `~/.oxios/workspace/seeds/` | Ouroboros seed specs |
| `~/.oxios/workspace/skills/` | Unified skill definitions (Programs + Skills) |
| `~/.oxios/workspace/sessions/` | Session data (ephemeral) |
| `~/.oxios/spaces/` | Space data (index + per-space dirs) |
| `~/.oxicode/auth.json` | oxi-cli credentials |

### CLI Quick Reference

```bash
cargo build                          # Build everything
cargo test --workspace               # Run all tests
cargo run                            # Start daemon (background)
cargo run -- --foreground            # Start in foreground
cargo run -- run --json "prompt"     # Single-shot execution
cargo run -- run --json --session "$SID" "follow-up"  # Multi-turn
```

### OrchestrationResult JSON Shape

```json
{
  "response": "string",
  "session_id": "uuid",
  "primary_project_id": "uuid | null",
  "project_tag": "[emoji label] | null",
  "agent_id": "uuid | null",
  "phase_reached": "interview | execute",
  "evaluation_passed": true,
  "output": "string | null"
}
```
