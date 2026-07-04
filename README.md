# Daedalus — Agent Runtime

Daedalus 是一个跑在你自己电脑上的 AI 任务执行器。你可以把它理解成“本地 Agent 后台”：ChatGPT、飞书 Bot、CLI 或其他工具把任务交给它，它负责在本机项目里调用模型、读写文件、执行命令、记录状态，并把结果结构化返回。它不追求做一个完整聊天应用，而是专注做一件事：把外部入口发来的任务，安全、可追踪地在本地跑完。

一个用 Rust 从零实现的本地 Agent 运行时。包含 9 状态 Agent Loop、NDJSON over UDS IPC、SQLite 持久化、Gate 错误路由、HTTP API。

> **项目状态：已完成，不再活跃开发。**
>
> 本项目为 Agent Runtime 架构的工程实践。从协议设计到状态机实现，从 LLM Provider 抽象到工具系统，覆盖了一个自托管 Agent 后台该有的核心模块。代码保留作为参考实现；如果你想 fork 在此基础上继续做，欢迎。

代码规模：Rust daemon ~13K 行（248 个测试通过），Python CLI ~1.9K 行。

## 组件

| 组件 | 语言 | 角色 |
|------|------|------|
| `daedalusd` | Rust | 后台守护进程：Agent Loop、LLM Provider、Tool Registry、SQLite、Gate、HTTP/UDS 服务 |
| `daedalus-orch` | Python | CLI 客户端（`daedalus ping` / `daedalus run`） |

## 架构

```
 daedalusd (Rust)
      │                                           │
      │  NDJSON over UDS                HTTP API  │  health/tasks/history
      │  /tmp/daedalusd.sock        127.0.0.1:9800│  UDS dispatch/permission
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

## 实现的核心能力

### Agent Loop — 9 状态状态机

`BuildingPrompt → SendingToLLM → ReceivingStream → ExecutingTool → AwaitingPermission → BuildingResponse → Done / Failed`。用 Rust enum + match 表达，编译器强制穷举所有路径。`CancellationToken` 贯穿所有 I/O，支持超时和手动取消。

### IPC 协议 — NDJSON over UDS

Rust 与 Python 的唯一边界。serde enum dispatch（`#[serde(tag = "type")]`），两阶段解析，req_id/ts 校验。10 种消息类型：`system.ping/pong/error`、`task.dispatch/stream/done/error`、`permission.request/response`、`session.rejoin`、`system.ack`。

### LLM Provider — 双后端 + fallback 链

Anthropic 和 OpenAI Compat 两个 adapter，共享 `LLMProvider` trait。`Router` 按 `model_strategy` 走 fallback 链，支持流式 SSE 解析。

### Tool Registry — 4 个内置工具

`file_read` / `file_write` / `terminal` / `task_done`。每个工具声明 `allowed_agents`，按 Agent 隔离权限。

### Gate — 错误路由 + 自动重试

`ErrorCode` 10 变体 + `SemanticTag` 6 标签 + `GateRouter` 决定每个错误该 retry / switch agent / fail。`AutoRevision` 在 retry 前注入失败反馈，让 LLM 改变策略。

### 持久化 — SQLite + 生命周期追踪

`tasks` 表 9 状态生命周期（queued → running → done/error/cancelled）。`HeartbeatLoop` 周期写心跳，`orphan.rs` 扫描失联任务。`events` 表 + `system.ack` + `session.rejoin` 提供 durable execution。

### HTTP API — 管理面

- `GET /api/health` — daemon 状态、uptime、DB 连通性
- `GET /api/tasks` — 任务列表（状态/agent 过滤，limit 1..=100）
- `GET /api/tasks/:run_id` — 单任务详情
- `GET /api/config/models` — 模型摘要（不含密钥）
- `POST /api/models/validate` — 单模型连通性探测

### Run Transcript + Hooks

每个 task 在 `~/.daedalus/runs/<run_id>/` 下生成：

- `transcript.jsonl` — append-only，记录 `task.started` / `task.done` / `task.error`
- `summary.md` — Markdown 摘要（run_id、goal、status、result）
- `artifacts/` — 预留给大输出

Hooks 默认禁用。在 `~/.daedalus/config/daemon.yaml` 配置后，task 完成/失败时调用 shell 脚本（30s 超时，失败不阻塞主任务）。示例：[examples/hooks/obsidian-inbox-task-done.sh](examples/hooks/obsidian-inbox-task-done.sh)。

## 运行

### 前置

```bash
# 确保这些文件存在且配置正确：
#   ~/.daedalus/SOUL.md                          — Agent 身份
#   ~/.daedalus/config/managed-agents.yaml       — Agent 定义（含 default-worker agent）
#   ~/.daedalus/models.yaml                       — 模型池（含 api_key_env 引用）
# 环境变量：
#   export ANTHROPIC_API_KEY=sk-ant-...
#   # 或 openai_compat:
#   export DEEPSEEK_API_KEY=sk-...
```

### Python CLI

```bash
python3 -m pip install -e daedalus-orch

# daemon 必须单独在跑
daedalus run "echo hello"
daedalus run "目标" --agent default-worker --timeout 300

# 或从 daemon health 拿 socket：
SOCK=$(curl -sf http://127.0.0.1:9800/api/health | python3 -c "import sys,json; print(json.load(sys.stdin)['socket_path'])")
daedalus run "echo hello" --socket "$SOCK"
```

### (removed: frontend dev section)

```bash
# frontend dev removed
```

### 最小验证

```bash
# 构建
cargo build

# 启动 daemon
./target/debug/daedalusd

# ping
daedalus ping
# → {"type":"system.pong","ts":"...","req_id":"..."}

# HTTP API
curl http://127.0.0.1:9800/api/health

# 停止
kill $(pgrep daedalusd)
```

### 自定义路径

```bash
DAEDALUSD_SOCK=/tmp/test.sock \
DAEDALUSD_STATE_DIR=/tmp/test-state \
DAEDALUSD_HTTP_ADDR=127.0.0.1:9800 \
  ./target/debug/daedalusd &

daedalus ping --socket /tmp/test.sock
```

## 环境变量

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
| `DAEDALUS_RUNS_DIR` | `~/.daedalus/runs` | Root directory for run artifacts |
| `DAEDALUS_DAEMON_CONFIG_PATH` | `~/.daedalus/config/daemon.yaml` | Optional runtime config for `runs_dir` and hooks |

## 开发验证

```bash
# Rust
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace            # 248 passed
cargo clippy --workspace -- -D warnings

# Python
PYTHONPATH=daedalus-orch pytest daedalus-orch/tests/test_client.py

# 端到端 smoke
scripts/smoke-phase2.sh
# smoke-phase4 removed
scripts/smoke-phase5.sh

# frontend build removed
```

## 没有做的事

- Ω-Agent / MCP Bridge / cross-task DAG
- Pipeline execution engine
- WebSocket / SSE real-time push
- Task cancel / retry API
- Credential hot-reload / Gemini / Cohere providers
- Remote access / authentication / HTTPS
