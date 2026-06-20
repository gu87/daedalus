#!/bin/bash
# dogfood.sh — one-command Daedalus dogfood startup.
# Launches daedalusd + daedalus-desktop (Electron).
# Prerequisites: Rust, Node.js, npm install, ~/.daedalus/ configs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

TMP_DIR=$(mktemp -d /tmp/daedalus-dogfood-XXXXXX)
SOCK_PATH="$TMP_DIR/daedalusd.sock"
STATE_DIR="$TMP_DIR/state"
HTTP_ADDR="127.0.0.1:9800"

# Check port not in use.
if lsof -i :9800 > /dev/null 2>&1; then
  echo "ERROR: port 9800 is already in use" >&2
  rm -rf "$TMP_DIR"
  exit 1
fi

DAEMON_PID=""
cleanup() {
  if [ -n "${DAEMON_PID:-}" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "=== Daedalus Dogfood ==="

# 1. Build
echo "building daedalusd..."
cargo build -p daedalusd
echo "building daedalus-desktop..."
(cd daedalus-desktop && npm run build)

# 2. Start daemon (env vars only, no CLI flags)
echo "starting daedalusd..."
DAEDALUSD_SOCK="$SOCK_PATH" \
DAEDALUSD_STATE_DIR="$STATE_DIR" \
DAEDALUSD_HTTP_ADDR="$HTTP_ADDR" \
  "$PROJECT_DIR/target/debug/daedalusd" &
DAEMON_PID=$!

# 3. Wait for daemon health, abort if daemon dies.
ready=0
for i in $(seq 1 30); do
  if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
    echo "ERROR: daemon exited prematurely" >&2
    exit 1
  fi
  if curl -sf "http://$HTTP_ADDR/api/health" > /dev/null 2>&1; then
    ready=1
    echo "daemon ready after ${i}s"
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  echo "ERROR: daemon did not become ready within 30s" >&2
  exit 1
fi

# 4. Start Electron desktop in foreground (Ctrl+C kills Electron, trap kills daemon).
echo "starting daedalus-desktop..."
echo "=== Dogfood running ==="
echo "HTTP: http://$HTTP_ADDR"
echo "Press Ctrl+C to stop"
DAEDALUSD_HTTP_ADDR="http://$HTTP_ADDR" \
DAEDALUSD_SOCK="$SOCK_PATH" \
  npm run electron:dev --prefix daedalus-desktop
