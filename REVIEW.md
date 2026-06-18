# P5.2 实现完成报告：Durable Execution

> commits: `f5b6b29`, `353744c`, `2020275`, `c8c2340`, `a2dcbff`, `8691a49`

---

## 1. 4 项 Codex 二次返修 — 全部完成

| # | 要求 | 实现 |
|---|------|------|
| 1 | parse_event_id 校验 task 前缀 | `protocol::parse_event_id()` 校验 `{task_id}:{seq}` 格式；session.rejoin 校验 task_id 匹配 |
| 2 | system.ack event_id 格式校验 | `validate_message` 中调用 parse_event_id；格式错误 → ProtocolError |
| 3 | permission 测试真实 temp ledger | `make_ledger(&dir)` helper；10 个测试补 `TempDir` |
| 4 | 并发 append 安全 | `EXCLUSIVE` transaction 序列化同 task 写入 |

---

## 2. 实现要点

- **event_id 格式**: `{task_id}:{seq}`（task-scoped）
- **events 表**: migration v2, `UNIQUE(task_id, seq)` + `UNIQUE(event_id)`
- **Ledger**: async API, `EXCLUSIVE` txn 内原子 seq 分配 + INSERT
- **send_reliable_event()**: 统一封装（生成 event_id → 写 ledger → 发送）
- **daemon 全部推送路径** (9 条) 走 reliable send
- **session.rejoin**: last_event_id None=全部回放, Some=校验 task 前缀→seq>查询→parse_message 反序列化
- **system.ack**: req_id 必填, event_id 格式校验, rows>0 幂等, unknown→system.error
- **PermissionBroker +task_id**: 签名穿透（AgentLoop 不改状态机）

---

## 3. 测试结果

```
cargo test --test durable    ✅ 15 passed
cargo test --test permission ✅ 13 passed
cargo test --lib             ✅ 225 passed
cargo test --workspace       ✅ 434 passed, 0 failed
                               (full_dispatch 5 tests pre-existing hang, not P5.2)
cargo clippy                 ✅ clean
cargo fmt                    ✅ clean
```

### durable 15 个测试

| 测试 | 路径 |
|------|:---:|
| append_event_generates_task_scoped_seq | Ledger |
| append_event_payload_contains_event_id | Ledger |
| query_events_since_orders_by_seq | Ledger |
| mark_acked_sets_acked_at | Ledger |
| system_ack_marks_event | Ledger |
| send_reliable_event_via_channel | reliable.rs |
| system_ack_marks_event_via_control_route | **control::route** |
| system_ack_unknown_event_returns_error | **control::route** |
| session_rejoin_replays_via_control_route | **control::route** |
| session_rejoin_without_last_event_id_replays_all | **control::route** |
| permission_request_uses_reliable_event | IpcPermissionBroker |
| session_rejoin_rejects_mismatched_task_id | **control::route** |
| session_rejoin_rejects_malformed_last_event_id | **control::route** |
| system_ack_malformed_event_id_rejected | **control::route** |
| append_event_concurrent_same_task_gets_unique_seq | concurrent |

---

## 4. 返回 Codex 复审

P5.2 二次返修完毕。请 Codex 审查。
