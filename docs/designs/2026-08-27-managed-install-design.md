# Managed Install — rustup-model updater

**Date**: 2026-08-27
**Status**: approved (design review in conversation, 2026-08-26/27)
**Supersedes**: cargo-only binary update strategy in `src/commands/update.rs` and `src/api/routes/system.rs`

## Problem

`oxios update` assumes `cargo install` is the binary distribution channel. Real machines
have several channels (cargo, Homebrew tap, manual release-tarball copies, dev symlinks),
and PATH order silently decides which one runs. On 2026-08-26 an update "succeeded"
(`cargo install` wrote `~/.cargo/bin/oxios` 1.44.0) while every subsequent `oxios`
invocation — including the daemon restart spawned by the updater itself — executed a
stale `~/bin/oxios → oxios-1.43.1` copy. The same trap had already struck on 8/19
(the "livestream6 stale deployment" incident recorded in `docs/designs/`), and
versioned manual copies accumulated ~700 MB in `~/bin`.

Root cause: three install channels, no single source of truth, an updater whose success
criterion (cargo exit code) differs from the user's (the command I type runs the new
version), and no PATH audit anywhere.

## Goals

1. **Binary distribution is the primary channel.** GitHub Release tarball +
   sha256 is the canonical artifact; `cargo install` remains a supported fallback.
