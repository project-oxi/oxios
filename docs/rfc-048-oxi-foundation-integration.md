# RFC-048: Oxi Foundation Integration

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this RFC task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Proposed

**Goal:** Layer Oxi Foundation bootstrap, shared provider/model profiles, Keychain-backed secrets, and portable skill/persona packages onto the already-complete oxibrain migration, while retaining Oxios's embedded `oxicode-sdk` executor and separating Brain consolidation from Oxios knowledge-note curation.

**Architecture:** Oxios remains the orchestration/experience plane. Its `BrainConnection` talks to the standalone oxibrain daemon, which is the only durable-memory data plane; unavailable Brain service is an explicit degraded capability, never a local persistence fallback. `OxiosEngine` continues to embed `oxicode_sdk::Oxicode` in-process. Foundation is a versioned `~/.oxi/foundation/v1` filesystem contract with OS Keychain locators, not a provider proxy or a CLI subprocess protocol. Shared packages declare abstract requirements; CSpace and `AccessGate` decide their actual Oxios authority.

**Tech Stack:** Rust, `oxibrain-client`, `oxicode-sdk` embedded runtime, Tokio, TOML/JSON, macOS Keychain via the existing secure credential dependency pattern, CSpace/RBAC/permission gates, existing installer/release scripts.

## Decisions

1. **RFC-047 stays historical and implemented.** It established external Brain connectivity. This RFC adds bootstrap, profile/keychain, package, and naming layers; it does not repeat or replace its memory migration.
2. **No default Oxicode CLI spawning.** `OxiosEngine` and `AgentRuntime` retain the in-process SDK execution path. An external worker is a future explicit job protocol only; no silent fallback or shell-out is allowed.
3. **One Foundation registry, host-specific enforcement.** Shared skills/personas declare capability requirements; they neither receive a CSpace nor grant rights. `AccessGate` still makes every allow/deny decision.
4. **Two different operations keep different names.** Brain consolidation generates derived, sourced, uncertain episodes. Oxios knowledge-note curation reads/writes its user-visible KnowledgeBase. Neither subsumes the other.
5. **Credentials leave application auth files.** Foundation profiles contain only non-secret metadata and Keychain locators. Existing environment variables remain explicit automation overrides, but `~/.oxios/auth.json` and `~/.oxicode/auth.json` cease to be the normal persistent secret source after migration.

## Shared Foundation Contract

```text
~/.oxi/
  foundation/v1/profiles.json      # schema-versioned non-secret profile registry
  foundation/v1/packages.lock      # immutable resolved packages: version/digest/source/trust
  brain/oxibrain.sock              # daemon discovery default
```

A profile includes an `id`, provider kind, endpoint, model identifier, declared model capabilities, allowed roles, and `{ service, account }` Keychain locator. Roles are `memory.extract`, `memory.consolidate`, `coding.primary`, and `assistant.general`. The registry contains no API keys or OAuth tokens.

A package manifest has an immutable digest, optional `targets`, optional persona, and abstract `requires`: for example `workspace.read`, `workspace.patch`, `shell.execute`, `browser.navigate`, `brain.query`, and `schedule.manage`. Oxios accepts a package only after source/digest/trust verification, translates only supported requirements to a `CapabilityTemplate`, and then evaluates that template through CSpace, RBAC, permissions, and execution policy.

## Implementation Tasks

### 1. Establish RFC boundaries and repair the architecture documentation

