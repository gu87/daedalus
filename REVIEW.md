# P4.1 实现完成报告：HTTP API 骨架 + 健康检查

> Phase 4 起点。daedalusd 新增 axum HTTP server，与 UDS server 并行运行。
> 暴露 `GET /api/health` 端点。仅本地 loopback。

---

## 1. 修改文件清单（8 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/Cargo.toml` | 修改 | + `axum = "0.7"`，tokio + `net` feature |
| `daedalusd/src/config.rs` | 修改 | DaedalusConfig + `http_addr: String`；+ `validate_http_addr()`（拒绝非 loopback）；5 个单元测试 |
| `daedalusd/src/http/mod.rs` | **新增** | HTTP 子模块声明（health + server） |
| `daedalusd/src/http/health.rs` | **新增** | `HttpState` + `GET /api/health` handler（status/uptime/socket_path/db_ok） |
| `daedalusd/src/http/server.rs` | **新增** | `run_http(listener: TcpListener, state, shutdown)` — axum Router + graceful shutdown |
| `daedalusd/src/main.rs` | 修改 | 验证 http_addr loopback → bind TCP → spawn HTTP server → 与 UDS 并行；shutdown 传播 |
| `daedalusd/src/lib.rs` | 修改 | + `pub mod http` |
| `daedalusd/tests/http_health.rs` | **新增** | 3 个集成测试（health ok / db not ok / graceful shutdown） |

**未改文件**：`gate.rs`、`error.rs`、`agent/loop.rs`、`agent/state.rs`、`ipc/*`、`db/*`、`types.rs`

---

## 2. 核心行为

### 2.1 HttpState — 轻量独立状态

```rust
pub struct HttpState {
    pub ctx: Arc<DaemonContext>,   // 只读引用，不修改
    pub started_at: Instant,
    pub socket_path: String,
    pub db_path: PathBuf,
}
```

不修改 DaemonContext，不重构 daemon 核心结构。

### 2.2 Loopback 强制

- 默认 `127.0.0.1:9800`
- 环境变量 `DAEDALUSD_HTTP_ADDR`
- 拒绝 `0.0.0.0`、公网 IP、空 host
- `localhost`、`127.0.0.1`、`[::1]` 允许
- 非法地址 → daemon 启动失败 exit(1)

### 2.3 /api/health 响应

```json
{ "status": "ok", "uptime_seconds": 42, "socket_path": "/tmp/daedalusd.sock", "db_ok": true }
```

### 2.4 Shutdown 传播

HTTP server + UDS server + orphan scanner 共享同一个 `CancellationToken`。SIGTERM/SIGINT → token.cancel() → 全部退出。

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 393 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

### 测试分布

| 测试套 | passed | 变化 |
|--------|-------:|:---:|
| lib unit | 224 | +5 config + gate 单元测试不变 |
| agent_loop | 20 | — |
| db_registry | 23 | — |
| error_code | 10 | — |
| full_dispatch | 5 | — |
| gate_daemon | 23 | — |
| http_health | **3** | **新增** |
| permission | 13 | — |
| prompt | 35 | — |
| protocol | 6 | — |
| provider | 26 | — |
| tool_registry | 5 | — |
| **合计** | **393** | **+8** |

---

## 4. 返回 Codex 复审

P4.1 实现完毕，393 测试全过。请 Codex 审查。
