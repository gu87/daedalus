# P3.1a 实现完成报告：SourceProvider 框架 + 文件型 Memory 注入

> 基于 Phase 2 收口状态（commit `ddd7fae`），按 REVIEW.md 方案实现。

---

## 1. 修改文件清单（4 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/agent/prompt_sources.rs` | **新增** | `SourceProvider` trait + `MemoryPaths` + 9 个 Provider + shared parser |
| `daedalusd/src/agent/prompt.rs` | 修改 | 重构为 Provider 链；删除 `load_soul_md`/`load_skills`；保留 `load_agent_section` |
| `daedalusd/src/agent/mod.rs` | 修改 | + `pub mod prompt_sources` |
| `daedalusd/tests/prompt.rs` | **新增** | 28 个 Provider 单元/集成测试 |

---

## 2. 未触碰模块确认

| 模块 | 状态 |
|------|:---:|
| `db/registry` | ✅ 零变更 |
| `types.rs` | ✅ 零变更 |
| `config.rs` / `DaedalusConfig` | ✅ 零变更 |
| `ipc/` | ✅ 零变更 |
| `llm/` | ✅ 零变更 |
| `tools/` | ✅ 零变更 |
| `daemon.rs` | ✅ 零变更 |

---

## 3. 验证结果

```
cargo fmt --all -- --check               ✅ 通过
cargo test --test prompt                 ✅ 29 passed
cargo test --workspace                   ✅ 290 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅ 通过
```

> `long_line_returns_error_and_closes` 是预存问题（P1.2 引入），本次 P3.1a 未触碰 `ipc/server.rs`。

### 测试分布

| 测试套 | passed | 变化 |
|--------|-------:|:---:|
| lib unit | 171 | +2（prompt_sources 内部 tokenize/frontmatter 单测） |
| prompt integration | **29** | **新增** |
| agent_loop integration | 20 | — |
| db_registry integration | 15 | — |
| full_dispatch integration | 5 | — |
| permission integration | 13 | — |
| protocol integration | 6 | — |
| provider integration | 26 | — |
| tool_registry integration | 5 | — |
| **合计** | **290** | **+29** |

### 新增测试（29 个）

- SoulProvider: present / missing
- MemoryProvider: present / missing
- UserProvider: missing
- PreferencesProvider: valid / bad JSON / missing
- FeedbackProvider: valid / bad JSON / missing
- ProjectContextProvider: valid / bad JSON / missing
- AuthorityMapProvider: valid / bad YAML / missing
- AgentConfigProvider: valid / missing agent
- SkillsProvider: dir missing / no match / match / stopwords / stable sort top 5（断言包含 Skill 0..4、不含 Skill 5..9）
- MemoryPaths::with_home: 2 tests
- Full chain: labels / optional missing / order stable
- load_agent_section: still works

---

## 4. 不实现（延后 P3.1b+）

- AgentHistoryProvider + db/registry::list_recent_runs
- ErrorCode enum / Gate / CriteriaRegistry / Monitor / FeedbackIngestor
- TaskStatus / pipeline.sqlite
- system.ack / event replay / ledger / disk queue
- event_id 生成 / 单调性
- HTTP API / UI / cc-haha
