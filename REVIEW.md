# Codex 审查文档

> P2.5 实现方案 v3：权限转发 + IPC 双向 Session + 可靠性协议预留

---

## 范围

P2.5 做三件事：
1. **IPC 双向 Session** — daemon 能通过有界 writer channel 主动推送消息
2. **permission.request / permission.response 往返** — IpcPermissionBroker 通过 IPC 请求权限并等待响应
3. **event_id / session.rejoin 协议预留** — 字段定义 + 解析，不做生成/回放

**不做**：`task.dispatch → AgentLoop → task.done` 端到端 → P2.7。

---

## 1. 文件边界

| 文件 | 操作 | 内容 |
|------|:---:|------|
| `daedalusd/src/ipc/session.rs` | **新增** | `Session`, `SessionState`, `PendingPerm` |
| `daedalusd/src/ipc/peer.rs` | **重写** | `Session::spawn()` — reader/writer loops |
| `daedalusd/src/ipc/control.rs` | **修改** | 异步路由 + `permission.response` → pending map 匹配 |
| `daedalusd/src/ipc/protocol.rs` | **修改** | 已知类型集合 + `SessionRejoin` |
| `daedalusd/src/ipc/mod.rs` | **修改** | +`session` 子模块 |
| `daedalusd/src/agent/permission.rs` | **扩展** | +`IpcPermissionBroker`；trait 新增 `req_id` 参数 |
| `daedalusd/src/types.rs` | **修改** | 六种 Phase 2 消息 +`event_id`；`SessionRejoin` struct |
| `daedalusd/tests/permission.rs` | **新增** | broker/session 集成测试（不接 AgentLoop） |
| `daedalus-orch/daedalus/orch/client.py` | **修改** | +`dispatch()` 雏形 |
| `daedalus-orch/tests/test_client.py` | **扩展** | permission handler 协议测试 |

---

## 2. Session / SessionState / PendingPerm（v3 修正）

```rust
// daedalusd/src/ipc/session.rs

pub struct Session {
    pub writer_tx: mpsc::Sender<Message>,
    pub state: Arc<SessionState>,
    reader_handle: JoinHandle<()>,
    writer_handle: JoinHandle<()>,
}

pub struct SessionState {
    pub pending_permissions: Mutex<HashMap<String, PendingPerm>>,
    pub shutdown: CancellationToken,
    next_perm_id: AtomicU64,
}

pub struct PendingPerm {
    pub req_id: String,                           // 关联的 task req_id
    pub sender: oneshot::Sender<PermissionDecision>,
}
```

**修正点**：`pending_permissions` 的 value 从裸 `oneshot::Sender` 改为 `PendingPerm` struct，携带 `req_id` 和 `sender`。Response 匹配时先校验 `pending.req_id == response.req_id`。

---

## 3. 双向 Session 生命周期

### Session::spawn(stream)

```
1. 创建 Arc<SessionState> { pending_permissions: Mutex::new(HashMap::new()), shutdown: CancellationToken::new(), next_perm_id: AtomicU64::new(1) }
2. 创建 mpsc::channel::<Message>(64)
3. spawn reader loop
4. spawn writer loop
5. 返回 Session { writer_tx, state, reader_handle, writer_handle }
```

### Reader loop

```
loop {
    match read_line() {
        Ok(Some(text)) => {
            parse → control::route(&state, msg, &writer_tx)
        }
        Ok(None) | Err(_) => {
            // EOF or I/O error → shutdown
            state.shutdown.cancel();
            drain_pending(&state);
            break;
        }
    }
}
```

### Writer loop

```
loop {
    match writer_rx.recv().await {
        Some(msg) => {
            if write_all(json_line).is_err() {
                // Write failure → shutdown
                state.shutdown.cancel();
                drain_pending(&state);
                break;
            }
        }
        None => break,  // channel closed
    }
}
```

### drain_pending（v3 修正）

```
fn drain_pending(state: &SessionState) {
    let mut map = state.pending_permissions.lock().unwrap();
    let drained: Vec<(String, PendingPerm)> = map.drain().collect();
    drop(map);
    // drop sender → receiver gets RecvError::Closed
    // → IpcPermissionBroker maps to Cancelled
    for (_, perm) in drained {
        drop(perm.sender);
    }
}
```

**修正点**：reader EOF + writer 写失败**都**主动调用 `state.shutdown.cancel()` + `drain_pending()`。不依赖 "drop writer_tx 关闭 reader sender clone" 这种不可靠的间接路径。

### 有界 channel

`mpsc::channel::<Message>(64)`。满时 sender.send().await 阻塞，自然反压。

---

## 4. PermissionBroker trait（v3 扩展 req_id）

