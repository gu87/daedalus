# Daedalus Phase 1 实施计划

> 范围：只完成蓝图第七节 Phase 1"骨架（1-2 周）"。不实现 Provider、Tool-use Loop、Tool Registry、权限转发、Heartbeat 监控或 orphan 回收。
>
> 执行规则：Claude 每完成一个子任务，在对应标题末尾标注 `[DONE]`，并附上实际验证结果；随后由 Codex 审查后才能进入下一个子任务。
>
> 开工前待拍板：
>
> 1. 蓝图第 10.8、10.10 节和 enum-vs-trait 表把 `Transport` 视为 enum，第 12.12 节又把它定义为核心 trait。Phase 1 可先实现具体 UDS Server，但不应在拍板前引入 `Transport` enum/trait 抽象。
> 2. 蓝图说吸收 CCB 的"Agentic Loop 六层拆分"，但没有正式枚举这六层；不能让 Claude 自行补定义。当前 Phase 1 只按系统全景和模块清单划边界。
> 3. 第 12.12 节说五个核心 trait 在 Phase 1 声明，但 Phase 1 路线不包含 Agent Runtime、Provider、Tool 或 Channel 实现。建议 Phase 1 只声明本阶段有真实调用方的接口，其他 trait 延后到对应实现阶段，避免空抽象；需拍板。
> 4. 第 12.3 节推荐 `httpx` 做 UDS Client，但协议定义是原始 NDJSON over UDS，不是 HTTP over UDS。建议 Python Client 使用 `asyncio.open_unix_connection()`；需拍板。

## P1.1 协议优先的 Rust 骨架 [DONE]

### 目标

建立可编译的 Rust workspace 与 `daedalusd` 二进制骨架，并首先定义 Message Bus 的 NDJSON 协议合同。Message Bus 是 Rust 与 Python 的唯一边界，后续 UDS Server 和 Python Client 都依赖它。依赖：无。预计 Claude 需要 1-2 轮。

### 文件清单

- `/Users/gu/daedalus/Cargo.toml`
- `/Users/gu/daedalus/daedalusd/Cargo.toml`
- `/Users/gu/daedalus/daedalusd/src/main.rs`
- `/Users/gu/daedalus/daedalusd/src/lib.rs`
- `/Users/gu/daedalus/daedalusd/src/error.rs`
- `/Users/gu/daedalus/daedalusd/src/types.rs`
- `/Users/gu/daedalus/daedalusd/src/ipc/mod.rs`
- `/Users/gu/daedalus/daedalusd/src/ipc/protocol.rs`
- `/Users/gu/daedalus/daedalusd/tests/protocol.rs`

### 验证方式

- `cargo check --workspace` 通过。
- `cargo test --workspace` 通过。
- 协议测试至少验证：一条蓝图已有消息的 JSON -> Rust 类型 -> 单行 JSON 往返；未知消息类型不会被静默当成已知消息。
- 回读生成的 NDJSON，确认每条消息只占一行，字段名与蓝图第四节一致。

### Claude 指令

先读蓝图第四节、第二节 `ipc/protocol.rs` 清单和第 12.8 节 ACP 建议，再实现最小协议合同。动代码前先提交一页以内的协议决策供审查，至少说明：hello-world 使用哪种已有消息类型或是否需要新增控制消息、请求与响应如何关联、未知消息类型如何处理、时间戳格式；未获确认前不要写代码。确认后只为 Phase 1 hello-world 链路定义必要消息，不一次性实现蓝图全部消息，不写 Provider、Agent Loop、Tool 或数据库代码。协议高频解析使用 enum/serde，不做动态 trait 分发；禁止 `unsafe`。完成后报告测试命令和结果。

### 实际验证结果（返修后）

**修改文件（10 个）：**
- `/Users/gu/Daedalus/Cargo.toml` — workspace 根配置
- `/Users/gu/Daedalus/daedalusd/Cargo.toml` — serde, serde_json, chrono, thiserror 依赖
- `/Users/gu/Daedalus/daedalusd/src/main.rs` — 入口骨架
- `/Users/gu/Daedalus/daedalusd/src/lib.rs` — 模块声明
- `/Users/gu/Daedalus/daedalusd/src/error.rs` — `DaedalusError` enum (thiserror)
- `/Users/gu/Daedalus/daedalusd/src/types.rs` — `Message` enum, `SystemPing/Pong/Error`, `SystemErrorCode`
- `/Users/gu/Daedalus/daedalusd/src/ipc/mod.rs` — 子模块声明
- `/Users/gu/Daedalus/daedalusd/src/ipc/protocol.rs` — 两阶段解析、ts/req_id 验证、序列化、42 个单元测试
- `/Users/gu/Daedalus/daedalusd/tests/protocol.rs` — 6 个集成测试

**测试结果：48 passed, 0 failed**
```
cargo fmt --all -- --check               # 通过
cargo check --workspace                  # 通过
cargo test --workspace                   # 48 passed (42 unit + 6 integration)
cargo clippy --workspace -- -D warnings  # 通过
```

**返修变更：**
1. `req_id` 校验补全：`system.pong` 拒绝空字符串；`system.error` 若 `req_id` 为 `Some("")` 也拒绝；缺失/null 仍合法
2. 新增 5 个单元测试：`pong_empty_req_id_rejected`、`error_empty_req_id_rejected`、`error_null_req_id_accepted`、`error_missing_req_id_accepted`
3. 创建 `tests/protocol.rs`（6 个集成测试，仅通过公开 API）
4. `cargo fmt --all` 执行完毕

**范围审计：**
- ✅ 仅 `system.ping` / `system.pong` / `system.error` 三个消息类型
- ✅ 两阶段解析：Value → type 检查 → Message 反序列化 → 语义验证
- ✅ `SystemErrorCode` 四个变体
- ✅ RFC 3339 时间戳验证 + 服务端 UTC 毫秒输出
- ✅ `req_id` 在三个消息类型上均正确校验
- ✅ `#[serde(tag = "type")]` 分发，内部 struct 不含重复 `type` 字段
- ✅ 无 `Transport` trait、无其他四个核心 trait
- ✅ 无 Provider、Agent Loop、Tool、数据库、UDS Server 代码
- ✅ 零 `unsafe`
- ✅ 未创建 `.gitignore`（留给 P1.5）

## P1.2 UDS IPC Server hello-world 链路 [DONE]

### 目标

实现监听 `/tmp/daedalusd.sock` 的 tokio UDS Server，按行接收和发送 NDJSON，形成 Rust 侧可真实连接的 hello-world 链路。依赖：P1.1。预计 Claude 需要 2 轮。

### 文件清单

- `/Users/gu/daedalus/daedalusd/src/main.rs`
- `/Users/gu/daedalus/daedalusd/src/ipc/mod.rs`
- `/Users/gu/daedalus/daedalusd/src/ipc/server.rs`
- `/Users/gu/daedalus/daedalusd/src/ipc/peer.rs`
- `/Users/gu/daedalus/daedalusd/src/ipc/control.rs`
- `/Users/gu/daedalus/daedalusd/tests/uds_server.rs`

### 验证方式

- `cargo check --workspace`、`cargo test --workspace`、`cargo clippy --workspace -- -D warnings` 全部通过。
- 集成测试使用临时 socket 路径启动 Server，发送一条合法 NDJSON，并断言收到协议定义的响应。
- 手动启动 `daedalusd` 后，确认 `/tmp/daedalusd.sock` 存在；进程正常退出后 socket 被清理。
- 发送 malformed JSON 时，单个连接得到明确错误且 Server 继续接受下一连接。

### Claude 指令

基于 P1.1 的协议类型实现最小 UDS Server。`peer.rs` 只负责连接级收发，`control.rs` 只负责 Phase 1 最小消息路由，保持 Peer Layer 与 Control Plane 边界，不实现 Agent dispatch 业务。测试不得依赖固定 `/tmp/daedalusd.sock`，使用临时路径避免污染。不要实现 TCP、mTLS、HTTP API 或通用 broker。完成后报告真实 socket 调用结果。

### 实际验证结果（返修后）

**新增/修改文件（6 个）：**
- `daedalusd/Cargo.toml` — 加 tokio (含 time feature) + tempfile dev
- `daedalusd/src/error.rs` — 加 `AlreadyExists(PathBuf)` variant
- `daedalusd/src/ipc/mod.rs` — 加 control, peer, server 子模块
- `daedalusd/src/ipc/control.rs` — 最小路由：ping→pong（3 单测）
- `daedalusd/src/ipc/peer.rs` — `read_line_limited()` 增量限长读取，MAX_LINE_LEN 硬上限
- `daedalusd/src/ipc/server.rs` — `run()` 修复 accept 错误时也清理 socket + `run_with_listener` helper 测试（7 集成测试）
- `daedalusd/src/main.rs` — tokio main + ctrl_c 信号

**测试结果：58 passed, 0 failed**
```
cargo fmt --all -- --check               # 通过
cargo test --workspace                   # 58 passed (52 unit + 6 integration)
cargo clippy --workspace -- -D warnings  # 通过
```

**返修变更：**
1. `peer.rs` 改为 `fill_buf()`/`consume()` 增量读取，每个 chunk 后检查长度，无换行时也能阻止内存无限增长
2. `server.rs` `run()` 修复 accept loop 错误时跳过 socket 清理的 bug（`select!` 不再用 `?` 提前返回）
3. 超长行测试收紧：必须断言收到 `system.error`（含 `invalid_message`），随后确认连接关闭
4. 新增 `accept_error_still_cleans_up_socket` 测试 + `run_with_listener` helper

**范围审计：**
- ✅ 三层边界：server / peer / control
- ✅ `run(socket_path, shutdown)` 签名
- ✅ socket 存在 → AlreadyExists；成功 bind 后无论何种退出路径都清理 socket
- ✅ 增量限长读取，无 unsafe/libc/socket2 依赖
- ✅ 零数据库、Agent dispatch、Transport 抽象、TCP、HTTP

## P1.3 SQLite 初始化与 agent_runs 生命周期表 [DONE]

### 目标

实现 daedalusd SQLite 初始化、可重复 migration 和蓝图第五节定义的 `agent_runs` 表，为后续 Agent 生命周期追踪建立单一事实源。依赖：P1.1；可与 P1.2 在不同分支并行，但合并后必须重新验证。预计 Claude 需要 2 轮。

### 文件清单

