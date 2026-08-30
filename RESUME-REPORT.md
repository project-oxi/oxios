# Resume Report — project-roots/persona-workbench stream

Branch: `feat/project-roots-workbench` @ `dfebd8b73` (merge `27af88866` + gate fixes)
Worktree: `/Volumes/MERCURY/PROJECTS/oxios-roots-wt`
Date: 2026-08-31

## 1. Merge

`git merge main` (main @ `4d2385655`) — 26 files conflicted, all resolved by hand.
The prior merge on this branch (`5996eb888`) had only brought in 5 main commits; this
merge reconciled the entire brain-binding + issues/todo + project-domain stream
(33 commits: `22fd78e7b..4d2385655`).

### Conflict decisions (file → resolution)

| File | Decision |
|---|---|
| `crates/oxios-kernel/src/project/project_db.rs` (8 hunks) | Branch's reduced schema (root_paths JSON + RFC3339) kept day-one with `default_brain_space TEXT` in the DDL; main's guarded `migrate_default_brain_space` ALTER re-added to the new schema path (idempotent PRAGMA check, only fires for intermediate-build tables); insert/select/row-mapping use reduced columns + brain default at index 4; main's `INSERT OR REPLACE` NOT taken (branch's `INSERT INTO … ON CONFLICT(id)` kept); tests unioned: branch legacy-drop test + brain round-trip + migration tests adapted to reduced schema. |
| `crates/oxios-kernel/src/project/mod.rs` | Reduced `Project` struct (branch) + `default_brain_space: Option<String>` (serde default/skip-none) + `Project::new` initializes `None`; re-exports unioned (branch `validate_root_paths` + main's milestone/paths); main's detection re-export NOT taken (detection.rs stays deleted). |
| `crates/oxios-kernel/src/project/manager.rs` | Branch's slim manager kept (detect/link_memory/unlink_memory/get_project_memory_ids/save_project/clear_legacy_paths dropped — they depend on deleted detection.rs / project_memory table); re-added reduced `update_project_bundle(id, instructions, default_brain_space: Option<Option<String>>)` (tri-state; no mount_ids). |
| `crates/oxios-kernel/src/kernel_handle/project_api.rs` | Branch's reduced facade; `update_project_bundle(id, instructions, default_brain_space)` tri-state without mount-ID parsing; `anyhow::{Context, Result}` imported; ProjectInfo carries `default_brain_space` (auto-merged side did this). |
| `crates/oxios-kernel/src/agent_runtime.rs` (6 hunks) | Union everywhere: `execute_inner`/`run_agent` signatures take `root_paths: &[PathBuf]` (branch) + `brain_space: Option<&str>` + `project_id: Option<Uuid>` (main); `mount_paths` gone. Body kept branch's exec-scope derivation (root_paths[0] = CWD, grants = roots only, temp-dir fallback) AND main's brain gating (prompt line, capability filter, recall block, persistence-hook skip, compaction skip, OXIOS_BRAIN_SPACE env via callback). Deny comment updated to "project root path" instead of mount. Result-bus call resolved to main's `tool_results.take`. |
| `crates/oxios-kernel/src/tools/kernel_bridge.rs` (4) | Domain list: `"issue"` in, `"mount"` out; tool-count test updated 27 → 28 (9 always-on incl. todo + 19 kernel domains). |
| `crates/oxios-kernel/src/tools/registration.rs` (2) | `KernelDomain { "project" }` → `ProjectTool + IssueTool` (both doc tables); IssueTool registration kept, MountTool not resuscitated; LAYER0_EXEMPT parity auto-merged from main. |
| `crates/oxios-kernel/src/tools/builtin/mod.rs` | SDK-bridge path registers IssueTool with `project_id: None` (turn binding happens in `register_tools_from_cspace_gated`). |
| `crates/oxios-kernel/src/tools/mod.rs`, `search_provider.rs` | Main's refactor taken: `StructuredResultBus` lives in `structured_results.rs`; branch's inline `SearchResultBus` dropped (branch's changes to search_provider were cosmetic — verified via `git diff 57baeea89 HEAD`). |
| `crates/oxios-kernel/src/state_store.rs` | Branch's deletion of a duplicate promotion test kept (single auto-merged copy survives). |
| `crates/oxios-kernel/src/kernel_handle/mod.rs` | `run_goal` → `handle_unified` passes 6 `None`s (project_id, role, model_override, model_params, persona_id, brain_space) matching the merged 10-param signature. |
| `src/api/error.rs` (3) | Union: branch `UnprocessableEntity`/`NotImplemented` + main `Locked` (Display + StatusCode arms). |
| `src/api/routes/mod.rs` (2) | `mod issue_routes;` in, `mod mount_routes;` out; issue/milestone routes in, mount routes and memory-unlink route stay deleted. |
| `src/api/routes/project_routes.rs` (3) | Branch create (root_paths + instructions inline; POST carries no brain default — web does follow-up PUT like main); PUT routes `default_brain_space` through bundle update (absent = keep, null = clear, string = set) and additionally applies name/roots/instructions when present; `deserialize_some` imported. `UpdateProjectRequest` (auto-merged) already had the tri-state field. |
| `src/kernel.rs` | Branch deletion kept: RFC-025 mount migration (`migrate_projects_to_mounts`/`unique_mount_name`) not resuscitated. |
| `src/main.rs` | Branch deletion kept: CLI project-create mount-link block removed. |
| `src/api/routes/chat.rs`, `events.rs`, `orchestrator.rs`, `gate.rs` | Auto-merged cleanly and verified: BOTH `project_id` (branch) and `brain_space` (main) threaded through request → metadata → session persist → WS ingress → ExecEnv → Session/ExecEnv → AgentContext. |
| `web/src/types/index.ts` | `Project`: branch `root_paths`/`instructions` + main `default_brain_space?: string | null`. |
| `web/src/stores/chat.ts` (2) | Picker state: `activeBrainSpace` kept, `activeMountIds` dropped; `loadSession` union restores both branch `effectiveProfile` and main `activeBrainSpace` (metadata fallback); `brainDefaults` + `resolveBrainSpace` send path intact. |
| `web/src/routes/chat.tsx` (3) | `WorkbenchShell` import + `useBrainBindingSync()` kept together; chat body = branch surface (scroll button, minimap, text-selection, fanout grid, compare panel) + main `TodoProgressBar` above ChatInput; ChatInput receives NO mount props. persona-picker mount auto-attach correctly dropped (mounts deleted). |
| `web/src/routes/projects/$projectId.tsx` | Branch's `ProjectDetailsCard` composition + issues/milestones Tabs added back (paths/memories/activity tabs not resurrected — their components are deleted). |
| `web/src/components/project/create-project-dialog.tsx` (5) | RootPathsEditor + ProjectBrainSelect; state/reset include `defaultBrainSpace` only; onSuccess does follow-up `update.mutate({ id, default_brain_space })` (generic `projects.updateError` toast) mirroring main's PUT-after-create; no mount chips. |
| `web/src/components/project/edit-project-dialog.tsx` (3) | Roots + instructions + brain select; submit sends tri-state `default_brain_space`. The effect's `setInstructions(project.instructions ?? '')` seeding line preserved (previously-regressed main bug not reintroduced). |
| `web/src/i18n/locales/{en,ko}.json` (3 each) | chat: folderPickerLocalOnly + todo/issue/brain blocks; projects: staleRoot keys + tabs.issues/milestones; tail: workbench + issues + milestones. en verified structurally; ko initially lost `workbench.*` in resolution (caught by the i18n-parity test) and was repaired. |

### Mounts stay deleted
No mount source files were resurrected by the merge (main hadn't modified `src/mount*`
or `crates/oxios-kernel/src/mount/` since the merge base, so branch deletions carried).
Remaining "mount" strings are docs/history only.

## 2. Brain-default re-application evidence

- `project_db.rs::default_brain_space_round_trip` — Some("work") → None → Some("personal") on
  the reduced schema. PASSES.
- `project_db.rs::default_brain_space_migration_adds_nullable_column` — guarded ALTER on a
  pre-column reduced table, idempotent, rows read back `None`. PASSES.
- Manager tri-state (`update_project_bundle`) + facade + REST PUT route + web dialogs
  (create = follow-up PUT; edit = tri-state select) all wired; live round-trip verified in
  the smoke (§4d), including persistence across daemon restart.

## 3. Gates (§10.8/§11)

- `cargo fmt --all -- --check` — clean (after `cargo fmt --all`).
- `cargo clippy --workspace --all-features --all-targets -- -D warnings` — exit 0
  (fixed: `cloned_ref_to_slice_refs` in execution_scope_contract, `unnecessary_to_owned` +
  borrow-order in project/paths, exact `.oxi` component match, dead `TestApp.config_path`,
  needless `mut`).
- `cargo check --workspace --all-features` — exit 0.
- `cargo nextest run --workspace --all-features --no-fail-fast` — **1820 passed, 0 failed,
  14 skipped**; includes `project_roots_contract` (8) and `execution_scope_contract` (10).
- Doc tests — ok (0 doc tests in tree).
- Web: `tsc --noEmit` clean; vitest **690/690** (84 files); `biome check src` clean
  (2 auto-fixed import sorts); `vite build` succeeds (pre-existing >500 kB chunk warning).

## 4. First-boot smoke (isolated)

Setup: `oxios --config $SMOKE/home/config.toml start --foreground` with
`OXI_BRAIN_DIR=$SMOKE/brain`, `$SMOKE = mktemp -d`; config `[gateway] port = 47615`,
`[security] auth_enabled = false`, dummy `[engine]` credentials to clear the onboarding gate.
State landed in `$SMOKE/home` only (verified listing).

Results:
- (a) Folderless project: `POST /api/projects {root_paths: []}` → 201; record keeps
  `root_paths: []` + instructions (instructions-only, no ambient fs scope).
- (b) Multi-root project: two roots → 201; order preserved (macOS `/tmp` → `/private/tmp`
  canonicalization applied by design).
- (c) `GET /api/brain/spaces` → `[]` (200, reachable).
- (d) Tri-state round-trip on `PUT /api/projects/:id`: `"work"` → set; `null` → cleared;
  `"personal"` + rename → both applied; combined PUT (brain + name + root_paths) → all
  applied. After daemon restart, values persist (SQLite).

## 5. Deviations / notes

- **First smoke attempt touched the real home (~8 s).** `oxios start` daemonizes and takes
  its config from `--config` (default `~/.oxi/oxios/config.toml`), not env alone; my first
  run with only env overrides bootstrapped a daemon on default port 4200 using the default
  home. It was killed immediately (PID 76611) and wrote only fresh-home bootstrap artifacts
  (e.g. appended to `~/.oxios/logs/oxios.log`). The re-run used `--config` into the temp
  home; everything after that stayed isolated. No running user daemon was affected (none
  was listening on 4200 at that moment; none is running now).
- Smoke used a dummy `[engine] api_key` solely to bypass the interactive onboarding gate;
  no LLM calls were made.
- `repo` project name was renamed to `repo2` during the round-trip smoke (expected).
- The i18n parity test caught a real ko.json regression during resolution — fixed in
  commit `dfebd8b73`.

## 6. Commits

- `27af88866` — Merge branch 'main' into feat/project-roots-workbench (full reconcile)
- `dfebd8b73` — fix(gates): post-merge clippy, i18n parity, and binding-hook hardening
