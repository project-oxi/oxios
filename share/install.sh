#!/bin/sh
# oxios managed installer — macOS ARM64.
# curl -fsSL https://raw.githubusercontent.com/project-oxi/oxios/main/share/install.sh | sh
set -eu

REPO="project-oxi/oxios"
ASSET="oxios-aarch64-apple-darwin.tar.gz"

# Resolve the oxios home — mirrors oxios_kernel::oxi_home::{oxi_home,oxios_home}:
#   $OXIOS_HOME > $OXI_HOME/oxios > $HOME/.oxi/oxios
if [ -n "${OXIOS_HOME:-}" ]; then
  OXIOS_ROOT="$OXIOS_HOME"
elif [ -n "${OXI_HOME:-}" ]; then
  OXIOS_ROOT="${OXI_HOME%/}/oxios"
else
  OXIOS_ROOT="$HOME/.oxi/oxios"
fi

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ;;
  *) echo "unsupported platform: $(uname -s) $(uname -m) (macOS ARM64 only)" >&2; exit 1 ;;
esac

# Capture response first so curl's exit status is checked directly (a partial/error
# body that happens to contain a tag_name field can't slip past).
resp=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest") \
  || { echo "failed to fetch latest release metadata" >&2; exit 1; }
tag=$(printf %s "$resp" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | tr -d '\r')
[ -n "$tag" ] || { echo "failed to resolve latest release" >&2; exit 1; }
ver=${tag#v}

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
base="https://github.com/$REPO/releases/download/$tag"
curl -fsSL -o "$tmp/$ASSET"        "$base/$ASSET"
curl -fsSL -o "$tmp/$ASSET.sha256" "$base/$ASSET.sha256"
(cd "$tmp" && shasum -a 256 -c "$ASSET.sha256" >/dev/null) || { echo "sha256 mismatch" >&2; exit 1; }

dest="$OXIOS_ROOT/versions/$ver"
mkdir -p "$dest" "$OXIOS_ROOT/bin"
tar -xzf "$tmp/$ASSET" -C "$dest"
chmod 755 "$dest/oxios"

# Use a path relative to $OXIOS_ROOT/bin/ — matches `flip_launcher` in Rust
# (src/managed_install.rs) and keeps `current_target_version` consistent.
# Absolute paths were silently broken: `current_target_version` only parses
# the 4-component relative form (`../versions/<v>/oxios`), so a fresh
# install.sh install made `oxios update --rollback` fail with
# "no previous version to roll back to" until a real `oxios update`
# overwrote the launcher with the relative form.
ln -sfn "../versions/$ver/oxios" "$OXIOS_ROOT/bin/.oxios.tmp"
mv -f "$OXIOS_ROOT/bin/.oxios.tmp" "$OXIOS_ROOT/bin/oxios"

# keep-2: keep current version + 1 previous (the 2 newest by version-sort).
# Walk in reverse-version order excluding current; drop the newest non-current
# (that's the "previous" we keep); rm the rest. Every step's exit status is
# visible to set -e — no subshell, no pipeline masking.
# ls | grep | sort -rV is intentional: version dirs are release tags (numeric/dot),
# and we need sort -V which POSIX globs can't do.
# shellcheck disable=SC2010
list=$(cd "$OXIOS_ROOT/versions" && ls -1 | grep -v "^$ver$" | sort -rV)
# shellcheck disable=SC2086
# set -- $list must word-split; quoted form collapses the list to a single arg.
set -- $list
# Guard: on a fresh install (or sole-current-version case) the list is empty;
# shift would fail under set -eu. The "previous" we KEEP only exists when
# there's a newer-of-excluding-current entry — if there isn't, keep nothing
# extra and move on.
if [ $# -gt 0 ]; then
  shift  # drop the newest non-current entry — that's the "previous" we KEEP
fi
for old in "$@"; do
  rm -rf "$OXIOS_ROOT/versions/$old" || { echo "prune failed: $old" >&2; exit 1; }
done

rc="$HOME/.zshrc"; [ -f "$rc" ] || rc="$HOME/.zprofile"
# Rewrite any managed PATH block, not just skip when one exists: a block
# pointing at a previous OXIOS_ROOT (e.g. the legacy ~/.oxios home) satisfies
# a marker-only check and leaves `oxios` unresolvable. Drop every managed
# block, then append one for the current root.
if grep -q 'BEGIN oxios (managed)' "$rc" 2>/dev/null; then
  tmp_rc=$(mktemp)
  sed "/# BEGIN oxios (managed)/,/# END oxios (managed)/d" "$rc" > "$tmp_rc" ||
    { rm -f "$tmp_rc"; echo "failed to update PATH block in $rc" >&2; exit 1; }
  mv "$tmp_rc" "$rc" ||
    { rm -f "$tmp_rc"; echo "failed to update PATH block in $rc" >&2; exit 1; }
fi
# SC2016 disable: \$PATH must remain literal in the user's rc file so it expands at
# shell startup, not at printf time.
# shellcheck disable=SC2016
printf '\n# BEGIN oxios (managed)\nexport PATH="%s/bin:\$PATH"\n# END oxios (managed)\n' "$OXIOS_ROOT" >> "$rc"
echo "PATH entry for $OXIOS_ROOT/bin ensured in $rc — restart your shell or: export PATH=\"$OXIOS_ROOT/bin:\$PATH\""

echo "installed oxios $ver -> $OXIOS_ROOT/bin/oxios"
echo "shadow check:"
found=0
oldifs=$IFS; IFS=:
for d in $PATH; do
  [ -x "$d/oxios" ] || continue
  case "$(cd "$d" && pwd -P)" in "$OXIOS_ROOT/bin") continue ;; esac
  echo "  note: another oxios at $d/oxios (may shadow; run 'oxios doctor' after install)"
  found=1
done
IFS=$oldifs
if [ "$found" = 0 ]; then echo "  none"; fi