```rust
#[async_trait]
pub trait PermissionBroker: Send + Sync {
    async fn request_permission(
        &self,
        agent_id: &str,
        req_id: &str,
        tool_call: &ToolCall,
    ) -> Result<PermissionDecision, AgentError>;
}
```

### FakePermissionBroker（同步适配）

```rust
pub struct FakePermissionBroker {
    pub decision: PermissionDecision,
    pub delay: Option<Duration>,
}

impl PermissionBroker for FakePermissionBroker {
    // 忽略 req_id，直接返回预设值
}
```

### IpcPermissionBroker

```rust
pub struct IpcPermissionBroker {
    state: Arc<SessionState>,
    writer_tx: mpsc::Sender<Message>,
    default_timeout: Duration,  // 默认 30s
}

impl PermissionBroker for IpcPermissionBroker {
    async fn request_permission(
        &self, agent_id: &str, req_id: &str, tool_call: &ToolCall,
    ) -> Result<PermissionDecision, AgentError> {
        // 1. 生成 permission_id: "perm-{next_perm_id.fetch_add(1)}"
        // 2. 创建 oneshot::channel()
        // 3. lock → insert PendingPerm { req_id, sender }
        // 4. writer_tx.send(permission.request{permission_id, req_id, ...})
        // 5. tokio::select! { oneshot, timeout, shutdown }
        // 6. 取结果，从 map remove
    }
}
```

---

## 5. Permission 超时与错误语义（v3）

| 场景 | 返回值 | 理由 |
|------|--------|------|
| oneshot → Approved | `Ok(Approved)` | 正常 |
| oneshot → Denied | `Ok(Denied)` | 用户拒绝 |
| `default_timeout` 到期 | **`Ok(Denied)`** | 安全优先，权限超时 = 拒绝 |
| `state.shutdown.cancelled()` | **`Err(Cancelled)`** | Session 关闭 |
| oneshot sender dropped (drain) | **`Err(Cancelled)`** | 连接断开 |

权限超时 ≠ 任务超时。`TaskTimeout` 只在 AgentLoop 外层 deadline 触发。

---

## 6. Pending Permission 生命周期

### 创建

1. `IpcPermissionBroker::request_permission()` → `state.next_perm_id.fetch_add(1)` → `"perm-42"`
2. 创建 `oneshot::channel()`
3. `state.pending_permissions.lock().insert("perm-42", PendingPerm { req_id, sender })`
4. `writer_tx.send(permission.request{permission_id: "perm-42", req_id, agent_id, tool, args})`

### Response 匹配（control::route）

```
收到 permission.response { permission_id, req_id, decision }:

1. lock state.pending_permissions
2. get(permission_id):
   - None → return system.error(detail: "unknown permission id 'perm-42'")
3. let perm = entry
4. if perm.req_id != response.req_id:
   → return system.error(detail: "req_id mismatch: expected '...', got '...'")
5. remove entry from map
6. release lock
7. perm.sender.send(decision)
```

### 重复 / 未知 / mismatch

| 情况 | 行为 |
|------|------|
| permission_id 不在 map | `system.error`（detail 含 permission_id） |
| permission_id 在 map，req_id 不匹配 | `system.error`（detail 含 expected/got） |
| 同一 permission_id 发两次 | 第一次 → 处理 + remove；第二次 → 不在 map → error |
| shutdown / 连接断开 | drain_pending → drop all senders → broker 收到 Cancelled |

### permission_id 生成

`AtomicU64` 自增，格式 `perm-{n}`，零外部依赖。

---

## 7. AgentLoop req_id 策略（v3 修正）

**P2.5 不将 IpcPermissionBroker 接入真实 AgentLoop。** 理由：
- AgentLoop 的 `req_id` 来自 `task.dispatch` 消息，而 P2.5 不做 task.dispatch 路由
- 强制接入会导致空字符串 req_id 或 unsafe 假设

### P2.5 范围

- `PermissionBroker` trait 的 `req_id` 参数存在，但 `AgentLoop` 的调用点继续使用 `FakePermissionBroker`（忽略 req_id）
- `IpcPermissionBroker` 的正确性在 `tests/permission.rs` 中通过 fake session/peer 集成测试验证，不依赖 AgentLoop
- AgentLoop ↔ IpcPermissionBroker 的真实 wiring 留到 **P2.7**，届时 `req_id` 从 `task.dispatch` 消息中获取

### FakePermissionBroker

```rust
// 适配新 trait 签名，忽略 req_id
impl PermissionBroker for FakePermissionBroker {
    async fn request_permission(
        &self, _agent_id: &str, _req_id: &str, _tool_call: &ToolCall,
    ) -> Result<PermissionDecision, AgentError> {
        // 保持现有行为
    }
}
```

