# P5.2 实现方案（Codex 二次修订版）：Durable Execution

> 基于 P5.1（commit `bb9160a`）。
> 已拍板：task-scoped event_id、SQLite events 表、仅标记 ack 不重发、
> `ipc/reliable.rs` 统一封装、PermissionBroker task_id 穿透。

---

## 0. 本次修订要点

| # | 修订 | 说明 |
|---|------|------|
| 1 | append_event API | `build_msg` 闭包在 txn 内调用，**payload_json 包含最终 event_id** |
| 2 | session.rejoin payload | 用 `protocol::parse_message()` 反序列化，失败返回 system.error |
| 3 | 测试 | 新增 `replayed_payload_contains_generated_event_id` |

---

## 1. 目标

1. 新增 `events` 表（migration v2），含 `UNIQUE(task_id, seq)`
2. 新增 `system.ack` 消息类型（Client → Server）
3. 新增 `db/ledger.rs` — async `append_event()` / `query_events_since()` / `mark_acked()`
4. 新增 `ipc/reliable.rs` — `send_reliable_event()` 统一封装
5. 实现 `session.rejoin` 回放（writer_tx 逐条推送，payload 反序列化复用 protocol::parse_message）
6. daemon.rs / permission.rs 推送点改用 `send_reliable_event()`

---

## 2. events 表 schema

```sql
CREATE TABLE events (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id       TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    event_id      TEXT NOT NULL UNIQUE,
    message_type  TEXT NOT NULL,
    payload_json  TEXT NOT NULL,
    acked_at      INTEGER NULL,
    created_at    INTEGER NOT NULL,
    UNIQUE(task_id, seq)
);
CREATE INDEX idx_events_task_seq ON events(task_id, seq);
```

---

## 3. Ledger API（async，build_msg 在 txn 内调用）

```rust
// db/ledger.rs

pub struct StoredEvent {
    pub event_id: String,
    pub message_type: String,
    pub payload_json: String,
    pub seq: u32,
}

impl Ledger {
    pub fn new(db_path: &Path) -> Self;

    /// Atomically allocate seq, call build_msg(event_id), serialise the result,
    /// and INSERT in one transaction.  Returns (event_id, final_message).
    ///
    /// `message_type` is stored in the events table (e.g. "task.done").
    /// The payload_json stored already contains the generated event_id
    /// because build_msg is called inside the txn.
    pub async fn append_event(
        &self,
        task_id: &str,
        message_type: &str,
        build_msg: impl FnOnce(String) -> Message + Send + 'static,
    ) -> Result<(String, Message)>;

    /// Return events with seq > after_seq for task_id, ordered by seq ASC.
    /// Pass `None` as after_seq to replay all events from the beginning (seq >= 0).
    pub async fn query_events_since(
        &self, task_id: &str, after_seq: Option<u32>,
    ) -> Result<Vec<StoredEvent>>;

    /// Mark an event as acknowledged by the client.
    pub async fn mark_acked(&self, event_id: &str, acked_at: i64) -> Result<()>;
}
```

`append_event` 内部 txn（在 spawn_blocking 内执行）：
```sql
BEGIN;
SELECT COALESCE(MAX(seq) + 1, 0) FROM events WHERE task_id = ?;  -- → seq
-- event_id = "{task_id}:{seq}"
-- msg = build_msg(event_id)             ← in-txn, payload_json 含 event_id
-- payload_json = protocol::serialize_message(&msg)
INSERT INTO events (task_id, seq, event_id, message_type, payload_json, created_at)
VALUES (?, ?, ?, ?, ?, ?);
COMMIT;
-- return (event_id, msg)
```

所有方法通过 `spawn_blocking` + 短连接实现。

---

## 4. send_reliable_event()

```rust
// ipc/reliable.rs

pub async fn send_reliable_event(
    writer_tx: &mpsc::Sender<Message>,
    ledger: &Ledger,
    task_id: &str,
    message_type: &str,
    build_msg: impl FnOnce(String) -> Message + Send + 'static,
) -> Result<(), SendError>
```

内部：`ledger.append_event(task_id, message_type, build_msg).await` → 获取 `(event_id, final_msg)` → `writer_tx.send(final_msg).await`。

---

## 5. system.ack

```
Client → Server:  { "type": "system.ack", "ts": "...", "event_id": "task-1:3",
                    "req_id": "ack-1" }

Rules:
- event_id 格式必须是 "{task_id}:{seq}"，空字符串拒绝
- req_id 必填，方便错误响应关联
- 成功 mark_acked → 不返回响应（静默）
- unknown event_id → 返回 system.error（req_id 关联）
- 已 ack 的 event_id 再次 ack → 静默成功（幂等）
```

---

## 6. session.rejoin 回放（含 payload 反序列化）

