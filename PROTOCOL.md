# Daedalus HTTP API Protocol v1

Daedalus is a Rust agent runtime daemon. It accepts tasks, executes them via AgentLoop, persists state in SQLite, and returns results.

## Design principle

Daedalus is the engine, not the car. It handles "receive task → execute safely → persist state → return result". It does NOT decide which agent to dispatch to — that's the orchestrator's job.

## Endpoints

### `POST /api/tasks` — create a new task

Request:
```json
{
  "agent_id": "default-worker",
  "goal": "Write a hello.txt file",
  "context": "(optional extra context)"
}
```

Response (201):
```json
{
  "run_id": "run-xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
  "task_id": "task-xxxxxxxxxxxx",
  "status": "queued"
}
```

Errors:
- 400: empty agent_id or goal
- 500: internal error

### `GET /api/tasks/:run_id` — query task status

Response:
```json
{
  "run_id": "run-xxx",
  "agent_id": "default-worker",
  "task_id": "task-xxx",
  "status": "waiting_for_verification",
  "spawned_at": 1700000000,
  "completed_at": 1700000300,
  "error_taxonomy": null
}
```

Status values: `queued`, `running`, `waiting_for_verification`, `done`, `failed`, `cancelled`, `blocked`, `dispatched`, `orphaned`.

### `GET /api/tasks?status=&agent_id=&limit=` — list tasks

- `status`: filter by status (optional)
- `agent_id`: filter by agent (optional)
- `limit`: max results, 1-100 (default 20)

### `GET /api/health` — daemon health

Returns uptime, DB connectivity, socket path. No auth.

## What Daedalus does NOT provide

- SSE / WebSocket streaming
- Permission endpoint (handled by IPC)
- Agent orchestration (which agent to dispatch)
- External agent subprocess tool (Phase 2)
- Feishu / webhook integration
- Client SDKs

## Non-goals

- Multi-agent scheduling
- Built-in Obsidian memory
- Plugin marketplace
- Desktop UI