### AgentLoop 调用点（P2.4，P2.5 不动）

```rust
// 传入占位 req_id，因为 FakeBroker 忽略它
self.permission_broker
    .request_permission(&self.agent_id, "", &tool_call)  // "" 仅为满足签名
    .await
```

**禁止发出空 req_id 的 permission.request。** `IpcPermissionBroker` 在构造时验证 `req_id` 非空，否则返回 AgentError。

---

## 8. SystemErrorCode 使用规则（v3 修正）

**P2.5 不新增 SystemErrorCode 变体。** 所有新错误场景使用已有 `SystemErrorCode::InvalidMessage`，通过 `detail` 区分：

| 场景 | error 字段 | detail 示例 |
|------|-----------|------------|
| `task.dispatch` 未实现 | `InvalidMessage` | `"task.dispatch not implemented in Phase 2"` |
| `session.rejoin` 未实现 | `InvalidMessage` | `"session.rejoin replay not implemented in Phase 2"` |
| 未知 permission_id | `InvalidMessage` | `"unknown permission id 'perm-99'"` |
| req_id mismatch | `InvalidMessage` | `"req_id mismatch: expected 'r1', got 'r2'"` |

`SystemErrorCode` 枚举保持 Phase 1 四个变体不变。

---

## 9. SessionRejoin / event_id 字段合同（v3 补全）

### SessionRejoin

```rust
// daedalusd/src/types.rs
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionRejoin {
    pub ts: String,
    pub req_id: String,                       // 非空
    pub task_id: String,                      // 非空
    pub last_event_id: Option<String>,        // None 合法，Some("") → invalid
}
```

校验规则：
- `req_id` 非空
- `task_id` 非空
- `last_event_id` 为 `Some("")` → `InvalidMessage`（detail: "last_event_id must not be empty if present"）
- `last_event_id` 为 `None` → 合法（首次连接，无历史）

### event_id

六种 Phase 2 消息 struct 增加 `event_id: Option<String>`。

P2.5 行为：
- 解析 + 序列化 + 协议往返测试
- 不生成、不保证单调、不做去重
- `Some("")` → `InvalidMessage`（与 last_event_id 一致）

---

## 10. 测试计划（v3）

| # | 测试 | 类型 | 方法 |
|:---:|------|:---:|------|
| 1 | FakePermissionBroker 保持 | Rust unit | 已有测试（适配新 trait 签名后） |
| 2 | IpcPermissionBroker Approved | Rust 集成 | fake session → send permission.response Approved → Ok(Approved) |
| 3 | IpcPermissionBroker Denied | Rust 集成 | fake session → send Denied → Ok(Denied) |
| 4 | Permission timeout → Denied | Rust 集成 | 不写 response → timeout 到期 → **Ok(Denied)** |
| 5 | Connection lost → Cancelled | Rust 集成 | drain_pending → broker 返回 Err(Cancelled) |
| 6 | req_id mismatch → system.error | Rust 集成 | 写 mismatch req_id → daemon 返回 system.error InvalidMessage |
| 7 | Duplicate perm_id → error | Rust 集成 | 同 perm_id 发两次 → 第二次返回 error |
| 8 | Unknown perm_id → error | Rust 集成 | 写不存在的 perm_id → daemon 返回 error |
| 9 | Python handler Approved/Denied | Python test | mock server → permission.request → handler → response |
| 10 | Python no handler → default denied | Python test | 不传 handler → 自动 denied |
| 11 | event_id 字段往返 | Rust unit | 消息带 event_id → 序列化/解析保留值 |
| 12 | event_id Some("") → invalid | Rust unit | 空字符串拒绝 |
| 13 | session.rejoin → system.error | Rust 集成 | 发 rejoin → daemon 返回 InvalidMessage "not implemented" |
| 14 | session.rejoin last_event_id Some("") → invalid | Rust unit | 空 last_event_id 拒绝 |

---

## 11. 不实现清单

| 项目 | 归口 |
|------|:---:|
| `task.dispatch` 路由到 AgentLoop | P2.7 |
| AgentLoop ↔ IpcPermissionBroker wiring | P2.7 |
| topic / subscription | Phase 3 |
| permission cache / rate limit | Phase 3 |
| ledger / replay / disk queue | Phase 3 |
| event_id 生成 / 单调保证 / 去重 | Phase 3 |
| Gate / TaskStatus / ErrorCode | Phase 3 |
| 多 Agent 并发权限 | Phase 3 |
| 自动重连 | Phase 3 |
| `system.ack` | Phase 3 |
| `session.rejoin` 回放逻辑 | Phase 3 |
| 新增 `SystemErrorCode` 变体 | Phase 3 |

---

等待 Codex 审查。