```
Client → Server:  SessionRejoin { task_id: "task-1", last_event_id, req_id }

last_event_id 语义：
  - Some("task-1:3") → parse seq=3，查询 seq > 3 的事件
  - None → 从头回放该 task 的全部事件，查询 seq >= 0

SQL：
  - Some(seq): WHERE task_id = ? AND seq > ? ORDER BY seq ASC
  - None:      WHERE task_id = ? ORDER BY seq ASC

control.rs 分支：
  - parse last_event_id → Option<u32>
  - ledger.query_events_since(task_id, after_seq).await
  - for each StoredEvent:
      msg = protocol::parse_message(&payload_json)   ← P5.2 复用协议校验
      if parse fails:
        writer_tx.send(system.error(req_id, "failed to replay event {event_id}"))
        return None  ← 停止回放
      writer_tx.send(msg).await
  - return None
```

返回类型不改：仍为 `Option<Message>`。

---

## 7. PermissionBroker task_id 穿透（方案 A）

```rust
// agent/permission.rs — trait 签名变更
pub trait PermissionBroker: Send + Sync {
    async fn request_permission(
        &self, agent_id: &str, req_id: &str, tool_call: &ToolCall,
        task_id: &str,                    // ← P5.2 新增
    ) -> Result<PermissionDecision, AgentError>;
}
```

| 调用方 | 传值 |
|--------|------|
| AgentLoop::AwaitingPermission | `task_id = task_card.task_card_id` |
| FakePermissionBroker | 忽略 task_id |
| IpcPermissionBroker | 用 task_id 调 `send_reliable_event()` |

签名穿透，不改 Agent Loop 状态机逻辑。

---

## 8. 文件边界

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalusd/src/types.rs` | 修改 | + `SystemAck` + `Message::SystemAck` |
| `daedalusd/src/ipc/protocol.rs` | 修改 | `system.ack` 已知类型 + 校验 |
| `daedalusd/src/db/migrations.rs` | 修改 | migration v2：+ `events` 表 |
| `daedalusd/src/db/ledger.rs` | **新增** | `Ledger`（async append/query/mark_acked） |
| `daedalusd/src/ipc/reliable.rs` | **新增** | `send_reliable_event()` |
| `daedalusd/src/ipc/control.rs` | 修改 | `session.rejoin` 回放（含 parse_message）；`system.ack` 处理 |
| `daedalusd/src/daemon.rs` | 修改 | TaskDone/TaskError/TaskStream → `send_reliable_event()` |
| `daedalusd/src/agent/permission.rs` | 修改 | trait + `task_id`；IpcPermissionBroker → `send_reliable_event()` |
| `daedalusd/src/agent/loop.rs` | 修改 | AwaitingPermission 传入 task_id（仅签名穿透） |
| `daedalusd/tests/durable.rs` | **新增** | 9 个集成测试 |

**不改**：peer.rs、Gate 路由、Agent Loop 状态机逻辑、HTTP、SQLite 既有 agent_runs 语义

---

## 9. 测试计划（9 个）

| # | 测试 | 场景 | 断言 |
|:--|------|------|------|
| 1 | append_event_generates_task_scoped_seq | 同一 task 连续 append 2 次 | event_id = "t1:0", "t1:1" |
| 2 | append_event_payload_contains_event_id | append task.done | ledger payload_json 含 `"event_id":"t1:0"` |
| 3 | append_event_is_transactional_unique_seq | 同 task 同 seq 第二次 insert | 违反 UNIQUE(task_id, seq)，失败 |
| 4 | query_events_since_orders_by_seq | insert seq 0,3,1 → query since 0 | 返回 seq 1,3 按 ASC |
| 5 | mark_acked_sets_acked_at | insert → mark_acked | acked_at IS NOT NULL |
| 6 | system_ack_marks_event | 发送 valid SystemAck | events 表 acked_at 被设置 |
| 7 | system_ack_unknown_event_returns_error | 发送 unknown event_id | 返回 system.error |
| 8 | session_rejoin_replays_multiple_events | insert 3 条 → SessionRejoin(last=Some("t1:0")) | writer_rx 收到 seq 1,2；TaskDone.event_id == Some("t1:1") |
| 9 | session_rejoin_without_last_event_id_replays_all | insert 3 条 → SessionRejoin(last=None) | writer_rx 收到全部 3 条（seq 0,1,2） |

测试 2 同时覆盖：**replayed payload 反序列化后 event_id 正确**。

---

## 10. 不做清单

| 约束 | 状态 |
|------|:---:|
| 未 ack 超时自动重发 | ✅ |
| global event_id 序列 | ✅ |
| disk queue 文件存储 | ✅ |
| peer.rs 写 ledger | ✅ |
| 修改 Gate / Agent Loop 状态机 / HTTP | ✅ |
| 修改 SQLite agent_runs 语义 | ✅ |
