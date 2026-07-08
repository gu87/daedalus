# 测试报告

当前任务：Narrative Backend Phase 1g - check_knowledge 读取 knowledge.yaml

验收时间：2026-07-08 14:34 +0800

结论：PASS

验收范围复核：
- `git status --short --branch` 显示本轮工作树改动仅涉及 `daedalusd/src/tools/narrative.rs`、`daedalusd/tests/narrative_tools.rs`、`work/callbacks.md`。
- `git diff --stat` / `git diff` 复核结果与任务卡一致：业务代码只改 `check_knowledge` 的 YAML 读取逻辑及其测试。
- 未见数据库迁移/表结构、Session API/HTTP 路由、Agent Loop、SSE、Godot、LLM、Gate 文件改动。
- `work/callbacks.md` 已存在 developer 本轮回传：`[2026-07-08 14:29 +0800] [developer] ...`

测试与验证：
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test narrative_tools`：通过，`6 passed`
- `cargo test -p daedalusd --test tool_registry`：通过，`5 passed`
- `cargo test -p daedalusd --test http_session`：通过，`4 passed`
- `cargo test -p daedalusd --test http_tasks`：通过，`15 passed`
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

验收结论依据：
- `check_knowledge` 已改为从 `ToolContext.work_dir/narrative/characters/{npc_id}/knowledge.yaml` 读取规则，不再依赖硬编码 `zhang_san` 事实表。
- `daedalusd/tests/narrative_tools.rs` 已覆盖：
  - `knows` 在 `denial` 直接允许；
  - `knows` 带 `unlock_condition: "stage >= partial"` 时，`vague` 拒绝、`partial` 允许；
  - `hides` 带 `reveal_stage: breakdown` 时，`partial` 拒绝、`breakdown` 允许；
  - 缺少 knowledge 文件返回 `allowed = false` 且 `reason = "knowledge_file_missing"`；
  - 未知 `fact_id` 返回 `allowed = false` 且 `reason = "unknown_fact"`；
  - 无效 `confession_stage` 仍拒绝为 `ToolError::InvalidInput`。

风险与说明：
- `/Users/gu/daedalus-courtroom-demo` 当前位于 `main` 且存在既有脏工作树；本次验收未修改该仓库，也未见本轮 diff 落入该仓库，但无法仅凭当前状态证明其中既有改动与本轮绝对无关。
- 当前实现仍是第一版文件直读方案，每次调用同步读取单个 YAML；这与本轮任务卡范围一致，不构成阻断。
- 当前线程无法主动跨会话回传，总控结论已按要求写入 `work/callbacks.md` 兜底。
