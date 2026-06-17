# P2.6 返修报告：Heartbeat + 超时 + Orphan 回收 + 生命周期接线

> 基于 P2.5 已提交状态（commit `69c242e`），含 Codex 审查返修。

---

## 1. 返修项

| # | 问题 | 修复 |
|:--|------|------|
| 1 | `BuildingPrompt` 中 `build_system_prompt(...).map_err(...)?` 绕过 `LoopState::Failed` | 改为 `match { Ok => SendingToLLM, Err => Failed }` |
| 2 | Done / Failed 分支 `let _ = spawn_blocking { ... }` 静默吞错 | 增加 4 级 eprintln warning：db open error / transition error / rows=0 / join error |
| 3 | 缺少 prompt 失败的 lifecycle 测试 | 新增 `scenario_20_lifecycle_prompt_failure_writes_error` |

---

## 2. 修改文件（3 个）

| 文件 | 变更 |
|------|------|
| `daedalusd/src/agent/loop.rs` | BuildingPrompt `?` → `match`；Done/Failed DB write 显式 warning |
| `daedalusd/tests/agent_loop.rs` | 新增 scenario_20 |
| `REVIEW.md` | 本报告 |

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                    ✅ 254 passed, 0 failed, 1 skipped
cargo clippy --workspace -- -D warnings   ✅ 通过
```

### 测试分布

| 测试套 | passed | 变化（vs P2.5） |
|--------|-------:|:---:|
| lib unit | 169 | +12 |
| agent_loop integration | **20** | **+5**（#16-#20 lifecycle） |
| db_registry integration | 15 | +6 |
| permission integration | 13 | — |
| protocol integration | 6 | — |
| provider integration | 26 | — |
| tool_registry integration | 5 | — |
| **合计** | **254** | **+28** |

唯一跳过：`ipc::server::tests::long_line_returns_error_and_closes`（预存，P2.6 未引入）。

---

## 4. 所有 lifecycle 启动后的错误路径均进入 Failed arm

`run_with_lifecycle` 执行流程中，一旦 `transition_to_running` 成功 + HeartbeatLoop 启动后，以下所有可恢复错误均通过 `LoopState::Failed` 收口：

| 错误来源 | 原路径 | 返修后 |
|----------|--------|--------|
| `build_system_prompt` 失败 | `return Err`（绕过 lifecycle） | `LoopState::Failed { ToolFailure }` |
| `check_tool_access` 拒绝（ExecutingTool） | `return Err`（绕过 lifecycle） | `LoopState::Failed { ToolFailure }` |
| `check_tool_access` 拒绝（AwaitingPermission） | `return Err`（绕过 lifecycle） | `LoopState::Failed { ToolFailure }` |
| `execute_with_cancel` 失败（ExecutingTool） | `return Err`（绕过 lifecycle） | `LoopState::Failed { ... }` |
| `execute_with_cancel` 失败（AwaitingPermission） | `return Err`（绕过 lifecycle） | `LoopState::Failed { ... }` |
| `stream_with_cancel` 错误 | 已走 `LoopState::Failed` | 不变 |
| `next_chunk_with_cancel` 错误 | 已走 `LoopState::Failed` | 不变 |
| MaxIterations | 已走 `LoopState::Failed` | 不变 |
| Cancel / Timeout（CancellationToken） | 已走 `LoopState::Failed` | 不变 |

`LoopState::Failed` arm 统一执行：`hb.abort()` → `spawn_blocking` DB transition → `return Err(AgentError)`。DB 写失败不影响主返回值，仅 eprintln warning。

---

## 5. 边界遵守

| # | 规则 | 状态 |
|:--|------|:----:|
| 1 | 不修改 `AgentLoop::run(task, timeout)` 签名 | ✅ |
| 2 | 不修改 `AgentRuntime` trait | ✅ |
| 3 | `transition_to_running` 在入口立即执行 | ✅ |
| 4 | `touch_heartbeat` 带 `WHERE status='running'` | ✅ |
| 5 | P2.6 不负责 insert queued row | ✅ |
| 6 | 不改 SQLite schema | ✅ |
| 7 | 不碰 P2.5 IPC/permission/client | ✅ |
| 8 | 不定义 ErrorCode/TaskStatus | ✅ |
