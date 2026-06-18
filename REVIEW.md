# P3.4 实现方案：daemon Gate 接线 + AutoRevision 重试

> 基于 P3.3（commit `d47ce7c`），将 GateRouter 接入 daemon 错误处理路径，实现 AutoRevision 重试。
>
> **状态：方案按 Codex 意见修订，待审查**

---

## 0. Codex 审查记录

### 第一轮

| # | 拍板 | 结论 |
|:--|------|------|
| 1 | from_provider_error | P3.4 不接入。不改 AgentError、不改 error.rs。只用 `from_error_kind()` |
| 2 | retry feedback | 用 `pending_retry_feedback: Option<String>`，BuildingPrompt 中 system prompt 之后再 push |
| 3 | build-before-insert | 每次 attempt 都先 build 后 insert。retry build 失败 → 不创建 DB row → 发最终 TaskError |
| 4 | PermissionBroker | 每次 retry 新建 IpcPermissionBroker |
| 5 | 中间失败不发 TaskError | 客户端只收最终一条终态消息 |
| 6 | SwitchAgent | 降级 HardStop，eprintln warning + 发 TaskError |
| 7 | gate_criteria_path | 接入 DaedalusConfig，env var + 默认路径，文件不存在 = defaults |
| 8 | 测试计划 | 6 个测试 |

### 第二轮

| # | 拍板 | 结论 |
|:--|------|------|
| 1 | 状态标注 | 审查阶段不写"已通过" |
| 2 | 文件清单补全 | 所有 DaemonContext/DaedalusConfig struct literal 须补字段，列出全部 14 个构造点 |
| 3 | CancellationToken 每次新建 | retry attempt 创建新 CancellationToken + 新 IpcPermissionBroker + factory.build |
| 4 | task_card.clone() | run_with_lifecycle 消费 TaskCard，loop 内每次传 clone |
| 5 | retry build 失败 error_taxonomy | 用 `ErrorCode::Unknown.as_str()`，detail 含 "failed to build retry AgentLoop:" |
| 6 | gate YAML 错误 → 启动失败 | 坏配置不静默降级，eprintln + exit(1) |

---

## 1. 前置分析

### 1.1 已完成依赖

| 子任务 | commit | 提供 |
|--------|--------|------|
| P3.2 ErrorCode | `86a9283` | 10 个 ErrorCode 变体 + `from_error_kind()` |
| P3.3 Gate | `d47ce7c` | GateAction / GateContext / CriteriaRegistry / GateRouter |

### 1.2 当前 daemon 错误路径（要改的位置）

```
daemon.rs spawn_task → tokio::spawn:
  ┌─ AgentLoop::run_with_lifecycle()
  │
  ├─ Ok(outbox) → Message::TaskDone → writer_tx.send
  └─ Err(agent_error) → ErrorCode::from_error_kind() → Message::TaskError → writer_tx.send
                                                                  ↑
                                                          单次发送，无重试
```

P3.4 在此插入 GateRouter：
```
  └─ Err(agent_error) → ErrorCode::from_error_kind() → GateContext → GateRouter::route()
       │
       ├─ HardStop → Message::TaskError → break
       ├─ AutoRevision → set_retry_feedback → rebuild AgentLoop → insert retry row → run again
       └─ SwitchAgent → eprintln warning → HardStop → Message::TaskError → break
```

---

## 2. 目标

1. 将 GateRouter 接入 DaemonContext，启动时从 gate-criteria.yaml 构造
2. 重构 `spawn_task()` 的 tokio::spawn 闭包为带 Gate 决策的重试循环
3. 实现 AutoRevision：失败 → 注入反馈 → 重建 AgentLoop → 重新执行，每次 attempt 保持 build-before-insert

---

