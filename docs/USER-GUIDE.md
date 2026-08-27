# Oxios — User Guide

This guide is the operator manual for Oxios on a single machine: install,
update, rollback, and PATH hygiene. For day-to-day product usage see the
[Getting Started Guide](getting-started.md).

Other references:

- **[Getting Started Guide](getting-started.md)** — onboarding, CLI, daemon
- **[Architecture](ARCHITECTURE.md)** — system architecture (English)
- **[REST API Reference](api-reference.md)** — HTTP API

---

## Installation (Managed Install — recommended)

Oxios installs as a small launcher symlink plus a versioned binary store,
modelled on `rustup`. A single `curl | sh` call sets it up; subsequent
`oxios update` calls flip the launcher atomically and keep the previous
version on disk for instant rollback.

```bash
curl -fsSL https://raw.githubusercontent.com/project-oxi/oxios/main/share/install.sh | sh
```

What this does:

1. Resolves the latest GitHub Release via the API.
2. Downloads `oxios-aarch64-apple-darwin.tar.gz` and its `.sha256`, verifies
   the checksum, then extracts the binary into
   `$OXIOS_HOME/versions/<v>/oxios` (default `$HOME/.oxios`).
3. Flips `$OXIOS_HOME/bin/oxios` to point at the new version with a single
   `rename(2)` — the running binary's inode is never unlinked, so the
   installer is safe to re-run.
4. Prepends `$OXIOS_HOME/bin` to `PATH` by appending a managed block to
   `~/.zshrc` (or `~/.zprofile` if `~/.zshrc` is absent). The block is
   idempotent — the installer will not duplicate it on a second run.
5. Prunes older version directories (`keep-2`: current + previous).
6. Prints a PATH shadow report listing any other `oxios` entries that may
   take precedence over the managed one.

Override the install location with `OXIOS_HOME`:

```bash
OXIOS_HOME=$HOME/.oxios-experimental curl -fsSL ... | sh
```

Supported platform: **macOS ARM64** only. The installer prints a clear
error and exits non-zero on any other platform.

### Verify

```bash
$ oxios --version
oxios 1.44.0
$ which oxios
/Users/you/.oxios/bin/oxios
$ readlink ~/.oxios/bin/oxios
../versions/1.44.0/oxios
```

### Alternative channels

The installer script is the recommended path, but the binary is also
distributed through:

| Channel | How to install / update |
|---|---|
| **Pre-built Release tarball** (this script, above) | `curl \| sh` — managed install with rollback |
| **Homebrew tap** | `brew install project-oxi/oxios/oxios`; updates via `brew upgrade oxios`. `oxios update` prints the brew command and exits. |
| **`cargo install`** (fallback) | `cargo install oxios --locked`. Use `oxios update --via cargo` to keep updating via this path. |
| **Build from source** | `git clone https://github.com/project-oxi/oxios && cd oxios && cargo build --profile dist`. The resulting binary is treated as a dev build by `oxios update`, which refuses auto-update and suggests `cargo build`. |

---

## Updating

```bash
$ oxios update
  ⬡ Oxios Update
  ────────────────────────────────────────────────
  Channel:  Managed   (launcher → ~/.oxios/versions/1.44.0/oxios)
  Latest:   1.44.1
  Plan:     install → flip → prune → restart daemon

  Downloading…  done (8.1 MB)
  sha256…       ok
  Extracting…   done
  Flipping launcher…  done
  Pruning…      removed 1.43.0
  Restarting daemon…  done
  ✓ Oxios is now 1.44.1.
```

`oxios update` is channel-aware. The detected channel depends on where the
**executing** `oxios` lives (resolved through symlinks), not on whichever
binary happens to be first on `PATH`:

| Channel | Detection | Update behaviour |
|---|---|---|
| `Managed` | exe under `$OXIOS_HOME/versions/` or `$OXIOS_HOME/bin/` | tarball → flip → prune → daemon restart |
| `Brew` | exe under `/opt/homebrew/` or `/usr/local/Cellar/` | prints `brew upgrade oxios`, exits 0 |
| `Cargo` | exe under `$CARGO_HOME/bin` / `~/.cargo/bin` | same managed install path; one-time migration prompt to repoint `~/.cargo/bin/oxios` to the launcher |
| `Dev` | exe under a directory inside an `oxios` workspace (heuristic: ancestor has `Cargo.toml` + `crates/oxios-kernel/`) | refuses auto-update; suggests `cargo build` |
| `Unmanaged` | any other writable location | same managed install path + migration plan (see below) |

### Dry run