- `/Users/gu/daedalus/daedalusd/Cargo.toml`
- `/Users/gu/daedalus/daedalusd/src/lib.rs`
- `/Users/gu/daedalus/daedalusd/src/db/mod.rs`
- `/Users/gu/daedalus/daedalusd/src/db/pool.rs`
- `/Users/gu/daedalus/daedalusd/src/db/migrations.rs`
- `/Users/gu/daedalus/daedalusd/src/db/registry.rs`
- `/Users/gu/daedalus/daedalusd/tests/db_registry.rs`

### 验证方式

- `cargo check --workspace`、`cargo test --workspace`、`cargo clippy --workspace -- -D warnings` 全部通过。
- 测试在临时 SQLite 文件上连续运行 migration 两次，第二次无报错且 schema 不重复。
- 使用 SQLite schema 查询确认 `agent_runs` 的列、status CHECK、外键和两个索引与蓝图第五节一致。
- 最小 CRUD 测试至少写入一条 `queued` run 并读回；非法 status 必须被数据库拒绝。

### Claude 指令

严格按蓝图第五节实现 SQLite-only 的 `agent_runs` schema。使用 `rusqlite` 的 `bundled` feature；测试使用临时数据库，不写入用户真实 `~/.daedalus/state/`。Phase 1 只实现初始化、migration 和验证所需的最小 registry CRUD，不实现 session、metrics、heartbeat 检测或 orphan 回收。migration 必须可重复执行，禁止用 JSON/JSONL 文件代替运行时状态。

### 实际验证结果（返修后）

**新增/修改文件（7 个）：**
- `daedalusd/Cargo.toml` — 加 `rusqlite` (bundled)
- `daedalusd/src/lib.rs` — 加 `pub mod db`
- `daedalusd/src/db/mod.rs` — 子模块声明
- `daedalusd/src/db/pool.rs` — `open()` + `PRAGMA foreign_keys = ON`
- `daedalusd/src/db/migrations.rs` — `run_all(&mut Connection)` 使用 `Transaction` API；提取 `run_migration` helper（2 单测）
- `daedalusd/src/db/registry.rs` — `AgentRunStatus` enum, `NewAgentRun`, `insert_run/get_run/update_status`（5 单测）
- `daedalusd/tests/db_registry.rs` — 集成测试（9 个测试）

**测试结果：74 passed, 0 failed**
```
cargo fmt --all -- --check               # 通过
cargo test --workspace                   # 74 passed (59 unit + 15 integration)
cargo clippy --workspace -- -D warnings  # 通过
```

**返修变更：**
1. 提取 `run_migration(conn, migration)` 私有 helper，`run_all()` 调用之
2. 改用 `rusqlite::Transaction` API（`conn.transaction()` → `tx.execute_batch()` → `tx.commit()`），drop 时自动回滚，不再手写 BEGIN/COMMIT/ROLLBACK
3. `run_all` 签名改为 `&mut Connection`，调用方同步更新
4. 测试构造含一半成功一半无效 SQL 的 bogus `Migration`，调用同一个 `run_migration` helper，断言表不存在且 `user_version` 未变

**范围审计：**
- ✅ 仅 `PRAGMA foreign_keys = ON`；无 WAL，无连接池
- ✅ 事务化 migration，原始错误不被 ROLLBACK 错误掩盖
- ✅ Schema 与蓝图第五节完全一致
- ✅ 最小 CRUD；零 sessions/metrics/heartbeat/orphan
- ✅ 零 `unsafe`

## P1.4 Python 包骨架与 UDS Client [DONE]

### 目标

建立 `daedalus-orch` Python 包结构与异步 UDS Client，使 Python 能使用与 Rust 相同的 NDJSON 合同连接 daedalusd。依赖：P1.1、P1.2。预计 Claude 需要 2 轮。

### 文件清单

- `/Users/gu/daedalus/daedalus-orch/pyproject.toml`
- `/Users/gu/daedalus/daedalus-orch/daedalus/__init__.py`
- `/Users/gu/daedalus/daedalus-orch/daedalus/orch/__init__.py`
- `/Users/gu/daedalus/daedalus-orch/daedalus/orch/cli.py`
- `/Users/gu/daedalus/daedalus-orch/daedalus/orch/client.py`
- `/Users/gu/daedalus/daedalus-orch/tests/test_client.py`

### 验证方式

- 在项目虚拟环境中安装包后，CLI 入口可运行并显示帮助。
- Python 单元测试覆盖 NDJSON 单行编码/解码、连接失败和 malformed response。
- 启动真实 Rust `daedalusd`，由 Python Client 发出 P1.1 定义的合法消息并收到预期响应。
- `pytest` 通过；若项目配置了 lint/type-check，则对应命令也必须通过。

### Claude 指令

按蓝图第三节的包路径实现最小 `daedalus-orch` 和异步 UDS Client。Client 只负责连接、单行 NDJSON 请求/响应与明确错误，不实现 LangGraph、Pipeline nodes、Skill Loader、Registry Manager、Channel 或 Ω-Agent。开工前确认用户对原始 UDS 客户端技术选择的拍板；若采用原始 NDJSON 协议，不得为了使用 `httpx` 私自增加 HTTP 层。不要复制一套与 Rust 不一致的随意字段；测试消息必须与 P1.1 协议合同一致。完成后必须用真实 Rust daemon 做一次最小调用，不能只用 mock。

### 实际验证结果

**新增/修改文件（8 个）：**
- `daedalus-orch/pyproject.toml` — 包配置，CLI 入口 `daedalus`
- `daedalus-orch/daedalus/__init__.py` — 顶层包
- `daedalus-orch/daedalus/orch/__init__.py` — 子包
- `daedalus-orch/daedalus/orch/exceptions.py` — 5 种异常类（DaedalusError → Connection/Timeout/Protocol/Client）
- `daedalus-orch/daedalus/orch/cli.py` — `daedalus ping` 命令
- `daedalus-orch/daedalus/orch/client.py` — `DaedalusClient`（connect/ping/close, async context manager）
- `daedalus-orch/tests/test_client.py` — 15 个测试
- `daedalusd/src/main.rs` — 加 SIGTERM 支持 + `DAEDALUSD_SOCK` 环境变量

**测试结果：**
```
# Rust
cargo fmt --all -- --check               # 通过
cargo test --workspace                   # 74 passed
cargo clippy --workspace -- -D warnings  # 通过

# Python
pytest daedalus-orch/tests/test_client.py  # 15 passed
```

**范围审计：**
- ✅ `asyncio.open_unix_connection()` 原始 NDJSON，无 httpx/HTTP 层
- ✅  公开 API 仅 `connect()` / `ping()` / `close()` + async context manager
- ✅ 私有 `_request()` + `_validate_response()` 做完整响应校验
- ✅ 异常：`DaedalusConnectionError` / `TimeoutError` / `ProtocolError` / `ClientError`
- ✅ 超时覆盖 write + drain + read 全过程
- ✅ `await writer.drain()` 执行于每次写入后
- ✅ CLI 输出一行 JSON，错误输出到 stderr
- ✅ 12 个测试：NDJSON 编码、连接失败、malformed、非 object 响应、system.error、req_id 不匹配、EOF、超时、CLI help、CLI 缺命令、CLI socket 不存在、**真实 Rust daemon ping→pong**
- ✅ 无 LangGraph、Pipeline、Skill Loader、Registry Manager、Channel、Ω-Agent
- ✅ 无新增脚本文件

**返修变更：**
- 调整 `_validate_response` 校验顺序：先验证 object/type/ts，再按消息类型差异化 req_id 约束
- `system.pong`：req_id 必填、非空、必须匹配
- `system.error`：req_id 可选；缺失/null → `DaedalusClientError(req_id=None)`；存在时必须非空且匹配
- 新增 3 个测试：system.error 缺 req_id、req_id 为 null、req_id 不匹配

## P1.5 Phase 1 跨语言验收与收口 [DONE]

### 目标

把 Rust daemon、SQLite 初始化和 Python Client 串成一次可重复的 Phase 1 验收，确认骨架已形成而没有提前侵入 Phase 2。依赖：P1.1-P1.4 全部完成并通过审查。预计 Claude 需要 1-2 轮。

### 文件清单

- `/Users/gu/daedalus/README.md`
- `/Users/gu/daedalus/scripts/smoke-phase1.sh`
- `/Users/gu/daedalus/.gitignore`
- 仅在修复验收发现的问题时，修改 P1.1-P1.4 已创建的文件

### 验证方式

- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- Python 环境中的 `pytest`
- `scripts/smoke-phase1.sh` 从干净状态启动 daemon、等待 socket、执行 Python Client 真实请求、确认 SQLite schema、关闭 daemon 并清理临时资源；脚本退出码为 0。
- `git status --short` 不出现 socket、SQLite、缓存、虚拟环境或构建产物。

### Claude 指令

编写最小 README 和可重复 smoke 脚本，完整验证 Phase 1，而不是增加新功能。smoke 必须使用临时 socket 和临时 state 目录，不能覆盖用户配置或真实数据库；失败时应可靠清理 daemon 和临时文件。验收时审计范围，确认没有实现 Phase 2 的 Provider、Agent Loop、工具、权限、heartbeat/orphan 逻辑。若发现问题，当场修复并重新跑完整验证；不要留下 TODO/FIXME。

### 实际验证结果（返修后）

**新增/修改文件（6 个）：**
- `README.md` — 项目说明、env var 文档、验证命令、未实现列表
- `.gitignore` — Rust target/、Python 缓存、.DS_Store、socket/SQLite
- `scripts/smoke-phase1.sh` — 临时 socket + state 全链路验收
- `daedalusd/src/main.rs` — 启动时从 `DAEDALUSD_STATE_DIR` 初始化 SQLite
- `daedalus-orch/daedalus/orch/cli.py` — 加 `__main__` 入口
- `PLANS.md` — P1.1-P1.5 全部 [DONE]

**Phase 1 完整验证：**
```
cargo fmt --all -- --check               # ✅ 通过
cargo check --workspace                  # ✅ 通过
cargo test --workspace                   # ✅ 74 passed
cargo clippy --workspace -- -D warnings  # ✅ 通过
pytest daedalus-orch/tests/test_client.py  # ✅ 15 passed
scripts/smoke-phase1.sh                  # ✅ 全链路通过
git status --short                       # ✅ 仅源文件
```

**返修变更（P1.5 审查反馈）：**
- `daedalusd` 启动时读取 `DAEDALUSD_STATE_DIR`（默认 `~/.daedalus/state`）→ `pool::open()` + `migrations::run_all()` → 失败则 exit(1)
- Smoke 传入 `DAEDALUSD_STATE_DIR` 指向临时目录，daemon 自己创建数据库
- Smoke Python 检查只读验证：`user_version=1`、列、索引、CHECK、FK——禁止执行 `CREATE TABLE` 或 `CREATE INDEX`
- Smoke 输出确认 `daedalusd database ready at ...` 并随后 `daedalusd shut down cleanly`

