# P2.7 返修完成报告：Phase 2 跨模块验收与收口

> 基于 P2.1–P2.6 全部 [DONE] 状态，含 Codex 审查返修。

---

## 1. 返修项

| # | 问题 | 修复 |
|:--|------|------|
| 1 | 缺少真实权限往返集成测试 | 新增 `full_dispatch_permission_roundtrip` |
| 2 | 缺少 task.error 集成测试 | 新增 `full_dispatch_task_error_provider_fatal` |
| 3 | Python `test_dispatch_success_response` 期待 stream 直接返回 | 改为 serv 先发 stream 再发 done，断言 `result["streams"][0]` + `result["done"]` |
| 4 | `asyncio.timeout` 不兼容 Python 3.9 | 改为 `asyncio.wait_for` |

---

## 2. 修改文件清单（对齐 git status）

| 文件 | 操作 |
|------|:---:|
| `Cargo.lock` | 自动更新（+ uuid） |
| `daedalusd/Cargo.toml` | + `uuid` |
| `daedalusd/src/lib.rs` | + `pub mod daemon` |
| `daedalusd/src/daemon.rs` | **新增** |
| `daedalusd/src/agent/loop.rs` | `LifecycleContext` + `req_id` |
| `daedalusd/src/ipc/control.rs` | `route()` + `ctx`；TaskDispatch → spawn_task |
| `daedalusd/src/ipc/peer.rs` | ctx 贯通 |
| `daedalusd/src/ipc/server.rs` | + `run_with_context` / `run_with_listener` |
| `daedalusd/src/main.rs` | `DaemonContext` + `run_with_context()` |
| `daedalusd/tests/full_dispatch.rs` | **新增**（5 个集成测试） |
| `daedalusd/tests/agent_loop.rs` | `LifecycleContext` + `req_id`（5 处） |
| `daedalus-orch/daedalus/orch/client.py` | `dispatch()` 循环到终态 + `asyncio.wait_for` |
| `daedalus-orch/daedalus/orch/exceptions.py` | + `from __future__ import annotations` |
| `daedalus-orch/tests/test_client.py` | `test_dispatch_success_response` 改为 stream+done |
| `scripts/smoke-phase2.sh` | **新增** |
| `REVIEW.md` | P2.7 方案 + 返修 + 完成报告 |
| `PLANS.md` | P2.7 [DONE] + 验证结果 |

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 259 passed, 0 failed, 1 skipped
cargo clippy --workspace -- -D warnings  ✅ 通过
pytest daedalus-orch/tests/test_client.py  ✅ 17 passed, 3 skipped
```

### 测试分布

| 测试套 | passed |
|--------|-------:|
| lib unit | 169 |
| agent_loop integration | 20 |
| db_registry integration | 15 |
| full_dispatch integration | **5** |
| permission integration | 13 |
| protocol integration | 6 |
| provider integration | 26 |
| tool_registry integration | 5 |
| **Rust 合计** | **259** |
| **pytest** | **17** |

唯一跳过：`long_line_returns_error_and_closes`（预存）+ 3 个 CLI/daemon 集成（需二进制文件）。

### full_dispatch 测试明细（5 个）

| # | 场景 | 断言 |
|:--|------|------|
| 1 | task.dispatch → task.done（无 tool call） | outbox.status, event_id=None |
| 2 | event_id 透传 | ev-123 原样带回 |
| 3 | DB lifecycle | status='done', completed_at + outbox_json |
| 4 | **permission 往返** | permission.request req_id 匹配 → response approved → task.done → DB done |
| 5 | **task.error** | ProviderFatal → task.error, error_taxonomy='provider_fatal', DB error |
