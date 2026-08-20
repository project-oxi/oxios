# Contributing to Oxios

Thank you for your interest in contributing to Oxios! This guide will help you get started.

## Development Setup

```bash
# Build everything
cargo build

# Run all tests
cargo test --workspace

# Check formatting
cargo fmt --all -- --check

# Run clippy
cargo clippy --workspace -- -D warnings

# Security audit
cargo audit
```

## Architecture

Oxios is an **Agent Operating System** in Rust. Key components:

| Crate | Purpose |
|-------|---------|
| `oxios-kernel` | Core: supervisor, scheduler, event bus, tools, memory |
| `oxios-ouroboros` | Unified intent flow: assess → crystallize → execute → review |
| `oxios-gateway` | Channel-agnostic message hub |
| `oxios-web` | Web dashboard (Axum backend + Dioxus/WASM frontend) |
| `oxios-cli` | CLI channel |
| `oxios-telegram` | Telegram channel |

**Dependency graph:** `oxios → oxios-kernel → oxicode-sdk (crates.io)`

## Code Conventions

- **Language:** Code, comments, docs, commits — English. User-facing messages — Korean.
- **Rust:** `#![warn(missing_docs)]` on public crates. `anyhow` for apps, `thiserror` for libs.
- **Error handling:** Use `?` operator or `.context()?`. Avoid `.unwrap()` in production code — use `.expect("invariant description")` only for provably-safe cases.
- **Naming:** Crates `oxios-<component>`, public API `verb_noun`.
- **Testing:** Unit tests in `#[cfg(test)] mod tests`. Integration tests in `tests/` per crate.
- **Commits:** `<type>(<scope>): <description>` — scopes: kernel, ouroboros, gateway, web, cli, docs.

## Adding a New Tool

1. Define in `crates/oxios-kernel/src/tools/<name>_tool.rs`
2. Add a `ToolDescriptor` (with `registered_name` = the tool's `name()` impl) to the kernel catalog in `capability/descriptor.rs`
3. Add the registration arm in `tools/registration.rs::register_from_resolved_profile`
4. Test with `cargo test -p oxios-kernel`
5. Audit the execution path in `access_manager/` if sensitive

## Pull Request Process

1. Ensure `cargo fmt --all -- --check` passes
2. Ensure `cargo clippy --workspace -- -D warnings` passes
3. Ensure `cargo test --workspace` passes
4. Ensure `cargo audit` has no unfixed vulnerabilities
5. Update CHANGELOG.md if applicable

## Getting Help

- Open an issue on GitHub
- Read `docs/ARCHITECTURE.md` for system design
- Read `AGENTS.md` for the full codebase overview
