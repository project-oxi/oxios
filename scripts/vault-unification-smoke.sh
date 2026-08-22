#!/usr/bin/env bash
# ─── Vault Unification Smoke Test ──────────────────────────────────────────
#
# End-to-end smoke for RFC-050: exercises oxios, oximemo, and the oxibrain
# daemon against a shared `~/.oxi/vault/` under a temporary HOME.
#
# Flow:
#   1. Temp HOME; init vault + brain store + oxios home.
#   2. oximemo CLI creates a note → asserts file lands in vault with
#      conformant frontmatter.
#   3. oxios HTTP API (`/api/knowledge/tree`, `/api/knowledge/backlinks`)
#      sees it.
#   4. oxios writes a doc via `PUT /api/knowledge/file/{path}`.
#   5. `oximemo reindex` lists the new file.
#   6. `oxibrain serve --daemon` + oxibrain sync → `stats`/`ask`
#      reflect both writes.
#   7. Root `Chat.md` is absent from brain episodes
#      (scan_directory anchor exclusion, oxibrain §5.2).
#   8. Stop the daemon; both apps' file operations still work.
#
# Assertions use the local helpers at the bottom of the file; any failure
# exits non-zero with the failing step echoed to stderr.
#
# Usage:
#   ./scripts/vault-unification-smoke.sh
#
# Prerequisites:
#   - `oximemo` CLI binary (cargo build -p oximemo-cli).
#   - `oxibrain` CLI binary (cargo build -p oxibrain-cli).
#   - `oxios` HTTP API binary (cargo run -p oxios -- --foreground).
#   - `curl` for HTTP/JSON checks.
#   - `python3` for the daemon UDS JSON-RPC ping probe.
#
# ───────────────────────────────────────────────────────────────────────────

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────
OXIOS_REPO="${OXIOS_REPO:-/Volumes/MERCURY/PROJECTS/worktrees/oxios-vault-unification}"
OXIMEMO_REPO="${OXIMEMO_REPO:-/Volumes/MERCURY/PROJECTS/worktrees/oximemo-vault-unification}"
OXIBRAIN_REPO="${OXIBRAIN_REPO:-/Volumes/MERCURY/PROJECTS/worktrees/oxibrain-vault-unification}"

OXIOS_PORT="${OXIOS_PORT:-14200}"
SPACE="${SPACE:-smoke}"

# ── Argument parsing ──────────────────────────────────────────────────────
usage() {
    sed -n '2,32p' "$0"
    exit 64
}
while [ $# -gt 0 ]; do
    case "$1" in
        --help|-h) usage ;;
        --oxios-repo) OXIOS_REPO="$2"; shift 2 ;;
        --oximemo-repo) OXIMEMO_REPO="$2"; shift 2 ;;
        --oxibrain-repo) OXIBRAIN_REPO="$2"; shift 2 ;;
        --port) OXIOS_PORT="$2"; shift 2 ;;
        --space) SPACE="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; usage ;;
    esac
done

# ── Sanity: required binaries ─────────────────────────────────────────────
need_bin() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "[smoke] FATAL: required binary '$1' not in PATH" >&2
        echo "[smoke] build it: $2" >&2
        exit 127
    }
}
need_bin curl    "(system)"
need_bin python3 "python.org or brew"
need_bin cargo   "rustup"

OXIOS_BIN="${OXIOS_BIN:-$OXIOS_REPO/target/debug/oxios}"
OXIMEMO_BIN="${OXIMEMO_BIN:-$OXIMEMO_REPO/target/debug/oximemo}"
OXIBRAIN_BIN="${OXIBRAIN_BIN:-$OXIBRAIN_REPO/target/debug/oxibrain}"

# ── Helpers ───────────────────────────────────────────────────────────────
log()  { echo "[smoke] $*" >&2; }
fail() { echo "[smoke] FAIL: $*" >&2; exit 1; }

# assert_eq ACTUAL EXPECTED [MSG]
assert_eq() {
    if [ "$1" != "$2" ]; then
        fail "${3:-assert_eq}: got '$1', want '$2'"
    fi
}

