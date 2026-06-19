# P5.3b 实现完成报告：Pipeline ↔ daemon/Gate/Durable Events 接线

> 基于 P5.3a + P5.2。commit `2f4d832`, fixup 待提交。

---

## 1. 核心交付

| 能力 | 状态 |
|------|:---:|
| 首次 dispatch build+insert 成功 → insert_task(Created→Dispatched→Running) | ✅ |
| TaskDone → WaitingForVerification | ✅ |
| HardStop → Failed | ✅ |
| SwitchAgent 成功 → Running→Blocked→Dispatched→Running | ✅ |
| SwitchAgent build/insert 失败 → Failed | ✅ |
| AutoRevision build/insert 失败 → Failed | ✅ |
| orphan scan → tasks→Failed | ✅ |
| **12 个 update_pipeline 调用全部遵守 send_reliable_event → update_pipeline 顺序** | ✅ |

---

## 2. 测试

```
cargo test --test pipeline_daemon  ✅ 4 passed
cargo test --test db_registry      ✅ 23 passed
cargo clippy --workspace           ✅ clean
```

---

## 3. 不做清单

| 约束 | 状态 |
|------|:---:|
| 不改 pipeline/status.rs | ✅ |
| 不改 pipeline/db.rs API | ✅ |
| 不改 Gate 路由逻辑 | ✅ |
| 不改 IPC/HTTP | ✅ |
| 不做 Pipeline 执行引擎 / Ω-Agent / MCP | ✅ |
