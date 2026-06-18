#!/usr/bin/env bash
# smoke-phase4.sh — Phase 4 full-chain acceptance.
# Verifies all HTTP API endpoints + daedalus-desktop build.
# Uses a local Python mock OpenAI-compatible server. No external network.
set -euo pipefail

echo "=== Phase 4 Smoke ==="

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DAEMON_BIN="$PROJECT_DIR/target/debug/daedalusd"
TMP_DIR=$(mktemp -d /tmp/smoke-phase4-XXXXXX)
SOCK_PATH="$TMP_DIR/daedalusd.sock"
STATE_DIR="$TMP_DIR/state"
MODELS_YAML="$TMP_DIR/models.yaml"

# P4.6 fixup: dynamic ports to avoid conflicts.
HTTP_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
MOCK_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')

# P4.6 fixup: guard cleanup for unset PIDs under set -u.
DAEMON_PID=""
MOCK_PID=""

cleanup() {
  echo "--- cleaning up ---"
  if [ -n "${DAEMON_PID:-}" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  if [ -n "${MOCK_PID:-}" ]; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# ── build daemon if needed ────────────────────────────────────────────
if [ ! -x "$DAEMON_BIN" ]; then
  echo "building daedalusd..."
  cargo build -p daedalusd 2>&1 | tail -1
fi

# ── start mock OpenAI-compatible server ───────────────────────────────
echo "starting mock server on 127.0.0.1:$MOCK_PORT ..."
python3 -c "
import http.server, json

class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get('content-length', 0))
        data = json.loads(self.rfile.read(n)) if n else {}
        assert data.get('model') == 'upstream-x', f'bad model: {data}'
        assert data.get('max_tokens') == 1, f'bad max_tokens: {data}'
        assert data.get('temperature') == 0.0, f'bad temperature: {data}'
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({
            'choices':[{'message':{'content':'pong'}}]
        }).encode())
    def log_message(self, f, *a): pass

s = http.server.HTTPServer(('127.0.0.1', $MOCK_PORT), H)
s.serve_forever()
" &
MOCK_PID=$!
sleep 0.5

# ── prepare models.yaml ───────────────────────────────────────────────
cat > "$MODELS_YAML" <<EOF
providers:
  test_prov:
    type: openai_compat

models:
  - id: local-model
    provider: test_prov
    base_url: http://127.0.0.1:$MOCK_PORT/v1
    api_key_env: TEST_API_KEY
    model_id: upstream-x
EOF

export TEST_API_KEY=sk-test

# ── start daedalusd ───────────────────────────────────────────────────
echo "starting daedalusd on $SOCK_PATH (HTTP 127.0.0.1:$HTTP_PORT) ..."
DAEDALUSD_SOCK="$SOCK_PATH" \
DAEDALUSD_STATE_DIR="$STATE_DIR" \
DAEDALUSD_HTTP_ADDR="127.0.0.1:$HTTP_PORT" \
DAEDALUS_MODELS_YAML="$MODELS_YAML" \
"$DAEMON_BIN" &
DAEMON_PID=$!

for i in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:$HTTP_PORT/api/health" > /dev/null 2>&1; then
    echo "daedalusd ready after ${i}s"
    break
  fi
  sleep 1
done

# ── smoke tests ────────────────────────────────────────────────────────
failures=0
BASE="http://127.0.0.1:$HTTP_PORT"

check_get() {
  local desc="$1" url="$2" expected="$3" py_assert="${4:-}"
  printf "  %-40s " "$desc"
  local status
  status=$(curl -sS -o /tmp/smoke_body.txt -w "%{http_code}" "$url")
  if [ "$status" != "$expected" ]; then
    echo "FAIL (status=$status)"
    failures=$((failures + 1))
    return
  fi
  if [ -n "$py_assert" ]; then
    if ! python3 -c "$py_assert" < /tmp/smoke_body.txt 2>/dev/null; then
      echo "FAIL (body)"
      failures=$((failures + 1))
      return
    fi
  fi
  echo "OK"
}

# 1. health
check_get "GET /api/health" "$BASE/api/health" 200 \
  "import sys,json; d=json.load(sys.stdin); assert d['status']=='ok'; assert d['db_ok']==True"

# 2. task list
check_get "GET /api/tasks" "$BASE/api/tasks" 200 \
  "import sys,json; d=json.load(sys.stdin); assert 'tasks' in d"

# 3. task detail — 404 (200 covered by P4.2 integration tests)
check_get "GET /api/tasks/:run_id (404)" "$BASE/api/tasks/no-such-run" 404 \
  "import sys,json; d=json.load(sys.stdin); assert 'not found' in d['error']"

# 4. config models — no sensitive fields
check_get "GET /api/config/models" "$BASE/api/config/models" 200 \
  "import sys,json; d=json.load(sys.stdin); m=d['models'][0]; \
   assert m['id']=='local-model'; assert m['provider']=='test_prov'; assert m['type']=='openai_compat'; \
   raw=json.dumps(m); assert 'api_key_env' not in raw; assert 'base_url' not in raw; assert 'model_id' not in raw"

# 5. validate — reachable via mock
printf "  %-40s " "POST /api/models/validate"
status=$(curl -sS -o /tmp/smoke_body.txt -w "%{http_code}" \
  -X POST "$BASE/api/models/validate" \
  -H "Content-Type: application/json" \
  -d '{"model_id":"local-model"}')
if [ "$status" != "200" ]; then
  echo "FAIL (status=$status)"
  failures=$((failures + 1))
else
  python3 -c "
import json
d=json.load(open('/tmp/smoke_body.txt'))
assert d['reachable']==True, f'expected reachable:true, got {d}'
assert d['model_id']=='local-model'
assert d['latency_ms'] is not None
" && echo "OK" || { echo "FAIL (body)"; failures=$((failures + 1)); }
fi

# ── results ────────────────────────────────────────────────────────────
echo ""
if [ "$failures" -eq 0 ]; then
  echo "=== Phase 4 smoke: ALL PASSED ==="
else
  echo "=== Phase 4 smoke: $failures FAILED ==="
  exit 1
fi

# ── daedalus-desktop build ─────────────────────────────────────────────
echo ""
echo "building daedalus-desktop..."
cd "$PROJECT_DIR/daedalus-desktop"
npm run build 2>&1 | tail -2
echo "daedalus-desktop build OK"

echo ""
echo "=== Phase 4 acceptance complete ==="
