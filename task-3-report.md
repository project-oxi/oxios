# Task 3 — fix round 1

## Status
Complete.

## Commit
- `fix(kernel): reuse descriptor ToolContractVersion; correct profile docs`

## Changes — `crates/oxios-kernel/src/capability/profile.rs`

1. **Duplicate `ToolContractVersion` removed.**
   - Deleted the local `pub struct ToolContractVersion` redefinition (the
     `major: u32` / `minor: u32` mirror at lines ~225–236).
   - Added `ToolContractVersion` to the existing
     `use super::descriptor::{...}` import. The descriptor version is the
     canonical, serde-equipped type; `provider_compatibility::ExactContract`
     now references the same type as `ToolDescriptor::contract_version`.
   - Adjusted the section break (one blank line between
     `ProviderCompatibilityPolicy` and `DynamicProviderContract`).

2. **Doc fix A — `select_tools` ordering claim.**
   Replaced the "preserving the catalog's source order" line with an
   explicit selector-major description. Implementation iterates `include`
   outer and `catalog` inner, so the resulting order follows the
   include selectors, not catalog source order. Body comment above the
   loop updated to match.

3. **Doc fix B — module doc set formula.**
   Changed `include ∪ dynamic − exclude` to
   `include − exclude (dynamic providers add tools at resolution time,
   not here)` in the module-level overview.

4. **Doc fix C — `validate_publish` check ordering.**
   Renumbered the doc list to match the actual code order:
   1. ceiling coverage,
   2. `required_core` ⊆ selected,
   3. core size ≤ `max_active_tools`.

5. **Doc fix D — `ProfileError::UnknownTool` doc.**
   Reworded from "typically a required-core id or a dynamic-provider name"
   to "raised by the resolver when a profile references a tool absent
   from the catalog", matching the unreachable-today note.

## Verification
- `cargo nextest run -p oxios-kernel 'capability::profile::'` — 2/2 pass
  (run twice for stability).
- `cargo check -p oxios-kernel --all-features` — clean.
- `cargo check -p oxios-kernel --all-features --all-targets` — clean
  (no unused-import or dead-code warnings).

## Tests
- `capability::profile::tests::exclude_always_wins_over_include` — pass.
- `capability::profile::tests::validate_rejects_ceiling_violation_and_missing_core`
  — pass.
