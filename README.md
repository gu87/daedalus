# Daedalus — Agent OS

Phase 1 skeleton: Rust daemon (`daedalusd`) + Python CLI (`daedalus`),
communicating via NDJSON over a Unix domain socket.

## Phase 1 capabilities

- `daedalusd` — tokio-based UDS server with NDJSON protocol parsing
- `daedalus ping` — Python CLI sends `system.ping`, receives `system.pong`
- SQLite `agent_runs` table (schema v1, migration runner, minimal CRUD)
- Protocol: three message types (`system.ping`, `system.pong`, `system.error`)
- Two-stage NDJSON parsing with `malformed_json`/`missing_type`/
  `unknown_message_type`/`invalid_message` error codes
- Incremental line-length enforcement (1 MiB cap)
- Graceful shutdown with socket cleanup

## Quick start

```bash
# Build the Rust daemon
cargo build

# Start the daemon (detached or in another terminal)
./target/debug/daedalusd

# Ping it from Python
daedalus ping
# → {"type":"system.pong","ts":"...","req_id":"..."}

# Stop the daemon
kill $(pgrep daedalusd)   # or Ctrl-C
```

Custom paths (for testing):

```bash
# Full test isolation:
DAEDALUSD_SOCK=/tmp/test.sock \
DAEDALUSD_STATE_DIR=/tmp/test-state \
  ./target/debug/daedalusd &

daedalus ping --socket /tmp/test.sock
```

Environment variables:

| Variable | Default | Purpose |
|----------|---------|---------|
| `DAEDALUSD_SOCK` | `/tmp/daedalusd.sock` | Unix domain socket path |
| `DAEDALUSD_STATE_DIR` | `~/.daedalus/state` | SQLite database directory |

The daemon creates `$DAEDALUSD_STATE_DIR` on startup and initialises
`daedalusd.sqlite` (schema v1, `agent_runs` table) before accepting
connections.

## Development verification

```bash
# Rust
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings

# Python
pytest daedalus-orch/tests/test_client.py

# End-to-end smoke
scripts/smoke-phase1.sh
```

## Architecture

```
daedalusd (Rust)          daedalus-orch (Python)
     │                          │
     │  NDJSON over UDS         │
     │  /tmp/daedalusd.sock     │
     └──────────────────────────┘
```

### Rust crate (`daedalusd/`)

| Module | Purpose |
|--------|---------|
| `ipc/protocol.rs` | NDJSON encode/decode, two-stage parsing |
| `ipc/server.rs` | UDS accept loop, lifecycle, socket cleanup |
| `ipc/peer.rs` | Connection-level read/write, line-length cap |
| `ipc/control.rs` | Message routing (ping → pong) |
| `db/pool.rs` | SQLite connection + PRAGMAs |
| `db/migrations.rs` | Transactional schema migrations |
| `db/registry.rs` | `agent_runs` CRUD |

### Python package (`daedalus-orch/`)

| Module | Purpose |
|--------|---------|
| `client.py` | Async UDS client |
| `exceptions.py` | Error hierarchy |
| `cli.py` | `daedalus ping` command |

## What Phase 1 does NOT include

- Provider Layer (Anthropic, OpenAI, etc.)
- Agent Tool-use Loop state machine
- Tool Registry / built-in tools / MCP bridge
- Permission relay (`permission.request`/`permission.response`)
- Heartbeat monitoring / timeout detection / orphan recovery
- LangGraph Pipeline runner / StateGraph
- Skill Loader / Registry Manager / Ω-Agent
- Feishu Channel Plugin / OutboundChannel
- Transport abstraction (TCP, mTLS)
- HTTP API / Web Dashboard
- Any of the five core traits from §12.12

## Protocol & extension boundaries

The Message Bus uses NDJSON over UDS (§4 of the blueprint). All messages
carry RFC 3339 timestamps and an optional `req_id` for request/response
correlation.  Unknown message types receive `system.error` responses.

Phase 2+ will add `task.dispatch`, `task.stream`, `task.done`,
`permission.request/response`, `system.heartbeat`, and pipeline messages.
The protocol enum is extended by adding variants to `Message` (Rust) and
corresponding validation in `client.py` (Python).
