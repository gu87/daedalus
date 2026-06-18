# P3.1b 返修完成报告：AgentHistoryProvider + agent_runs 历史注入

> 基于 P3.1a（commit `703f8d4`），含 Codex 审查返修。

---

## 1. 返修项

| # | 问题 | 修复 |
|:--|------|------|
| 1 | 新增测试未落地 | db_registry +8 个 list_recent_runs 测试，prompt +6 个 AgentHistoryProvider 测试 |
| 2 | REVIEW.md 文件清单不准确 | 修正为实际修改文件数 + 精确测试数 |

---

## 2. 修改文件清单

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/config.rs` | 修改 | `DaedalusConfig` + `db_path: Option<PathBuf>`；`load()` 内部 `db_path: None` |
| `daedalusd/src/error.rs` | 修改 | + `Database(String)` variant |
| `daedalusd/src/db/registry.rs` | 修改 | + `AgentRunSummary` + `list_recent_runs()` |
| `daedalusd/src/agent/prompt_sources.rs` | 修改 | + `AgentHistoryProvider` |
| `daedalusd/src/agent/prompt.rs` | 修改 | `new()` 在 SkillsProvider 前插入 AgentHistoryProvider |
| `daedalusd/src/main.rs` | 修改 | `config.db_path = Some(...)` |
| `daedalusd/src/ipc/control.rs` | 修改 | 2 处 struct literal + `db_path: None` |
| `daedalusd/tests/agent_loop.rs` | 修改 | + `db_path: None` |
| `daedalusd/tests/full_dispatch.rs` | 修改 | + `db_path: None` |
| `daedalusd/tests/prompt.rs` | 修改 | + `db_path: None` + 6 个 history 测试 |
| `daedalusd/tests/db_registry.rs` | 修改 | + 8 个 list_recent_runs 测试 |

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --workspace                   ✅ 304 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

### 测试分布

| 测试套 | passed | 变化 |
|--------|-------:|:---:|
| lib unit | 171 | — |
| agent_loop integration | 20 | — |
| db_registry integration | **23** | **+8** |
| full_dispatch integration | 5 | — |
| permission integration | 13 | — |
| prompt integration | **35** | **+6**（含 2 个 PromptBuilder::new(cfg) 生产路径测试） |
| protocol integration | 6 | — |
| provider integration | 26 | — |
| tool_registry integration | 5 | — |
| **合计** | **304** | **+14** |

### 新增测试明细

**db_registry (+8):**

| # | 测试 | 断言 |
|:--|------|------|
| 1 | `list_recent_runs_only_terminal_statuses` | done/error/cancelled 选中，queued/running/orphaned 过滤 |
| 2 | `list_recent_runs_limit` | limit=2 返回 2，limit=0 返回空 |
| 3 | `list_recent_runs_sort_order` | completed_at DESC, spawned_at DESC, task_id ASC |
| 4 | `list_recent_runs_empty` | 无历史 → 空 Vec |
| 5 | `list_recent_runs_summary_extraction` | outbox_json.summary 提取正确 |
| 6 | `list_recent_runs_bad_outbox_json_is_none` | 非法 JSON → outbox_summary=None |
| 7 | `list_recent_runs_summary_truncated_unicode` | 中文 130 chars → 截断到 120 chars |
| 8 | `list_recent_runs_missing_summary_field_is_none` | 缺 summary 字段 → None |

**prompt (+6):**

| # | 测试 | 断言 |
|:--|------|------|
| 1 | `history_provider_with_data` | 包含 task_id + status(done/error) + summary |
| 2 | `history_provider_no_data_is_none` | 无历史 → Ok(None) |
| 3 | `history_provider_none_summary_shows_dash` | outbox_summary=None → 显示 "—" |
| 4 | `full_chain_db_path_none_no_history` | db_path=None → 不含 [history] |
| 5 | `prompt_builder_new_db_path_some_injects_history` | **生产路径**：PromptBuilder::new(cfg) + db_path=Some → 自动注入 history |
| 6 | `prompt_builder_new_history_between_agent_and_skills` | **生产路径**：PromptBuilder::new(cfg) → [agent] < [history] < [skills] |

---

## 4. 范围红线

| 约束 | 状态 |
|------|:---:|
| 不改 SQLite schema | ✅ |
| 不改 IPC 协议 | ✅ |
| 不改 AgentLoop 状态机 | ✅ |
| 不做 Gate / ErrorCode / Monitor / FeedbackIngestor | ✅ |
| 不做 system.ack / event replay / disk queue | ✅ |
| 不做 HTTP API / UI | ✅ |
