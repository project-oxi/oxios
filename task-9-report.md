# Task 9 — fix round 1

## Status
Complete.

## Commit
- `fix(kernel): descriptor registered names match tool name() impls`

## Changes

1. **`registered_name` corrections — `capability/descriptor.rs`.**
   Three long-tail descriptors carried names that diverged from the tools'
   `AgentTool::name()` impls, so the resolver/prompt surface advertised a
   name the registry never answers to:
   - `kernel.email.send`: `email` → `send_email` (`email_tool.rs:110`)
   - `kernel.image.gen`: `image_gen` → `image_generation`
     (`image_generation_tool.rs:88`)
   - `kernel.screenshot.capture`: `screenshot` → `browse_screenshot`
     (`screenshot_tool.rs:38`)

   Capability domains are untouched: `KernelDomain { domain: "email" }`,
   `"image_gen"`, and `ResourceRef::Browser` are capability identities
   (also referenced by `builtin_profiles.rs` ceilings), not tool names.

2. **Test rows — `tools/registration.rs`.**
   `catalog_long_tail_tools_are_addressable` case tuples are
   `(descriptor_id, registered_name, domain, rights)`; the three rows'
   name column updated to the corrected names. Domain/rights columns
   unchanged (still `email`/`image_gen` KernelDomain WRITE,
   Browser EXECUTE).

3. **Repo doc recipes — `AGENTS.md` + `CONTRIBUTING.md`.**
   Both "Adding a New Tool" recipes and the AGENTS.md "Tool registration"
   convention bullet still pointed at the deleted
   `tools/kernel_bridge.rs::register_all_kernel_tools()`. Rewritten to the
   live path: add a `ToolDescriptor` to the kernel catalog in
   `capability/descriptor.rs` (`registered_name` = the tool's `name()`
   impl), then add the registration arm in
   `tools/registration.rs::register_from_resolved_profile` (keyed by
   descriptor id). AGENTS.md recipe renumbered 4→5 steps accordingly.
   Historical RFC/design docs under `docs/` intentionally left as-is
   (Task 12 docs pass).

## Verification
- `cargo nextest run -p oxios-kernel` — 1010/1010 pass, 1 skipped
- `cargo nextest run -p oxios-kernel --all-features` — 1044/1044 pass,
  12 skipped
- `cargo check -p oxios-kernel --all-features` — clean
- `grep kernel_bridge|register_all_kernel_tools AGENTS.md CONTRIBUTING.md`
  — 0 hits

## Notes
- Resolution closes the implementation report's Concern #3 (registered_name
  vs `name()` divergence flagged for Task 10 wiring).