## 3. 文件边界

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalusd/src/daemon.rs` | **修改** | DaemonContext +gate_router；重构 tokio::spawn 闭包为 Gate 重试循环；每次 attempt build-before-insert |
| `daedalusd/src/agent/loop.rs` | **修改** | + `pending_retry_feedback: Option<String>`；+ `set_retry_feedback()`；BuildingPrompt 中 system prompt 之后注入反馈 |
| `daedalusd/src/config.rs` | **修改** | DaedalusConfig + `gate_criteria_path: String`（env var + 默认路径） |
| `daedalusd/src/main.rs` | **修改** | 构造 CriteriaRegistry → GateRouter；启动时 YAML 坏配置 → exit(1) |
| `daedalusd/src/ipc/server.rs` | **修改** | 3 处 DaemonContext literal 补 `gate_router` 字段 |
| `daedalusd/src/ipc/control.rs` | **修改** | 2 处 DaemonContext literal + 2 处 DaedalusConfig literal 补字段 |
| `daedalusd/tests/agent_loop.rs` | **修改** | 1 处 `test_config()` DaedalusConfig literal 补字段 |
| `daedalusd/tests/prompt.rs` | **修改** | 2 处 DaedalusConfig literal 补字段 |
| `daedalusd/tests/full_dispatch.rs` | **修改** | 1 处 DaedalusConfig literal 补字段；1 处 DaemonContext literal 补字段 |
| `daedalusd/tests/gate_daemon.rs` | **新增** | 6 个集成测试（§7） |

**汇总：所有 DaedalusConfig/DaemonContext struct literal 必须显式补全新字段。**

### DaedalusConfig 构造点（8 处）

| # | 文件:行 | 函数/上下文 | 需补 `gate_criteria_path` |
|:--|------|------|:--:|
| 1 | `config.rs:135` | `DaedalusConfig::load()` | 读 env var + 默认路径 |
| 2 | `tests/agent_loop.rs:56` | `test_config()` | `"/dev/null"` |
| 3 | `tests/prompt.rs:391` | `full_chain_contains_expected_labels` | `"/dev/null"` |
| 4 | `tests/prompt.rs:612` | `prompt_builder_new_db_path_some_injects_history` | `"/dev/null"` |
| 5 | `tests/full_dispatch.rs:182` | `setup_config()` | `"/dev/null"` |
| 6 | `ipc/control.rs:109` | `make_ctx()` | `"/dev/null"` |
| 7 | `ipc/control.rs:238` | `task_dispatch_build_failure_returns_system_error` | `"/dev/null"` |

注意：`ipc/server.rs:31` / `ipc/server.rs:34` / `ipc/server.rs:372` / `ipc/server.rs:375` 用 `DaedalusConfig::load()`，不由 struct literal 构造，不受影响。

### DaemonContext 构造点（5 处）

| # | 文件:行 | 上下文 | 需补 `gate_router` |
|:--|------|------|:--:|
| 1 | `main.rs:39` | 生产启动 | `Arc::new(GateRouter::new(registry, MAX_TASK_RETRIES))` |
| 2 | `ipc/server.rs:30` | `run()` — Phase 1 兼容 | `Arc::new(GateRouter::new(CriteriaRegistry::defaults(), 5))` |
| 3 | `ipc/server.rs:371` | 测试 `accept_error_still_cleans_up_socket` | 同上 |
| 4 | `ipc/control.rs:108` | `make_ctx()` | 同上 |
| 5 | `ipc/control.rs:237` | `task_dispatch_build_failure_returns_system_error` | 同上 |
| 6 | `tests/full_dispatch.rs:205` | `start_server()` | 同上 |

**不修改**：gate.rs、error.rs、types.rs、ipc/protocol.rs、db/registry.rs、agent/state.rs、agent/prompt.rs、agent/prompt_sources.rs

---

## 4. 类型变更

### 4.1 DaemonContext 增加 gate_router

```rust
pub struct DaemonContext {
    pub config: DaedalusConfig,
    pub db_path: std::path::PathBuf,
    pub factory: Arc<dyn AgentLoopFactory>,
    pub gate_router: Arc<crate::gate::GateRouter>,  // NEW
}
```

### 4.2 DaedalusConfig 增加 gate_criteria_path

```rust
pub struct DaedalusConfig {
    pub soul_path: String,
    pub managed_agents_path: String,
    pub skills_dir: String,
    pub models_yaml_path: String,
    pub db_path: Option<std::path::PathBuf>,
    pub gate_criteria_path: String,  // NEW
}
```

`DaedalusConfig::load()`：
- `DAEDALUS_GATE_CRITERIA_PATH` env var（如果设置）
- 默认 `{HOME}/.daedalus/config/gate-criteria.yaml`

### 4.3 AgentLoop 增加 pending_retry_feedback

```rust
pub struct AgentLoop {
    // ... existing fields ...
    /// P3.4: retry feedback set by daemon before each retry attempt.
    /// Consumed once in BuildingPrompt, after system prompt push.
    pending_retry_feedback: Option<String>,  // NEW
}

impl AgentLoop {
    /// Set retry feedback for the next run.
    /// Called by daemon AFTER factory.build() and BEFORE run_with_lifecycle().
    pub fn set_retry_feedback(&mut self, detail: &str) {
        self.pending_retry_feedback = Some(format!(
            "Previous attempt failed: {detail}\n\
             Please analyse the failure and try a different approach."
        ));
    }
}
```

### 4.4 BuildingPrompt 注入点

在 `AgentLoop::run_inner()` 的 `LoopState::BuildingPrompt` arm 中，system prompt push 之后：

```rust
// 现有：push system prompt
self.messages.push(ChatMessage { role: "system", content: system });