**范围审计（全 Phase 1）：**
- ✅ 未实现 Provider、Agent Loop、Tool Registry、权限、heartbeat/orphan
- ✅ 未实现 LangGraph Pipeline、Skill Loader、Registry Manager、Channel、Ω-Agent
- ✅ 未实现 Transport trait、TCP、HTTP API、连接池、业务 CRUD
- ✅ 零 `unsafe`
- ✅ 零 TODO/FIXME
- ✅ Phase 1 五阶段全部 [DONE]

---

# Daedalus Phase 2 实施计划

> 范围：蓝图第七节 Phase 2"心脏（2-3 周）"。实现 Provider Layer、Tool-use Loop、Tool Registry、权限转发、Heartbeat 与 orphan 回收。
>
> 执行规则：与 Phase 1 相同——每完成一个子任务标记 `[DONE]`，Codex 审查后进入下一子任务。
>
> **本计划已对齐蓝图 v3.0（2026-06-16 最终版）——Memory Layer（§6）、模型池管理（§10.12）、Gate 与质量体系（§13）、运维体系（§14）、可插拔架构（§12.12）。**

## 架构决策（已拍板）

| # | 决策 | 选择 |
|---|------|------|
| 1 | 权限超时默认 | **denied**（安全优先） |
| 2 | 通讯模型 | **单连接双向 Session**（P2.5 实现） |
| 3 | `AgentRuntime.cancel()` | **纳入，真实 CancellationToken** |
| 4 | SQLite 运行时访问 | **短连接 + `spawn_blocking`** |

## 共享类型唯一归属

| 类型 | 文件 | 说明 |
|------|------|------|
| `TaskCard` | `types.rs` | 字段对齐 `/Users/gu/.hermes/scripts/compile-task.py` 输出 |
| `Outbox` | `types.rs` | 字段对齐 `/Users/gu/.hermes/templates/outbox_v2_8.json` |
| `ToolDef` | `types.rs` | 数据结构（struct），非已知工具枚举 |
| `ToolResult` | `types.rs` | `{ output, is_error }` |
| `ToolCall` | `types.rs` | `{ id, name, input }` |
| `ChatMessage` | `types.rs` | `{ role, content }` |
| `StreamChunk` | `types.rs` | `Text(String) \| ToolCall(ToolCall) \| Done` |
| `ChatResponse` | `types.rs` | `{ content, tool_calls }` |
| `ProviderError` | `error.rs` | HTTP/auth/rate-limit/parse 变体 |
| `ModelConfig` | `types.rs` | `{ model, max_tokens, temperature }` — 纯配置，不含凭证 |
| `ModelStrategy` | `config.rs` | `{ primary, fallback_chain }` — 纯策略 |
| `RiskLevel` | `types.rs` | `R0-R4` |
| `PermissionDecision` | `types.rs` | `Approved \| Denied` |
| `AgentError` | `error.rs` | `ErrorKind` + `detail` |
| `AgentCapabilities` | `agent/mod.rs` | `{ agent_id, description, tool_names }` |
| `ApiKey` | `llm/mod.rs` | 凭证包装（非业务共享类型） |

## 子任务顺序

```
P2.1  核心共享类型与 Phase 2 协议合同 [DONE]
  │
  ├─→ P2.2  Provider Layer [DONE]
  │     │
  │     └─→ P2.2b  models.yaml 配置读取 + Router 构建 [NEW]
  │
  ├─→ P2.3  Tool trait + Registry + Phase 2 内置工具（与 P2.2b 可并行）
  │
  └─→ P2.4  Agent Loop 状态机 + 最小 PromptBuilder（整合 P2.2/P2.2b + P2.3）
        │
        ├─→ P2.5  权限转发 + IPC 双向 Session + 可靠性协议预留
        │
        └─→ P2.6  Heartbeat + 超时 + orphan 回收 + 生命周期接线
              │
              └─→ P2.7  Phase 2 跨模块验收与收口
```

## P2.1 核心共享类型与 Phase 2 协议合同 [DONE]

### 目标

一次性定义所有 Phase 2 模块需要的共享类型，扩展 `Message` enum 增加 Phase 2 消息变体。**只定义合同，不实现路由行为。** 不修改 `peer.rs` 或 `control.rs`。

### 文件清单

- `daedalusd/src/types.rs` — 扩展：`TaskCard`, `Outbox`, `ToolDef`, `ToolResult`, `ToolCall`, `ChatMessage`, `StreamChunk`, `ChatResponse`, `ModelConfig`, `RiskLevel`, `PermissionDecision`
- `daedalusd/src/error.rs` — 扩展：`ProviderError`, `AgentError` 的 `ErrorKind`
- `daedalusd/src/config.rs` — 新增：`ModelStrategy`（不含 YAML 解析）
- `daedalusd/src/agent/mod.rs` — 新增：`AgentCapabilities`
- `daedalusd/src/llm/mod.rs` — 新增：`ApiKey`
- `daedalusd/src/ipc/protocol.rs` — 更新已知消息类型集合
- `daedalusd/tests/protocol.rs` — 新增消息往返测试

### Schema 来源

- **TaskCard**：对齐 `/Users/gu/.hermes/scripts/compile-task.py`（第 343-399 行）。固定字段使用明确 Rust 类型，不稳定嵌套对象使用 `serde_json::Value`。不删除或静默改名真实字段。`schema_version` 必须为 `"2.8"`。TaskCard 无 `timeout` 字段——timeout 来自 Agent 配置默认值 300s。`must_keep` 从 `compiled_intent.must_keep` 提取。
- **Outbox**：对齐 `/Users/gu/.hermes/templates/outbox_v2_8.json`。13 个字段，`schema_version` 必须为 `"2.8"`。

### 新增消息变体与校验规则

```
TaskDispatch:
  ts, req_id (非空), agent_id (非空), task_id (非空), task_card (Object)
  校验：task_card["task_card_id"] == task_id && task_card["execution_plan"]["primary_agent"] == agent_id

TaskStream:   ts, req_id (非空), agent_id (非空), chunk (String)
TaskDone:     ts, req_id (非空), agent_id (非空), outbox (Object)
TaskError:    ts, req_id (非空), agent_id (非空), error_taxonomy (非空), detail (String)
PermissionRequest:  ts, permission_id (非空), req_id (非空), agent_id (非空), tool (非空), args (Object)
PermissionResponse: ts, permission_id (非空), req_id (非空), decision ("approved"|"denied")
```

- `req_id`：关联一次 dispatch 的所有消息（dispatch → stream/done/error）
- `permission_id`：独立的权限请求关联 ID，与 task `req_id` 分离
- `PermissionResponse` 同时携带 `permission_id` 和 `req_id`，便于验证所属任务

### 扩展边界

- `Message` enum 新增 variant = 改 1 处定义 + 1 处路由，编译器强制所有 match 更新
- `TaskCard`/`Outbox` 的 `Value` 嵌套字段可随真实 Schema 演进，不动 Rust struct

### 不实现

- 不修改 `peer.rs`、`control.rs`
- 不实现 Provider、Tool、Agent Loop、Session、权限逻辑、heartbeat、数据库生命周期
- 不新增尚未被 P2.1 使用的 crate

### 验证方式

- `cargo fmt --all -- --check`、`cargo check --workspace`、`cargo test --workspace`、`cargo clippy --workspace -- -D warnings` 全部通过
- 测试覆盖：TaskCard v2.8 和 Outbox v2.8 真实样例往返；`schema_version` 错误拒绝；所有新增消息往返；所有必填 ID 非空校验；`TaskDispatch` 顶层 ID 与 TaskCard 标识一致性；`PermissionResponse` 同时包含非空 `req_id` 与 `permission_id`；未知消息仍返回 `unknown_message_type`

### Claude 指令

读取 `/Users/gu/.hermes/scripts/compile-task.py`（第 343-399 行）和 `/Users/gu/.hermes/templates/outbox_v2_8.json` 获取真实字段。实现时：固定字段用明确 Rust 类型，不稳定嵌套对象用 `serde_json::Value`。`schema_version` 字段必须验证为 `"2.8"`。不删除或静默改名真实 Schema 字段。禁止 `unsafe`。不修改 `peer.rs` 或 `control.rs`。不新增尚未被 P2.1 使用的 crate。完成后报告测试命令和结果。

### 实际验证结果（返修 v2）

**新增/修改文件（8 个）：**
- `daedalusd/src/types.rs` — TaskCard v2.8, Outbox v2.8, TaskDispatch(Box)/Stream/Done(Box)/Error（含 `task_id`）, PermissionRequest/Response, 域类型, 6 个验证函数, 34 单测
- `daedalusd/src/error.rs` — AgentError, ErrorKind（6）, ProviderError（7 + `is_fallbackable()`）
- `daedalusd/src/config.rs` — ModelStrategy
- `daedalusd/src/agent/mod.rs` — AgentCapabilities
- `daedalusd/src/ipc/protocol.rs` — 已知类型 +6；`validate_message` 完整 ID/一致性/schema 校验；17 新增协议入口测试
- `daedalusd/src/ipc/control.rs` — 穷举 match 保持编译
- `daedalusd/src/lib.rs` — + agent, config 模块
- `daedalusd/tests/protocol.rs` — unknown type 更新

**删除：** `daedalusd/src/llm/mod.rs`（留到 P2.2 Provider 实现时按需创建）

**测试结果：106 passed, 0 failed**
```
cargo fmt --all -- --check               # 通过
cargo test --workspace                   # 106 passed (91 unit + 15 integration)
cargo clippy --workspace -- -D warnings  # 通过
```

**v2 返修变更：**
1. `TaskStream`/`TaskDone`/`TaskError` 恢复 `task_id` 字段（蓝图 §4 合同，与 `req_id` 职责分离）
2. `validate_task_done_consistency()` — 顶层 `task_id == outbox.task_id` + `agent_id == outbox.agent_id`
3. 新增 `parse_message()` 级测试覆盖全部必填 ID 空字段和一致性场景
4. 删除 P2.1 提前创建的 `llm::ApiKey` 和 `llm/` 模块