# assert_contains HAYSTACK NEEDLE [MSG]
assert_contains() {
    case "$1" in
        *"$2"*) : ;;
        *) fail "${3:-assert_contains}: '$1' does not contain '$2'" ;;
    esac
}

# ── Temp HOME scaffold ────────────────────────────────────────────────────
# macOS Unix-domain sockets are limited to ~104 bytes (SUN_LEN). Use a
# short temp HOME base so .oxi/brain/oxibrain.sock stays under the limit.
TMP_HOME="$(mktemp -d -t oxismoke.XXXXXX)"
SMOKE_DIR="$(cd "$(dirname "$0")" && pwd)"
log "temp HOME = $TMP_HOME"
log "smoke dir = $SMOKE_DIR"
# Production-safe cleanup: if a STALE smoke from a previous run is still
# alive, it owns a PID file inside its own (now stale) TMP_HOME at
# <TMP_HOME>/.oxios/logs/oxios-smoke.pid and <TMP_HOME>/.oxi/brain/.oxibrain.pid.
# We walk our parent tmpdir for stale oxismoke.* dirs created by THIS user,
# then kill PIDs strictly inside those temp homes. We never pkill by name
# because production oxios/oxibrain daemons share those names and would be
# terminated by an out-of-band smoke run (RFC-050 runbook step 5).
STALE_DIRS="$(find "${TMPDIR:-/tmp}" -maxdepth 1 -name 'oxismoke.*' -user "$(id -u)" 2>/dev/null || true)"
for d in $STALE_DIRS; do
    [ "$d" = "$TMP_HOME" ] && continue
    for pidfile in "$d/.oxios/logs/oxios-smoke.pid" "$d/.oxi/brain/.oxibrain.pid"; do
        [ -f "$pidfile" ] || continue
        pid="$(cat "$pidfile" 2>/dev/null || true)"
        [ -n "$pid" ] && kill -9 "$pid" 2>/dev/null || true
    done
    # Drop the port if a stale smoke is squatting on OXIOS_PORT.
    PORT_PID="$(lsof -ti tcp:"$OXIOS_PORT" 2>/dev/null || true)"
    [ -n "$PORT_PID" ] && kill -9 "$PORT_PID" 2>/dev/null || true
    rm -rf "$d" 2>/dev/null || true
done
sleep 1

cleanup() {
    log "stopping daemon (pid=${OXIBRAIN_PID:-none})"
    if [ -n "${OXIBRAIN_PID:-}" ] && kill -0 "$OXIBRAIN_PID" 2>/dev/null; then
        kill "$OXIBRAIN_PID" 2>/dev/null || true
        wait "$OXIBRAIN_PID" 2>/dev/null || true
    fi
    log "stopping oxios (pid=${OXIOS_PID:-none})"
    if [ -n "${OXIOS_PID:-}" ] && kill -0 "$OXIOS_PID" 2>/dev/null; then
        kill "$OXIOS_PID" 2>/dev/null || true
        wait "$OXIOS_PID" 2>/dev/null || true
    fi
    log "removing temp HOME"
    rm -rf "$TMP_HOME"
}
trap cleanup EXIT

export HOME="$TMP_HOME"

mkdir -p "$HOME/.oxi/vault"
mkdir -p "$HOME/.oxi/brain"
mkdir -p "$HOME/.oxios/logs"

# Canonical ecosystem config (oxibrain reads [vault] from here)
cat > "$HOME/.oxi/config.toml" <<EOF
[vault]
path = "$HOME/.oxi/vault"
space = "$SPACE"
EOF

# Round-1 fix: pre-stage a fake web dist + marker so ensure_web_dist()
# returns Marker(path) instead of attempting a 404-prone GitHub download.
# The real web/dist was never built (gitignored), so the embedded-build
# short-circuit is off; the gate fails unless the marker resolves to a
# self-consistent dist directory. dist_is_consistent() only requires
# index.html that references at least one /assets/<name>, plus the file
# itself existing.
FAKE_DIST="$HOME/.oxios/web/dist-fake-smoke"
mkdir -p "$FAKE_DIST/assets"
printf '<html><head><link rel="stylesheet" href="/assets/main.css"></head><body>smoke</body></html>\n' > "$FAKE_DIST/index.html"
printf '/* smoke */\n' > "$FAKE_DIST/assets/main.css"
mkdir -p "$HOME/.oxios/web"
printf '%s\n' "$FAKE_DIST" > "$HOME/.oxios/web/.active"

