#!/usr/bin/env bash
set -euo pipefail

# ── P2.7 smoke: real daemon ping + Rust full_dispatch fake injection ──

TMPDIR=$(mktemp -d)
SOCK="${TMPDIR}/daedalusd.sock"
STATE="${TMPDIR}/state"
CONFIG_DIR="${TMPDIR}/config"

# All config goes into TMPDIR, never ~/.daedalus.
export DAEDALUSD_SOCK="${SOCK}"
export DAEDALUSD_STATE_DIR="${STATE}"
export DAEDALUS_SOUL_PATH="${CONFIG_DIR}/SOUL.md"
export DAEDALUS_MANAGED_AGENTS_PATH="${CONFIG_DIR}/managed-agents.yaml"
export DAEDALUS_SKILLS_DIR="${CONFIG_DIR}/skills"
export DAEDALUS_MODELS_YAML="${CONFIG_DIR}/models.yaml"

mkdir -p "${STATE}" "${CONFIG_DIR}/skills"

echo "You are Daedalus Phase 2." > "${DAEDALUS_SOUL_PATH}"

cat > "${DAEDALUS_MANAGED_AGENTS_PATH}" <<'YAML'
agents:
  test-agent:
    role_summary: "Phase 2 smoke test agent"
    tools: [task_done]
    permission: ask_user
    model_strategy:
      primary:
        model: fake
      fallback_chain: []
YAML

cat > "${DAEDALUS_MODELS_YAML}" <<'YAML'
providers: {}
models: []
YAML

cleanup() {
    kill "$DAEMON_PID" 2>/dev/null || true
    rm -rf "$TMPDIR"
}
trap cleanup EXIT

# ── A 段: 真实 daemon 启动 + Python ping ──

echo "=== P2.7 smoke: starting daedalusd ==="
cargo run --release &
DAEMON_PID=$!

for i in $(seq 1 30); do
    [ -S "$SOCK" ] && break
    sleep 0.1
done
[ -S "$SOCK" ] || { echo "FATAL: daedalusd socket never appeared"; exit 1; }

echo "=== P2.7 smoke: Python ping ==="
python -m daedalus.orch.cli ping --sock "$SOCK"
echo "ping OK"

# ── B 段: Rust full_dispatch fake 注入测试 ──

echo "=== P2.7 smoke: Rust full_dispatch integration tests ==="
cargo test --test full_dispatch -- --nocapture

echo "=== P2.7 smoke PASSED ==="