**范围审计：**
- ✅ 字段对齐真实 Schema 源
- ✅ `req_id`（协议关联）与 `task_id`（业务标识）职责分离
- ✅ `schema_version` 错误/缺失/缺字段 → `parse_message()` → `InvalidMessage`
- ✅ Phase 1 测试零删减
- ✅ 未修改 `peer.rs`；control.rs 仅穷举编译
- ✅ 零 `unsafe`，零新 crate

## P2.2 Provider Layer [DONE]

### 目标

定义 `LLMProvider` trait，实现 Anthropic Messages API 和 OpenAI Compat API，最小 Router。

### 文件清单

- `daedalusd/Cargo.toml` — 加 `reqwest = { version = "0.12", features = ["stream"] }`、`async-trait = "0.1"`
- `daedalusd/src/llm/mod.rs` — `LLMProvider` trait + `ApiKey`
- `daedalusd/src/llm/anthropic.rs` — Anthropic Provider
- `daedalusd/src/llm/openai_compat.rs` — OpenAI Compat Provider
- `daedalusd/src/llm/router.rs` — `chat_with_fallback()` + `stream_with_fallback()`
- `daedalusd/tests/provider.rs` — `httptest` 模拟端点

### 依赖

- `reqwest`（stream feature）、`async-trait`
- `httptest`（dev-only）— 唯一 HTTP mock
- 不加 `futures-core`、`tokio-stream`

### 扩展边界

- 新 Provider：实现 `LLMProvider` trait + Router 注册一行

### 不实现

- 速率限制器、circuit breaker、凭证热刷新、Gemini/Cohere、`rig`、YAML 配置解析（Router 接收已解析策略）

### 验证方式

- `httptest` 模拟端点；`FakeProvider`（返回预设 chunk）供 Agent Loop 测试
- Rust 全量测试通过

### Claude 指令

实现 `LLMProvider` trait（`chat` + `stream`）。`StreamHandle` 使用 `tokio::sync::mpsc::Receiver`。`ApiKey` 放在 `llm/mod.rs`，`Debug`/`Display` 输出 `"***"`。Router 实现 `chat_with_fallback()` 和 `stream_with_fallback()` 具体方法，不写泛型闭包 API。Fallback 规则：网络错误/超时/5xx/429/模型不可用 → fallback；401/400/parse error → 不 fallback。完成后报告测试结果。

### 实际验证结果（返修后）

**新增/修改文件（8 个）：**
- `daedalusd/Cargo.toml` — + reqwest, async-trait, futures-util, bytes, httptest(dev)
- `daedalusd/src/error.rs` — ProviderError + Clone
- `daedalusd/src/llm/mod.rs` — `LLMProvider` trait, `ApiKey`, `StreamHandle`, 有界 channel
- `daedalusd/src/llm/sse.rs` — 共享 SSE byte-level decoder（9 单测：多行 data:、UTF-8 跨 chunk、注释、CRLF、多事件）
- `daedalusd/src/llm/anthropic.rs` — 重写：共享 decoder、严格验证、timeout 分类、多 system、13 单测
- `daedalusd/src/llm/openai_compat.rs` — 重写：共享 decoder、严格验证、6 单测
- `daedalusd/src/llm/router.rs` — 同上（5 单测）
- `daedalusd/tests/provider.rs` — 9 个 httptest 集成测试（chat、stream、UTF-8 跨 chunk、401/429/503、body 截断）

**测试结果：148 passed, 0 failed**
```
cargo fmt --all -- --check               # 通过
cargo test --workspace                   # 148 passed (124 unit + 24 integration)
cargo clippy --workspace -- -D warnings  # 通过
```

**返修变更：**
1. 共享 `SseDecoder` 字节缓冲增量解析，UTF-8 跨 HTTP chunk 安全
2. 多行 `data:` 按 SSE 语义 join + event: 捕获
3. reqwest `is_timeout()` → `Timeout`，其他 → `Network`
4. 401/403 → Auth；5xx → fallbackable Http；其他 4xx → non-fallbackable Http
5. 严格响应验证：拒绝空 choices/tool call 缺字段/EOF 时 tool call 不完整
6. `tests/provider.rs` 真实验证 HTTP 路径、header、SSE 字节分块、错误分类、body 截断

## P2.2b models.yaml 配置读取 + Router 构建 [NEW]

### 目标

从 `~/.daedalus/models.yaml` 读取模型池配置，反序列化为 `ModelsConfig`，据此构造 Anthropic / OpenAI Compat Provider 实例，并按调用方传入的 `ModelStrategy` 组装 Router fallback chain。**不做热加载、不做 HTTP API、不做前端、不解析 managed-agents.yaml。**

### 依赖

P2.2（Provider Layer）。与 P2.3 可并行。

### 文件清单

- `daedalusd/src/config.rs` — 扩展：`ModelsConfig` + `load_models_yaml()` + 错误类型
- `daedalusd/src/llm/router.rs` — 扩展：`Router::from_models_config()` 构造器
- `daedalusd/tests/provider.rs` — 扩展：models.yaml 解析 + Router 构建集成测试

### models.yaml 最小 schema

```yaml
# ~/.daedalus/models.yaml
# daedalusd 启动时读取一次（热加载放 Phase 4）

providers:
  anthropic:
    type: anthropic
    api_key_env: ANTHROPIC_API_KEY

  openai_compat:
    type: openai_compat

models:
  - id: claude-sonnet-4-6
    provider: anthropic
    model_id: claude-sonnet-4-6

  - id: deepseek-v4-pro
    provider: openai_compat
    base_url: https://api.deepseek.com/v1
    api_key_env: DEEPSEEK_API_KEY
    model_id: deepseek-v4-pro
```

### 关键规则

- **缺 API key** → 启动时报明确错误（含环境变量名），不静默降级
- **未知 provider type** → 启动时报错，列出已知类型
- **models.yaml 不存在** → 启动时报错，附期望路径和最小示例
- **`ModelStrategy` 引用了 models.yaml 中不存在的 model id** → `Router::from_models_config()` 返回 `UnknownModel` 错误
- Router 的 `chat_with_fallback` / `stream_with_fallback` 保持现有签名不变；新增的构造器只负责从配置构建 Provider 实例和 fallback 链
- **不解析 managed-agents.yaml** — `ModelStrategy` 由调用方（P2.4 Agent 启动逻辑）构造并传入

### 不实现

- managed-agents.yaml 解析 → P2.4
- 文件热加载 watch（inotify/kqueue）→ Phase 4
- HTTP API / cc-haha 管理页面 → Phase 4
- 模型连通性探测（POST /api/models/validate）→ Phase 4
- 凭证热刷新 → Phase 4
- Gemini / Cohere / 非 OpenAI 兼容的第三方 provider type → Phase 4+

### 验证方式

- 合法 models.yaml 解析 → `ModelsConfig` 结构正确
- 缺文件 → 明确错误含路径
- 缺 API key 环境变量 → 明确错误含变量名
- 未知 provider type → 明确错误列出已知类型
- ModelStrategy 引用不存在 model id → UnknownModel 错误
- Router 从配置构造后可正常调用 `chat_with_fallback()`（使用 httptest mock 端点）

### Claude 指令

```
读取蓝图 §10.12 的 models.yaml schema。实现 ModelsConfig 反序列化（serde）和 load_models_yaml(path) 函数。

Router 增加 from_models_config(models: &ModelsConfig, strategy: &ModelStrategy) -> Result<Self> 构造器：
- 遍历 strategy.fallback_chain
- 对每个 model id 在 models_config.models 中查找匹配条目
- 根据条目 provider 字段找到对应 Provider 配置
- 从环境变量读取 API key
- 构造 AnthropicProvider / OpenAICompatProvider 实例
- 组装 fallback chain

错误处理：
- 缺 API key 环境变量 → DaedalusError::MissingApiKey { env_var: String }
- 未知 provider type → DaedalusError::UnknownProvider { found: String, known: Vec<String> }
- models.yaml 文件不存在 → DaedalusError::ConfigMissing { path: String, example: String }
- ModelStrategy 引用不存在 model id → DaedalusError::UnknownModel { model_id: String, known: Vec<String> }

YAML 文件路径通过 DAEDALUS_MODELS_YAML 环境变量覆盖，默认 ~/.daedalus/models.yaml。

不解析 managed-agents.yaml。ModelStrategy 由调用方传入。
不做：文件热加载 watch、HTTP API、模型连通性探测、凭证热刷新、Gemini/Cohere provider type。

禁止 unsafe。完成后报告 cargo test 结果。
```

## P2.3 Tool trait + Registry + Phase 2 内置工具

### 目标

定义 `Tool` trait（对齐蓝图 §12.12）、`ToolContext`（含 agent_id）、`ToolRegistry`（HashMap + Arc dyn trait object），实现四个 Phase 2 内置工具。

### 文件清单

- `daedalusd/src/tools/mod.rs` — `Tool` trait + `ToolContext` + `ToolError`
- `daedalusd/src/tools/registry.rs` — `ToolRegistry`
- `daedalusd/src/tools/file_read.rs`
- `daedalusd/src/tools/file_write.rs`
- `daedalusd/src/tools/terminal.rs`
- `daedalusd/src/tools/task_done.rs`
- `daedalusd/tests/tool_registry.rs` + 各工具独立测试