// P3.4 新增：inject retry feedback AFTER system prompt
if let Some(feedback) = self.pending_retry_feedback.take() {
    self.messages.push(ChatMessage {
        role: "system",
        content: feedback,
    });
}

// 现有：进入 SendingToLLM
LoopState::SendingToLLM { messages: self.messages.clone(), ... }
```

消息顺序保证：`[system prompt]` → `[retry feedback]` → SendingToLLM。

---

## 5. daemon spawn_task 重构伪代码

```
spawn_task(&self, td, writer_tx, session_state):
  0. clone td fields (run_id, agent_id, task_id, req_id, event_id, task_card)
  1. IpcPermissionBroker::new(session_state, writer_tx.clone(), PERMISSION_TIMEOUT)
  2. factory.build(agent_id, Arc::new(perm_broker), CancellationToken::new())
     → Err → return Some(system.error)  ← 不创建 DB row
     → Ok(agent_loop) → continue
  3. insert_run(queued, parent=None, depth=0)
     → Err → return Some(system.error)
  4. tokio::spawn async {
       let mut retry_count: u32 = 0;
       let mut prev_run_id = first_run_id.clone();
       let mut agent_loop = <from step 2>;
       loop {
         // (a) run — note: task_card.clone() each iteration
         result = agent_loop.run_with_lifecycle(lc, task_card.clone(), timeout).await
         match result {
           Ok(outbox) → { send TaskDone; break }
           Err(agent_error) → {
             ec = ErrorCode::from_error_kind(&agent_error.reason);
             ctx = GateContext { error_code: ec, agent_id, task_id, retry_count };
             match self.gate_router.route(&ctx) {
               HardStop → { send TaskError; break }
               SwitchAgent { .. } → {
                 eprintln("daedalusd gate: switch_agent not implemented in P3.4, treating as hard_stop");
                 send TaskError; break
               }
               AutoRevision → {
                 retry_count += 1;
                 // (b) NEW CancellationToken per retry
                 cancel = CancellationToken::new();
                 // (c) NEW IpcPermissionBroker per retry
                 broker = Arc::new(IpcPermissionBroker::new(
                     session_state.clone(), writer_tx.clone(), PERMISSION_TIMEOUT));
                 // (d) build-before-insert
                 new_al = match factory.build(agent_id.clone(), broker, cancel) {
                   Ok(al) → al,
                   Err(e) → {
                     // retry build failure → no new DB row, send final TaskError
                     send TaskError {
                       error_taxonomy: ErrorCode::Unknown.as_str(),
                       detail: format!("failed to build retry AgentLoop: {e}"),
                     };
                     break;
                   }
                 };
                 // (e) insert retry row
                 new_run_id = format!("run-{}", Uuid::new_v4());
                 insert_run(queued, parent=prev_run_id, depth=retry_count)
                   → Err → { send TaskError; break }
                 // (f) inject feedback + swap state
                 new_al.set_retry_feedback(&agent_error.detail);
                 agent_loop = new_al;
                 prev_run_id = new_run_id;
                 lc = LifecycleContext { run_id: new_run_id, db_path, req_id };
                 continue;  // ← back to top of loop
               }
             }
           }
         }
       }
     }
```

### 关键行为

| 场景 | 行为 |
|------|------|
| 首次 build 失败 | `system.error`，无 DB row |
| 首次 run 成功 | TaskDone，DB done |
| 首次 run 失败 + HardStop | TaskError（最终），DB error |
| retry build 失败 | TaskError（最终），error_taxonomy=`"unknown"`，detail 含 "failed to build retry AgentLoop:"，无新 DB row |
| retry run 成功 | TaskDone（最终），之前每次 attempt 有独立 DB 行 |
| SwitchAgent | TaskError（最终），eprintln warning |
| global cap 触发 | TaskError（最终），同 HardStop |

---

## 6. main.rs GateRouter 构造 + 错误处理

```rust
// 构造 GateRouter
let registry = match daedalusd::gate::CriteriaRegistry::with_overrides(
    &config.gate_criteria_path,
) {
    Ok(r) => r,
    Err(e) => {
        eprintln!("daedalusd fatal: invalid gate criteria config: {}", e);
        std::process::exit(1);
    }
};
let gate_router = Arc::new(daedalusd::gate::GateRouter::new(registry, MAX_TASK_RETRIES));

