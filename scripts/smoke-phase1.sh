#!/usr/bin/env bash
set -euo pipefail

# ── Phase 1 smoke test ────────────────────────────────────────────────
# Starts daedalusd on a temp socket, pings it, verifies SQLite schema,
# and cleans up.  Exits 0 on success, non-zero on any failure.

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

pass() { echo -e "${GREEN}PASS${NC} $*"; }
fail() { echo -e "${RED}FAIL${NC} $*"; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DAEMON="$PROJECT_DIR/target/debug/daedalusd"
PYTHON="${PYTHON:-python3.12}"

[ -x "$DAEMON" ] || fail "daedalusd not found — run 'cargo build' first"

# ── temporary workspace ────────────────────────────────────────────────

WORK_DIR="$(mktemp -d)"
SOCK="$WORK_DIR/daedalusd.sock"
STATE_DIR="$WORK_DIR/state"
mkdir -p "$STATE_DIR"
DB="$STATE_DIR/daedalusd.sqlite"

DAEMON_PID=""
cleanup() {
    if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# ── start daemon ──────────────────────────────────────────────────────

echo "=== Start daedalusd ==="
DAEDALUSD_SOCK="$SOCK" DAEDALUSD_STATE_DIR="$STATE_DIR" "$DAEMON" </dev/null &
DAEMON_PID=$!

DEADLINE=$((SECONDS + 10))
while [ ! -S "$SOCK" ]; do
    [ $SECONDS -le $DEADLINE ] || fail "daemon did not create socket within 10s"
    sleep 0.1
done
pass "daemon started (pid=$DAEMON_PID)"

# ── ping ──────────────────────────────────────────────────────────────

echo "=== Ping ==="
PING_OUT="$(/tmp/daedalus-venv/bin/daedalus ping --socket "$SOCK" 2>&1)"
PING_RC=$?

[ $PING_RC -eq 0 ] || fail "daedalus ping failed (exit $PING_RC): $PING_OUT"
echo "$PING_OUT" | grep -q '"system.pong"' || fail "not pong: $PING_OUT"
pass "ping → pong"

# ── verify SQLite schema (daemon-created DB, read-only) ──────────────

echo "=== Verify SQLite schema ==="
SCHEMA_PY="$WORK_DIR/check_schema.py"
cat > "$SCHEMA_PY" << 'PYEOF'
import sqlite3, sys

db = sys.argv[1]
conn = sqlite3.connect(db)
conn.execute("PRAGMA foreign_keys = ON")

# ── 1. user_version must be 1 (daemon ran migration) ───────────────
v = conn.execute("PRAGMA user_version").fetchone()[0]
assert v == 1, f"expected user_version=1, got {v}"

# ── 2. agent_runs table must exist ──────────────────────────────────
table = conn.execute(
    "SELECT name FROM sqlite_master WHERE type='table' AND name='agent_runs'"
).fetchone()
assert table is not None, "agent_runs table not found"

# ── 3. columns ──────────────────────────────────────────────────────
cols = {row[1] for row in conn.execute("PRAGMA table_info('agent_runs')")}
expected = {"run_id","agent_id","task_id","parent_run_id","status",
            "spawn_depth","spawned_at","heartbeat_at","completed_at",
            "timeout_seconds","error_taxonomy","outbox_json"}
assert cols == expected, f"column mismatch: {cols - expected} missing, {expected - cols} extra"

# ── 4. indexes ──────────────────────────────────────────────────────
idxs = {row[1] for row in conn.execute("PRAGMA index_list('agent_runs')")}
assert "idx_agent_status" in idxs, f"idx_agent_status missing (have {idxs})"
assert "idx_orphan_check" in idxs, f"idx_orphan_check missing (have {idxs})"

# ── 5. CHECK constraint: reject illegal status ─────────────────────
conn.execute("INSERT INTO agent_runs(run_id,agent_id,task_id,status,spawn_depth,spawned_at) VALUES('r1','a','t','queued',0,1700000000)")
try:
    conn.execute("INSERT INTO agent_runs(run_id,agent_id,task_id,status,spawn_depth,spawned_at) VALUES('r2','a','t','bad',0,1700000000)")
    assert False, "CHECK constraint did not fire"
except sqlite3.IntegrityError:
    pass

# ── 6. FK constraint: reject nonexistent parent ────────────────────
try:
    conn.execute("INSERT INTO agent_runs(run_id,agent_id,task_id,status,parent_run_id,spawn_depth,spawned_at) VALUES('r3','a','t','queued','ghost',0,1700000000)")
    assert False, "FK constraint did not fire"
except sqlite3.IntegrityError:
    pass

conn.close()
print("OK")
PYEOF

SCHEMA_RESULT="$("$PYTHON" "$SCHEMA_PY" "$DB" 2>&1)"
[ "$SCHEMA_RESULT" = "OK" ] || fail "schema check: $SCHEMA_RESULT"
pass "schema verified (user_version, columns, indexes, CHECK, FK)"

# ── stop daemon ───────────────────────────────────────────────────────

echo "=== Shutdown ==="
kill "$DAEMON_PID"
wait "$DAEMON_PID" 2>/dev/null || true
DAEMON_PID=""
sleep 0.2

[ ! -S "$SOCK" ] || fail "socket not cleaned up: $SOCK"
pass "socket cleaned up"

echo
echo -e "${GREEN}=== Phase 1 smoke test PASSED ===${NC}"