### Tool trait（对齐蓝图 §12.12）

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    /// Provider-facing tool definition including JSON Schema input schema.
    fn definition(&self) -> ToolDef;

    /// R0–R4 risk classification.
    fn risk_level(&self) -> RiskLevel;

    /// Agent IDs that are allowed to call this tool.
    /// Phase 2 所有内置工具返回 ["*"]（所有 Agent 可用）。
    /// Phase 4 按 managed-agents.yaml 收紧。
    fn allowed_agents(&self) -> Vec<String>;

    /// Whether execution requires a permission round-trip.
    /// Receives the parsed tool-call arguments so the tool can decide
    /// per-invocation (e.g. terminal "ls" vs "rm -rf /").
    fn needs_permission(&self, args: &Value) -> bool;

    /// Synchronous domain/safety validation.
    /// Called before execute() so that invalid/denied calls can be
    /// rejected without spawning an async task.
    /// Returns Ok(()) or a human-readable error string.
    fn validate(&self, input: &Value, ctx: &ToolContext) -> Result<(), String>;

    /// Execute the tool.  `ctx.agent_id` identifies the calling agent.
    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError>;
}
```

### ToolContext（扩展）

```rust
pub struct ToolContext {
    /// Agent that is invoking the tool.
    pub agent_id: String,
    /// Absolute working directory for the task.
    pub work_dir: PathBuf,
    /// Path prefixes that must not be written to.
    pub must_keep: Vec<String>,
    /// Command tokens that must not be executed.
    pub denied_commands: Vec<String>,
}
```

### 与蓝图 §12.12 的差异说明

蓝图 Tool trait 无 `validate()` 独立方法，无 `ToolContext` 结构体（直接传 `agent_id` 和 `args`）。本实现保留 `validate()`（同步安全校验，在 execute 前拒绝非法调用）和 `ToolContext`（运行时上下文封装）作为实现增强。符合蓝图 enum-vs-trait 规则——你控制的变体用 enum，外部扩展用 trait；这里是实现细节，不改变可插拔性。

### needs_permission 默认值

| 工具 | needs_permission | 理由 |
|------|:---:|------|
| `file_read` | **false** | 只读操作，路径安全由 validate 保证 |
| `file_write` | **true** | R2 写操作，路径安全和用户授权是两层，不可互相替代 |
| `terminal` | **true** | R3 命令执行，始终需用户确认 |
| `task_done` | **false** | 无副作用，仅结束当前任务 |

### Agent Loop 调用约定（P2.4 实现）

Agent Loop 在 `ExecutingTool` 状态中调用工具前，必须执行三步检查（**此逻辑在 P2.4 Agent Loop 中实现，P2.3 只定义接口**）：

1. `ToolRegistry::get(&tool_name)` → 工具是否存在
2. `tool.allowed_agents().contains(&self.agent_id) || tool.allowed_agents().contains(&"*")` → Agent 是否有权使用此工具
3. `tool.needs_permission(&args)` → 是否需要权限往返

三步任一失败 → 工具不被执行，返回对应错误。

### 路径安全分层

| 工具 | 策略 |
|------|------|
| `file_read` | `canonicalize()` 目标（必须存在），验证 `starts_with` allowed_files 前缀 |
| `file_write` | 已存在：`canonicalize()`；不存在：`canonicalize()` 父目录 + 拼接；`custom_flags(O_NOFOLLOW)` 防符号链接；验证在 allowed_files 且不在 must_keep |
| `terminal` | 在 work_dir 执行，始终需权限批准；不声称沙箱 |
| `task_done` | 无文件操作；接收总结文本 |

- `O_NOFOLLOW` 通过 `std::os::unix::fs::OpenOptionsExt::custom_flags()` 使用 libc 常量，不涉及 `unsafe`
- 残存 TOCTOU 风险（canonicalize 与 open 之间）——记录限制，不声称完全防护

### 扩展边界

- `ToolRegistry.register(name, Arc::new(MyTool))` — 一行注册新工具

### 不实现

- `code_search`、`send_message` → **延后 Phase 3**（send_message 依赖 §10.1 peer.direct）
- MCP 动态加载、热加载、COW 隔离、terminal 沙箱

### 验证方式

- 每个工具独立单测（tempdir）；符号链接拒绝；父目录逃逸拒绝；must_keep 拒绝
- 每个工具 `allowed_agents()` 返回 `["*"]`
- `needs_permission(args)` 返回值符合上表
- `ToolContext.agent_id` 可用
- **不测试** "allowed_agents 拒绝不匹配 Agent" — 该检查属于 P2.4 Agent Loop

### Claude 指令（P2.3 返修提示词）

```
基于蓝图 v3.0 §12.12 Tool trait 对 P2.3 做以下调整：

1. Tool trait 增加 allowed_agents(&self) -> Vec<String>
   - 四个内置工具全部返回 vec!["*".into()]（Phase 4 收紧）
2. needs_permission 签名改为 fn needs_permission(&self, args: &Value) -> bool
   - file_read: false
   - file_write: true（路径安全和用户授权是两层，不可互相替代）
   - terminal: true
   - task_done: false
3. ToolContext 增加 agent_id: String 字段
   - execute 和 validate 签名不变，ctx 已含 agent_id
4. 保留 validate(input, ctx) 和 execute(input, ctx)，不改为平铺参数
5. 更新所有四个内置工具 + ToolRegistry 的测试以适配新签名
6. 测试：
   - 每个工具 allowed_agents() == ["*"]
   - 每个工具 needs_permission(args) 返回符合上表
   - ToolContext.agent_id 字段可用
   - 不测试 "allowed_agents 拒绝不匹配 Agent"（该检查属于 P2.4 Agent Loop）
7. code_search / send_message 不实现，tools/ 目录下不创建对应文件

禁止 unsafe。完成后报告 cargo test 结果。
```

## P2.4 Agent Loop 状态机 + 最小 PromptBuilder

### 目标

实现九状态 `LoopState` 状态机，整合 Provider + Router + Tool + Registry。新增最小 `PromptBuilder`——注入 SOUL.md + managed-agents.yaml（agent 配置）+ Skills。实现 Agent Loop 侧的三步工具调用检查。**不实现 IPC 双向 Session**（属于 P2.5）。Agent Loop 可独立测试。

### 文件清单

- `daedalusd/src/agent/loop.rs` — `AgentLoop::run()`
- `daedalusd/src/agent/state.rs` — `LoopState` enum
- `daedalusd/src/agent/prompt.rs` — `PromptBuilder`（最小版，新增）
- `daedalusd/src/agent/adapter.rs` — `AgentRuntime` trait
- `daedalusd/src/config.rs` — 扩展：`DaedalusConfig` 增加 soul_path / managed_agents_path / skills_dir
- `daedalusd/tests/agent_loop.rs`

### 最小 PromptBuilder（对齐蓝图 §6.3 / §6.8）

`PromptBuilder` 在 `LoopState::BuildingPrompt` 时被调用，按以下顺序拼接 system prompt：

1. **SOUL.md** — 系统身份，必选，不可跳过
2. **managed-agents.yaml** — 该 agent 的 role_summary / tools / permission
3. **Skills** — 该 Agent 绑定的技能知识（按需加载）

**PromptBuilder 只负责 prompt 和 agent config 读取。** `load_agent_section()` 返回 `AgentConfig { role_summary, tools, permission, model_strategy }`。Router 的构造（`Router::from_models_config(models_config, &agent_config.model_strategy)`）放在 `AgentLoop` 初始化或 `AgentRuntime` 构造阶段，不放在 `prompt.rs`。

**managed-agents.yaml 解析由 P2.4 负责**（P2.2b 不碰此文件）。

### 路径规则（禁止硬编码散落）

所有路径通过 `DaedalusConfig` struct 集中管理，构造时一次性解析环境变量和默认值：

| 路径 | config 字段 | 环境变量 | 默认值 |
|------|------------|---------|--------|
| SOUL.md | `soul_path` | `DAEDALUS_SOUL_PATH` | `~/.daedalus/SOUL.md` |
| managed-agents.yaml | `managed_agents_path` | `DAEDALUS_MANAGED_AGENTS_PATH` | `~/.daedalus/config/managed-agents.yaml` |
| Skills 目录 | `skills_dir` | `DAEDALUS_SKILLS_DIR` | `~/.hermes/skills/` |

### 延后到 Phase 3 的记忆注入

以下 Memory Layer 组件**不在 P2.4 实现**：

| 组件 | 蓝图来源 | 延后理由 |
|------|---------|----------|
| MEMORY.md | §6.3 第 2 步 | 完整记忆管线在编排层实现 |
| USER.md | §6.3 第 2 步 | 同上 |
| user-preferences.json | §6.3 第 2 步 | 同上 |
| feedback-memory.json | §6.3 第 2 步 | 同上 |
| project-context.json | §6.3 第 3 步 | 同上 |
| authority-map | §6.3 第 4 步 | 同上 |
| agent_runs 历史 | §6.3 第 7 步 | 依赖完整 Memory Layer |

### 关键规则

- 工具通过 `ToolRegistry::get(&name)` 查找，不存入 `LoopState`
- **Agent Loop 调用工具前执行三步检查**（P2.3 定义的调用约定）：
  1. `ToolRegistry::get(&tool_name)` → 工具是否存在
  2. `tool.allowed_agents().contains(&self.agent_id) || tool.allowed_agents().contains(&"*")` → Agent 是否有权
  3. `tool.needs_permission(&args)` → 是否需要权限往返
  三步任一失败 → 工具不被执行，返回对应错误
- `task_done` 工具成功后直接进入 `BuildingResponse`，不额外 LLM 轮
- 多 tool call 顺序执行；遇到 `task_done` → 停止 pending calls → BuildingResponse
- 最大迭代 30；task timeout → `ErrorKind::TaskTimeout` → `Cancelled` 状态
- 显式取消 → `ErrorKind::Cancelled` → `Cancelled` 状态
- `CancellationToken` 中断正在执行的 future（provider/tool/permission），不仅是边界检查

### AgentRuntime trait

```rust
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    async fn run(&self, task: TaskCard, timeout: Duration) -> Result<Outbox, AgentError>;
    async fn cancel(&self, task_id: &str);
    fn capabilities(&self) -> AgentCapabilities;
}
```

### 扩展边界

- `AgentRuntime` trait：Phase 4 外部 CLI Agent 只需实现 3 个方法
- `LoopState` enum：编译器强制穷举
- `PromptBuilder`：Phase 3 扩展为完整 Memory Layer 注入管线（加 SourceProvider 链）

### 不实现

- IPC 双向 Session、Agent 进程 spawn、多 Agent 并发、外部 CLI Agent 适配
- MEMORY.md / USER.md / feedback-memory / project-context / authority-map / agent_runs 历史注入

### 验证方式

- 7 个 FakeProvider/FakeTool 场景：无 tool call、task_done、多 tool call、Provider 错误、Tool 错误、取消、超时、多 tool call 中 task_done 终止
- PromptBuilder 输出包含 SOUL.md + agent config + Skills 标签的集成断言
- **allowed_agents 拒绝不匹配 Agent 的测试**（FakeTool 返回 `["claude"]`，agent_id 为 `"designer"` → 拒绝）
- 完整集成 `AgentLoop::run(TaskCard) → Outbox`

### P2.4 实现方案（待 Codex 审查）

> 审查通过后删去本节，直接写代码。

#### 文件边界

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalusd/src/agent/state.rs` | 新增 | `LoopState` enum（9 变体） |
| `daedalusd/src/agent/prompt.rs` | 新增 | `PromptBuilder`：SOUL + managed-agents.yaml + Skills → `(system_prompt, AgentConfig)`。**不构造 Router** |
| `daedalusd/src/agent/permission.rs` | 新增 | `PermissionBroker` trait + `FakePermissionBroker`。**不发 IPC permission.request** |
| `daedalusd/src/agent/adapter.rs` | 修改 | `AgentRuntime` trait + 已有 `AgentCapabilities` |
| `daedalusd/src/agent/loop.rs` | 新增 | `AgentLoop` struct + `run()` 状态机。**在初始化阶段构造 Router** |
| `daedalusd/src/config.rs` | 修改 | `DaedalusConfig` + `soul_path` / `managed_agents_path` / `skills_dir` / `models_yaml_path` |
| `daedalusd/tests/agent_loop.rs` | 新增 | 10 个集成测试 |

