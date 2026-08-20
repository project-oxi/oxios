# Task 8 — fix round 1

## Status
Complete.

## Commit
- `fix(kernel): kernel manifest from resolved effective CSpace`

## Changes

### 1. Manifest now derives from `resolved.effective_cspace` (the actual fix)

`AgentRuntime::execute_inner` built the system-prompt kernel manifest from
the legacy bounding CSpace:

```rust
let domains = cspace.active_domains();
```

`cspace` is the agent's bounding capability space (the input to resolution).
Tool selection can attenuate domains away — e.g. a profile that removes
every `kernel.browse.*` selector drops `ResourceRef::Browser` from the
*resolved* effective CSpace. The bounding CSpace still reports the
attenuated domain as "active", which would prompt the model with
capabilities it cannot actually call.

Replaced the inline manifest block with a call to a new pure helper that
reads from the resolved effective CSpace:

```rust
let kernel_manifest =
    build_kernel_manifest_from_cspace(&resolved.effective_cspace);
```

The helper itself is a thin wrapper around
`tools::retrieval::build_kernel_manifest` (returns `None` on an empty
domain set). Extracting it makes the manifest construction trivially
unit-testable in isolation — see the new test below — and keeps the
runtime code a one-liner with no inline logic.

### 2. New unit test: `build_kernel_manifest_from_cspace_emits_exactly_cspace_domains`

`crates/oxios-kernel/src/agent_runtime.rs` (`#[cfg(test)] mod tests`).

Builds three CSpaces for the same agent and asserts the helper reflects
exactly the domains of whichever CSpace it is handed:

- **bounding** — `space`, `knowledge`, `program`.
- **resolved (attenuated)** — `space`, `knowledge` only. `program` was
  removed by a profile selector that required it.
- **empty** — no domains.

Asserts:

1. The manifest from the resolved CSpace contains
   `"Active domains: knowledge, space"` and does NOT mention `program`.
2. Sanity check: the manifest from the bounding CSpace does mention
   `program`. Without this guard the first assertion would also pass if
   the implementation accidentally walked both CSpaces (or no CSpace at
   all) — the guard makes the regression loud by ensuring the bounding
   and resolved manifests actually differ.
3. An empty CSpace produces `None` (so the prompt stays well-formed).

`CSpace::active_domains()` sorts + dedups the domain list before
returning, so the test asserts the sorted order
(`"knowledge, space"`), not the insertion order.

### 3. Stale doc-comment fixes

| File | Stale claim | New claim |
|------|-------------|-----------|
| `crates/oxios-kernel/src/agent_runtime.rs` module doc (line 13) | "Registers tools via `register_tools_from_cspace()`" | "Resolves the persona execution profile against that CSpace and registers tools via `register_from_resolved_profile()`" |
| `crates/oxios-kernel/src/token_maxing/maxer.rs` (line 11-12) | The maxer's agents run through `register_tools_from_cspace_gated`, which DENYs high-risk tools at the `AccessGate` | The maxer sets `ExecEnv.cspace_hint = "standard"`, so the AgentRuntime resolves the `"standard"` persona execution profile and registers via `register_from_resolved_profile()`. High-risk tools are removed at the profile selector, never reach the registry. The legacy gated path is exported for backward compatibility but is not exercised by the maxer. |
| `crates/oxios-kernel/src/tools/registry.rs` (line 8-9) | "The catalog is a superset of the tools registered in [`super::registration::register_tools_from_cspace_gated`]" | "The catalog is a superset of the tools reachable via the agent runtime's live path: persona execution profile → `capability::resolver::resolve` → [`super::registration::register_from_resolved_profile`]. The legacy gated path is still exported but is no longer the authoritative registration source." |

### 4. Verification

- `cargo nextest run -p oxios-kernel` — 1010 tests run, 1010 passed, 1 skipped.
- `cargo check -p oxios-kernel --all-features` — clean.

The new test `build_kernel_manifest_from_cspace_emits_exactly_cspace_domains`
is included in the 1010 and passes.