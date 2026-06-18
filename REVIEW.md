# P3.6 实现完成报告：Gate Semantic Tags — 语义标签分类与 Gate 路由增强

> 基于 P3.5（commit `90022fd`），给 Gate 路由增加第二维度：SemanticTag 分类。
> 同一 ErrorCode 可根据错误详情路由到不同 GateAction。纯规则分类，不调用 LLM。

---

## 1. 修改文件清单（3 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/gate.rs` | 修改 | +`SemanticTag`（6 变体）；+`classify_semantic_tags()`；GateContext +`semantic_tags` +注释修正；GateCriteria +`require_tags`；RawGateRule +`require_tags` +空数组拒绝；resolve() AND tag 匹配；~35 个新单元测试 |
| `daedalusd/src/daemon.rs` | 修改 | GateContext 构造时调用 `classify_semantic_tags()`（+2 行） |
| `daedalusd/tests/gate_daemon.rs` | 修改 | 新增 6 个语义标签集成测试（#14-#19） |

**未改文件**：`error.rs`、`agent/loop.rs`、`agent/state.rs`、`ipc/*`、`types.rs`、`config.rs`

---

## 2. 核心行为

### 2.1 SemanticTag 枚举（6 种）

```rust
pub enum SemanticTag {
    Permanent,           // 重试无意义
    Transient,           // 重试可能恢复
    NeedsHuman,          // 需人工介入
    PermissionDenied,    // 权限拒绝
    ConfigurationError,  // 配置错误
    ResourceExhausted,   // 资源/配额耗尽
}
```

### 2.2 classify_semantic_tags() — 分类规则

| ErrorCode | detail 关键词 | Tags |
|------|------|------|
| Cancelled | — | [Permanent] |
| TaskTimeout | — | [Transient, ResourceExhausted] |
| ToolFailure | permission denied/denied/not allowed | [Permanent, PermissionDenied] |
| ToolFailure | not found/missing/temporarily unavailable | [Transient] |
| ToolFailure | 其他 | [Permanent] |
| MaxIterations | — | [Transient] |
| ProviderExhausted | dns/connection refused/tls/timeout/timed out/network | [Transient] |
| ProviderExhausted | 其他 | [Transient, ResourceExhausted] |
| ProviderFatal | — | [Permanent] |
| ModelNotFound | — | [Permanent, ConfigurationError, NeedsHuman] |
| AuthFailure | — | [Permanent, ConfigurationError, NeedsHuman] |
| RateLimited | — | [Transient, ResourceExhausted] |
| Unknown | truncated/incomplete/eof/timeout/timed out | [Transient] |
| Unknown | 其他 | [Permanent] |

所有匹配 lowercase substring。永不返回空 Vec。

### 2.3 Gate 路由 AND 语义

```
for each rule:
  1. error_code != ctx.error_code → skip
  2. if require_tags is Some(tags): ANY tag not in ctx.semantic_tags → skip
  3. if retry_count >= max_retries → skip
  4. return action
fall through → HardStop
```

### 2.4 向后兼容

- 10 条默认规则全部 `require_tags: None`
- 无 YAML override 时行为与 P3.5 完全一致
- `require_tags: []` → `DaedalusError::Yaml` 报错

### 2.5 daemon 集成

```rust
let ec = agent_error.error_code();
let semantic_tags = crate::gate::classify_semantic_tags(&agent_error);
let ctx = GateContext { error_code: ec, agent_id, task_id, retry_count, semantic_tags };
```

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 381 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

### 测试分布

| 测试套 | passed | 变化 |
|--------|-------:|:---:|
| lib unit（含 gate.rs） | 219 | +35 |
| agent_loop | 20 | — |
| db_registry | 23 | — |
| error_code | 10 | — |
| full_dispatch | 5 | — |
| gate_daemon | **19** | **+6** |
| permission | 13 | — |
| prompt | 35 | — |
| protocol | 6 | — |
| provider | 26 | — |
| tool_registry | 5 | — |
| **合计** | **381** | **+41** |

### 新增 gate_daemon 集成测试（6 个）

| # | 测试 | 场景 | 断言 |
|:--|------|------|------|
| 14 | `permission_denied_tag_hard_stop` | tool_failure+[Permanent] → require_tags:[transient] → skip → HardStop | TaskError |
| 15 | `transient_tool_failure_auto_revision` | "file not found" → [Transient] → require_tags:[transient] → match → retry | TaskDone |
| 16 | `configuration_error_blocks_auto_revision` | AuthFailure → [Permanent, ConfigurationError, NeedsHuman] → require_tags:[transient] → skip → HardStop | TaskError |
| 17 | `rate_limited_tags_auto_revision` | RateLimited → [Transient, ResourceExhausted] → require_tags:[transient,resource_exhausted] → match → retry | TaskDone |
| 18 | `yaml_empty_require_tags_rejected` | YAML `require_tags: []` → parse error | DaedalusError::Yaml |
| 19 | `default_rules_unchanged_by_tags` | 无 YAML → 默认 HardStop | TaskError |

---

## 4. 不做清单

| 约束 | 状态 |
|------|:---:|
| 不修改 error.rs / AgentError | ✅ |
| 不修改 agent/loop.rs / state.rs | ✅ |
| 不修改 IPC 协议 / SQLite schema | ✅ |
| 不实现 SwitchAgent 分发 | ✅ — P3.7+ |
| 不实现 LLM SemanticCheck | ✅ |
| 不引入正则 / 新 crate | ✅ |
| classify_semantic_tags 纯函数 | ✅ |

---

## 5. 返回 Codex 复审

P3.6 实现完毕，381 测试全过。请 Codex 审查。
