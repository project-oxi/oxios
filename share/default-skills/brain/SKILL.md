---
name: brain
description: Persistent memory over the oxibrain CLI — remember durable facts and recall them across sessions. Use when storing user preferences, behavioral patterns, session observations, or answering questions about past events, people, and projects.
---

# Brain — persistent memory

Durable memory lives in the local `oxibrain` store. You write facts; the
kernel extracts and indexes them in the background. Everything is via the
`oxibrain` binary (pre-allowed in exec structured mode).

**`<brain-dir>`** is the brain data directory from `[brain] dir` in
`~/.oxios/config.toml` (default `~/.oxi/brain`). **Always pass
`--dir <brain-dir>`** so the agent CLI lane writes into the same store
the kernel reads from — otherwise the agent's writes land in the default
store and the kernel sees an empty brain (split-brain).

## When to write (`ingest`)

- Durable facts about the user: preferences, corrections, decisions.
- Behavioral patterns worth carrying to the next session.
- One fact per call; short declarative sentences.
- Write the fact to a scratch file first (exec structured mode has no stdin
  wiring), then ingest the file:

```
binary: "oxibrain", mode: "structured",
args: ["ingest", "--dir", "<brain-dir>", "--space", "<space>", "<scratch-file>"]
```

## When to read

- Task start, or whenever the user references people/projects/past events.
- Use `ask` first; follow entities with `page` for depth.

```
binary: "oxibrain", mode: "structured",
args: ["ask", "--dir", "<brain-dir>", "<question>"]
binary: "oxibrain", mode: "structured",
args: ["page", "--dir", "<brain-dir>", "<entity-id>"]
```

## Navigation verbs

- `oxibrain timeline --dir <brain-dir> <entity> [--from --to]` — how beliefs changed.
- `oxibrain why --dir <brain-dir> <statement-id>` — provenance and confidence.
- `oxibrain contradictions --dir <brain-dir>` — unresolved conflicts (worth surfacing).

## What NOT to store

- User notes belong to the `knowledge` tool (the vault), never the brain.
- Secrets, tokens, one-off turn details, or anything the user asked to forget.
- Whole documents — the vault is indexed separately as documents.