```bash
$ oxios update --dry-run
  Channel:  Managed
  Latest:   1.44.1
  Plan:     versions/1.44.1/oxios  (flip target)
            prune 1.43.0            (keep 1.44.0 + 1.44.1)
            shadows ahead of ~/.oxios/bin:
              /Users/you/bin/oxios → /Users/you/bin/oxios-1.44.0  (≈78 MB)
```

Dry run never mutates the filesystem, never replaces the
launcher, and never restarts the daemon. The release metadata IS
fetched (so the plan can show the actual latest version); only the
tarball download + extract + flip are skipped.

### Rollback

If an update misbehaves, flip back to the previous version with a single
command:

```bash
$ oxios update --rollback
  Channel:  Managed
  Current:  1.44.1
  Rollback: 1.44.0
  Flipping launcher…  done
  Restarting daemon…  done
  ✓ Oxios is now 1.44.0.
```

The store keeps the previous version on disk indefinitely after every
update (`keep-2` retention). Rollback requires ≥ 2 installed versions;
the command exits non-zero with a clear message otherwise.

### `oxios update --via cargo`

Force the legacy `cargo install oxios --locked` path even when a managed
install exists. Use this when you specifically want to test the cargo
build of the current release, or when the release tarball is
inaccessible.

### PATH shadow migration

When `oxios update` runs from a `Cargo` or `Unmanaged` channel, it audits
`$PATH` for other `oxios` entries. For each shadow ahead of the managed
launcher:

- **Symlink** (e.g. `~/bin/oxios → ~/bin/oxios-1.43.1`) → repointed to
  `$OXIOS_HOME/bin/oxios`. The previous target is preserved as
  `~/bin/oxios-1.43.1` (the original symlink target, renamed in place).
- **Regular file** (e.g. `~/bin/oxios-1.42.0`) → renamed to
  `~/bin/oxios-1.42.0.bak.<YYYYMMDD>` (one copy preserved for rollback).

On a TTY the migration lists every touched path and asks for confirmation;
on a non-TTY it prints the exact commands instead and requires
`--adopt` to apply them.

---

## Doctor — diagnostics & cleanup

```bash
$ oxios doctor
  ⬡ Oxios Doctor — System Diagnostics
  ────────────────────────────────────────────────
  ✓ Config file present (/Users/you/.oxios/config.toml)
  ✓ Credentials found (sk-a…xxxx, via config.toml)
  ✓ Workspace directory (/Users/you/.oxios/workspace)
  ✓ Daemon is running
  ⚠ No MCP servers configured
  ✓ Default model: anthropic/claude-sonnet-4-20250514
  ✓ oxicode CLI available (shared auth store)
  ✓ Port 4200 listening (daemon active)
  ✓ Managed install: 1.44.0 (launcher → versions/1.44.0/oxios)
  ────────────────────────────────────────────────
  Shadow report:
    /Users/you/bin/oxios-1.42.0-livestream  (regular file, ≈78 MB)
    /Users/you/bin/oxios-1.42.0-livestream2 (regular file, ≈78 MB)
    /Users/you/bin/oxios-1.42.0-livestream3 (regular file, ≈78 MB)
    /Users/you/bin/oxios-1.42.0-livestream4 (regular file, ≈78 MB)
    /Users/you/bin/oxios-1.42.0-livestream5 (regular file, ≈78 MB)
    /Users/you/bin/oxios-1.42.0-livestream6 (regular file, ≈78 MB)
  ────────────────────────────────────────────────
  8 checks passed, 1 advisory (PATH shadows listed above).
```

`oxios doctor --cleanup` clears leftover `.bak.*` files and stale
versioned copies reported in the shadow list. On a TTY it asks for
confirmation before deleting; on a non-TTY it prints the `rm` lines and
exits without touching the filesystem. Pass `--yes` to skip the prompt in
scripts:

```bash
oxios doctor --cleanup --yes
```

---

## Uninstalling

```bash
# 1. Stop the daemon
$ oxios stop

# 2. Remove the managed install + state
$ rm -rf ~/.oxios

# 3. Remove the PATH block from your shell profile
#    (delete the lines between "# BEGIN oxios (managed)" and "# END oxios (managed)"
#     in ~/.zshrc or ~/.zprofile)

# 4. If installed via brew:  brew uninstall oxios
#    If installed via cargo: cargo uninstall oxios
```

For per-machine resets without touching other channels, see
[Getting Started → Uninstalling](getting-started.md#11-uninstalling).

---

*Built by [a7garden](https://github.com/a7garden). Licensed under
[MIT](https://github.com/project-oxi/oxios/blob/main/LICENSE).*