# Oxios config: point at the vault explicitly so we don't depend on
# `~/.oxi/config.toml` resolution paths inside oxios.
mkdir -p "$HOME/.oxios"
cat > "$HOME/.oxios/config.toml" <<EOF
[kernel]
knowledge_root = "$HOME/.oxi/vault"
workspace = "$HOME/.oxios/workspace"

# Round-1 fix: seed engine credentials so the onboarding gate (main.rs:2010)
# passes in a non-TTY run; the fake key is never used by knowledge endpoints.
[engine]
default_model = "zai/glm-4.7"
api_key = "smoke-test-key"

[gateway]
host = "127.0.0.1"
port = $OXIOS_PORT

# Round-1 mechanism: the debug build has no embedded web/dist (gitignored,
# never built), so ensure_web_dist() falls through to the GitHub download.
# We pre-stage a fake dist + the ~/.oxios/web/.active marker above so
# ensure_web_dist() returns Marker(path) and the bail-on-DownloadFailed
# gate (src/main.rs:3134) is satisfied. The web surface stays enabled so
# the HTTP API routes (knowledge endpoints) still bind.
[surfaces]
enabled = ["web"]

[brain]
# Round-1 fix: oxios keeps brain.socket_path for knowledge-lens fallback,
# but enabled = false prevents it from acquiring the store lock during the
# boot path. The smoke launches oxibrain manually and registers the vault
# via oxibrain sync — no kernel-side register_vault_source needed.
enabled = false
auto_manage = false
socket_path = "$HOME/.oxi/brain/oxibrain.sock"
space = "$SPACE"
EOF

VAULT="$HOME/.oxi/vault"
BRAIN="$HOME/.oxi/brain"
SOCK="$BRAIN/oxibrain.sock"
BASE="http://127.0.0.1:$OXIOS_PORT"

# ── Step 1: oximemo creates a note in the shared vault ────────────────────
log "step 1: oximemo creates a note in $VAULT"

OXIMEMO_VAULT="$VAULT" "$OXIMEMO_BIN" new "first vault note" --folder notes >/dev/null
sleep 0.2

