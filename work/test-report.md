# 测试报告：Narrative Backend Phase 2b

> developer 完成后填写验证命令和结果；reviewer 只读复核后追加验收结论。

## Developer 验证

[2026-07-09 12:30 +0800] Phase 2b developer 验证：

- `git fetch origin`：通过；当前分支 `codex/narrative-backend` 包含 `2774458 chore: dispatch narrative phase 2b`
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：15 passed
  - 覆盖 `task_prompt_injects_output_contract_and_knowledge_boundary`
  - 覆盖 `missing_knowledge_file_still_dispatches_with_status`
  - 覆盖 `invalid_knowledge_file_still_dispatches_with_status`
  - Phase 2a validator、aggressive + evidence、build / TaskError / timeout 恢复 processing 测试继续通过
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- `TaskDispatch.task_card.goal` / `compiled_intent` / `context.project_context.data` 已注入 NPC Reply JSON 输出契约、允许字段、禁止字段、当前 session 状态和玩家输入。
- `/message` 构造 TaskCard 时会读取 `{resolved_root}/narrative/characters/{npc_id}/knowledge.yaml`，注入 `visible_facts` 与不含隐藏内容的 `locked_facts`。
- knowledge 缺文件或 YAML 无效不会导致 `/message` 500，会继续 dispatch 并在 TaskCard 中记录 `knowledge_status`。

## Reviewer 验收

[2026-07-09 12:33 +0800] PASS

- 只读复核 `work/task.md`、当前工作树与实际 diff；本轮业务改动落在 `daedalusd/src/http/session.rs`、`daedalusd/tests/http_session.rs`，未见 DB 迁移、HTTP 路由、Agent Loop、SSE、Godot、LLM、Gate 文件改动。
- `build_session_task_dispatch(...)` 已把 NPC Reply JSON 输出契约注入 `task_card.goal`、`compiled_intent.narrative_output_contract`、`context.project_context.data.narrative_output_contract`，契约明确 `task_done.summary` 必须是 JSON object string，列出允许字段和禁止字段。
- `compiled_intent` 与 `context.project_context.data` 已包含当前 `session_id`、`case_id`、`npc_id`、`current_confession_stage`、`pressure_level`、`evidence_id`、`player_text`、`history`。
- `knowledge_prompt_snapshot(...)` 会读取 `{resolved_root}/narrative/characters/{npc_id}/knowledge.yaml`，注入 `knowledge_boundary`；测试 `task_prompt_injects_output_contract_and_knowledge_boundary` 已证明可见 `knows` 带 `fact_id/content` 注入，未到阶段的 `knows` 只作为 locked fact 注入 `fact_id/unlock_stage`，`hides` 只注入 `fact_id/reveal_stage` 且不注入隐藏 `content`。
- `missing_knowledge_file_still_dispatches_with_status` 和 `invalid_knowledge_file_still_dispatches_with_status` 已证明 knowledge 缺文件或坏 YAML 不会让 `/message` 返回 500，并会在 TaskCard 中记录 `knowledge_status = missing/invalid`。
- Phase 2a validator、aggressive + evidence、build failure、TaskError、timeout 恢复 `is_processing = 0` 的既有测试仍在 `http_session` 中通过。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 15 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。

剩余风险：

- 当前 prompt/knowledge snapshot 仍在 `session.rs` 内，是 Phase 2b 的最小实现；后续继续扩展时可考虑拆到独立模块降低 handler 文件体积。
- `/Users/gu/daedalus-courtroom-demo` 当前仍是既有脏工作树；本次未修改该仓库，也未见本轮后端 diff 落入其中，但无法仅凭当前状态证明其中既有改动与本轮绝对无关。
