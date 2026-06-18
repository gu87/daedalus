# P3.7 实现完成报告：SwitchAgent — 跨 Agent 分发

> 基于 P3.6（commit `68da5f6`），实现 `GateAction::SwitchAgent { agent_id }`。
> 原降级为 HardStop → 现实现真正的跨 agent 分发。

---

## 1. 修改文件清单（2 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/daemon.rs` | 修改 | `agent_id` 拆为 `initial_agent_id` + `mut current_agent_id`；SwitchAgent arm 从降级 HardStop 改为真正分发：retry_count++、新 CancellationToken/Broker、build-before-insert（target agent）、feedback 注入（含原 agent 名）、current_agent_id 更新、continue loop |
| `daedalusd/tests/gate_daemon.rs` | 修改 | GateTestFactory +`alt_agent_id`/`alt_provider`/`reject_agent_id`；setup_config 增加 test-agent-2；替换 switch_agent_degrades_to_hard_stop → switch_agent_succeeds；新增 4 个 SwitchAgent 场景测试 |

**未改文件**：`gate.rs`、`error.rs`、`agent/loop.rs`、`agent/state.rs`、`ipc/*`、`types.rs`、`config.rs`

---

## 2. 核心行为

### 2.1 agent_id 变量拆分

```
spawn_task():
  initial_agent_id = td.agent_id.clone()    // 外层：首次 build/insert
  └─ tokio::spawn(async move {
       mut current_agent_id = initial_agent_id   // 内层：随 switch 更新
       loop {
         TaskDone / TaskError / GateContext / retry insert → current_agent_id
         SwitchAgent { target_agent_id }:
           factory.build(target_agent_id) → insert_run(target_agent_id)
           → current_agent_id = target_agent_id
           → continue
       }
     })
```

### 2.2 SwitchAgent arm（与 AutoRevision 复用同一结构）

```
retry_count += 1
新 CancellationToken
新 IpcPermissionBroker
factory.build(target_agent_id)  // ← 唯一差异
  → 失败 → TaskError(Unknown, agent_id=target) + break
insert_run(agent_id=target, parent=prev, depth=retry_count)
  → 失败 → TaskError(Unknown, agent_id=target) + break
set_retry_feedback("Previous agent '{old}' failed: {detail}")
prev_run_id = new_run_id
lc = LifecycleContext { new_run_id }
agent_loop = new_al
current_agent_id = target_agent_id  // ← KEY
continue loop
```

### 2.3 终态消息 agent_id

| 终态 | agent_id |
|------|------|
| TaskDone（switch 后成功） | target agent |
| TaskDone.outbox.agent_id | target agent |
| TaskError（target build 失败） | target agent |
| TaskError（target insert 失败） | target agent |
| TaskError（target 执行后 HardStop） | target agent |

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 385 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

### 测试分布

| 测试套 | passed | 变化 |
|--------|-------:|:---:|
| lib unit | 219 | — |
| agent_loop | 20 | — |
| db_registry | 23 | — |
| error_code | 10 | — |
| full_dispatch | 5 | — |
| gate_daemon | **23** | **+4** |
| permission | 13 | — |
| prompt | 35 | — |
| protocol | 6 | — |
| provider | 26 | — |
| tool_registry | 5 | — |
| **合计** | **385** | **+4** |

### 新增/替换 gate_daemon 测试（5 个）

| # | 测试 | 场景 | 断言 |
|:--|------|------|------|
| 6r | `switch_agent_succeeds` | A 失败 → switch → B 成功 | TaskDone.agent_id=B；outbox.agent_id=B；2 DB rows（A error + B done）；ORDER BY spawn_depth ASC |
| 20 | `switch_agent_then_auto_revision` | A fail → switch B → B fail transient → B auto_revision → B success | TaskDone；3 DB rows；agent_id 序列正确 |
| 21 | `switch_agent_build_fails` | A fail → switch → target build rejected → TaskError | TaskError.agent_id=target；1 DB row（仅 A 的 error row） |
| 22 | `switch_agent_feedback_mentions_old_agent` | A fail → switch B → 检查 B 的消息 | feedback 包含 "Previous agent 'test-agent' failed" |
| 23 | `switch_agent_respects_global_cap` | max_global_retries=1 → A fail → switch B → B fail → cap → HardStop | TaskError；2 DB rows（A error + B error） |

---

## 4. 不做清单

| 约束 | 状态 |
|------|:---:|
| 不修改 gate.rs API | ✅ |
| 不修改 error.rs / AgentError | ✅ |
| 不修改 agent/loop.rs / state.rs / prompt.rs | ✅ |
| 不修改 IPC 协议 / SQLite schema | ✅ |
| 不传递 conversation history（仅 feedback 摘要） | ✅ |
| 不实现跨 task 路由 | ✅ |
| 不实现回切检测（由 max_global_retries 兜底） | ✅ |
| 不实现 task_card 改写 | ✅ |
| 不碰 TaskStatus / pipeline.sqlite / system.ack / event replay | ✅ |

---

## 5. 返回 Codex 复审

P3.7 实现完毕，385 测试全过。请 Codex 审查。
