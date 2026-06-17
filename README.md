# Daedalus — Agent OS

Phase 2 skeleton: Rust daemon (`daedalusd`) + Python CLI (`daedalus`),
communicating via NDJSON over a Unix domain socket. Provider Layer,
Tool-use Loop, Tool Registry, permission relay, heartbeat/orphan, and
full `task.dispatch → task.done` lifecycle.

## Phase 2 capabilities

- `daedalusd` — tokio UDS server, Agent Loop state machine, SQLite lifecycle
- **Provider Layer** — `LLMProvider` trait, Anthropic + OpenAI Compat adapters,
  SSE streaming, Router with fallback chain, `models.yaml` config
- **Agent Loop** — 9-state `LoopState` machine (`BuildingPrompt` → `SendingToLLM`
  → `ReceivingStream` → `ExecutingTool` → `AwaitingPermission` → `BuildingResponse`
  → `Done`/`Failed`), `CancellationToken`-based timeout/cancel, 30-iteration cap
- **Tool Registry** — `Tool` trait (`definition`, `risk_level`, `allowed_agents`,
  `needs_permission`, `validate`, `execute`), four built-in tools (`file_read`,
  `file_write`, `terminal`, `task_done`)
- **Permission Broker** — `PermissionBroker` trait, `IpcPermissionBroker` with
  `permission.request`/`permission.response` over UDS, session-scoped pending map,
  default timeout → denied
- **Bidirectional IPC Session** — bounded writer channel (cap 64), daemon-initiated
  push for `permission.request`/`task.done`/`task.error`
- **Heartbeat + Orphan** — `HeartbeatLoop` (30 s interval, `spawn_blocking` short
  connections), `scan_orphans` (90 s cutoff, 60 s scan interval)
- **DB Lifecycle** — `agent_runs`: `queued → running → done/error/cancelled/orphaned`,
  all transitions with `WHERE status = <expected>` guards
- **Full dispatch chain** — `task.dispatch → DaemonContext::spawn_task → AgentLoop::
  run_with_lifecycle → task.done/task.error` over UDS
- Python `dispatch()` reads `permission.request`/`task.stream`/`task.done`/`task.error`
  in a loop, returns `{done, streams}`
- Phase 1 protocol retained: `system.ping`/`system.pong`/`system.error`

## Quick start

```bash
# Build
cargo build

# Start the daemon
./target/debug/daedalusd

# Ping it
daedalus ping
# → {"type":"system.pong","ts":"...","req_id":"..."}

# Stop
kill $(pgrep daedalusd)
```

Custom paths:

```bash
DAEDALUSD_SOCK=/tmp/test.sock \
DAEDALUSD_STATE_DIR=/tmp/test-state \
  ./target/debug/daedalusd &

daedalus ping --socket /tmp/test.sock
```

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `DAEDALUSD_SOCK` | `/tmp/daedalusd.sock` | Unix domain socket path |
| `DAEDALUSD_STATE_DIR` | `~/.daedalus/state` | SQLite database directory |
| `DAEDALUS_SOUL_PATH` | `~/.daedalus/SOUL.md` | System identity prompt |
| `DAEDALUS_MANAGED_AGENTS_PATH` | `~/.daedalus/config/managed-agents.yaml` | Agent definitions |
| `DAEDALUS_SKILLS_DIR` | `~/.hermes/skills` | Skill markdown files |
| `DAEDALUS_MODELS_YAML` | `~/.daedalus/models.yaml` | Model pool config |

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
```

## Architecture

```
 daedalusd (Rust)                           daedalus-orch (Python)
      │                                           │
      │  NDJSON over UDS                          │
      │  /tmp/daedalusd.sock                      │
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
 │  types.rs          ── Message enum, TaskCard,   │
 │                       Outbox, ToolDef, etc.     │
 │  config.rs         ── DaedalusConfig, models.yaml│
 │  error.rs          ── DaedalusError, AgentError │
 └────────────────────────────────────────────────┘
```

## What Phase 2 does NOT include

- MEMORY.md / USER.md / feedback-memory / project-context injection → Phase 3
- Gate / CriteriaRegistry / SemanticCheck / GateRouter → Phase 3
- ErrorCode enum → Phase 3
- TaskStatus (9 states) / pipeline.sqlite → Phase 3
- `system.ack` / ledger / event replay / disk queue → Phase 3
- `code_search` / `send_message` tools → Phase 3
- models.yaml hot-reload / HTTP API / cc-haha dashboard → Phase 4
- DAEDALUS.md / Omega / performance tuning → Phase 4+
- External CLI Agent adapter → Phase 4
- Transport abstraction (TCP, mTLS) → Phase 4+
