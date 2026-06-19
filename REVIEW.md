# P5.3a 实现方案：TaskStatus + tasks 表 + 基础 CRUD

> 基于 P5.2（commit `15f64a9`）。
> 只做 schema/CRUD/状态类型，不接 pipeline 执行、不做 UI、不做 Ω-Agent。
> 存储：共享 daedalusd.sqlite（非独立 pipeline.sqlite）。

---

## 1. 目标

1. 新增 `TaskStatus` 枚举（9 状态）
2. 在 `daedalusd.sqlite` 中新增 `tasks` 表（migration v3）
3. 新增 `pipeline/` 模块：`status.rs` + `db.rs`
4. 基础 CRUD：`insert_task`、`update_status`、`get_task`
5. `transition()` 状态转换规则

---

## 2. TaskStatus 9 状态

```
Created → Dispatched → Running → WaitingForVerification
  → Completed / NeedsHumanReview / Failed / Blocked / Discarded
```

```rust
pub enum TaskStatus {
    Created,
    Dispatched,
    Running,
    WaitingForVerification,
    Completed,
    NeedsHumanReview,
    Failed,
    Blocked,
    Discarded,
}
```

### 转换规则

| 当前状态 | 允许转换到 |
|---------|-----------|
| Created | Dispatched |
| Dispatched | Running |
| Running | WaitingForVerification, Failed, Blocked |
| WaitingForVerification | Completed, NeedsHumanReview, Failed |
| Completed | —（终态） |
| NeedsHumanReview | Completed, Failed |
| Failed | —（终态） |
| Blocked | Dispatched（重试）, Discarded |
| Discarded | —（终态） |

- 终态不可再转换（`transition()` 返回 Err）
- 不在允许列表中的转换返回 Err

---

## 3. tasks 表 schema（migration v3）

```sql
CREATE TABLE IF NOT EXISTS tasks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         TEXT NOT NULL UNIQUE,    -- 业务 task_id
    agent_id        TEXT,
    status          TEXT NOT NULL DEFAULT 'created'
                    CHECK(status IN (
                        'created','dispatched','running','waiting_for_verification',
                        'completed','needs_human_review','failed','blocked','discarded'
                    )),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
```

- 共享 `daedalusd.sqlite`，不创建独立数据库文件
- 通过 `task_id` 与 `agent_runs.run_id` / `agent_runs.task_id` 松耦合关联

---

## 4. 文件边界

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalusd/src/lib.rs` | 修改 | + `pub mod pipeline` |
| `daedalusd/src/pipeline/mod.rs` | **新增** | Pipeline 模块声明 |
| `daedalusd/src/pipeline/status.rs` | **新增** | `TaskStatus` 枚举 + `as_str()` + `parse_status()` + `transition()` 转换规则 |
| `daedalusd/src/db/migrations.rs` | 修改 | migration v3：+ `tasks` 表；测试更新 user_version→3 + tasks/索引存在验证 |
| `daedalusd/src/pipeline/db.rs` | **新增** | `insert_task()` / `update_status()` / `get_task()` CRUD；**update_status 内部执行 transition 校验** |
| `daedalusd/tests/pipeline_status.rs` | **新增** | TaskStatus 转换规则单元测试 |
| `daedalusd/tests/pipeline_db.rs` | **新增** | tasks CRUD 集成测试（含非法转换被拒） |

**不改**：daemon.rs、Agent Loop、Gate、IPC、HTTP、Durable Execution

---

## 5. 测试计划

### pipeline_status.rs（~10 个）

| # | 测试 | 断言 |
|:--|------|------|
| 1 | `as_str_all_nine` | 9 个变体映射到 snake_case 字符串 |
| 2 | `parse_status_all_nine` | 双向转换无损失 |
| 3 | `parse_invalid_returns_none` | "bogus" → None |
| 4 | `transition_created_to_dispatched` | Ok |
| 5 | `transition_running_to_waiting` | Ok |
| 6 | `transition_terminal_rejected` | Completed → X → Err |
| 7 | `transition_invalid_rejected` | Created → Running → Err |
| 8 | `transition_dispatched_to_running` | Ok |
| 9 | `transition_waiting_to_completed` | Ok |
| 10 | `transition_running_to_failed` | Ok |

### pipeline_db.rs（~5 个）

| # | 测试 | 断言 |
|:--|------|------|
| 11 | `insert_and_get_task` | 写入 → 读回字段完整 |
| 12 | `update_status_valid` | Created → Dispatched → 读回 status=dispatched，**updated_at 已更新** |
| 13 | `update_status_invalid_transition_rejected` | Created → Running（非法）→ Err，**读回仍是 Created**，DB 未被修改 |
| 14 | `update_status_invalid_status_string_rejected` | 非法状态字符串 "bogus" → Err |
| 15 | `get_nonexistent_returns_none` | 不存在的 task_id → None |

### update_status 行为

- 读取当前 `status`
- 调用 `TaskStatus::transition(current, next)` 校验
- 合法 → `UPDATE status, updated_at`
- 非法转换 → 返回 `Err`，不修改 DB
- 非法 status 字符串（非 9 变体之一）→ 返回 `Err`

---

## 6. 不做清单

| 约束 | 状态 |
|------|:---:|
| Pipeline 执行引擎 | ✅ — P5.3b |
| daemon 接线（spawn_task 中创建 TaskStatus） | ✅ — P5.3b |
| Gate 集成（WaitingForVerification → 审批） | ✅ — P5.3b |
| Durable Events 接线 | ✅ — P5.3b |
| 独立 pipeline.sqlite | ✅ — 共享 daedalusd.sqlite |
| Ω-Agent、MCP Bridge | ✅ — Phase 5+ |
| UI / 可视化 | ✅ |
| 跨 task DAG | ✅ |