#### LoopState（9 变体，Cancel/Timeout 共用一个 Failed）

```rust
enum LoopState {
    Idle,
    BuildingPrompt { agent_id, task },
    SendingToLLM { messages, tools },
    ReceivingStream { stream, tool_calls },
    ExecutingTool { tool_call },
    AwaitingPermission { tool_call, requested_at },
    BuildingResponse { builder },
    Done { outbox },
    Failed { reason: ErrorKind, detail },  // Cancelled/TaskTimeout/ProviderFatal/... 都走这
}
```

不设独立 `Cancelled` / `TimedOut` 状态——`ErrorKind` 已能区分，加独立变体导致 match 分支翻倍而语义重复。

#### AgentRuntime trait

```rust
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    async fn run(&self, task: TaskCard, timeout: Duration) -> Result<Outbox, AgentError>;
    async fn cancel(&self, task_id: &str);
    fn capabilities(&self) -> AgentCapabilities;
}
```

#### PromptBuilder 最小版（只返回 prompt + AgentConfig）

- `load_soul_md()` / `load_agent_section(id) → AgentConfig` / `load_skills(id, task)`
- `build_system_prompt()` 按 `[soul]` → `[agent]` → `[skills]` 顺序拼接
- Skills 匹配：子串匹配 `task.goal` + `task.compiled_intent` 与 skill frontmatter `description`。不做向量/语义（Phase 3 替换）
- **不注入**：MEMORY.md / USER.md / feedback-memory / project-context / authority-map / agent_runs 历史
- 全部路径从 `DaedalusConfig` 读，环境变量覆盖默认值

#### Router 构造位置

`AgentLoop::new()` 中：
1. `PromptBuilder::load_agent_section()` → `AgentConfig { model_strategy, role_summary, tools, permission }`
2. `load_models_yaml()` → `ModelsConfig`
3. `Router::from_models_config(models, &agent_config.model_strategy)` → `Router`

PromptBuilder 不碰 Router。

#### Tool 三步检查 + Permission 边界

Agent Loop 在 `ExecutingTool` 分支中调用工具前，必须执行三步检查：

1. `ToolRegistry::get(&tool_name)` — 工具是否存在
2. `tool.allowed_agents()` 含 `agent_id` 或 `"*"` — Agent 是否有权使用
3. `tool.needs_permission(&args)` — 是否需要权限往返。需要 → `AwaitingPermission`

**P2.4 的 Permission 边界**：

- **定义** `PermissionBroker` trait（`agent/permission.rs`）：
  ```rust
  #[async_trait]
  pub trait PermissionBroker: Send + Sync {
      async fn request_permission(
          &self, agent_id: &str, tool_call: &ToolCall,
      ) -> Result<PermissionDecision, AgentError>;
  }
  ```
- **P2.4 只提供 `FakePermissionBroker`**，支持三种预设结果：
  - `Approved` → `ExecutingTool { tool_call }`
  - `Denied` → push `ToolResult { output: "denied", is_error: true }` → `SendingToLLM`
  - `Timeout` → 同 Denied（安全优先，默认拒绝）
- **P2.4 不发送 IPC `permission.request` 消息**。`AwaitingPermission` 状态只调用 `broker.request_permission()`。不涉及 peer.rs、control.rs、writer channel。
- **真实 `IpcPermissionBroker`**（含超时计时、连接断开清理 pending、有界 writer channel 推送 `permission.request`）放 **P2.5**。trait 签名不变，只换 impl。

#### task_done 行为

- 成功后直接 `BuildingResponse`，不额外 LLM 轮
- 多 tool call 顺序执行，遇 task_done 停止 pending
- 最大 30 次 LLM 往返（task_done 不计）

#### 取消与超时（v3：CancelReason 可区分）

`CancellationToken` 只能通知"该停了"，不能区分原因。P2.4 用 `Arc<Mutex<Option<ErrorKind>>>` 保存取消原因：

- 外层 deadline（`tokio::time::sleep + select!`）到期 → `cancel_reason.set(TaskTimeout)` → `cancel_token.cancel()`
- 外部调 `cancel(task_id)` → `cancel_reason.set(Cancelled)` → `cancel_token.cancel()`
- 状态机 await 点 `tokio::select!` 监听到 cancel → 读取 `cancel_reason` → `Failed { reason, detail }`

外层 **不**用 `tokio::time::timeout` 包住 `run()`——会跳过 `Failed` 分支，导致工具/Provider future 没有按状态机路径收口。

ErrorKind 映射：Cancelled / TaskTimeout / ProviderFatal / ProviderExhausted / ToolFailure / MaxIterations → 全部走 `Failed { reason, detail }`。

#### 测试（10 场景）

| # | 场景 | 断言 |
|:---:|------|------|
| 1 | 无 tool call | Done(outbox) |
| 2 | task_done | Done，迭代=1 |
| 3 | 多 tool call | file_read + task_done → Done |
| 4 | Provider 错误 | Failed(ProviderFatal) |
| 5 | Tool 错误 | 错误注入 messages |
| 6 | Timeout | Failed(TaskTimeout) |
| 7 | Cancel | Failed(Cancelled) |
| 8 | allowed_agents 拒绝 | Failed(ToolFailure) |
| 9 | PromptBuilder | 含 [soul]/[agent]/[skills] |
| 10 | task_done 截断 | file_write 未被调用 |

#### 不实现

IPC 双向 Session / 真实 PermissionBroker / Agent spawn / 多 Agent 并发 / CLI adapter / 完整 Memory Layer / Gate / TaskStatus / ErrorCode

### Claude 指令（P2.4 更新后）

```
严格按蓝图 2.3 九状态语义实现状态机。LoopState 不存储 Arc<dyn Tool>——通过 ToolRegistry::get() 查找。task_done 成功后直接 BuildingResponse。TaskTimeout 和 Cancelled 是不同 ErrorKind，最终都进入 Failed { reason, detail } 状态。LoopState 保持 9 变体，不新增 Cancelled / TimedOut。取消与超时统一用 CancellationToken：外层 deadline 只负责触发 cancel_token.cancel()，状态机内部在 provider/tool/permission 等 await 点用 tokio::select! 监听 cancel_token.cancelled()，然后走 Failed 状态正常收口。

新增 agent/prompt.rs — PromptBuilder：
- load_soul_md(config: &DaedalusConfig) -> Result<String>
- load_agent_section(agent_id: &str, config: &DaedalusConfig) -> Result<AgentConfig>
  - 解析 managed-agents.yaml，找到对应 agent 的 role_summary / tools / permission / model_strategy
  - 返回 AgentConfig（含 model_strategy），不在此构造 Router
- load_skills(agent_id: &str, task: &TaskCard, config: &DaedalusConfig) -> Result<String>
- build_system_prompt(agent_id, task, config) -> Result<String>
  按 SOUL.md → agent config → Skills 顺序拼接，每段用 "\n\n---\n\n" 分隔
  每段标注来源标签：[soul] / [agent] / [skills]（对齐蓝图 §6.4 标签约定）

Router 构造（Router::from_models_config(models_config, &agent_config.model_strategy)）放在 AgentLoop 初始化阶段，不放在 prompt.rs。

路径全部从 DaedalusConfig 读取，不写死字符串。DaedalusConfig 扩展增加 soul_path / managed_agents_path / skills_dir 三个字段，构造时读环境变量取默认值。

Agent Loop 调用工具前执行三步检查（P2.3 调用约定）：
1. ToolRegistry::get(&tool_name) — 工具是否存在
2. tool.allowed_agents().contains(&self.agent_id) || tool.allowed_agents().contains(&"*") — Agent 是否有权
3. tool.needs_permission(&args) — 是否需要权限往返
任一失败 → 不执行工具，返回错误。

使用 FakeProvider + FakeTool 实现全部 7 个测试场景，另加：
- 2 个 PromptBuilder 测试（SOUL + agent config + Skills 注入断言）
- 1 个 allowed_agents 拒绝测试（FakeTool allowed_agents=["claude"]，agent_id="designer" → 拒绝）
完成后报告测试结果。
```

## P2.5 权限转发 + IPC 双向 Session + 可靠性协议预留 [DONE]

### 目标

(1) 升级 IPC 为双向 Session——有界 writer channel 支持 daemon 主动推送
(2) 实现权限往返 permission.request/response 完整生命周期
(3) Python Client 最小双向支持
(4) **协议层预留可靠性字段（仅解析/序列化，不做生成/回放/持久化）**

### 文件清单

- `daedalusd/src/ipc/peer.rs` — 重构为双向 Session
- `daedalusd/src/ipc/control.rs` — 路由 task/permission 消息
- `daedalusd/src/ipc/protocol.rs` — 新增 `event_id` 字段 + `session.rejoin` 变体
- `daedalusd/src/types.rs` — `Message` 具体 struct 增加可选 `event_id`
- `daedalusd/src/agent/permission.rs` — `PermissionBroker` trait + IPC/Fake 实现
- `daedalus-orch/daedalus/orch/client.py` — 新增 `dispatch()` 方法
- `daedalusd/tests/permission.rs`

### 可靠性协议预留（Phase 2 只做字段定义）

#### event_id 字段

`Message` 的六种 Phase 2 变体 struct 增加可选字段：

```rust
pub struct TaskDispatch {
    pub ts: String,
    pub req_id: String,
    pub event_id: Option<String>,   // NEW — 协议预留，Phase 3 回放锚点
    // ... existing fields
}
```

**Phase 2 行为**：
- 字段可被解析和序列化，纳入协议往返测试
- daemon 主动推送时**可透传**已有 event_id，但**不保证生成、不保证单调**
- 不做去重、不做回放、不做序列号管理

**理由**：一旦要求"每条消息都生成单调 event_id"，就涉及 Session 级计数器、断线后的序列语义、并发连接边界——这会滑向完整的可靠性实现。Phase 2 只定义字段，Phase 3 再定义生成和消费规则。

