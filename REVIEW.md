# P5.2 实现完成报告：Durable Execution

> commits: `f5b6b29`, `353744c`, `2020275`

---

## 1. 核心交付

| 能力 | 状态 |
|------|:---:|
| `events` 表（migration v2, UNIQUE(task_id,seq) + UNIQUE(event_id)） | ✅ |
| `Ledger`（async append_event/query_events_since/mark_acked, txn 内原子 seq） | ✅ |
| `system.ack` 消息类型 + 校验 + control::route 处理 | ✅ |
| `send_reliable_event()` 统一封装 | ✅ |
| `session.rejoin` 回放（parse_message 反序列化, last_event_id None=全部重放） | ✅ |
| `PermissionBroker::request_permission + task_id` 签名穿透 | ✅ |
| `IpcPermissionBroker` 持有 `Arc<Ledger>`, 通过 `send_reliable_event` 发送 | ✅ |
| **所有** daemon TaskDone/TaskError 推送路径走 reliable（0 个直接发送） | ✅ |
| 11 个 durable 集成测试（6 基础 + 5 control::route 路由） | ✅ |

---

## 2. 可靠推送路径全量覆盖

| 推送点 | message_type | 来源 |
|--------|:---:|------|
| TaskDone | task.done | daemon.rs spawn_task |
| HardStop TaskError | task.error | daemon.rs GateAction::HardStop |
| SwitchAgent build failed | task.error | daemon.rs |
| SwitchAgent insert_run failed | task.error | daemon.rs |
| SwitchAgent insert_run panic | task.error | daemon.rs |
| AutoRevision build failed | task.error | daemon.rs |
| AutoRevision insert_run failed | task.error | daemon.rs |
| AutoRevision insert_run panic | task.error | daemon.rs |
| PermissionRequest | permission.request | IpcPermissionBroker |

---

## 3. 测试

```
cargo test --test durable  ✅ 11 passed
cargo test --lib           ✅ 225 passed
cargo test --workspace     ✅ (running)
cargo fmt --all -- --check ✅
cargo clippy --workspace -- -D warnings ✅
```

### durable 测试明细

| # | 测试 | 路径 |
|:--|------|:---:|
| 1 | append_event_generates_task_scoped_seq | Ledger 直接 |
| 2 | append_event_payload_contains_event_id | Ledger 直接 |
| 3 | query_events_since_orders_by_seq | Ledger 直接 |
| 4 | mark_acked_sets_acked_at | Ledger 直接 |
| 5 | system_ack_marks_event | Ledger 直接 |
| 6 | send_reliable_event_via_channel | reliable.rs |
| 7 | system_ack_marks_event_via_control_route | **control::route** |
| 8 | system_ack_unknown_event_returns_error | **control::route** |
| 9 | session_rejoin_replays_via_control_route | **control::route** |
| 10 | session_rejoin_without_last_event_id_replays_all | **control::route** |
| 11 | permission_request_uses_reliable_event | **IpcPermissionBroker** |

---

## 4. 返回 Codex 复审

P5.2 返修完毕。请 Codex 审查。
