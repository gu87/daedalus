# Daedalus — Agent OS

Phase 5: Rust daemon (`daedalusd`) + Electron desktop (`daedalus-desktop`) + Python CLI (`daedalus`).

## Phase 3–5 capabilities

### Phase 3 — Gate & Quality System
- **ErrorCode taxonomy** — 10 variants: Cancelled, TaskTimeout, ToolFailure, MaxIterations, ProviderExhausted, ProviderFatal, ModelNotFound, AuthFailure, RateLimited, Unknown
- **Gate routing** — `CriteriaRegistry` + `GateRouter` with YAML override (`gate-criteria.yaml`)
- **AutoRevision** — retry loop with build-before-insert, feedback injection, global retry cap
- **ProviderError granularity** — `AgentError::error_code()` single source of truth, preserved through `LoopState::Failed`
- **SemanticTag** — 6 tags (Permanent/Transient/NeedsHuman/PermissionDenied/ConfigurationError/ResourceExhausted), AND matching in Gate criteria
- **SwitchAgent** — cross-agent dispatch sharing the same retry loop as AutoRevision

### Phase 5 — DAEDALUS.md, Durable Execution & Pipeline
- **DAEDALUS.md** — project-level agent instructions injected into system prompt (`[soul] → [daedalus] → [memory]`)
- **Durable Execution** — `system.ack` / event ledger (`events` table) / `session.rejoin` replay
- **Pipeline** — `TaskStatus` 9-state lifecycle + `tasks` table + daemon event wiring (12 update points)
- **daedalus-desktop** — Electron + React UI skeleton (mock data, no real IPC yet)

### Phase 4 — HTTP API & Observability
- **`GET /api/health`** — daemon status, uptime, DB connectivity
- **`GET /api/tasks`** — task list with status/agent filtering, limit 1..=100
- **`GET /api/tasks/:run_id`** — single task detail
- **`GET /api/config/models`** — model summary (id/provider/type only, no secrets)
- **`POST /api/models/validate`** — single-model connectivity probe (Router production path, 10s timeout)
- **daedalus-desktop** — Electron + React + TypeScript UI skeleton (mock data, zero backend)

## Quick start

```bash
# Build
cargo build

# Start the daemon
./target/debug/daedalusd

# Ping it
daedalus ping
# → {"type":"system.pong","ts":"...","req_id":"..."}

# HTTP API (daemon must be running)
curl http://127.0.0.1:9800/api/health

# Stop
kill $(pgrep daedalusd)
```

Custom paths:

```bash
DAEDALUSD_SOCK=/tmp/test.sock \
DAEDALUSD_STATE_DIR=/tmp/test-state \
DAEDALUSD_HTTP_ADDR=127.0.0.1:9800 \
  ./target/debug/daedalusd &

daedalus ping --socket /tmp/test.sock
```

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `DAEDALUSD_SOCK` | `/tmp/daedalusd.sock` | Unix domain socket path |
| `DAEDALUSD_STATE_DIR` | `~/.daedalus/state` | SQLite database directory |
| `DAEDALUSD_HTTP_ADDR` | `127.0.0.1:9800` | HTTP management API listen address (loopback only) |
| `DAEDALUS_SOUL_PATH` | `~/.daedalus/SOUL.md` | System identity prompt |
| `DAEDALUS_MANAGED_AGENTS_PATH` | `~/.daedalus/config/managed-agents.yaml` | Agent definitions |
| `DAEDALUS_SKILLS_DIR` | `~/.hermes/skills` | Skill markdown files |
| `DAEDALUS_MODELS_YAML` | `~/.daedalus/models.yaml` | Model pool config |
| `DAEDALUS_GATE_CRITERIA_PATH` | `~/.daedalus/config/gate-criteria.yaml` | Gate routing override |
| `DAEDALUS_MD_PATH` | `./DAEDALUS.md` | Project-level agent instructions |

## Development verification

```bash
# Rust
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings

# Python
PYTHONPATH=daedalus-orch pytest daedalus-orch/tests/test_client.py

# Phase 2 end-to-end smoke
scripts/smoke-phase2.sh

# Phase 4 HTTP API smoke (requires Python 3)
scripts/smoke-phase4.sh

# Phase 5 smoke
scripts/smoke-phase5.sh

# daedalus-desktop
cd daedalus-desktop && npm run build
```

## Architecture

```
 daedalusd (Rust)                           daedalus-desktop (Electron+React)
      │                                           │
      │  NDJSON over UDS                HTTP API  │  (future IPC bridge)
      │  /tmp/daedalusd.sock        127.0.0.1:9800│
      └───────────────────────────────────────────┘

 ┌────────────────── daedalusd ──────────────────┐
 │  ipc/server.rs  ── accept loop                │
 │  ipc/peer.rs    ── bidirectional Session       │
 │  ipc/control.rs ── message routing             │
 │  ipc/session.rs ── SessionState, pending perms │
 │  daemon.rs      ── DaemonContext, factory      │
 │                                                │
 │  agent/loop.rs      ── 9-state machine         │
 │  agent/state.rs     ── LoopState enum          │
 │  agent/prompt.rs    ── PromptBuilder            │
 │  agent/permission.rs── PermissionBroker         │
 │  agent/heartbeat.rs ── HeartbeatLoop            │
 │                                                │
 │  llm/mod.rs        ── LLMProvider trait        │
 │  llm/anthropic.rs  ── Anthropic adapter        │
 │  llm/openai_compat.rs ── OpenAI Compat adapter │
 │  llm/router.rs     ── fallback chain           │
 │  llm/sse.rs        ── SSE byte decoder         │
 │                                                │
 │  tools/mod.rs      ── Tool trait               │
 │  tools/registry.rs ── ToolRegistry              │
 │  tools/file_read.rs / file_write.rs            │
 │  tools/terminal.rs / task_done.rs              │
 │                                                │
 │  db/pool.rs        ── SQLite connection        │
 │  db/migrations.rs  ── schema v1                │
 │  db/registry.rs    ── agent_runs CRUD           │
 │  db/orphan.rs      ── orphan scanner            │
 │                                                │
 │  gate.rs           ── GateRouter, SemanticTags │
 │  error.rs          ── DaedalusError, AgentError,│
 │                       ErrorCode                │
 │                                                │
 │  http/             ── axum HTTP server          │
 │  http/health.rs    ── GET /api/health           │
 │  http/tasks.rs     ── GET /api/tasks            │
 │  http/config.rs    ── GET /api/config/models    │
 │  http/validate.rs  ── POST /api/models/validate │
 │                                                │
 │  types.rs          ── Message enum, TaskCard,   │
 │                       Outbox, ToolDef, etc.     │
 │  config.rs         ── DaedalusConfig, models.yaml│
 └────────────────────────────────────────────────┘
```

## What Phase 5 does NOT include

- Ω-Agent / MCP Bridge / cross-task DAG → Phase 5+
- Pipeline execution engine
- WebSocket / SSE real-time push
- Task cancel / retry API
- daedalus-desktop real IPC / backend integration
- Credential hot-reload / Gemini / Cohere providers → Phase 5+