**Files:**
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/INDEX.md`
- Modify: `docs/getting-started.md`
- Modify: `README.md`
- Modify: `docs/rfc-041-host-integrations.md`
- Modify: `docs/rfc-003-knowledge-separation.md`
- Retain as history: `docs/rfc-047-oxibrain-migration.md`

- [ ] Add RFC-048 to `docs/INDEX.md` as a live RFC and link it from the Brain Connector, executor, skills/personas, and credential architecture sections.
- [ ] Update the `ARCHITECTURE.md` version header to the current product version. Repair stale source links to `skill/manager.rs`, fix duplicated/out-of-order section headings, then document the Foundation registry, external Brain discovery, Keychain provider resolution, and abstract-to-CSpace capability binding.
- [ ] State in the orchestrator and runtime sections that `OxiosEngine` embeds `oxicode_sdk::Oxicode` and `AgentRuntime::run_agent` builds the SDK agent in-process. Explicitly reject using an installed `oxicode` CLI as the default worker.
- [ ] Amend RFC-041's “no Keychain” and provider exclusion assumptions. Keep its join-at-the-API-layer and host-integration requirements, but make the Foundation Keychain backend a host credential source and make package requirements declarative rather than authoritative.
- [ ] Keep RFC-047 unchanged except for a front-matter “superseded for bootstrap/profile guidance by RFC-048” link. Do not reopen the completed migration design.
- [ ] Update `getting-started.md` and `README.md` to show Foundation onboarding rather than instructing users to export long-lived provider keys. Document the explicit environment-variable override for non-interactive CI only.
- [ ] Update RFC-003 so its two knowledge systems remain clear: agent memory is Brain-ledger data; KnowledgeBase notes are user-facing material. Define the renamed curation feature there.

**Acceptance:** A new reader finds one current installation, credential, Brain, executor, package, and Dream story; no guide asserts that persistent API keys belong in Oxios/Oxicode auth JSON.

### 2. Build idempotent Foundation bootstrap into installation and first run

**Files:**
- Modify: `scripts/install.sh`
- Modify: `src/cli.rs`
- Modify: `share/default-config.toml`
- Modify: `crates/oxios-kernel/src/config.rs`
- Create: `crates/oxios-kernel/src/foundation/bootstrap.rs`
- Create: `crates/oxios-kernel/src/foundation/mod.rs`
- Create: `crates/oxios-kernel/tests/foundation_bootstrap.rs`

- [ ] Keep the current verified Oxios binary download and PATH setup. Add an idempotent post-install/first-run bootstrap which discovers or installs a compatible `oxibrain` release through the product release mechanism, initializes `~/.oxi/brain` only when absent, and starts `oxibrain serve --daemon` on the Foundation default socket. Do not start a second daemon when the protocol handshake finds a compatible existing daemon.
- [ ] Do not hard-code the previous RFC-047 HTTP port bootstrap. The default integration is the Unix socket `~/.oxi/brain/oxibrain.sock`; an explicit `[brain].socket_path` remains supported for managed deployments.
- [ ] Add `FoundationConfig` to the config authority with registry path, bootstrap enablement, and explicit endpoint overrides. Preserve `BrainSection { enabled, socket_path, space }`; its default becomes the shared discovery helper instead of a duplicated path literal.
- [ ] Add user-facing CLI commands or onboarding steps under the existing CLI design for `foundation status`, `foundation bootstrap`, and non-secret `foundation profile` registration. These commands must report actions and never print a secret.
- [ ] Use the versioned `BrainClient` discovery/handshake contract to classify daemon state as compatible, unavailable, or incompatible. Incompatible is a clear actionable error; unavailable retains current degraded behavior. Do not inspect the Brain SQLite store.
- [ ] Test fresh bootstrap, idempotent rerun, explicit endpoint override, incompatible daemon refusal, missing executable with an actionable installation result, and unavailable daemon without blocking an ordinary Oxios turn.

**Acceptance:** A standard installation produces one discoverable Brain daemon and a non-secret Foundation directory without a manual RFC-047 recipe or duplicate daemon.

### 3. Add Foundation profile metadata and OS Keychain credential resolution

**Files:**
- Create: `crates/oxios-kernel/src/foundation/profile.rs`
- Modify: `crates/oxios-kernel/src/credential.rs`
- Modify: `crates/oxios-kernel/src/engine.rs`
- Modify: `crates/oxios-kernel/src/agent_runtime.rs`
- Modify: `share/default-config.toml`
- Create: `crates/oxios-kernel/tests/foundation_profiles.rs`
- Create: fixture files under the existing kernel test-fixture convention

- [ ] Parse and validate the shared `profiles.json` schema before use: schema version, unique profile ID, role allow-list, provider metadata, capability declarations, and non-secret Keychain locator. Reject duplicate IDs, unknown versions, raw credential fields, unsupported profile roles, and endpoint/model combinations that cannot be built by the embedded SDK.
- [ ] Add `CredentialSource::FoundationKeychain` and an OS Keychain implementation to `CredentialStore`. The normal order is explicit process/runtime override, validated Foundation Keychain locator, then only documented compatibility sources during migration. Never copy a secret into config, logs, `profiles.json`, telemetry, or an error value.
- [ ] Resolve model roles through a `FoundationProfileResolver` at `OxiosEngine::resolve_model`/provider construction. `coding.primary` and `assistant.general` must select profiles allowed for those roles; a caller cannot pass an arbitrary profile ID around access policy.
- [ ] Preserve embedded `oxicode-sdk` construction. The profile resolver provides model metadata and a credential to the SDK provider factory; it does not fork `oxicode`, tunnel through a new model gateway, or alter CSpace tools.
- [ ] Make profile unavailability explicit: missing Keychain item, denied role, or unsupported output capability produces an actionable provider error and does not silently choose a different remote account. Existing explicit environment variables can be selected intentionally for CI.
- [ ] Test valid role resolution, role denial, malformed/secret-bearing profile rejection, missing Keychain item, environment override, failure redaction, and provider construction staying in-process.

**Acceptance:** Oxios uses one registered provider profile and one Keychain credential source while its executor remains embedded and no credential persists in application auth files during normal operation.

### 4. Import shared skills/personas through CSpace rather than granting rights

**Files:**
- Create: `crates/oxios-kernel/src/foundation/packages.rs`
- Modify: `crates/oxios-kernel/src/skill/manager.rs`
- Modify: `crates/oxios-kernel/src/persona/manager.rs`
- Modify: `crates/oxios-kernel/src/capability/template.rs`
- Modify: `crates/oxios-kernel/src/capability/resolve.rs`
- Modify: `crates/oxios-kernel/src/access_manager/gate.rs`
- Modify: `crates/oxios-kernel/src/tools/registration.rs`
- Create: `crates/oxios-kernel/tests/foundation_packages.rs`

- [ ] Add a read-only Foundation package registry importer. Verify schema version, target includes `oxios`, source trust, immutable digest, and lockfile identity before a package becomes a candidate. Do not write to the shared lockfile from an agent turn.
- [ ] Retain existing precedence: bundled/default packages, shared immutable Foundation packages, user/workspace packages, and project overrides according to the existing `SkillManager`/persona rules. Record the selected package digest in the runtime snapshot/audit context.
- [ ] Map abstract requirements only through a reviewed table into `CapabilityTemplate` and `ResourceRef`. For example, `schedule.manage` maps to existing cron/scheduler resources, `brain.query` maps to read-only `BrainApi`, and unsupported requirements remain unavailable rather than becoming a broad wildcard capability.
- [ ] Feed the result through `resolve_cspace`, `AccessGate`, RBAC, permission checks, and execution policy. A valid signature/digest is not an authorization decision; package installation never bypasses `DenyLayer` reasons.
- [ ] Keep prompt construction selective. A persona/request chooses compatible package content; do not append every Foundation `SKILL.md` to every agent prompt.
- [ ] Test digest/source/target failure; read-only Brain query vs denied Brain write; an abstract shell requirement denied by CSpace; a package allowed by CSpace but denied by RBAC; overlay precedence; and audit context retaining the resolved digest.

**Acceptance:** One package registry is usable by Oxios, but only Oxios policy grants concrete CSpace resources and tools.

### 5. Split Brain consolidation from KnowledgeBase curation

**Files:**
- Rename: `crates/oxios-kernel/src/knowledge_dream.rs` to `knowledge_curation.rs`
- Modify: the corresponding module index and all call sites
- Modify: `crates/oxios-kernel/src/config.rs`
- Modify: `share/default-config.toml`
- Modify: `src/cli.rs`
- Modify: `crates/oxios-kernel/src/kernel_handle/brain_api.rs`
- Create or modify: focused kernel/CLI tests for both operations

- [ ] Rename `KnowledgeDream`, `KnowledgeDreamConfig`, `KnowledgeDreamReport`, and user-facing “dream” commands/metrics to `KnowledgeCuration`, `KnowledgeCurationConfig`, and `KnowledgeCurationReport`. Preserve its actual operation: scan raw KnowledgeBase notes, curate, write app-owned note updates, and report.
- [ ] Delete or migrate stale `LearningConfig.dream_*` settings only after every caller has moved. Do not leave aliases that make Brain consolidation and note curation ambiguous.
- [ ] Expose the existing Brain operation under an explicit `brain consolidate` command/API path. It delegates to the daemon's consolidation semantics and returns sourced/uncertain derived-episode results; it must not write Markdown notes or invoke `git_layer.commit_file`.
- [ ] Keep knowledge-note curation explicitly opt-in and scoped to app-owned/generated notes according to existing safety checks. It remains unrelated to Brain's `EpisodeKind::Derived` lifecycle.
- [ ] Test that `brain consolidate` uses the Brain connector and never changes a KnowledgeBase file; test that `knowledge curate` changes only eligible app-owned notes and does not call the Brain write/consolidation path.

**Acceptance:** “Dream” no longer names two incompatible write behaviors, and users can independently reason about durable-memory consolidation and note curation.

### 6. Migrate credentials safely and retire only obsolete secret paths

**Files:**
- Modify: `crates/oxios-kernel/src/credential.rs`
- Modify: `src/cli.rs`
- Create: `crates/oxios-kernel/src/foundation/migrate.rs`
- Create: `crates/oxios-kernel/tests/foundation_credential_migration.rs`
- Modify: `docs/getting-started.md`

- [ ] Implement an explicit, user-invoked credential migration command. It reads a known legacy entry, writes it to the profile's validated Keychain locator, verifies retrieval through `CredentialStore`, then records a redacted migration receipt. It never prints the secret.
- [ ] Do not automatically delete `~/.oxios/auth.json` or shared `~/.oxicode/auth.json`. Offer a post-verification archival/removal instruction only after all configured profiles verify. Compatibility fallback remains time-bounded and emits a redacted deprecation warning.
- [ ] Make deletion/revocation use the Keychain locator and profile ID, require a confirmation path consistent with existing destructive credential commands, and leave the non-secret profile metadata intact for a clear “missing credential” diagnosis.
- [ ] Test successful migration, Keychain write/read failure with legacy file untouched, partial migration, idempotent rerun, redacted receipts/logs, and correct source precedence after migration.

**Acceptance:** Credential adoption never risks losing an API key and normal operation no longer needs plaintext auth JSON.

### 7. Verify the installed product rather than only unit surfaces

- [ ] Run targeted Foundation bootstrap/profile/package/credential/curation tests and existing degraded-Brain tests during implementation.
- [ ] Run the repository's documented formatter, test, lint, and build/release gates.
- [ ] Smoke test a clean temporary HOME: install Oxios; bootstrap one compatible Brain daemon; register a non-secret profile and Keychain secret; start an embedded-sdk agent with a profile role; load a shared package that requests `brain.query`; verify AccessGate's decision; run `brain consolidate`; run `knowledge curate`; and confirm the two outputs affect only their respective stores.
- [ ] Repeat the smoke path with the Brain daemon stopped. The turn must complete in existing degraded mode, package `brain.query` must be unavailable rather than falling back to a local memory database, and diagnostics must distinguish unavailable from incompatible.

**Acceptance:** Installation, credentials, executor choice, package authorization, and the two similarly named operations all work on the actual product surface.

## Non-Goals

- Re-implementing RFC-047 or moving Brain SQLite access into Oxios.
- Spawning the Oxicode CLI as Oxios's default executor.
- A global package being globally authorized.
- A shared model proxy or mandatory provider gateway.
- Automatic destructive deletion of legacy credential files.