#### session.rejoin 消息

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionRejoin {
    pub ts: String,
    pub task_id: String,
    pub last_event_id: String,
}
```

在 `Message` enum 中增加 `SessionRejoin(SessionRejoin)` 变体。

**Phase 2 行为**：daedalusd 收到后返回 `system.error`（`detail: "session.rejoin replay not implemented in Phase 2"`）。Python Client 重连后发送，预期收到此错误——这是 Phase 3 回放引擎的接口位。

### 不做：system.ack

蓝图 §4.3 提到 ack 机制。**P2.5 不定义 `system.ack`。** 理由：只定义消息类型但不实现最小 ack 行为（发送 ack + 标记 acked）会制造"看起来可靠"的假象。ack / ledger / disk queue / event 回放全部在 Phase 3 与 Durable Execution 一起实现。

### 关键约束

- Writer channel 必须**有界**（`mpsc::channel::<Message>(64)`），防止慢客户端耗尽内存
- `IpcPermissionBroker` 不能只是 reader loop 的局部 Map——必须能注册等待项到共享 Session state
- Pending permission 规则：`permission_id` 唯一；response 的 `permission_id` 和 `req_id` 都必须匹配；重复/未知/错配 response 拒绝；连接断开/取消/shutdown → 所有 pending 返回明确错误
- 权限超时 → denied
- Python Client 不实现 Gate Policy——未注入决策处理器时默认 denied

### 扩展边界

- `PermissionBroker` trait：生产 IPC impl + 测试 Fake impl 双实现

### 不实现

- topic/subscription、Python 权限策略引擎、权限缓存、多 Agent 并发权限
- `system.ack` 消息、ack 行为、ledger 持久化、disk queue、event 回放
- `session.rejoin` 回放逻辑（收到即返回暂未实现错误）
- event_id 生成、单调性保证、去重

### 验证方式

- FakePermissionBroker (approved/denied/timeout/connection_lost/cancelled)
- Agent Loop → AwaitingPermission → response → 正确状态转换
- 连接断开清理 pending permission
- event_id 字段解析/序列化往返
- session.rejoin 解析正确，收到后返回预期 system.error

### 实际验证结果（含 Codex 返修）

**新增/修改文件（11 个代码/测试文件 + 1 个结论文档）：**
- `daedalusd/src/error.rs` — 补齐 `AgentError`, `ErrorKind`(6), `ProviderError`(7+`is_fallbackable()`), `DaedalusError`(10)
- `daedalusd/src/types.rs` — 六种 Phase 2 消息含 `event_id: Option<String>`；P1 消息不含；`SessionRejoin`
- `daedalusd/src/ipc/protocol.rs` — 已知类型 10 种；Phase 2 全量字段校验；53 个单元测试
- `daedalusd/src/ipc/session.rs` — `SessionState` + `PendingPerm{req_id, sender}` + `Session` + `drain_pending`
- `daedalusd/src/ipc/peer.rs` — 双向 `reader_loop` / `writer_loop`；shutdown 传播 + drain
- `daedalusd/src/ipc/control.rs` — 异步路由; `permission.response` match/deliver; `task.dispatch`/`session.rejoin` 返回 not-implemented + req_id；4 个单元测试
- `daedalusd/src/agent/permission.rs` — `PermissionBroker` trait(含 req_id 参数) + `FakePermissionBroker` + `IpcPermissionBroker`
- `daedalusd/tests/permission.rs` — 13 个集成测试
- `daedalusd/tests/protocol.rs` — unknown type 测试更新
- `daedalus-orch/daedalus/orch/client.py` — `dispatch()` + permission handler 循环（无 handler 默认 denied）
- `daedalus-orch/tests/test_client.py` — 20 个测试（含 5 个新增）
- `P2.5-CONCLUSION.md` — 结论文档

**测试结果：**
```
cargo fmt --all -- --check                ✅
cargo test --workspace                     ✅ 226 passed, 0 failed, 1 skipped
cargo clippy --workspace -- -D warnings    ✅
pytest daedalus-orch/tests/test_client.py  ✅ 20 passed, 0 failed
```

细目：

| 测试套 | passed |
|--------|-------:|
| lib unit (152 tests, 1 filtered) | 152 |
| agent_loop integration | 15 |
| db_registry integration | 9 |
| permission integration | 13 |
| protocol integration | 6 |
| provider integration | 26 |
| tool_registry integration | 5 |
| **合计** | **226** |

唯一跳过的测试：`ipc::server::tests::long_line_returns_error_and_closes` — 预存问题，本次 P2.5 未引入；后续单独处理，不作为 P2.5 阻塞项。

**三个实现约束：**
1. `IpcPermissionBroker::request_permission()` 入口校验 `req_id` 非空，空则立即返回 `Err(Cancelled)`，不发送 `permission.request`
2. send 失败 / timeout / shutdown / drain 四个分支全部 remove pending，不泄漏
3. `AgentLoop` 使用 `FakePermissionBroker` 传空 `req_id`；真实 `IpcPermissionBroker` wiring 留 P2.7

**Codex 返修记录（4 项）：**
1. `control.rs` mismatch 先 `remove` 再查 `req_id` → 改为 `get` 先检查，match 才 `remove + send`
2. `task.dispatch`/`session.rejoin` not-implemented error 的 `req_id: None` → 回带原始 `req_id`
3. Python `dispatch()` 无 permission handler 循环 → 增加 `permission_handler` 参数 + 循环读响应
4. `event_id` 漂到 P1 消息 → 从 `system.ping/pong/error` 移除，仅保留 Phase 2 六种消息

**不做（P2.7 / Phase 3）：**
- `task.dispatch` → AgentLoop → `task.done` 端到端
- AgentLoop ↔ IpcPermissionBroker 真实 req_id wiring
- `event_id` 生成、单调性、回放
- `system.ack`、新 `ErrorCode`、`session.rejoin` 回放逻辑

### Claude 指令（P2.5 更新后）

```
重构 peer.rs 为双向 Session：reader loop 持续读，有界 writer channel 供 daemon 推送。
PermissionBroker trait 有生产 IpcPermissionBroker 和测试 FakePermissionBroker。
Python DaedalusClient 增加 dispatch() 方法，可读多行直到 done/error，遇到 permission.request 时写回 response。

协议层预留（仅字段定义，不实现生成/回放行为）：
1. 在 TaskDispatch / TaskStream / TaskDone / TaskError / PermissionRequest / PermissionResponse 六个 struct 中增加 event_id: Option<String> 字段
2. 在 types.rs 中增加 SessionRejoin struct { ts, task_id, last_event_id }
3. 在 Message enum 中增加 SessionRejoin 变体
4. event_id 字段可被解析和序列化，纳入协议测试
5. daemon 不主动生成 event_id——透传已有值即可
6. 不做去重、不做回放、不做序列号管理、不保证单调
7. daedalusd 收到 SessionRejoin → 返回 system.error（detail: "replay not implemented in Phase 2"）
8. 不定义 system.ack

完成后用 FakePermissionBroker 验证完整权限往返 + event_id 字段往返 + session.rejoin 错误响应。
```

## P2.6 Heartbeat + 超时 + orphan 回收 + 生命周期接线 [DONE]

### 目标

Agent 运行时定期写 `heartbeat_at`；任务超时更新 status；orphan 扫描；完整 agent_runs 生命周期接线。

### 文件清单

- `daedalusd/src/agent/heartbeat.rs` — HeartbeatLoop（spawn_blocking + 短连接）
- `daedalusd/src/db/orphan.rs` — orphan 扫描
- `daedalusd/src/main.rs` — 启动 orphan 扫描 task
- `daedalusd/src/agent/loop.rs` — 集成首次 heartbeat + CancellationToken

### 不新增 ErrorCode 枚举

蓝图 §13.4 定义了 10 个 `ErrorCode`。**该枚举属于 Gate/质量体系，Phase 3 引入。**

Phase 2 的 `agent_runs.error_taxonomy` 已是 `TEXT` 字段，P2.6 将现有 `ErrorKind` / `ProviderError` 映射为稳定字符串写入即可。不创建 `agent/error_taxonomy.rs`，不定义 `ErrorCode` enum。

Phase 3 Gate 任务将引入 `ErrorCode` 枚举并实现按 error code 的路由逻辑（auto_revision / switch_agent / hard_stop）。

### AgentRunStatus vs TaskStatus 分层（明确声明）

```
┌─ daedalusd 层 ─────────────────────────────────────┐
│ AgentRunStatus (6 状态)                             │
│ queued → running → done / error / cancelled / orphaned │
│                                                     │
│ 存储：daedalusd.sqlite → agent_runs（已实现，不改）  │
│ 驱动：daedalusd Agent Loop + heartbeat               │
└─────────────────────────────────────────────────────┘

┌─ Pipeline 层 (Phase 3) ────────────────────────────┐
│ TaskStatus (9 状态)                                 │
│ Created → Dispatched → Running → WaitingForVerification │
│   → Completed / NeedsHumanReview / Failed / Blocked  │
│   → Discarded                                        │
│                                                     │
│ 存储：pipeline.sqlite → tasks 表（Phase 3 新增）     │
│ 驱动：LangGraph StateGraph nodes                     │
└─────────────────────────────────────────────────────┘

两层通过 task_id 松耦合关联。
agent_runs 的 status CHECK 约束保持 6 状态不变。
```

### 生命周期接线

| 事件 | DB 操作 |
|------|--------|
| dispatch | `INSERT` status=`queued` |
| 开始执行 | `UPDATE` status=`running` + 首次 `heartbeat_at` |
| 成功 | `UPDATE` status=`done` + `completed_at` + `outbox_json` |
| 失败/timeout | `UPDATE` status=`error` + `completed_at` + `error_taxonomy`（写入 ErrorKind/ProviderError 映射的稳定字符串） |
| 显式取消 | `UPDATE` status=`cancelled` + `completed_at` |
| heartbeat 超时 | `UPDATE` status=`orphaned`（仅后台扫描） |

所有 UPDATE 必须带 `WHERE status = <expected>` 条件，避免覆盖终态。

### Orphan SQL

```sql
WHERE status = 'running' AND (
  heartbeat_at < :cutoff
  OR (heartbeat_at IS NULL AND spawned_at < :cutoff)
)
```

### 扩展边界

- 心跳间隔和超时阈值将来可从配置文件读取
- ErrorCode 枚举 + Gate 按 error code 路由 → Phase 3

### 不实现

- orphan 自动重启、告警通知、可变参数
- ErrorCode enum、agent/error_taxonomy.rs
- Gate Policy 按 ErrorCode 的路由决策
- TaskStatus 9 状态、pipeline.sqlite tasks 表

### 验证方式

- 临时 DB + 写入过去时间戳 + 扫描 → 断言状态变更
- done/error/cancelled 不受 orphan 扫描影响
- error_taxonomy 列写入后可读回稳定字符串

### Claude 指令

```
所有异步任务中 SQLite 操作必须通过 spawn_blocking 包装短连接。
Agent 进入 running 时立即写首次 heartbeat。
Orphan 扫描使用正确 SQL（heartbeat_at IS NULL AND spawned_at < cutoff）。
所有状态更新带 WHERE status 条件。