let ctx = Arc::new(DaemonContext {
    config: config.clone(),
    db_path: db_path.clone(),
    factory: Arc::new(DefaultAgentLoopFactory { config }),
    gate_router,
});
```

**Gate YAML 错误处理**：
- 文件不存在 → `with_overrides()` 返回 defaults，正常启动
- YAML 语法错误 / invalid action / invalid switch_agent target → `Err` → `eprintln` + `exit(1)`，不静默降级

---

## 7. 测试计划（6 个集成测试）

全部使用 `TestAgentLoopFactory`（注入 FakeProvider + FakeTool）+ `FakePermissionBroker` + 临时 DB + 真实 GateRouter。

| # | 测试 | 构造 | 断言 |
|:--|------|------|------|
| 1 | `hard_stop_default_tool_failure` | defaults 全 HardStop；FakeTool error → ToolFailure | 最终 TaskError；agent_runs 1 行 status=error，error_taxonomy=tool_failure |
| 2 | `auto_revision_single_retry_succeeds` | YAML: tool_failure→auto_revision(max_retries=3)；第一次 FakeTool error，第二次 success | 客户端只收 TaskDone；agent_runs 2 行（error + done）；第二行 parent_run_id=第一行 run_id，spawn_depth=1 |
| 3 | `auto_revision_max_retries_exceeded` | YAML: tool_failure→auto_revision(max_retries=2)；持续 error | 最终 TaskError；agent_runs 3 行全部 error；客户端只收 1 条 TaskError |
| 4 | `global_cap_blocks_auto_revision` | max_global_retries=1；YAML: tool_failure→auto_revision(max_retries=10) | retry_count=1 ≥ cap → HardStop；agent_runs 2 行 error；最终 TaskError |
| 5 | `retry_feedback_injected_after_system_prompt` | AutoRevision；第二次运行 FakeProvider 记录 messages | messages[0] 是 system prompt，随后包含 "Previous attempt failed:"；system prompt 在 feedback **前面** |
| 6 | `switch_agent_degrades_to_hard_stop` | YAML: tool_failure→switch_agent(target_agent=other) | 最终 TaskError；agent_runs 1 行 error |

### 不单独测试的路径（行为已在伪代码中定义）

- retry build 失败 → TaskError(error_taxonomy=`"unknown"`)，无新 DB row。可被测试 2/3 中注入 build-failing Factory 变体覆盖，但不创建独立测试。

---

## 8. 如何复用 P3.2 + P3.3

| 复用 | 方式 |
|------|------|
| `ErrorCode` (P3.2) | daemon 通过 `from_error_kind(&agent_error.reason)` 构造。P3.4 只用此路径 |
| `GateContext` (P3.3) | daemon 填充 4 字段 |
| `GateRouter::route()` (P3.3) | daemon 在 Err 分支调用 |
| `CriteriaRegistry::defaults()` / `with_overrides()` (P3.3) | main.rs 用 with_overrides(config.gate_criteria_path)，文件不存在 → defaults，YAML 坏 → exit(1) |
| `GateAction` (P3.3) | daemon match 三个变体 |
| `max_global_retries` (P3.3) | GateRouter::new(registry, MAX_TASK_RETRIES) |

---

## 9. 常量

```rust
/// Maximum task retries enforced by GateRouter global cap.
const MAX_TASK_RETRIES: u32 = 5;
```

`DEFAULT_TASK_TIMEOUT` 和 `PERMISSION_TIMEOUT` 不变。

---

## 10. 不做清单

| 约束 | 状态 |
|------|:---:|
| 不修改 error.rs（不给 AgentError 加 provider_error 字段） | ✅ |
| 不接入 from_provider_error() | ✅ — Phase 4 |
| 不修改 gate.rs（GateAction/CriteriaRegistry/GateRouter 签名不变） | ✅ |
| 不修改 IPC 协议 | ✅ |
| 不修改 SQLite schema | ✅ |
| 不修改 types.rs | ✅ |
| 不修改 agent/prompt.rs、agent/state.rs、agent/prompt_sources.rs | ✅ |
| 不实现 SwitchAgent 分发 | ✅ — P3.5+ |
| 不实现 SemanticCheck | ✅ — P3.5+ |
| 不发送中间 TaskError/TaskDone（lifecycle DB 写仍执行） | ✅ |
| gate YAML 坏配置不静默降级 | ✅ — exit(1) |
| 不创建 pipeline.sqlite / TaskStatus | ✅ |
| 不实现 system.ack / event replay / disk queue | ✅ |

---

## 11. 预期验证结果

```
cargo fmt --all -- --check               ✅
cargo test --workspace                   ✅ ~333 passed (+6 gate_daemon)
cargo clippy --workspace -- -D warnings  ✅
```
