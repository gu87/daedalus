# P3.2 实现完成报告：ErrorCode 枚举 + 统一错误分类

> 基于 P3.1b（commit `2826c02`），对齐蓝图 §13.4。

---

## 1. 修改文件清单（4 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/error.rs` | 修改 | + `ErrorCode` enum（10 变体）+ `as_str()` + `from_error_kind()` + `from_provider_error()` |
| `daedalusd/src/agent/loop.rs` | 修改 | 删除 `error_kind_to_taxonomy`；`Failed` arm 改用 `ErrorCode::from_error_kind().as_str()` |
| `daedalusd/src/daemon.rs` | 修改 | 删除 `error_kind_to_str`；改用 `ErrorCode::from_error_kind().as_str()` |
| `daedalusd/tests/error_code.rs` | **新增** | 10 个映射 + 序列化 + 兼容性测试 |

---

## 2. ErrorCode 定义

### 2.1 10 个变体 + as_str 字符串

| # | 变体 | `as_str()` | 来源 |
|:--|------|------|------|
| 1 | `Cancelled` | `"cancelled"` | ErrorKind::Cancelled |
| 2 | `TaskTimeout` | `"task_timeout"` | ErrorKind::TaskTimeout |
| 3 | `ToolFailure` | `"tool_failure"` | ErrorKind::ToolFailure |
| 4 | `MaxIterations` | `"max_iterations"` | ErrorKind::MaxIterations |
| 5 | `ProviderExhausted` | `"provider_exhausted"` | ErrorKind::ProviderExhausted |
| 6 | `ProviderFatal` | `"provider_fatal"` | ErrorKind::ProviderFatal |
| 7 | `ModelNotFound` | `"model_not_found"` | ProviderError::ModelNotFound |
| 8 | `AuthFailure` | `"auth_failure"` | ProviderError::Auth |
| 9 | `RateLimited` | `"rate_limited"` | ProviderError::RateLimited |
| 10 | `Unknown` | `"unknown"` | ProviderError::Parse / future catch-all |

序列化：`#[serde(rename_all = "snake_case")]`，`serde_json::to_string(&ErrorCode::TaskTimeout)` → `"\"task_timeout\""`。

### 2.2 from_error_kind 映射（6 种，穷尽 match）

| ErrorKind | ErrorCode |
|------|------|
| `Cancelled` | `Cancelled` |
| `TaskTimeout` | `TaskTimeout` |
| `ToolFailure` | `ToolFailure` |
| `MaxIterations` | `MaxIterations` |
| `ProviderExhausted` | `ProviderExhausted` |
| `ProviderFatal` | `ProviderFatal` |

**不映射到 Unknown**。`from_error_kind` 是穷尽 match，6 个变体显式覆盖。

### 2.3 from_provider_error 映射（7 种）

| ProviderError | ErrorCode |
|------|------|
| `Auth { .. }` | `AuthFailure` |
| `RateLimited { .. }` | `RateLimited` |
| `ModelNotFound(_)` | `ModelNotFound` |
| `Network(_)` | `ProviderExhausted` |
| `Timeout` | `ProviderExhausted` |
| `Http { status >= 500, .. }` | `ProviderExhausted` |
| `Http { status < 500, .. }` | `ProviderFatal` |
| `Parse(_)` | `Unknown` |

**定义 + 测试覆盖，P3.3 Gate 首次调用**。当前 Provider 错误路径仍通过 `is_fallbackable()` → `ErrorKind::ProviderExhausted/ProviderFatal` → `ErrorCode::from_error_kind()`。

---

## 3. 向后兼容

### 3.1 task.error.error_taxonomy

| 写入位置 | 旧实现 | 新实现 | 输出变化 |
|------|------|------|:---:|
| `daemon.rs` — `spawn_task` 发送 `TaskError` | `error_kind_to_str(&agent_error.reason)` | `ErrorCode::from_error_kind(&agent_error.reason).as_str()` | **无变化** |
| `agent/loop.rs` — `Failed` arm 写 DB | `error_kind_to_taxonomy(&reason).to_string()` | `ErrorCode::from_error_kind(&reason).as_str().to_string()` | **无变化** |

### 3.2 agent_runs.error_taxonomy

旧 6 个字符串与 `ErrorCode::as_str()` 逐一对应，完全一致：

```
"cancelled"           = ErrorCode::Cancelled.as_str()
"task_timeout"        = ErrorCode::TaskTimeout.as_str()
"tool_failure"        = ErrorCode::ToolFailure.as_str()
"max_iterations"      = ErrorCode::MaxIterations.as_str()
"provider_exhausted"  = ErrorCode::ProviderExhausted.as_str()
"provider_fatal"      = ErrorCode::ProviderFatal.as_str()
```

测试 `legacy_strings_unchanged` 逐一验证。

---

## 4. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 314 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

### 测试分布

| 测试套 | passed | 变化 |
|--------|-------:|:---:|
| lib unit | 171 | — |
| agent_loop | 20 | — |
| db_registry | 23 | — |
| error_code | **10** | **新增** |
| full_dispatch | 5 | — |
| permission | 13 | — |
| prompt | 35 | — |
| protocol | 6 | — |
| provider | 26 | — |
| tool_registry | 5 | — |
| **合计** | **314** | **+10** |

### 新增 error_code 测试

| # | 测试 | 覆盖 |
|:--|------|------|
| 1 | `from_error_kind_cancelled` | ErrorKind::Cancelled → Cancelled |
| 2 | `from_error_kind_task_timeout` | ErrorKind::TaskTimeout → TaskTimeout |
| 3 | `from_error_kind_all_six` | 6 种 ErrorKind 穷尽 |
| 4 | `from_provider_error_auth` | ProviderError::Auth → AuthFailure |
| 5 | `from_provider_error_rate_limited` | ProviderError::RateLimited → RateLimited |
| 6 | `from_provider_error_all_seven` | 7 种 ProviderError 全部 |
| 7 | `as_str_all_ten` | 10 个 ErrorCode 字符串 |
| 8 | `legacy_strings_unchanged` | 6 个旧字符串 = ErrorCode::as_str() |
| 9 | `serde_roundtrip` | serde 序列化往返 |
| 10 | `serde_all_ten_roundtrip` | 10 个变体全部往返 |

---

## 5. 不做清单

| 约束 | 状态 |
|------|:---:|
| 不改 SQLite schema（error_taxonomy 仍是 TEXT） | ✅ |
| 不改 IPC 协议（TaskError.error_taxonomy 仍是 String） | ✅ |
| 不改 AgentLoop 状态机 | ✅ |
| 不改变 task.error / error_taxonomy 旧输出 | ✅ |
| `from_provider_error` 只定义不接入 | ✅ — P3.3 Gate 启用 |
| GateRouter / CriteriaRegistry / SemanticCheck | ✅ 不做 |
| system.ack / event replay / TaskStatus / pipeline.sqlite | ✅ 不做 |