2. Updates always converge the *executed* binary, not just one channel's copy.
3. Bounded disk usage: keep-2 version retention, plus detection/cleanup of stale copies.
4. Single-platform reality respected: macOS ARM64 only — no cross-platform installer
   matrix, no cargo-dist adoption (it solves a problem we don't have).

## Non-goals

- Windows/Linux support, multi-arch installers.
- Signed binaries (TLS + same-origin sha256 is the current trust model).
- Auto-updating without user confirmation (the confirm prompt stays).
- Homebrew tap changes (tap bump job from v1.44.0 continues as-is).

## Architecture

```
$OXIOS_HOME/                       # default ~/.oxios/, config-path-derived (main.rs)
├── versions/
│   ├── 1.44.0/oxios               # extracted release binary
│   └── 1.44.1/oxios               # keep-2: older dirs pruned after each update
└── bin/
    └── oxios -> ../versions/1.44.1/oxios   # launcher symlink, atomically flipped
```

PATH integration: `$OXIOS_HOME/bin` is prepended to the shell profile by
`install.sh` and by `oxios update` migration (idempotent managed block, see below).

### Install channels and how `oxios update` treats each

Detected via `std::env::current_exe()` (symlinks resolved) + PATH scan:

| Channel | Detection | Update behavior |
|---|---|---|
| Managed | exe under `$OXIOS_HOME/versions/` or `$OXIOS_HOME/bin/` | tarball self-install → flip → prune |
| Manual copy / symlink (unmanaged) | anywhere else writable | offer migration: install managed, repoint symlinks, preserve one `.bak` |
| Homebrew | exe under `/opt/homebrew/` or `/usr/local/Cellar/` | print `brew upgrade oxios`, exit 0 (no-op) |
| Cargo | exe under `$CARGO_HOME/bin` (`~/.cargo/bin`) | treat as unmanaged → offer migration; `--via cargo` forces legacy path |
| Dev build | exe under a directory whose ancestors contain a `Cargo.toml` with `[workspace]` naming `crates/oxios-kernel` (cheap heuristic: ancestor Cargo.toml + `crates/` sibling) | refuse auto-update; suggest `cargo build` |

### `share/install.sh`

`curl -fsSL https://raw.githubusercontent.com/project-oxi/oxios/main/share/install.sh | sh`

1. Resolve latest release via GitHub API (`project-oxi/oxios`; the `a7garden/oxios`
   alias currently used in `update.rs` redirects — switch both call sites to one
   canonical constant).
2. Download `oxios-aarch64-apple-darwin.tar.gz` + `.sha256` to a temp dir; verify.
3. Extract into `$OXIOS_HOME/versions/<v>/`, flip launcher symlink.
4. PATH setup: append a managed block to `~/.zshrc` / `~/.zprofile` (only if the dir
   is not already on PATH in the script's own `PATH` check):
   ```sh
   # BEGIN oxios (managed)
   export PATH="$HOME/.oxios/bin:$PATH"
   # END oxios (managed)
   ```
   Idempotent: skip when a previous managed block exists. Never touch other PATH lines
   (the 8/26 incident showed user profiles prepend `~/bin` last — we must not reorder
   user lines, only ensure ours exists).
5. Shadow audit: scan `PATH` for other `oxios` entries, print each with remediation
   (non-interactive: list only; the binary's `oxios update` does interactive cleanup).
6. Disk-space precheck: require free space ≥ 3× tarball size before extracting.

### `oxios update` rework (`src/commands/update.rs`)

Replace the `cargo install` block (lines 186–251 today) with channel-aware logic:

```
managed exe:
  download tarball (reuse web_dist staging pattern: temp dir → validate → rename)
  → extract to versions/<new>/
  → atomic flip (create temp symlink $OXIOS_HOME/bin/.oxios.tmp → versions/<new>/oxios,
                 std::fs::rename over bin/oxios)
  → prune: keep current + previous, delete older version dirs
  → restart daemon (existing UpdateOutcome → main.rs flow, spawned via $OXIOS_HOME/bin/oxios)
  → PATH audit report

unmanaged exe:
  same install steps, then migration:
    - for each foreign `oxios` found in PATH (excluding $OXIOS_HOME/bin):
      - symlink → repoint to $OXIOS_HOME/bin/oxios (this is the case that fixes
        ~/bin/oxios → oxios-1.43.1)
      - real file → rename to <name>.bak.<date> (one preserved copy; flagged for
        `oxios doctor` cleanup)
    - TTY: confirm prompt listing every touched path; non-TTY: print commands only,
      require --adopt to apply

brew exe:  print `brew upgrade oxios`, return unchanged
dev exe:   bail with explanation
--via cargo: legacy `cargo install oxios --locked` path, kept verbatim
```

The HTTP/release-API plumbing (reqwest client, release JSON parse, SemVer guard
`validate_update_version` in `system.rs`) is reused; download code follows
`web_dist::sync_to_disk`'s staging-validate-persist pattern.

Web UI behavior is unchanged: release binaries embed the SPA (`is_embedded()`), cargo
builds keep the zip-download path. Binary update ⇒ UI update for release installs.

### API route (`api/routes/system.rs`)

`POST /api/system/update` gains the same channel detection. Managed installs use the
tarball path; unmanaged ones **do not auto-migrate** over HTTP (no TTY) — they return
the audit list + instructions. `--via cargo` equivalent: `body.via: "cargo"`.

### PATH audit & cleanup (`oxios doctor` + update report)

Enumerate `$PATH` entries (dedup, in order), stat each `<dir>/oxios`:
- resolve symlink targets, read each file's version cheaply? — no: report path, target,
  mtime, size. Version extraction spawns the binary with `--version`; do it only for
  TTY reports (≤ a few entries).
- Output: table of shadows ahead of `$OXIOS_HOME/bin` in PATH order, with suggested fix.
- `oxios doctor --cleanup` (TTY confirm): deletes `.bak.*` copies and stale versioned
  files listed by the audit; non-TTY prints the `rm` commands only.

### Rollback

`oxios update --rollback`: requires ≥2 versions in store; flips launcher to the
previous version, restarts daemon. Free given the store layout.

## Safety properties

- **Atomicity**: new version lands in its own dir; the only mutation of the live
  launcher is a single `rename(2)`. A failed download/verify leaves the current
  install untouched.
- **sha256**: verified against the `.sha256` asset before extraction; mismatch → abort
  + temp cleanup.
- **Disk precheck**: free-space check before extract (2026-08-26 StorageFull lesson).
- **No self-delete while running**: replacing `bin/oxios` (a symlink) never unlinks the
  running binary's inode; the daemon restart is what activates the new version.
- **Permissions**: refuse to `sudo`; if `$OXIOS_HOME` is not writable, error with
  remediation instead of partial install.

## Testing

- Unit (`src/commands/update.rs` `#[cfg(test)]`, tempdir):
  - prune semantics (keep-2, current protected)
  - atomic flip (temp symlink + rename over existing)
  - channel classification: managed/brew/cargo/dev/unmanaged paths (table-driven)
  - shadow audit ordering (PATH precedence)
  - SemVer tag → version dir name parsing
- Unit (`share/install.sh`): shellcheck in CI (new ci.yml step, fast) + a `bats`-free
  smoke: run installer against a `fake GitHub API` via env override pointing at a
  local file:// server is out of scope; instead extract the pure functions
  (sha verify, managed-block injection) into testable form where feasible.
- Integration: `oxios update --dry-run` against a fixture release JSON (existing
  pattern) prints the planned channel actions.
- Manual smoke on this machine: install.sh → `oxios update` from managed path →
  shadow audit output → rollback.

## Documentation & changelog

- `docs/USER-GUIDE.md`: new install/update section (curl installer primary,
  brew/cargo alternatives, rollback, cleanup).
- `CHANGELOG.md` under the next release: Added — managed install + installer script;
  Changed — `oxios update` binary strategy cargo → release tarball (cargo via flag).
- `share/default-skills` untouched; agent-facing surfaces unchanged.