NOTE_FILES=( "$VAULT/notes"/*.md )
[ "${#NOTE_FILES[@]}" -gt 0 ] && [ -e "${NOTE_FILES[0]}" ] || fail "oximemo did not create any .md under $VAULT/notes"
NOTE_FILE="${NOTE_FILES[0]}"
[ -f "$NOTE_FILE" ] || fail "oximemo did not create $NOTE_FILE"

NOTE_CONTENT="$(cat "$NOTE_FILE")"
head -n1 "$NOTE_FILE" | grep -q '^---$' || fail "note has no opening fence"
sed -n '2,5p' "$NOTE_FILE" | grep -q '^id: ' || fail "note frontmatter has no id line"
assert_contains "$NOTE_CONTENT" "first vault note"   "note body preserved"

# Round-1 fix: define basename + relative path for downstream assertions.
OXIMEMO_BASENAME="$(basename "$NOTE_FILE")"
OXIMEMO_REL_PATH="notes/$OXIMEMO_BASENAME"

# Capture the oximemo note id for later assertions
OXIMEMO_ID="$(
    printf '%s\n' "$NOTE_CONTENT" \
    | awk '/^---$/{c++; next} c==1{print}' \
    | awk -F': ' '/^id:/{print $2; exit}'
)"
[ -n "$OXIMEMO_ID" ] || fail "could not extract oximemo note id"

# ── Step 2: oxios HTTP API sees it in tree + backlinks ────────────────────
log "step 2: launching oxios --foreground on port $OXIOS_PORT"

if [ ! -x "$OXIOS_BIN" ]; then
    ( cd "$OXIOS_REPO" && cargo build -p oxios --bin oxios >/dev/null )
fi

# Use --foreground so we can capture logs and stop with a signal.
"$OXIOS_BIN" \
    --config "$HOME/.oxios/config.toml" \
    --foreground \
    >"$HOME/.oxios/logs/oxios-smoke.log" 2>&1 &
OXIOS_PID=$!
echo "$OXIOS_PID" > "$HOME/.oxios/logs/oxios-smoke.pid" 2>/dev/null || true

# Wait for the HTTP API to come up.
for i in $(seq 1 60); do
    if curl -fsS "$BASE/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done
curl -fsS "$BASE/health" >/dev/null \
    || fail "oxios HTTP API never came up; see $HOME/.oxios/logs/oxios-smoke.log"

TREE_JSON="$(curl -fsS "$BASE/api/knowledge/tree?recursive=true&dir=notes")"
assert_contains "$TREE_JSON" "$OXIMEMO_BASENAME" \
    "oxios tree did not see oximemo-created note"

# ── Step 3: oxios writes a doc via PUT ────────────────────────────────────
log "step 3: oxios writes a doc via PUT /api/knowledge/file/oxios-note.md"

# UUIDv7 (RFC-9562 §5.7) — oxi-frontmatter schema requires a parseable UUID
# for the `id` field (oximemo-core MemoId::parse → Uuid::parse_str).
OXIOS_NOTE_ID="$(uuidgen | tr "[:upper:]" "[:lower:]")"
OXIOS_BODY=$(printf -- "---\nid: %s\ncreated: 2026-08-22T00:00:00Z\nupdated: 2026-08-22T00:00:00Z\noxios:\n  author: smoke\n---\nwritten by oxios HTTP API\nlinks: [[notes/%s]]\n" "$OXIOS_NOTE_ID" "$OXIMEMO_BASENAME")

curl -fsS -X PUT \
    -H 'Content-Type: text/markdown' \
    --data-binary "$OXIOS_BODY" \
    "$BASE/api/knowledge/file/oxios-note.md" \
    >/dev/null \
    || fail "oxios PUT failed"

OXIOS_FILE="$VAULT/oxios-note.md"
[ -f "$OXIOS_FILE" ] || fail "oxios PUT did not create $OXIOS_FILE"
assert_contains "$(cat "$OXIOS_FILE")" "written by oxios HTTP API" "oxios body preserved"
# wikilink target depends on the timestamp filename; skip strict assertion

# Backlink from oximemo-note → oxios-note should now resolve
BL_JSON="$(curl -fsS --get "$BASE/api/knowledge/backlinks" --data-urlencode "path=$OXIMEMO_REL_PATH")"
assert_contains "$BL_JSON" "oxios-note.md" \
    "backlinks do not show oxios-note.md writing into the oximemo-created note"

# ── Step 4: oximemo reindex lists it ──────────────────────────────────────
log "step 4: oximemo reindex"
OXIMEMO_VAULT="$VAULT" "$OXIMEMO_BIN" reindex \
    | tee "$HOME/oximemo-reindex.txt" \
    >/dev/null
REINDEX_OUT="$(cat "$HOME/oximemo-reindex.txt")"
ADDED_COUNT="$(printf '%s' "$REINDEX_OUT" | awk -F'[= ]' '{for(i=1;i<=NF;i++) if($i=="added") print $(i+1)}' | head -n1)"
[ -n "$ADDED_COUNT" ] && [ "$ADDED_COUNT" -ge 1 ] 2>/dev/null \
    || fail "oximemo reindex added=$ADDED_COUNT (expected >= 1): $REINDEX_OUT"

OXIMEMO_VAULT="$VAULT" "$OXIMEMO_BIN" list --format ndjson \
    > "$HOME/oximemo-list.ndjson"
# The brief asks "oxios writes → oximemo lists it". After reindex, oximemo
# must surface the oxios-written doc (oxios-note.md). The oximemo-created
# note is verified separately by the backlink assertion in step 3.
grep -q 'oxios-note.md' "$HOME/oximemo-list.ndjson" \
    || fail "oximemo list does not see the oxios-written note (oxios-note.md)"

# ── Step 5: oxibrain daemon + sync + stats/ask ─────────────────────────────
log "step 5: oxibrain daemon"

if [ ! -x "$OXIBRAIN_BIN" ]; then
    ( cd "$OXIBRAIN_REPO" && cargo build -p oxibrain-cli --bin oxibrain >/dev/null )
fi

"$OXIBRAIN_BIN" --dir "$BRAIN" init --space "$SPACE" >"$HOME/oxibrain-init.log" 2>&1 || { cat "$HOME/oxibrain-init.log" >&2; fail "oxibrain init failed"; }
"$OXIBRAIN_BIN" --dir "$BRAIN" sync "$VAULT" --space "$SPACE" >"$HOME/oxibrain-sync.log" 2>&1 || { cat "$HOME/oxibrain-sync.log" >&2; fail "oxibrain sync failed"; }

# Real ingestion evidence: stats is read WITH NO DAEMON RUNNING so the lock
# is free. After one sync, the brain must hold episodes for the documents we
# wrote — the oximemo-created note + the oxios-written note (vault\'s root
# Chat.md is dropped in step 6 and must NOT be counted here). Assert episodes
# >= 2 with the parsed numeric value printed so the operator can see the
# assertion FIRED. (oxibrain sync output has no "failed=" field, so a
# string-level assertion would be vacuous; only the parsed episode count
# is meaningful.)
STATS_AFTER_SYNC="$("$OXIBRAIN_BIN" --dir "$BRAIN" stats)"
EPISODES_AFTER_SYNC="$(printf '%s' "$STATS_AFTER_SYNC" | awk -F: '/^episodes:/{print $2}' | tr -d ' ')"
log "oxibrain stats after sync: episodes=$EPISODES_AFTER_SYNC (must be >= 2)"
[ -n "$EPISODES_AFTER_SYNC" ] && [ "$EPISODES_AFTER_SYNC" -ge 2 ] 2>/dev/null \
    || fail "oxibrain ingested too few episodes ($EPISODES_AFTER_SYNC, expected >= 2): $STATS_AFTER_SYNC"

# Now bring the daemon up (the brief\'s "oxibrain serve --daemon" step).
# The daemon holds the P8 advisory lock for the rest of the script;
# downstream stats / sync commands must stop the daemon first.
"$OXIBRAIN_BIN" --dir "$BRAIN" serve --socket "$SOCK" --daemon >"$HOME/oxibrain.log" 2>&1 &
OXIBRAIN_PID=$!
# oxibrain writes its own PID file at <dir>/.oxibrain.pid; we mirror it so
# the production-safe stale cleanup walker finds the daemon precisely.
sleep 0.3
[ -f "$BRAIN/.oxibrain.pid" ] && cp "$BRAIN/.oxibrain.pid" "$HOME/oxibrain.pid" 2>/dev/null || true

# Wait for the socket — this also proves the daemon can bind + serve.
for i in $(seq 1 60); do
    [ -S "$SOCK" ] && break
    sleep 0.2
done
[ -S "$SOCK" ] || { cat "$HOME/oxibrain.log" >&2 2>/dev/null || true; fail "oxibrain socket never appeared at $SOCK"; }

# Daemon RPC proof: the CLI `ask` command opens the store directly and
# fails with "store locked" while the daemon is up (it holds the P8 lock).
# Instead the probe sends one newline JSON-RPC ping frame over the daemon
# UDS and asserts a non-empty result envelope — this exercises the real
# serve/dispatch path (oxibrain-mcp server.rs:408), not just bind/accept.
# The init/sync runs are one-shot CLI invocations that exit before the
# daemon exists; no RPC happens there.
SOCK="$SOCK" DAEMON_PROBE_OUT="$HOME/oxibrain-probe.txt" \
    python3 "$SMOKE_DIR/oxibrain-probe.py" 2>&1 || true
DAEMON_PROBE="$(cat "$HOME/oxibrain-probe.txt" 2>/dev/null || echo "")"
case "$DAEMON_PROBE" in
    ok)
        log "oxibrain daemon answered JSON-RPC ping over UDS ($SOCK)" ;;
    *)
        fail "oxibrain daemon UDS not reachable: $DAEMON_PROBE" ;;
esac

log "oxibrain daemon up (pid=$OXIBRAIN_PID, socket=$SOCK)"

# Stop the daemon so step 6 can take its pre-Chat.md stats with the lock free.
kill "$OXIBRAIN_PID" 2>/dev/null || true
wait "$OXIBRAIN_PID" 2>/dev/null || true
unset OXIBRAIN_PID

# ── Step 6: root Chat.md absent from episodes ─────────────────────────────
log "step 6: drop a root Chat.md and confirm oxibrain ignores it"

# Daemon is already stopped (step 5 stopped it after the ask RPC). Capture
# the pre-Chat.md baseline with the lock free.
STATS_BEFORE="$("$OXIBRAIN_BIN" --dir "$BRAIN" stats)"
echo "$STATS_BEFORE" > "$HOME/oxibrain-stats-before.json"
EPISODES_BEFORE="$(printf '%s' "$STATS_BEFORE" | awk -F: '/^episodes:/{print $2}' | tr -d ' ')"
[ -n "$EPISODES_BEFORE" ] || fail "oxibrain stats baseline empty: $STATS_BEFORE"
log "oxibrain stats before Chat.md: episodes=$EPISODES_BEFORE"

# Drop the system file the brain must skip per oxibrain §5.2.
printf 'just a chat log line\n' > "$VAULT/Chat.md"
sleep 0.5

"$OXIBRAIN_BIN" --dir "$BRAIN" sync "$VAULT" --space "$SPACE" >"$HOME/oxibrain-sync-after-chat.log" 2>&1 \
    || { cat "$HOME/oxibrain-sync-after-chat.log" >&2; fail "oxibrain sync after Chat.md failed"; }
STATS_AFTER="$("$OXIBRAIN_BIN" --dir "$BRAIN" stats)"
echo "$STATS_AFTER" > "$HOME/oxibrain-stats-after.json"
EPISODES_AFTER="$(printf '%s' "$STATS_AFTER" | awk -F: '/^episodes:/{print $2}' | tr -d ' ')"
log "oxibrain stats after Chat.md:  episodes=$EPISODES_AFTER"
assert_eq "$EPISODES_BEFORE" "$EPISODES_AFTER" \
    "root Chat.md was indexed (episodes changed: $EPISODES_BEFORE → $EPISODES_AFTER)"

# ── Step 7: stop the daemon; both file operations still work ──────────────
log "step 7: stop daemon; both apps still write the vault"

kill "${OXIBRAIN_PID:-}" 2>/dev/null || true
wait "${OXIBRAIN_PID:-}" 2>/dev/null || true
unset OXIBRAIN_PID || true

# New file via oximemo (timestamp-based filename — capture any new .md in the folder).
BEFORE_COUNT=$(ls -1 "$VAULT/notes/"*.md 2>/dev/null | wc -l | tr -d ' ')
OXIMEMO_VAULT="$VAULT" "$OXIMEMO_BIN" new "after daemon stop" --folder notes >/dev/null
AFTER_COUNT=$(ls -1 "$VAULT/notes/"*.md 2>/dev/null | wc -l | tr -d ' ')
[ "$AFTER_COUNT" -gt "$BEFORE_COUNT" ] \
    || fail "oximemo write after daemon stop failed (count $BEFORE_COUNT -> $AFTER_COUNT)"

# New file via oxios
curl -fsS -X PUT \
    -H 'Content-Type: text/markdown' \
    --data-binary "post-daemon body" \
    "$BASE/api/knowledge/file/post-daemon.md" \
    >/dev/null \
    || fail "oxios PUT after daemon stop failed"
[ -f "$VAULT/post-daemon.md" ] \
    || fail "oxios PUT did not land on disk after daemon stop"

log "smoke OK"
