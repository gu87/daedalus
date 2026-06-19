# P5.3b 实现方案（Codex 修订版）：Pipeline ↔ daemon/Gate/Durable Events 接线

> 基于 P5.3a（commit `f458671`）+ P5.2（Durable Execution）。

---

## 0. Codex 修订

| # | 修订 |
|---|------|
| 1 | SwitchAgent：`Running → Blocked → Dispatched → Running`（3 步） |
| 2 | tasks 表按 `task_id` 唯一，不按 agent 分行 |
| 3 | 首次 build/insert 失败不创建 tasks 行 |
| 4 | orphan 接线需要 task_id 以更新 tasks |

---

## 1. 状态映射表

| daemon 事件 | TaskStatus 转换 | 触发位置 |
|------------|----------------|---------|
| `task.dispatch` 首次 build+insert 成功 | insert_task(Created) → Dispatched → Running | daemon.rs spawn_task（build 成功 + insert_run 成功之后） |
| 首次 build 失败 / insert 失败 | **不创建 tasks 行** | — |
| AgentLoop 成功 (task.done) | Running → WaitingForVerification | daemon.rs TaskDone 分支 |
| Gate HardStop (task.error) | Running → Failed | daemon.rs HardStop 分支 |
| Gate SwitchAgent | Running → Blocked（当前 agent 中止）→ Dispatched → Running（新 agent） | daemon.rs SwitchAgent：Blocked 在 Gate 决策后，Dispatched 在 insert_run 成功后，Running 在 run_with_lifecycle 开始后 |
| Gate AutoRevision | **不更新**（保持 Running） | — |
| SwitchAgent/AutoRevision build/insert 失败 | Running → Failed | daemon.rs 对应 Err 分支 |
| agent_runs orphaned | Running → Failed | orphan.rs：scan_orphans 返回 (run_id, task_id) |

---

## 2. SwitchAgent 三步转换

```
agent-A fail → Gate SwitchAgent
  → update_status(task_id, "blocked", now)           ← Gate 决策后
  → factory.build(agent-B) + insert_run(agent-B)
  → update_status(task_id, "dispatched", now)          ← 新 run 已就绪
  → agent-B run_with_lifecycle 开始
  → update_status(task_id, "running", now)             ← 新 agent 执行中
```

P5.3a 已允许：Running→Blocked, Blocked→Dispatched, Dispatched→Running。

---

## 3. 首次 build/insert 失败边界

```
spawn_task():
  factory.build() → 失败 → 返回 system.error（不创建 tasks 行）
  insert_run()    → 失败 → 返回 system.error（不创建 tasks 行）
  成功 →
    insert_task(Created, now)
    update_status(Dispatched, now)
    update_status(Running, now)
    tokio::spawn(retry loop)
```

---

## 4. orphan 接线

`scan_orphans` 当前签名：
```rust
pub fn scan_orphans(conn: &Connection, cutoff: i64) -> Result<Vec<String>>
```

P5.3b 修改为返回 `Vec<(String, String)>` — `(run_id, task_id)`：

```rust
SELECT run_id, task_id FROM agent_runs WHERE status='running' AND ...
```

调用方（main.rs）在 transition_to_orphaned 成功後调用：
```rust
pipeline::db::update_status(&conn, task_id, "failed", now)?;
```

---

## 5. 文件边界

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalusd/src/daemon.rs` | 修改 | 8 个事件点同步推进 TaskStatus |
| `daedalusd/src/db/orphan.rs` | 修改 | scan_orphans 返回 (run_id, task_id)；调用方更新 tasks→Failed |
| `daedalusd/src/main.rs` | 修改 | scan_orphans 结果处理中新增 pipeline::db::update_status |
| `daedalusd/tests/pipeline_daemon.rs` | **新增** | 5 个集成测试 |

**不改**：pipeline/status.rs、pipeline/db.rs、gate.rs、IPC、HTTP、Agent Loop 状态机

---

## 6. 测试计划（5 个）

| # | 测试 | 场景 | 断言 |
|:--|------|------|------|
| 1 | `dispatch_creates_task_and_transitions_to_running` | task.dispatch 成功 | tasks 表 1 行：Created→Dispatched→Running |
| 2 | `task_done_transitions_to_waiting_for_verification` | FakeProvider 返回 done | tasks status = WaitingForVerification |
| 3 | `task_error_hardstop_transitions_to_failed` | FakeProvider error + HardStop | tasks status = Failed |
| 4 | `switch_agent_transitions_via_blocked` | A fail + switch B → B success | 同一 task_id：Blocked→Dispatched→Running→WaitingForVerification；agent_runs 有 A/B 两条 run，parent_run_id 正确 |
| 5 | `orphan_scanner_updates_tasks_to_failed` | agent_runs orphaned → scan | tasks status = Failed |

---

## 7. 不做清单

| 约束 | 状态 |
|------|:---:|
| Pipeline 执行引擎（跨 task DAG） | ✅ |
| WaitingForVerification → Gate 审批联动 | ✅ — P5.4+ |
| UI / 可视化 | ✅ |
| Ω-Agent、MCP Bridge | ✅ — Phase 5+ |
| 修改 pipeline/status.rs、pipeline/db.rs API | ✅ |
| 修改 Gate 路由逻辑 | ✅ |
| 修改 IPC/HTTP | ✅ |
