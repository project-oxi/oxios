# Oxios Agent OS — REST API Reference

> **Base URL:** `http://127.0.0.1:4200`
> **API Version:** 0.1.2
> **Body Size Limit:** 10 MB per request
> **Date:** 2026-05-17

---

## Table of Contents

1. [Authentication](#1-authentication)
2. [Chat](#2-chat)
3. [System](#3-system)
4. [Agents](#4-agents)
5. [Workspace & Projects](#5-workspace)
6. [Seeds](#6-seeds)
7. [Skills](#7-skills)
8. [Memory](#8-memory)
9. [Programs](#9-programs)
10. [Scheduler](#10-scheduler)
11. [Audit](#11-audit)
12. [Permissions](#12-permissions)
13. [Cron Jobs](#13-cron-jobs)
14. [Sessions](#14-sessions)
15. [Personas](#15-personas)
16. [Git](#16-git)
17. [Budget](#17-budget)
18. [Resources](#18-resources)
19. [Agent Groups](#19-agent-groups)
20. [Approvals (HitL)](#20-approvals-hitl)
21. [SSE Events](#21-sse-events)
22. [Host Tools](#22-host-tools)
23. [Metrics](#23-metrics)
24. [Error Responses](#24-error-responses)
25. [Rate Limiting](#25-rate-limiting)
26. [Pagination](#26-pagination)

---

## Overview

Oxios exposes a REST API built on Axum. All `/api/*` endpoints require Bearer token authentication (when auth is enabled). The `/health` endpoint and static assets are always public.

**Common Headers:**

| Header | Required | Description |
|---|---|---|
| `Authorization` | Yes (when auth enabled) | `Bearer <token>` |
| `Content-Type` | Yes (for POST/PUT) | `application/json` |

---

## 1. Authentication

Authentication uses Bearer tokens. When `auth_enabled` is `true` in the config, all `/api/*` endpoints require a valid token.

Tokens are validated against (in order):
1. The kernel's auth manager (registered tokens)
2. The `OXIOS_API_KEY` environment variable
3. The static API key from the config file

```sh
# Authenticate via Authorization header
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/status
```

**Unauthenticated Response:**

```json
HTTP 401 Unauthorized
```

---

## 2. Chat

### POST /api/chat

Send a message to the Oxios gateway for processing and receive a synchronous response.

**Authentication:** Required

**Request Body:**

```json
{
  "content": "Build a TODO app with React and SQLite",
  "user_id": "default",
  "session_id": ""
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `content` | `string` | Yes | — | The user's message. Max 64 KB. |
| `user_id` | `string` | No | `"default"` | User identifier for multi-tenant use. |
| `session_id` | `string` | No | `""` | Session ID for multi-turn conversations. |

**Response:** `200 OK`

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "echo": "Build a TODO app with React and SQLite",
  "reply": "I will help you build a TODO app...",
  "session_id": "abc-123-def",
  "phase": "code"
}
```

| Field | Type | Description |
|---|---|---|
| `id` | `string` | Message ID (UUID). |
| `echo` | `string` | Echo of the user's original message. |
| `reply` | `string` | The orchestrator's response. |
| `session_id` | `string?` | Session ID for multi-turn conversations. |
| `phase` | `string?` | Phase reached during orchestration. |

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/chat \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "content": "Build a TODO app with React and SQLite",
    "user_id": "alice",
    "session_id": "session-42"
  }'
```

### GET /api/chat/stream

WebSocket endpoint for real-time bidirectional chat streaming.

**Authentication:** Required (via `token` query parameter)

**Query Parameters:**

| Parameter | Type | Required | Description |
|---|---|---|---|
| `token` | `string` | Yes (when auth enabled) | Bearer token for authentication. |

**Protocol:**

- Client sends text messages (plain text content).
- Server broadcasts JSON-encoded kernel events.
- Connection is bidirectional: client sends, server streams events back.

**Example:**

```sh
# Connect via wscat
wscat -c "ws://127.0.0.1:4200/api/chat/stream?token=YOUR_TOKEN"
```

**JavaScript Example:**

```javascript
const ws = new WebSocket('ws://127.0.0.1:4200/api/chat/stream?token=YOUR_TOKEN');

ws.onopen = () => {
  ws.send('Build a REST API with Rust');
};

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('Event:', data);
};
```

---

## 3. System

### GET /health

Health check endpoint. **No authentication required.**

**Response:** `200 OK`

```json
{
  "status": "ok",
  "version": "0.2.0-alpha"
}
```

**Example:**

```sh
curl http://127.0.0.1:4200/health
```

### GET /api/status

System status with component-level health details.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "service": "oxios",
  "status": "running",
  "version": "0.2.0-alpha",
  "channels": ["web"],
  "uptime": "2h 15m 30s",
  "components": {
    "state_store": {
      "healthy": true,
      "detail": null
    },
    "event_bus": {
      "healthy": true,
      "detail": null
    },
    "memory": {
      "enabled": true,
      "index_size": 128,
      "total_entries": 512
    },
    "agents": {
      "active_count": 3,
      "total_forked": 42,
      "total_completed": 38,
      "total_failed": 1
    }
  }
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/status
```

### GET /api/config

Get the current Oxios configuration.

**Authentication:** Required

**Response:** `200 OK`

Returns the full `OxiosConfig` object serialized as JSON. Shape depends on config schema.

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/config
```

### PUT /api/config

Update the configuration. Validates the incoming JSON, persists to disk, and hot-reloads in memory.

**Authentication:** Required

**Request Body:** A valid `OxiosConfig` JSON object.

**Response:** `200 OK` — Returns the submitted config JSON.

```json
{
  "server": { "host": "127.0.0.1", "port": 4200 },
  "security": { "auth_enabled": true }
}
```

**Error Response:** `400 Bad Request` if the config shape is invalid.

**Example:**

```sh
curl -X PUT http://127.0.0.1:4200/api/config \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "server": { "host": "0.0.0.0", "port": 4200 },
    "security": { "auth_enabled": false }
  }'
```

---

## 4. Agents

### GET /api/agents

List all agent instances (paginated).

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `page` | `integer` | `1` | Page number (1-indexed). |
| `limit` | `integer` | `50` | Items per page (max 500). |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "id": "agent-550e8400",
      "name": "build-todo-app",
      "status": "Running",
      "created_at": "2026-05-17T10:30:00+00:00",
      "seed_id": "seed-abc-123"
    }
  ],
  "total": 42,
  "page": 1,
  "limit": 50
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/agents?page=1&limit=10"
```

### POST /api/agents/{id}/kill

Kill a running agent.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Agent ID to kill. |

**Response:** `200 OK` (empty body on success)

**Error Response:** `404 Not Found` if agent doesn't exist.

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/agents/agent-550e8400/kill \
  -H "Authorization: Bearer YOUR_TOKEN"
```

---

## 5. Workspace

Two workspace surfaces exist. `/api/workspace/*` browses the daemon's own
state workspace (`~/.oxios/workspace`). `/api/project/workspace/*` is the
session-scoped **project** filesystem API — a session bound to a project
reaches exactly its project's `root_paths` (there are no mounts).

### GET /api/workspace/tree

List the file tree of the workspace directory.

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Required | Description |
|---|---|---|---|
| `dir` | `string` | No | Subdirectory to list (relative to workspace root). |

**Response:** `200 OK`

```json
[
  {
    "name": "src",
    "is_dir": true,
    "size": 0
  },
  {
    "name": "Cargo.toml",
    "is_dir": false,
    "size": 512
  },
  {
    "name": "README.md",
    "is_dir": false,
    "size": 2048
  }
]
```

Directories are sorted before files; entries are sorted alphabetically within each group.

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/workspace/tree?dir=src"
```

### GET /api/workspace/file/{*path}

Read a file from the workspace.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `*path` | `string` | Relative file path within the workspace. |

**Response:** `200 OK` — File content with appropriate `Content-Type` header.

Supported MIME types: `.md` → `text/markdown`, `.json` → `application/json`, `.toml` → `application/toml`, `.yaml`/`.yml` → `application/yaml`, `.txt` → `text/plain`, `.html` → `text/html`, `.css` → `text/css`, `.js` → `application/javascript`. All others default to `text/plain`.

**Error Response:** `404 Not Found` or `403 Forbidden` (path traversal denied).

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/workspace/file/src/main.rs
```

### PUT /api/workspace/file/{*path}

Write or update a file in the workspace. Max body size: 1 MB.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `*path` | `string` | Relative file path within the workspace. |

**Request Body:** Plain text content (body is the file content).

**Response:** `200 OK` (empty body on success)

**Error Response:** `403 Forbidden` (path traversal), `413 Payload Too Large` (> 1 MB).

**Example:**

```sh
curl -X PUT http://127.0.0.1:4200/api/workspace/file/src/main.rs \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: text/plain" \
  -d 'fn main() { println!("Hello, Oxios!"); }'
```

### GET /api/project/workspace/tree

List the session's project workspace tree. The session's project roots are
the only reachable paths; a session without project binding returns
`409 Conflict`.

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Required | Description |
|---|---|---|---|
| `session_id` | `string` | No | Defaults to the most recent session. |
| `path` | `string` | No | Absolute directory to list (within roots). Defaults to `root_paths[0]`. |

**Response:** `200 OK` — `{ "roots": [...], "path": "...", "entries": [{ "name", "path", "is_dir", "size" }] }`
(`node_modules/`, `.git/`, `target/` and similar ignored entries are skipped).

**Error Response:** `409 Conflict` (no project roots), `403 Forbidden` (outside roots).

### GET /api/project/workspace/file

Read one file from the session's project workspace.

**Query Parameters:** `session_id` (optional), `path` (absolute file path within roots).

**Response:** `200 OK` — `{ "path", "content", "truncated", "size" }`.

### PUT /api/project/workspace/file

Write one file inside the session's project workspace (boundary-checked).

**Request Body:** JSON `{ "session_id"?, "path", "content" }`.

**Response:** `200 OK` — `{ "path", "bytes_written" }`.

**Error Response:** `409 Conflict` (no project roots), `403 Forbidden` (outside roots).

### GET /api/project/workspace/status

Git status for one project root of the session's project.

**Query Parameters:** `session_id` (optional), `root` (0-based root index, defaults to 0).

**Response:** `200 OK` — `{ "branch", "dirty_count" }`.

### GET /api/projects

List registered projects (paginated, `search` filter over name/instructions).

**Response:** `200 OK` — `{ "items": [Project], "total", "page", "limit" }` where
`Project = { id, name, root_paths, instructions, created_at, updated_at, last_active_at }`.

### POST /api/projects

Create a project. Root paths are validated (absolute, existing directory,
canonicalized, de-duplicated) **before** persistence; an empty list creates
a folderless project.

**Request Body:** `{ "name", "root_paths": [...], "instructions"? }`

**Response:** `201 Created` — Project.

**Error Response:** `422 Unprocessable Entity` (invalid root path / duplicate name).

### GET /api/projects/{id}

Get one project. **Response:** `200 OK` — Project, or `404 Not Found`.

### PUT /api/projects/{id}

Update name/root_paths/instructions (partial). Re-validates provided roots.
**Response:** `200 OK` — Project.

### DELETE /api/projects/{id}

Remove a project. Bound sessions are detached; their later turns run
without project filesystem context (never an ambient fallback).
**Response:** `204 No Content`.

### GET /api/projects/{id}/roots/status

Per-root liveness for a project — powers the stale-root badge in project
editing. Does not require a session.

**Response:** `200 OK`

```json
[
  { "path": "/home/me/src/oxios", "exists": true, "is_dir": true },
  { "path": "/home/me/old-notes", "exists": false, "is_dir": false }
]
```

`exists=false` (or `is_dir=false`) marks a stale root for the UI. Errors:
`404 Not Found` (unknown project id).

### POST /api/system/pick-folders

Open the native folder picker (local desktop host only, macOS). Cancelled
pickers return an empty list and change nothing; unavailable platforms get
`501 Not Implemented` with a local-only explanation.

**Request Body:** `{ "title"? }` (optional picker prompt).

**Response:** `200 OK` — `{ "paths": ["/absolute/dir", ...] }`.

**Error Response:** `501 Not Implemented` (non-macOS / remote host).

---

## 6. Seeds

### GET /api/seeds

List Ouroboros seeds (paginated).

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `page` | `integer` | `1` | Page number (1-indexed). |
| `limit` | `integer` | `50` | Items per page (max 500). |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "id": "seed-550e8400",
      "goal": "Build a REST API",
      "constraints_count": 3,
      "created_at": "2026-05-17T10:30:00+00:00"
    }
  ],
  "total": 15,
  "page": 1,
  "limit": 50
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/seeds?page=1&limit=20"
```

### GET /api/seeds/{id}

Get a specific seed by ID.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Seed ID. |

**Response:** `200 OK`

For JSON-format seeds:

```json
{
  "id": "seed-550e8400",
  "goal": "Build a REST API",
  "constraints": ["Use Rust", "Include tests"],
  "created_at": "2026-05-17T10:30:00+00:00",
  "generation": 2,
  "parent_seed_id": "seed-parent-001"
}
```

For markdown-format seeds:

```json
{
  "id": "my-seed",
  "content": "# My Seed\nBuild something great..."
}
```

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/seeds/seed-550e8400
```

### GET /api/seeds/{id}/evolution

Get the evolution lineage for a seed (parent chain traced back to root).

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Seed ID. |

**Response:** `200 OK`

```json
[
  {
    "id": "seed-root",
    "generation": 0,
    "goal": "Build a REST API",
    "parent_id": null,
    "score": 0.72,
    "passed": false
  },
  {
    "id": "seed-gen1",
    "generation": 1,
    "goal": "Build a REST API with proper error handling",
    "parent_id": "seed-root",
    "score": 0.89,
    "passed": true
  },
  {
    "id": "seed-550e8400",
    "generation": 2,
    "goal": "Build a production-ready REST API with tests",
    "parent_id": "seed-gen1",
    "score": 0.95,
    "passed": true
  }
]
```

Entries are ordered from root (oldest) to child (newest).

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/seeds/seed-550e8400/evolution
```

---

## 7. Skills

### GET /api/skills

List all registered skills (paginated).

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `page` | `integer` | `1` | Page number. |
| `limit` | `integer` | `50` | Items per page (max 500). |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "name": "code-review",
      "description": "Review code changes and provide feedback"
    },
    {
      "name": "deploy",
      "description": "Deploy the application to production"
    }
  ],
  "total": 2,
  "page": 1,
  "limit": 50
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/skills
```

### GET /api/skills/{name}

Get a specific skill's full content.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Skill name. |

**Response:** `200 OK`

```json
{
  "name": "code-review",
  "description": "Review code changes and provide feedback",
  "content": "# Code Review Skill\n\n## Steps\n1. Read diff...",
  "path": "/workspace/.oxios/skills/code-review.md"
}
```

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/skills/code-review
```

### POST /api/skills

Create a new skill. Max content size: 64 KB.

**Authentication:** Required

**Request Body:**

```json
{
  "name": "deploy",
  "description": "Deploy the application to production",
  "content": "# Deploy Skill\n\n## Steps\n1. Run tests\n2. Build\n3. Deploy"
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | `string` | Yes | — | Skill name (unique identifier). |
| `description` | `string` | Yes | — | Human-readable description. |
| `content` | `string` | No | `""` | Markdown content. Max 64 KB. |

**Response:** `200 OK`

```json
{
  "status": "created",
  "name": "deploy"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/skills \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "deploy",
    "description": "Deploy the application to production",
    "content": "# Deploy Skill\n\n1. Run tests\n2. Build\n3. Deploy"
  }'
```

### DELETE /api/skills/{name}

Delete a skill.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Skill name. |

**Response:** `200 OK`

```json
{
  "status": "deleted",
  "name": "deploy"
}
```

**Example:**

```sh
curl -X DELETE http://127.0.0.1:4200/api/skills/deploy \
  -H "Authorization: Bearer YOUR_TOKEN"
```

---

## 8. Memory

### GET /api/memory

List all memory entries (paginated). Includes both daily memory and knowledge base entries.

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `page` | `integer` | `1` | Page number. |
| `limit` | `integer` | `50` | Items per page (max 500). |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "name": "2026-05-17",
      "category": "daily"
    },
    {
      "name": "rust-patterns",
      "category": "knowledge"
    }
  ],
  "total": 25,
  "page": 1,
  "limit": 50
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/memory?page=1&limit=20"
```

### GET /api/memory/{name}

Get a specific memory entry by name.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Memory entry name. Searches `memory/` then `memory/knowledge/`. |

**Response:** `200 OK`

```json
{
  "name": "rust-patterns",
  "category": "knowledge",
  "content": "# Rust Patterns\n\n## Builder Pattern\n..."
}
```

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/memory/rust-patterns
```

### POST /api/memory

Create a new memory entry. Max content size: 32 KB.

**Authentication:** Required

**Request Body:**

```json
{
  "content": "Oxios uses an event-driven architecture with a broadcast bus.",
  "memory_type": "fact",
  "tags": ["architecture", "events"],
  "importance": 0.8
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `content` | `string` | Yes | — | Memory content. Max 32 KB. |
| `memory_type` | `string` | No | `"fact"` | One of: `fact`, `episode`, `knowledge`. |
| `tags` | `string[]` | No | `[]` | Tags for categorization. |
| `importance` | `float` | No | `0.5` | Importance score (0.0–1.0). |

**Response:** `200 OK`

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "created"
}
```

**Error Response:** `400 Bad Request` if `memory_type` is invalid.

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/memory \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "content": "Oxios uses an event-driven architecture",
    "memory_type": "fact",
    "tags": ["architecture"],
    "importance": 0.8
  }'
```

### POST /api/memory/search

Search memory entries by keyword.

**Authentication:** Required

**Request Body:**

```json
{
  "query": "architecture",
  "memory_type": "fact",
  "limit": 10
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | `string` | Yes | — | Search query string. |
| `memory_type` | `string` | No | `null` | Filter by type: `conversation`, `session`, `fact`, `episode`, `knowledge`. |
| `limit` | `integer` | No | `10` | Maximum results. |

**Response:** `200 OK`

```json
{
  "count": 2,
  "entries": [
    {
      "id": "a1b2c3d4",
      "type": "fact",
      "content": "Oxios uses an event-driven architecture",
      "tags": ["architecture"],
      "importance": 0.8,
      "created_at": "2026-05-17T10:30:00+00:00"
    }
  ]
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/memory/search \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"query": "architecture", "limit": 5}'
```

### POST /api/memory/semantic

Semantic search using HNSW approximate nearest neighbor index.

**Authentication:** Required

**Request Body:**

```json
{
  "query": "How does the event system work?",
  "memory_type": null,
  "limit": 10
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | `string` | Yes | — | Natural language query. |
| `memory_type` | `string` | No | `null` | Filter by type. |
| `limit` | `integer` | No | `10` | Maximum results. |

**Response:** `200 OK`

```json
{
  "count": 3,
  "entries": [
    {
      "id": "a1b2c3d4",
      "type": "fact",
      "content": "Oxios uses an event-driven architecture with a broadcast bus.",
      "tags": ["architecture", "events"],
      "importance": 0.8,
      "similarity": 0.94,
      "distance": 0.06,
      "created_at": "2026-05-17T10:30:00+00:00"
    }
  ],
  "engine": "hnsw"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/memory/semantic \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"query": "How does the event system work?", "limit": 5}'
```

---

## 9. Programs

### GET /api/programs

List all installed programs (paginated).

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `page` | `integer` | `1` | Page number. |
| `limit` | `integer` | `50` | Items per page (max 500). |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "name": "my-program",
      "version": "1.0.0",
      "description": "A custom Oxios program",
      "author": "oxios",
      "enabled": true,
      "tools_count": 3,
      "has_skill_content": true
    }
  ],
  "total": 5,
  "page": 1,
  "limit": 50
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/programs
```

### POST /api/programs

Install a program from a remote source (Git URL or tarball URL). Local path installation is not allowed via API.

**Authentication:** Required

**Request Body:**

```json
{
  "path": "https://example.com/my-program.tar.gz"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `path` | `string` | Yes | Git URL (`.git` suffix or `git@` prefix) or HTTP(S) tarball URL. Max 8192 chars. |

**Response:** `200 OK`

```json
{
  "status": "installed",
  "name": "my-program",
  "version": "1.0.0"
}
```

**Error Response:** `400 Bad Request` for invalid source or installation failure.

**Example:**

```sh
# Install from tarball
curl -X POST http://127.0.0.1:4200/api/programs \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"path": "https://example.com/my-program.tar.gz"}'

# Install from Git
curl -X POST http://127.0.0.1:4200/api/programs \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"path": "https://github.com/example/oxios-program.git"}'
```

### GET /api/programs/{name}

Get detailed information about a specific program.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Program name. |

**Response:** `200 OK`

```json
{
  "name": "my-program",
  "version": "1.0.0",
  "description": "A custom Oxios program",
  "author": "oxios",
  "enabled": true,
  "tools": [
    {
      "name": "analyze",
      "description": "Analyze code quality"
    }
  ],
  "skill_content": "# My Program Skill\n...",
  "path": "/workspace/.oxios/programs/my-program"
}
```

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/programs/my-program
```

### DELETE /api/programs/{name}

Uninstall a program.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Program name. |

**Response:** `200 OK`

```json
{
  "status": "uninstalled",
  "name": "my-program"
}
```

**Example:**

```sh
curl -X DELETE http://127.0.0.1:4200/api/programs/my-program \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### POST /api/programs/{name}/enable

Enable a disabled program.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Program name. |

**Response:** `200 OK`

```json
{
  "status": "enabled",
  "name": "my-program"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/programs/my-program/enable \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### POST /api/programs/{name}/disable

Disable a program (without uninstalling).

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Program name. |

**Response:** `200 OK`

```json
{
  "status": "disabled",
  "name": "my-program"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/programs/my-program/disable \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### GET /api/programs/{name}/host-requirements

Check host requirements for a program.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `name` | `string` | Program name. |

**Response:** `200 OK` — Returns the host requirements check result (shape depends on program manifest).

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/programs/my-program/host-requirements
```

---

## 10. Scheduler

### GET /api/scheduler/stats

Get scheduler statistics.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "queued": 5,
  "running": 2,
  "max_concurrent": 10,
  "rate_limit_per_minute": 60,
  "rate_remaining": 45
}
```

| Field | Type | Description |
|---|---|---|
| `queued` | `integer` | Number of tasks waiting in the queue. |
| `running` | `integer` | Number of currently executing tasks. |
| `max_concurrent` | `integer` | Maximum concurrent tasks allowed. |
| `rate_limit_per_minute` | `integer` | Rate limit (requests per minute). |
| `rate_remaining` | `integer` | Remaining requests in current window. |

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/scheduler/stats
```

### GET /api/scheduler/tasks

List queued and running tasks.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "queued": [
    {
      "id": "task-001",
      "description": "Build TODO app",
      "priority": "High",
      "status": "Queued",
      "created_at": "2026-05-17T10:30:00+00:00",
      "error": null
    }
  ],
  "running": [
    {
      "id": "task-002",
      "description": "Refactor auth module",
      "priority": "Normal",
      "status": "Running",
      "created_at": "2026-05-17T10:25:00+00:00",
      "error": null
    }
  ]
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/scheduler/tasks
```

---

## 11. Audit

### GET /api/audit/entries

Query audit trail entries within a sequence range.

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `from_seq` | `integer` | `0` | Starting sequence number (inclusive). |
| `to_seq` | `integer` | `100` | Ending sequence number (inclusive). |

**Response:** `200 OK`

```json
{
  "entries": [
    {
      "seq": 1,
      "timestamp": "2026-05-17T10:30:00+00:00",
      "agent_name": "builder-agent",
      "action": "file_write",
      "resource": "/workspace/src/main.rs",
      "allowed": true,
      "hash": "sha256:abc123...",
      "prev_hash": "sha256:def456..."
    }
  ],
  "count": 1
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/audit/entries?from_seq=0&to_seq=50"
```

### GET /api/audit/verify

Verify the cryptographic hash chain integrity of the entire audit trail.

**Authentication:** Required

**Response (valid chain):** `200 OK`

```json
{
  "valid": true,
  "entry_count": 150
}
```

**Response (broken chain):** `200 OK`

```json
{
  "valid": false,
  "entry_count": 150,
  "broken_at_seq": 42,
  "expected": "sha256:abc123...",
  "found": "sha256:def456..."
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/audit/verify
```

### GET /api/audit/agent/{agent_id}

Query all audit entries for a specific agent.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent_id` | `string` | Agent identifier. |

**Response:** `200 OK`

```json
{
  "entries": [
    {
      "seq": 5,
      "agent_name": "builder-agent",
      "action": "file_write",
      "resource": "/workspace/src/main.rs",
      "allowed": true
    }
  ],
  "count": 1
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/audit/agent/builder-agent
```

### POST /api/audit/export

Export audit entries as JSON starting from a sequence number.

**Authentication:** Required

**Request Body:**

```json
{
  "from_seq": 0
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `from_seq` | `integer` | No | `0` | Starting sequence number. |

**Response:** `200 OK`

```json
{
  "json": "[{\"seq\":0,...},{\"seq\":1,...}]",
  "entry_count": 150
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/audit/export \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"from_seq": 100}'
```

### POST /api/audit/flush

Flush in-memory audit entries to the StateStore for persistence.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "flushed": 150
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/audit/flush \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### GET /api/audit

Legacy audit log endpoint (paginated). Returns security audit entries in reverse chronological order.

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `page` | `integer` | `1` | Page number. |
| `limit` | `integer` | `50` | Items per page (max 500). |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "timestamp": "2026-05-17T10:30:00+00:00",
      "agent_name": "builder-agent",
      "action": "file_write",
      "resource": "/workspace/src/main.rs",
      "allowed": true,
      "reason": null
    }
  ],
  "total": 150,
  "page": 1,
  "limit": 50
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/audit?page=1&limit=20"
```

---

## 12. Permissions

### GET /api/permissions/{agent}

Get permissions for a specific agent.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent` | `string` | Agent name. |

**Response:** `200 OK`

```json
{
  "agent_name": "builder-agent",
  "allowed_tools": ["file_read", "file_write", "shell_exec"],
  "allowed_paths": ["/workspace/src"],
  "denied_paths": ["/etc", "/root"],
  "network_access": true,
  "max_execution_time_secs": 300,
  "max_memory_mb": 512,
  "can_fork": false
}
```

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/permissions/builder-agent
```

### PUT /api/permissions/{agent}

Update permissions for a specific agent.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent` | `string` | Agent name. |

**Request Body:**

```json
{
  "allowed_tools": ["file_read", "file_write"],
  "allowed_paths": ["/workspace/src"],
  "denied_paths": ["/etc"],
  "network_access": false,
  "max_execution_time_secs": 120,
  "max_memory_mb": 256,
  "can_fork": false
}
```

All fields are optional — only provided fields will be updated.

**Response:** `200 OK`

```json
{
  "status": "updated",
  "agent": "builder-agent"
}
```

**Example:**

```sh
curl -X PUT http://127.0.0.1:4200/api/permissions/builder-agent \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "network_access": false,
    "max_execution_time_secs": 120
  }'
```

---

## 13. Cron Jobs

### GET /api/cron-jobs

List all cron jobs.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "jobs": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "daily-backup",
      "schedule": "0 0 * * * *",
      "goal": "Create a backup of the workspace",
      "constraints": ["Include all source files"],
      "toolchain": "default",
      "priority": "Normal",
      "last_run": "2026-05-17T00:00:00+00:00",
      "last_success": true
    }
  ]
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/cron-jobs
```

### POST /api/cron-jobs

Create a new cron job.

**Authentication:** Required

**Request Body:**

```json
{
  "name": "daily-backup",
  "schedule": "0 0 * * * *",
  "goal": "Create a backup of the workspace",
  "constraints": ["Include all source files"],
  "acceptance_criteria": ["Backup file exists"],
  "toolchain": "default",
  "priority": "Normal"
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | `string` | Yes | — | Human-readable job name. |
| `schedule` | `string` | Yes | — | Cron expression (6-field: sec min hour day month weekday). |
| `goal` | `string` | Yes | — | The goal/prompt to execute. |
| `constraints` | `string[]` | No | `[]` | Optional constraints. |
| `acceptance_criteria` | `string[]` | No | `[]` | Optional acceptance criteria. |
| `toolchain` | `string` | No | `"default"` | Toolchain to use. |
| `priority` | `string` | No | `"Normal"` | Priority: `Low`, `Normal`, `High`, `Critical`. |

**Response:** `200 OK`

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/cron-jobs \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "daily-backup",
    "schedule": "0 0 * * * *",
    "goal": "Create a backup of the workspace",
    "constraints": ["Include all source files"]
  }'
```

### GET /api/cron-jobs/{id}

Get a specific cron job.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string (UUID)` | Cron job ID. |

**Response:** `200 OK` — Returns the full `CronJob` object.

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/cron-jobs/550e8400-e29b-41d4-a716-446655440000
```

### DELETE /api/cron-jobs/{id}

Delete a cron job.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string (UUID)` | Cron job ID. |

**Response:** `200 OK`

```json
{
  "deleted": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Example:**

```sh
curl -X DELETE http://127.0.0.1:4200/api/cron-jobs/550e8400-e29b-41d4-a716-446655440000 \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### POST /api/cron-jobs/{id}/edit

Update a cron job's configuration.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string (UUID)` | Cron job ID. |

**Request Body:** A `CronJobUpdate` object with optional fields to update.

**Response:** `200 OK`

```json
{
  "updated": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/cron-jobs/550e8400-e29b-41d4-a716-446655440000/edit \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"schedule": "0 */6 * * * *"}'
```

### POST /api/cron-jobs/{id}/trigger

Manually trigger a cron job execution.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string (UUID)` | Cron job ID. |

**Response:** `200 OK`

```json
{
  "job_id": "550e8400-e29b-41d4-a716-446655440000",
  "success": true,
  "summary": "Backup completed successfully. 42 files backed up."
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/cron-jobs/550e8400-e29b-41d4-a716-446655440000/trigger \
  -H "Authorization: Bearer YOUR_TOKEN"
```

---

## 14. Sessions

### GET /api/sessions

List recent sessions (paginated).

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `page` | `integer` | `1` | Page number. |
| `limit` | `integer` | `50` | Items per page (max 500). |

**Response:** `200 OK`

```json
{
  "items": [
    {
      "id": "session-abc-123",
      "user_id": "alice",
      "message_count": 5,
      "active_seed_id": "seed-550e8400",
      "created_at": "2026-05-17T10:00:00+00:00",
      "updated_at": "2026-05-17T10:30:00+00:00"
    }
  ],
  "total": 20,
  "page": 1,
  "limit": 50
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/sessions?page=1&limit=10"
```

### GET /api/sessions/{id}

Get a session with full message history.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Session ID. |

**Response:** `200 OK`

```json
{
  "id": "session-abc-123",
  "user_id": "alice",
  "project_id": "550e8400-e29b-41d4-a716-446655440000",
  "user_messages": ["Build a TODO app", "Add tests"],
  "agent_responses": [
    {
      "content": "I will help you build a TODO app...",
      "session_id": "session-abc-123",
      "seed_id": "seed-550e8400",
      "phase_reached": "code",
      "evaluation_passed": true,
      "timestamp": "2026-05-17T10:15:00+00:00"
    }
  ],
  "active_seed_id": "seed-550e8400",
  "active_persona_id": "persona-dev",
  "effective_profile": {
    "persona_id": "persona-dev",
    "tool_profile": "code",
    "affordances": ["diff", "files", "terminal", "worktree-fanout"]
  },
  "created_at": "2026-05-17T10:00:00+00:00",
  "updated_at": "2026-05-17T10:30:00+00:00",
  "metadata": {}
}
```

`project_id` is the top-level field with legacy `metadata.project_id(s)`
fallbacks. `effective_profile` is the server-derived snapshot of the bound
persona's affordances for this session's project (design §6.1); `null`
when no persona binds. Web UI affordances render from it — never from
persona capability strings.

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/sessions/session-abc-123
```

### DELETE /api/sessions/{id}

Delete a session.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Session ID. |

**Response:** `200 OK`

```json
{
  "status": "deleted",
  "id": "session-abc-123"
}
```

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -X DELETE http://127.0.0.1:4200/api/sessions/session-abc-123 \
  -H "Authorization: Bearer YOUR_TOKEN"
```

---

## 15. Personas

### GET /api/personas

List all personas.

**Authentication:** Required

**Response:** `200 OK`

```json
[
  {
    "id": "persona-dev",
    "name": "Developer",
    "role": "Software Engineer",
    "description": "A skilled software developer persona",
    "enabled": true,
    "personality_traits": ["analytical", "thorough", "pragmatic"]
  },
  {
    "id": "persona-creative",
    "name": "Creative",
    "role": "Designer",
    "description": "A creative design-focused persona",
    "enabled": true,
    "personality_traits": ["creative", "visual", "empathetic"]
  }
]
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/personas
```

### POST /api/personas

Create a new persona.

**Authentication:** Required

**Request Body:**

```json
{
  "name": "Reviewer",
  "role": "Code Reviewer",
  "description": "A thorough code review specialist",
  "system_prompt": "You are an expert code reviewer...",
  "enabled": true,
  "model": "gpt-4",
  "personality_traits": ["detail-oriented", "constructive"]
}
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | `string` | Yes | — | Persona display name. |
| `role` | `string` | Yes | — | Persona role. |
| `description` | `string` | Yes | — | Description. |
| `system_prompt` | `string` | No | `""` | System prompt for LLM. |
| `enabled` | `boolean` | No | `true` | Whether the persona is enabled. |
| `model` | `string` | No | `null` | Preferred LLM model. |
| `personality_traits` | `string[]` | No | `[]` | Personality descriptors. |

**Response:** `200 OK`

```json
{
  "status": "created",
  "id": "a1b2c3d4-e5f6-7890",
  "name": "Reviewer"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/personas \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Reviewer",
    "role": "Code Reviewer",
    "description": "A thorough code review specialist",
    "system_prompt": "You are an expert code reviewer."
  }'
```

### GET /api/personas/{id}

Get a specific persona with full details.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Persona ID. |

**Response:** `200 OK`

```json
{
  "id": "persona-dev",
  "name": "Developer",
  "role": "Software Engineer",
  "description": "A skilled software developer persona",
  "system_prompt": "You are an expert software developer...",
  "enabled": true,
  "model": "gpt-4",
  "personality_traits": ["analytical", "thorough"]
}
```

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/personas/persona-dev
```

### PUT /api/personas/{id}

Update a persona. Only provided fields are updated.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Persona ID. |

**Request Body:**

```json
{
  "name": "Senior Developer",
  "system_prompt": "You are a senior software engineer...",
  "enabled": true
}
```

All fields are optional (partial update).

**Response:** `200 OK`

```json
{
  "status": "updated",
  "id": "persona-dev"
}
```

**Example:**

```sh
curl -X PUT http://127.0.0.1:4200/api/personas/persona-dev \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "Senior Developer", "enabled": true}'
```

### DELETE /api/personas/{id}

Delete a persona. Cannot delete the last remaining persona.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Persona ID. |

**Response:** `200 OK`

```json
{
  "status": "deleted",
  "id": "persona-creative"
}
```

**Error Response:** `400 Bad Request` if deleting the last persona. `404 Not Found` if persona doesn't exist.

**Example:**

```sh
curl -X DELETE http://127.0.0.1:4200/api/personas/persona-creative \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### GET /api/personas/active

Get the currently active persona.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "id": "persona-dev",
  "name": "Developer",
  "role": "Software Engineer",
  "description": "A skilled software developer persona",
  "system_prompt": "You are an expert software developer...",
  "enabled": true
}
```

**Response (no active persona):**

```json
{
  "active": false,
  "message": "No active persona set"
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/personas/active
```

### PUT /api/personas/active

Set the active persona.

**Authentication:** Required

**Request Body:**

```json
{
  "id": "persona-dev"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `id` | `string` | Yes | Persona ID to activate. |

**Response:** `200 OK`

```json
{
  "status": "active",
  "id": "persona-dev",
  "name": "Developer"
}
```

**Example:**

```sh
curl -X PUT http://127.0.0.1:4200/api/personas/active \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"id": "persona-dev"}'
```

---

## 16. Git

### GET /api/git/log

Get the commit log (up to 100 entries).

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "entries": [
    {
      "hash": "abc123def456",
      "author": "oxios-agent",
      "message": "feat: add TODO app components",
      "timestamp": "2026-05-17T10:30:00+00:00"
    }
  ]
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/git/log
```

### GET /api/git/tags

List all Git tags.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "tags": ["v0.1.0", "0.1.2", "seed-evolution-3"]
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/git/tags
```

### POST /api/git/verify

Verify repository integrity.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "valid": true
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/git/verify \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### POST /api/git/restore

Restore a file to its state at a specific commit.

**Authentication:** Required

**Request Body:**

```json
{
  "hash": "abc123def456",
  "path": "src/main.rs"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `hash` | `string` | Yes | Commit hash to restore from. |
| `path` | `string` | Yes | Relative file path to restore. |

**Response:** `200 OK`

```json
{
  "restored": "src/main.rs",
  "from": "abc123def456"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/git/restore \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"hash": "abc123def456", "path": "src/main.rs"}'
```

---

## 17. Budget

### GET /api/budget/{agent_id}

Get budget status for an agent.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent_id` | `string` | Agent identifier. |

**Response:** `200 OK`

```json
{
  "agent_id": "builder-agent",
  "tokens_remaining": 45000,
  "calls_remaining": 95,
  "window_remaining_secs": 3420,
  "is_exhausted": false
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/budget/builder-agent
```

### POST /api/budget/{agent_id}

Set budget limits for an agent.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent_id` | `string` | Agent identifier. |

**Request Body:**

```json
{
  "token_budget": 50000,
  "calls_budget": 100,
  "window_secs": 3600
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `token_budget` | `integer` | Yes | Maximum tokens allowed in window. |
| `calls_budget` | `integer` | Yes | Maximum API calls in window. |
| `window_secs` | `integer` | Yes | Budget window duration in seconds. |

**Response:** `200 OK`

```json
{
  "set": true,
  "agent_id": "builder-agent"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/budget/builder-agent \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"token_budget": 50000, "calls_budget": 100, "window_secs": 3600}'
```

### DELETE /api/budget/{agent_id}

Remove budget limits for an agent.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent_id` | `string` | Agent identifier. |

**Response:** `200 OK`

```json
{
  "removed": true,
  "agent_id": "builder-agent"
}
```

**Example:**

```sh
curl -X DELETE http://127.0.0.1:4200/api/budget/builder-agent \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### POST /api/budget/{agent_id}/reserve

Reserve tokens from an agent's budget.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent_id` | `string` | Agent identifier. |

**Request Body:**

```json
{
  "tokens": 5000
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `tokens` | `integer` | Yes | Number of tokens to reserve. |

**Response:** `200 OK`

```json
{
  "reserved": true
}
```

**Error Response:** `500 Internal Server Error` if budget is exceeded.

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/budget/builder-agent/reserve \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tokens": 5000}'
```

### POST /api/budget/{agent_id}/reset

Reset an agent's budget window (restore full allocation).

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `agent_id` | `string` | Agent identifier. |

**Response:** `200 OK`

```json
{
  "reset": true,
  "agent_id": "builder-agent"
}
```

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/budget/builder-agent/reset \
  -H "Authorization: Bearer YOUR_TOKEN"
```

---

## 18. Resources

### GET /api/resources

Get the current system resource snapshot.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "cpu_percent": 45.2,
  "memory_percent": 62.8,
  "memory_used_mb": 6420,
  "memory_total_mb": 10240,
  "load_avg_1m": 2.1,
  "load_avg_5m": 1.8,
  "load_avg_15m": 1.5,
  "timestamp": "2026-05-17T10:30:00+00:00"
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/resources
```

### GET /api/resources/history

Get historical resource snapshots.

**Authentication:** Required

**Query Parameters:**

| Parameter | Type | Default | Description |
|---|---|---|---|
| `last_n` | `integer` | `30` | Number of most recent snapshots. |

**Response:** `200 OK`

```json
{
  "snapshots": [
    {
      "cpu_percent": 45.2,
      "memory_percent": 62.8,
      "timestamp": "2026-05-17T10:30:00+00:00"
    },
    {
      "cpu_percent": 38.1,
      "memory_percent": 60.5,
      "timestamp": "2026-05-17T10:25:00+00:00"
    }
  ],
  "count": 2
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  "http://127.0.0.1:4200/api/resources/history?last_n=10"
```

### GET /api/resources/overload

Check if the system is currently overloaded.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "overloaded": false,
  "threshold": {
    "cpu_percent": 90.0,
    "memory_percent": 90.0,
    "load_avg": 10.0
  }
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/resources/overload
```

---

## 19. Agent Groups

### GET /api/agent-groups

List all agent groups from the state store.

**Authentication:** Required

**Response:** `200 OK`

```json
[
  {
    "id": "group-550e8400",
    "name": "parallel-build",
    "agent_count": 3,
    "status": "running",
    "created_at": "2026-05-17T10:30:00+00:00"
  }
]
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/agent-groups
```

### GET /api/agent-groups/{id}

Get a specific agent group by ID.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string` | Agent group ID. |

**Response:** `200 OK` — Returns the full agent group object.

**Error Response:** `404 Not Found`

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/agent-groups/group-550e8400
```

---

## 20. Approvals (HitL)

Human-in-the-Loop (HitL) approval workflow for agent actions.

### GET /api/approvals

List all approval requests (pending and history).

**Authentication:** Required

**Response:** `200 OK`

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "subject": "agent:builder-agent",
    "action": "use_tool:shell_exec",
    "resource": "/workspace/scripts/deploy.sh",
    "reason": "Agent requests shell execution for deployment",
    "created_at": "2026-05-17T10:30:00+00:00",
    "status": "pending"
  }
]
```

Possible `action` values: `use_tool:{tool}`, `access_path:{path}`, `manage_agents`, `manage_programs`, `manage_workspaces`, `manage_rbac`, `view_audit_log`, `system_config`.

Possible `status` values: `pending`, `approved`, `rejected`, `expired`.

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/approvals
```

### POST /api/approvals/{id}/approve

Approve a pending approval request.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string (UUID)` | Approval request ID. |

**Response:** `200 OK`

```json
{
  "status": "approved",
  "id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Error Response:** `400 Bad Request` (invalid UUID) or `404 Not Found`.

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/approvals/550e8400-e29b-41d4-a716-446655440000/approve \
  -H "Authorization: Bearer YOUR_TOKEN"
```

### POST /api/approvals/{id}/reject

Reject a pending approval request.

**Authentication:** Required

**Path Parameters:**

| Parameter | Type | Description |
|---|---|---|
| `id` | `string (UUID)` | Approval request ID. |

**Response:** `200 OK`

```json
{
  "status": "rejected",
  "id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Error Response:** `400 Bad Request` (invalid UUID) or `404 Not Found`.

**Example:**

```sh
curl -X POST http://127.0.0.1:4200/api/approvals/550e8400-e29b-41d4-a716-446655440000/reject \
  -H "Authorization: Bearer YOUR_TOKEN"
```

---

## 21. SSE Events

### GET /api/events

Subscribe to a Server-Sent Events (SSE) stream of kernel events.

**Authentication:** Required

**Keep-Alive:** Ping every 30 seconds.

**Event Types:**

| Event Type | Description |
|---|---|
| `agent_created` | New agent spawned |
| `agent_started` | Agent begins execution |
| `agent_stopped` | Agent finished execution |
| `agent_failed` | Agent encountered an error |
| `message_received` | Message received on channel (content excluded) |
| `seed_created` | New Ouroboros seed created |
| `evaluation_complete` | Seed evaluation finished |
| `phase_started` | Orchestration phase started |
| `phase_completed` | Orchestration phase completed |
| `agent_output` | Agent produced output (content excluded) |
| `approval_requested` | HitL approval requested |
| `approval_resolved` | HitL approval approved/rejected |
| `memory_stored` | Memory entry stored |
| `memory_recalled` | Memory entries recalled |
| `agent_group_created` | Agent group created |
| `agent_group_member_completed` | Agent group member finished |
| `space_created` | Knowledge space created |
| `space_activated` | Knowledge space activated |
| `space_archived` | Knowledge space archived |
| `spaces_merged` | Knowledge spaces merged |
| `knowledge_cross_referenced` | Knowledge cross-referenced between spaces |

**Event Format (SSE):**

```
data: {"type":"agent_started","agent_id":"abc-123"}
```

**Example (curl):**

```sh
curl -N -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/events
```

**Example (JavaScript):**

```javascript
const eventSource = new EventSource('http://127.0.0.1:4200/api/events');

// Note: For auth, you may need to proxy or use a custom fetch-based SSE client
// since EventSource doesn't support custom headers.

eventSource.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log(`[${data.type}]`, data);
};
```

**Example (Python):**

```python
import sseclient
import requests
import json

response = requests.get(
    'http://127.0.0.1:4200/api/events',
    stream=True,
    headers={
        'Accept': 'text/event-stream',
        'Authorization': 'Bearer YOUR_TOKEN'
    }
)

client = sseclient.SSEClient(response)

for event in client.events():
    data = json.loads(event.data)
    print(f"[{data.get('type', 'unknown')}] {data}")
```

> **Note:** Sensitive data (message content, full seed content, LLM responses) is excluded from SSE events for security.

---

## 22. Host Tools

### GET /api/host-tools

Check availability of required and optional host tools.

**Authentication:** Required

**Response:** `200 OK`

```json
{
  "all_required_present": true,
  "missing_required": [],
  "optional_available": {
    "git": true,
    "docker": false,
    "node": true,
    "cargo": true
  }
}
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/host-tools
```

---

## 23. Metrics

### GET /api/metrics

Prometheus-compatible metrics endpoint.

**Authentication:** Required

**Response:** `200 OK` — Plain text in Prometheus exposition format.

```
# HELP oxios_agents_forked_total Total agents forked
# TYPE oxios_agents_forked_total counter
oxios_agents_forked_total 42

# HELP oxios_agents_completed_total Total agents completed
# TYPE oxios_agents_completed_total counter
oxios_agents_completed_total 38

# HELP oxios_agents_failed_total Total agents failed
# TYPE oxios_agents_failed_total counter
oxios_agents_failed_total 1
```

**Example:**

```sh
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://127.0.0.1:4200/api/metrics
```

**Prometheus Scrape Config:**

```yaml
scrape_configs:
  - job_name: 'oxios'
    static_configs:
      - targets: ['127.0.0.1:4200']
    metrics_path: '/api/metrics'
    bearer_token: 'YOUR_TOKEN'
```

---

## 24. Error Responses

All errors follow a consistent JSON format:

```json
{
  "error": "Description of the error"
}
```

**HTTP Status Codes:**

| Status | Code | Description |
|---|---|---|
| `400 Bad Request` | 400 | Invalid request body or parameters |
| `401 Unauthorized` | 401 | Missing or invalid authentication token |
| `403 Forbidden` | 403 | Path traversal or permission denied |
| `404 Not Found` | 404 | Resource not found |
| `413 Payload Too Large` | 413 | Request body exceeds size limit |
| `429 Too Many Requests` | 429 | Rate limit exceeded |
| `500 Internal Server Error` | 500 | Server-side error |

---

## 25. Rate Limiting

All `/api/*` endpoints are protected by a token-bucket rate limiter. The limiter is configured at server startup with a maximum requests-per-minute setting.

- **Refill rate:** `max_requests_per_minute / 60` tokens per second
- **Burst capacity:** `max_requests_per_minute`
- **Response when limited:** `HTTP 429 Too Many Requests`

---

## 26. Pagination

Many list endpoints support pagination with these query parameters:

| Parameter | Type | Default | Max | Description |
|---|---|---|---|---|
| `page` | `integer` | `1` | — | Page number (1-indexed). |
| `limit` | `integer` | `50` | `500` | Items per page. |

**Paginated Response Format:**

```json
{
  "items": [...],
  "total": 150,
  "page": 1,
  "limit": 50
}
```

- `items` contains the current page's items.
- `total` is the total number of items across all pages.
- Page 0 is treated as page 1 (underflow protection via `saturating_sub`).

---

## Endpoint Summary

| # | Method | Path | Auth | Description |
|---|---|---|---|---|
| 1 | `GET` | `/health` | No | Health check |
| 2 | `POST` | `/api/chat` | Yes | Send chat message |
| 3 | `GET` | `/api/chat/stream` | Yes* | WebSocket chat stream |
| 4 | `GET` | `/api/status` | Yes | System status with component health |
| 5 | `GET` | `/api/agents` | Yes | List agents (paginated) |
| 6 | `POST` | `/api/agents/{id}/kill` | Yes | Kill an agent |
| 7 | `GET` | `/api/config` | Yes | Get configuration |
| 8 | `PUT` | `/api/config` | Yes | Update configuration |
| 9 | `GET` | `/api/workspace/tree` | Yes | List workspace file tree |
| 10 | `GET` | `/api/workspace/file/{*path}` | Yes | Read workspace file |
| 11 | `PUT` | `/api/workspace/file/{*path}` | Yes | Write workspace file |
| 12 | `GET` | `/api/seeds` | Yes | List seeds (paginated) |
| 13 | `GET` | `/api/seeds/{id}` | Yes | Get a seed |
| 14 | `GET` | `/api/seeds/{id}/evolution` | Yes | Get seed evolution lineage |
| 15 | `GET` | `/api/skills` | Yes | List skills (paginated) |
| 16 | `GET` | `/api/skills/{name}` | Yes | Get a skill |
| 17 | `POST` | `/api/skills` | Yes | Create a skill |
| 18 | `DELETE` | `/api/skills/{name}` | Yes | Delete a skill |
| 19 | `GET` | `/api/memory` | Yes | List memory entries (paginated) |
| 20 | `POST` | `/api/memory` | Yes | Create a memory entry |
| 21 | `GET` | `/api/memory/{name}` | Yes | Get a memory entry |
| 22 | `POST` | `/api/memory/search` | Yes | Search memory |
| 23 | `POST` | `/api/memory/semantic` | Yes | Semantic search (HNSW) |
| 24 | `GET` | `/api/programs` | Yes | List programs (paginated) |
| 25 | `POST` | `/api/programs` | Yes | Install a program |
| 26 | `GET` | `/api/programs/{name}` | Yes | Get a program |
| 27 | `DELETE` | `/api/programs/{name}` | Yes | Uninstall a program |
| 28 | `POST` | `/api/programs/{name}/enable` | Yes | Enable a program |
| 29 | `POST` | `/api/programs/{name}/disable` | Yes | Disable a program |
| 30 | `GET` | `/api/programs/{name}/host-requirements` | Yes | Check host requirements |
| 31 | `GET` | `/api/scheduler/stats` | Yes | Scheduler statistics |
| 32 | `GET` | `/api/scheduler/tasks` | Yes | List scheduler tasks |
| 33 | `GET` | `/api/audit/entries` | Yes | Query audit entries by range |
| 34 | `GET` | `/api/audit/verify` | Yes | Verify audit chain integrity |
| 35 | `GET` | `/api/audit/agent/{agent_id}` | Yes | Query audit by agent |
| 36 | `POST` | `/api/audit/export` | Yes | Export audit entries |
| 37 | `POST` | `/api/audit/flush` | Yes | Flush audit to disk |
| 38 | `GET` | `/api/audit` | Yes | Legacy audit log (paginated) |
| 39 | `GET` | `/api/permissions/{agent}` | Yes | Get agent permissions |
| 40 | `PUT` | `/api/permissions/{agent}` | Yes | Update agent permissions |
| 41 | `GET` | `/api/cron-jobs` | Yes | List cron jobs |
| 42 | `POST` | `/api/cron-jobs` | Yes | Create a cron job |
| 43 | `GET` | `/api/cron-jobs/{id}` | Yes | Get a cron job |
| 44 | `DELETE` | `/api/cron-jobs/{id}` | Yes | Delete a cron job |
| 45 | `POST` | `/api/cron-jobs/{id}/edit` | Yes | Update a cron job |
| 46 | `POST` | `/api/cron-jobs/{id}/trigger` | Yes | Manually trigger a cron job |
| 47 | `GET` | `/api/sessions` | Yes | List sessions (paginated) |
| 48 | `GET` | `/api/sessions/{id}` | Yes | Get session with history |
| 49 | `DELETE` | `/api/sessions/{id}` | Yes | Delete a session |
| 50 | `GET` | `/api/personas` | Yes | List personas |
| 51 | `POST` | `/api/personas` | Yes | Create a persona |
| 52 | `GET` | `/api/personas/{id}` | Yes | Get a persona |
| 53 | `PUT` | `/api/personas/{id}` | Yes | Update a persona |
| 54 | `DELETE` | `/api/personas/{id}` | Yes | Delete a persona |
| 55 | `GET` | `/api/personas/active` | Yes | Get active persona |
| 56 | `PUT` | `/api/personas/active` | Yes | Set active persona |
| 57 | `GET` | `/api/git/log` | Yes | Git commit log |
| 58 | `GET` | `/api/git/tags` | Yes | List Git tags |
| 59 | `POST` | `/api/git/verify` | Yes | Verify repo integrity |
| 60 | `POST` | `/api/git/restore` | Yes | Restore file from commit |
| 61 | `GET` | `/api/budget/{agent_id}` | Yes | Get agent budget |
| 62 | `POST` | `/api/budget/{agent_id}` | Yes | Set agent budget |
| 63 | `DELETE` | `/api/budget/{agent_id}` | Yes | Remove agent budget |
| 64 | `POST` | `/api/budget/{agent_id}/reserve` | Yes | Reserve budget tokens |
| 65 | `POST` | `/api/budget/{agent_id}/reset` | Yes | Reset agent budget |
| 66 | `GET` | `/api/resources` | Yes | Resource snapshot |
| 67 | `GET` | `/api/resources/history` | Yes | Resource history |
| 68 | `GET` | `/api/resources/overload` | Yes | Overload check |
| 69 | `GET` | `/api/agent-groups` | Yes | List agent groups |
| 70 | `GET` | `/api/agent-groups/{id}` | Yes | Get agent group |
| 71 | `GET` | `/api/approvals` | Yes | List approval requests |
| 72 | `POST` | `/api/approvals/{id}/approve` | Yes | Approve a request |
| 73 | `POST` | `/api/approvals/{id}/reject` | Yes | Reject a request |
| 74 | `GET` | `/api/events` | Yes | SSE event stream |
| 75 | `GET` | `/api/host-tools` | Yes | Check host tools |
| 76 | `GET` | `/api/metrics` | Yes | Prometheus metrics |
| 77 | `GET` | `/api/project/workspace/tree` | Yes | List session project workspace tree |
| 78 | `GET` | `/api/project/workspace/file` | Yes | Read project workspace file |
| 79 | `PUT` | `/api/project/workspace/file` | Yes | Write project workspace file |
| 80 | `GET` | `/api/project/workspace/status` | Yes | Git status for a project root |
| 81 | `GET` | `/api/projects` | Yes | List projects (paginated) |
| 82 | `POST` | `/api/projects` | Yes | Create a project |
| 83 | `GET` | `/api/projects/{id}` | Yes | Get a project |
| 84 | `PUT` | `/api/projects/{id}` | Yes | Update a project |
| 85 | `DELETE` | `/api/projects/{id}` | Yes | Delete a project |
| 86 | `GET` | `/api/projects/{id}/roots/status` | Yes | Per-root exists/is_dir status |
| 87 | `POST` | `/api/system/pick-folders` | Yes | Native folder picker (local host) |

*\* WebSocket auth uses `?token=` query parameter instead of `Authorization` header.*
