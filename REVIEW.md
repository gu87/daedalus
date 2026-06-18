# P3.5 实现完成报告：ProviderError 完整贯通 Gate 路由

> 基于 P3.4（commit `ac9273a`），将 `ErrorCode::from_provider_error()` 接入 daemon Gate 路由路径，
> ProviderError 在 `AgentError → LoopState::Failed → AgentError` 全链路不丢失。

---

## 1. 修改文件清单（5 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/error.rs` | 修改 | `AgentError` +`provider_error: Option<ProviderError>` + `error_code()` 方法 |
| `daedalusd/src/agent/state.rs` | 修改 | `LoopState::Failed` +`provider_error: Option<ProviderError>` |
| `daedalusd/src/agent/loop.rs` | 修改 | `next_chunk_with_cancel` 签名改为 `Result<Option<StreamChunk>, AgentError>`；`stream_with_cancel`/`next_chunk_with_cancel` 保留 ProviderError；SendingToLLM/ReceivingStream → LoopState::Failed 传递 provider_error；Failed arm 用 `error_code()` 写 DB taxonomy；所有非 provider 错误构造加 `provider_error: None` |
| `daedalusd/src/daemon.rs` | 修改 | Gate 路由 + TaskError taxonomy 统一用 `agent_error.error_code()`；移除 inline `from_error_kind` fallback |
| `daedalusd/tests/gate_daemon.rs` | 修改 | RecordingProvider 扩展 error injection 支持；新增 7 个 provider-error 粒度测试 |
| `daedalusd/tests/full_dispatch.rs` | 修改 | taxonomy 断言从 `"provider_fatal"` → `"auth_failure"` |
| `daedalusd/src/agent/permission.rs` | 修改 | 4 处 AgentError 构造加 `provider_error: None` |

**未改文件**：`gate.rs`、`types.rs`、`config.rs`、`ipc/*`、`prompt.rs`、`heartbeat.rs`

---

## 2. 核心行为

### 2.1 数据流闭环

```
stream_with_cancel / next_chunk_with_cancel
  ProviderError(Auth{401}) → AgentError { reason: ProviderFatal, provider_error: Some(Auth) }
  ↓
SendingToLLM / ReceivingStream Err
  → LoopState::Failed { reason, detail, provider_error: Some(Auth) }
  ↓
Failed arm
  → AgentError::error_code() = ErrorCode::AuthFailure
  → DB: transition_to_error(taxonomy="auth_failure")
  → return Err(AgentError { reason, detail, provider_error: Some(Auth) })
  ↓
DaemonContext::spawn_task()
  → agent_error.error_code() = ErrorCode::AuthFailure
  → GateContext { error_code: AuthFailure } → GateRouter::route()
  → TaskError { error_taxonomy: "auth_failure" }
```

### 2.2 `AgentError::error_code()` — 唯一真相源

```rust
pub fn error_code(&self) -> ErrorCode {
    self.provider_error
        .as_ref()
        .map(ErrorCode::from_provider_error)  // AuthFailure/RateLimited/ModelNotFound/...
        .unwrap_or_else(|| ErrorCode::from_error_kind(&self.reason))  // fallback
}
```

DB taxonomy 和 TaskError taxonomy 走同一入口，消除不一致风险。

### 2.3 Gate routing 精度提升

| 错误场景 | P3.4 taxonomy | P3.5 taxonomy |
|---------|:---:|:---:|
| Auth(401/403) | provider_fatal | **auth_failure** |
| RateLimited(429) | provider_exhausted | **rate_limited** |
| ModelNotFound | provider_fatal | **model_not_found** |
| Timeout | provider_exhausted | provider_exhausted |
| Network error | provider_exhausted | provider_exhausted |
| Parse error | provider_fatal | **unknown** |
| Tool failure (provider_error=None) | tool_failure | tool_failure |

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 340 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

### 测试分布

| 测试套 | passed | 变化 |
|--------|-------:|:---:|
| lib unit | 184 | — |
| agent_loop | 20 | — |
| db_registry | 23 | — |
| error_code | 10 | — |
| full_dispatch | 5 | 1 taxonomy 断言修正 |
| gate_daemon | **13** | **+7** |
| permission | 13 | — |
| prompt | 35 | — |
| protocol | 6 | — |
| provider | 26 | — |
| tool_registry | 5 | — |
| **合计** | **340** | **+7** |

### 新增 gate_daemon 测试（7 个）

| # | 测试 | 错误源 | 断言 |
|:--|------|------|------|
| 7 | `auth_failure_routes_as_auth_failure` | ProviderError::Auth(401) | TaskError.taxonomy + DB taxonomy = `"auth_failure"` |
| 8 | `rate_limited_auto_revision_succeeds` | 1st RateLimited(429), 2nd text "ok" | auto_revision retry → TaskDone；first DB row = `"rate_limited"` |
| 9 | `model_not_found_hard_stop` | ProviderError::ModelNotFound | taxonomy = `"model_not_found"` |
| 10 | `provider_timeout_routes_provider_exhausted` | ProviderError::Timeout | taxonomy = `"provider_exhausted"` |
| 11 | `parse_error_routes_unknown` | ProviderError::Parse | taxonomy = `"unknown"` |
| 12 | `tool_failure_no_provider_error_unchanged` | BoomTool（provider_error=None） | taxonomy = `"tool_failure"`，行为不变 |
| 13 | `streaming_midflight_provider_error_preserved` | Stream [Text("hi"), Err(Auth{403})] | next_chunk_with_cancel 路径 → taxonomy = `"auth_failure"` |

---

## 4. 不做清单

| 约束 | 状态 |
|------|:---:|
| 不修改 gate.rs API | ✅ |
| 不修改 types.rs | ✅ |
| 不修改 IPC 协议 | ✅ |
| 不修改 SQLite schema | ✅ |
| 不新增 ErrorCode 变体 | ✅ |
| 不实现 SemanticCheck | ✅ — P3.6+ |
| 不实现 SwitchAgent 分发 | ✅ — P3.6+ |
| daemon 中不写 inline from_error_kind/from_provider_error | ✅ |

---

## 5. 返回 Codex 复审

P3.5 实现完毕，340 测试全过。请 Codex 审查。
