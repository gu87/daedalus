# Daedalus Phase 1 实施计划

> 范围：只完成蓝图第七节 Phase 1“骨架（1-2 周）”。不实现 Provider、Tool-use Loop、Tool Registry、权限转发、Heartbeat 监控或 orphan 回收。
>
> 执行规则：Claude 每完成一个子任务，在对应标题末尾标注 `[DONE]`，并附上实际验证结果；随后由 Codex 审查后才能进入下一个子任务。
>
> 开工前待拍板：
>
> 1. 蓝图第 10.8、10.10 节和 enum-vs-trait 表把 `Transport` 视为 enum，第 12.12 节又把它定义为核心 trait。Phase 1 可先实现具体 UDS Server，但不应在拍板前引入 `Transport` enum/trait 抽象。
> 2. 蓝图说吸收 CCB 的“Agentic Loop 六层拆分”，但没有正式枚举这六层；不能让 Claude 自行补定义。当前 Phase 1 只按系统全景和模块清单划边界。
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
