# P4.2 实现完成报告：Task 可观测性 API

> 基于 P4.1（commit `7f922cb`），新增只读任务查询端点。

---

## 1. 修改文件清单（5 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/db/registry.rs` | 修改 | `AgentRunStatus` + `parse_status()`；+ `ListRunsFilter` + `ListRunEntry` + `list_runs()`（参数化 SQL，ORDER BY spawned_at DESC, run_id ASC） |
| `daedalusd/src/http/tasks.rs` | **新增** | `GET /api/tasks` + `GET /api/tasks/:run_id` handlers；`TasksListResponse`/`TaskDetailResponse`/`ErrorResponse` |
| `daedalusd/src/http/mod.rs` | 修改 | + `pub mod tasks` |
| `daedalusd/src/http/server.rs` | 修改 | 注册 `/api/tasks` + `/api/tasks/:run_id` 路由 |
| `daedalusd/tests/http_tasks.rs` | **新增** | 9 个集成测试 |

**未改**：main.rs、health.rs、daemon.rs、gate.rs、error.rs、loop.rs、IPC、schema

---

## 2. 核心行为

### 2.1 API 端点

```
GET /api/tasks?status=error&agent_id=test&limit=50
→ { "tasks": [{ run_id, agent_id, task_id, status, spawned_at, completed_at, error_taxonomy }] }

GET /api/tasks/:run_id
→ { run_id, agent_id, task_id, status, spawned_at, completed_at, heartbeat_at, error_taxonomy, parent_run_id, spawn_depth }
```

### 2.2 关键规则

- **只读**：`spawn_blocking` + 短连接，不写任何行
- **status 校验**：必须是合法 `AgentRunStatus` 变体或省略，非法 → 400
- **limit 校验**：1..=100，超出范围 → 400（不 clamp）
- **排序**：`ORDER BY spawned_at DESC, run_id ASC`（稳定 tie-breaker）
- **错误格式**：`{ "error": "..." }`
- **复用** `get_run()`：详情端点直接调用已有函数

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --test http_tasks             ✅ 9 passed
cargo test --workspace                   ✅ 403 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

### http_tasks 测试（9 个）

| # | 测试 | 断言 |
|:--|------|------|
| 1 | list_all_tasks | 6 runs |
| 2 | list_filter_by_status | ?status=error → 2 error runs |
| 3 | list_filter_by_agent | ?agent_id=agent-0 → 4 runs |
| 4 | list_default_limit | 60 rows → 50 returned |
| 5 | list_invalid_status_400 | ?status=bogus → 400 |
| 6 | list_invalid_limit_400 | ?limit=0 / ?limit=200 → 400 |
| 7 | detail_existing_run | 200 + 完整字段 |
| 8 | detail_not_found_404 | 404 |
| 9 | empty_list | 空 DB → [] |

---

## 4. 返回 Codex 复审

P4.2 实现完毕，403 测试全过。请 Codex 审查。