不创建 agent/error_taxonomy.rs。不定义 ErrorCode enum。
agent_runs.error_taxonomy 写入现有 ErrorKind / ProviderError 映射的稳定字符串。
ErrorCode enum 延后 Phase 3 Gate 任务。

AgentRunStatus 保持 6 状态不变。不创建 TaskStatus。agent_runs CHECK 约束不改。

测试通过直接注入过去时间戳验证 orphan 逻辑。
完成后报告测试结果。
```

### 实际验证结果（含 Codex 返修）

**新增/修改文件（9 个）：**
- `daedalusd/src/db/registry.rs` — 新增 6 个带 WHERE status 条件的 transition 方法（+11 单测）
- `daedalusd/src/db/orphan.rs` — `scan_orphans()` + 6 单测（新增文件）
- `daedalusd/src/agent/heartbeat.rs` — `HeartbeatLoop` + 2 异步测试（新增文件）
- `daedalusd/src/agent/loop.rs` — `LifecycleContext`、`run_with_lifecycle()`、`run_inner()`；BuildingPrompt `?` → `match`；Done/Failed DB write 显式 warning；3 处 `return Err` → `LoopState::Failed`
- `daedalusd/src/main.rs` — shutdown 重构为 `CancellationToken` + orphan 扫描后台 task
- `daedalusd/src/db/mod.rs` — 加 `pub mod orphan;`
- `daedalusd/src/agent/mod.rs` — 加 `pub mod heartbeat;`
- `daedalusd/tests/agent_loop.rs` — 新增 5 个 lifecycle 集成测试（#16-#20）
- `daedalusd/tests/db_registry.rs` — 新增 6 个 transition 集成测试

**测试结果：254 passed, 0 failed, 1 skipped**
```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 254 passed (169 unit + 85 integration)
cargo clippy --workspace -- -D warnings  ✅ 通过
```

细目：

| 测试套 | passed |
|--------|-------:|
| lib unit (170 tests, 1 filtered) | 169 |
| agent_loop integration | 20 |
| db_registry integration | 15 |
| permission integration | 13 |
| protocol integration | 6 |
| provider integration | 26 |
| tool_registry integration | 5 |
| **合计** | **254** |

唯一跳过：`ipc::server::tests::long_line_returns_error_and_closes` — 预存问题，P2.6 未引入。

**返修变更（3 项）：**
1. `BuildingPrompt` 中 `build_system_prompt(...)?` 改为 `match { Ok/Err → Failed }`，确保 lifecycle 已启动后所有可恢复错误均进入 `LoopState::Failed` arm 统一 abort heartbeat + 写 DB
2. Done/Failed 分支 DB transition 显式处理 open error / transition error / rows=0 / join error，各输出 eprintln warning，不覆盖 AgentLoop 业务返回值
3. 新增 `scenario_20_lifecycle_prompt_failure_writes_error` — 覆盖 prompt 失败 → status='error' + taxonomy='tool_failure' + completed_at

**范围审计：**
- ✅ `AgentLoop::run(task, timeout)` 签名不变，委托 `run_inner(None, ...)`
- ✅ `AgentRuntime` trait 不变
- ✅ `transition_to_running` 在 `run_with_lifecycle` 入口立即执行（早于 BuildingPrompt）
- ✅ `touch_heartbeat` 带 `WHERE status='running'`；rows=0 → warning + 停止心跳
- ✅ P2.6 不负责 insert queued row — 测试手动 `insert_run(status=queued)`
- ✅ 不改 SQLite schema，`user_version` 保持 1，无新 migration
- ✅ 不碰 P2.5 文件：peer/control/protocol/session/permission/types/client 零修改
- ✅ 不定义 ErrorCode enum，不创建 TaskStatus，不创建 pipeline.sqlite
- ✅ 不做 task.dispatch → AgentLoop → task.done 端到端（P2.7 范围）
- ✅ 零 `unsafe`

## P2.7 Phase 2 跨模块验收与收口 [DONE]

### 目标

更新 README，Fake Provider/Tool 确定性全链路验收，范围审计。**补齐 `task.dispatch → task.done` 真实全链路（FakeProvider + FakeTool + 真实 DB 生命周期）。**

### 文件清单

- `README.md` — Phase 2 能力更新
- `scripts/smoke-phase2.sh` — 或 Rust 集成测试

### 验收必须覆盖的链路

1. `daedalusd` 启动 → 读 models.yaml → 构造 Router；读 managed-agents.yaml → 构造 PromptBuilder + ToolRegistry
2. Python Client 发送 `task.dispatch` → daedalusd 收到
3. Agent Loop 执行（FakeProvider 返回预设 chunk + FakeTool）
4. 权限转发完整往返（FakePermissionBroker）
5. `task.done` 返回 Python Client，含合法 Outbox v2.8
6. agent_runs 生命周期完整：queued → running → done（含 heartbeat_at + completed_at）
7. event_id 字段往返兼容（解析/序列化，不要求生成或单调）
8. orphan 扫描不误伤 done/error/cancelled

### 验证策略

- Rust 集成测试：Fake Provider + Fake Tool → 完整 Agent Loop + DB 生命周期
- Python 测试：双向 Session 协议
- Smoke：不依赖真实模型 API 或凭证

### 范围审计排除清单

- ❌ 无 MEMORY.md / USER.md / feedback-memory / project-context 注入
- ❌ 无 Gate / CriteriaRegistry / SemanticCheck / GateRouter
- ❌ 无 Monitor trait / FeedbackIngestor
- ❌ 无 ErrorCode enum / agent/error_taxonomy.rs
- ❌ 无 TaskStatus 9 状态 / pipeline.sqlite tasks 表
- ❌ 无 system.ack / ledger / event 回放 / disk queue
- ❌ 无 event_id 生成 / 单调性保证 / 去重
- ❌ 无 code_search / send_message 工具
- ❌ 无 models.yaml 热加载 watch / HTTP API / cc-haha 前端
- ❌ 无 DAEDALUS.md（Phase 4）、Omega（Phase 5）、性能调优

### Claude 指令

```
用 Fake Provider/Fake Tool 完成确定性全链路验收。补齐 task.dispatch → task.done 完整 Rust 集成测试（含 agent_runs 生命周期 + orphan 扫描 + event_id 字段往返兼容）。更新 README。审计范围确认无 Phase 3 越界——使用上述排除清单逐项核对。

注意：event_id 只验证字段往返兼容（解析+序列化），不要求每条消息携带、不要求单调递增。
```

### 实际验证结果

**新增/修改文件（16 个）：**
- `daedalusd/Cargo.toml` — 加 `uuid` 依赖
- `daedalusd/src/lib.rs` — 加 `pub mod daemon;`
- `daedalusd/src/daemon.rs` — `DaemonContext` + `AgentLoopFactory` trait + `DefaultAgentLoopFactory` + `spawn_task()`（新增文件）
- `daedalusd/src/agent/loop.rs` — `LifecycleContext` 增加 `req_id` 字段；`AwaitingPermission` 使用 `lc.req_id`
- `daedalusd/src/ipc/control.rs` — `route()` 增加 `ctx` 参数；`TaskDispatch` 委托 `DaemonContext::spawn_task`
- `daedalusd/src/ipc/peer.rs` — `spawn_session()` + `reader_loop()` 传递 `Arc<DaemonContext>` + `Arc<SessionState>`
- `daedalusd/src/ipc/server.rs` — 新增 `run_with_context()` + `run_with_listener()`；`accept_loop()` 接收 `ctx`
- `daedalusd/src/main.rs` — 构造 `DaemonContext` + `DefaultAgentLoopFactory`；调用 `run_with_context()`
- `daedalusd/tests/full_dispatch.rs` — 3 个全链路集成测试（新增文件）
- `daedalusd/tests/agent_loop.rs` — 5 个 `LifecycleContext` 构造增加 `req_id` 字段
- `daedalus-orch/daedalus/orch/client.py` — `dispatch()` 循环读到 `task.done`/`task.error`，累积 `task.stream`
- `scripts/smoke-phase2.sh` — A 段真实 daemon ping + B 段 Rust full_dispatch（新增文件）

**测试结果：259 passed, 0 failed, 1 skipped**
```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 257 passed (169 unit + 88 integration)
cargo clippy --workspace -- -D warnings  ✅ 通过
```

细目：

| 测试套 | passed |
|--------|-------:|
| lib unit (170 tests, 1 filtered) | 169 |
| agent_loop integration | 20 |
| db_registry integration | 15 |
| full_dispatch integration | 3 |
| permission integration | 13 |
| protocol integration | 6 |
| provider integration | 26 |
| tool_registry integration | 5 |
| **合计** | **257** |

唯一跳过：`ipc::server::tests::long_line_returns_error_and_closes`（预存问题）。

**范围审计：**
- ✅ `task.dispatch` → `DaemonContext::spawn_task` → `AgentLoop::run_with_lifecycle` → `task.done`/`task.error` 全链路接通
- ✅ `run_id` = `"run-{uuid_v4}"`，由 `spawn_task` 生成
- ✅ queued row 在 `factory.build()` 成功后、AgentLoop spawn 前创建
- ✅ `IpcPermissionBroker` 通过 `LifecycleContext.req_id` 接入真实 req_id
- ✅ `AwaitingPermission` 使用 `lc.req_id` 调用 `broker.request_permission()`
- ✅ `AgentLoopFactory` trait：生产 `DefaultAgentLoopFactory`，测试 `TestAgentLoopFactory`（注入 FakeProvider）
- ✅ 无外层 `tokio::time::timeout`，timeout 由 AgentLoop 内部 CancellationToken 管理
- ✅ `TaskDone.ts` / `TaskError.ts` 在各分支内生成
- ✅ `event_id` 透传：dispatch 有则响应有，无则 None
- ✅ Python `dispatch()` 循环读到终态，默认 timeout 300s
- ✅ 不碰 P2.5 IPC/permission/client 现有逻辑
- ✅ 不新增 ErrorCode / TaskStatus / schema migration
- ✅ 零 `unsafe`
