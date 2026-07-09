# 测试报告：Narrative Backend Phase 2a

> developer 完成后填写验证命令和结果；reviewer 只读复核后追加验收结论。

## Developer 验证

[2026-07-09 12:17 +0800] Phase 2a developer 验证：

- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：12 passed
  - 覆盖 `message_uses_fake_agent_loop_utterance_and_updates_state`
  - 覆盖 `non_json_summary_falls_back_without_leaking`
  - 覆盖 `forbidden_field_falls_back`
  - 覆盖 `invalid_emotion_falls_back`
  - 覆盖 `forbidden_term_falls_back`
  - 覆盖 `invalid_reveal_falls_back`
  - 覆盖 `aggressive_message_advances_stage_and_unlocks_clue`
  - 覆盖 `build_failure_resets_processing_flag`
  - 覆盖 `task_error_resets_processing_flag`
  - 覆盖 `timeout_resets_processing_flag`
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- `/api/session/:session_id/message` 现在只接受合法 NPC Reply JSON summary；非 JSON、未知字段、禁用字段、非法 emotion、forbidden term、非法 reveal 都会 fallback。
- fallback 路径不会把原始非法 summary 暴露到 response / messages / events；`npc_reply` 事件会记录 `validation_status` 和最小 `validation_error`。
- 合法 reveals 会并入 `revealed_clues`，并通过 `clue_unlocked` 事件写回到 `/state` 的 `unlocked_clues`。

## Reviewer 验收

[2026-07-09 12:21 +0800] PASS

- 只读复核 `work/task.md`、当前工作树与实际 diff；本轮业务改动落在 `daedalusd/src/http/session.rs`、`daedalusd/tests/http_session.rs`，未见 DB 迁移、HTTP 路由、Agent Loop、SSE、Godot、LLM、Gate 文件改动。
- `daedalusd/src/http/session.rs` 已要求 `TaskDone.outbox.summary` 必须解析为 NPC Reply JSON object，并校验允许字段、必填字段、`emotion` enum、`stage_delta`、`reveals`、`debug_tags`、`confidence`、递归禁止字段、`game_state.forbidden_terms` 与 reveal 白名单。
- 非 JSON summary 走 fallback，测试 `non_json_summary_falls_back_without_leaking` 已断言原始 summary 不进入 response 或 messages；`npc_reply` 事件记录 `validation_status = "fallback"` 和最小 `validation_error`。
- `forbidden_field_falls_back` 覆盖 `inner_thought` 禁止字段；实现中 `contains_forbidden_field` 对 `inner_thought` / `chain_of_thought` / `forbidden_leak` 递归检查。`invalid_emotion_falls_back`、`forbidden_term_falls_back`、`invalid_reveal_falls_back` 已覆盖对应 fallback。
- `message_uses_fake_agent_loop_utterance_and_updates_state` 已证明合法 JSON summary 的 `utterance` / `emotion` / `reveals` 进入 response、message、event，并让合法 reveal 进入 `revealed_clues` 与 `/state.unlocked_clues`。
- `aggressive_message_advances_stage_and_unlocks_clue` 仍覆盖 aggressive + evidence 阶段推进和线索事件；`build_failure_resets_processing_flag`、`task_error_resets_processing_flag`、`timeout_resets_processing_flag` 仍覆盖 `is_processing = 0` 恢复。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 12 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。

剩余风险：

- 当前 validator 仍在 `session.rs` 内，是 Phase 2a 的最小硬校验；后续若继续扩展规则，可能需要拆到独立模块以降低 handler 体积。
- `/Users/gu/daedalus-courtroom-demo` 当前仍是既有脏工作树；本次未修改该仓库，也未见本轮后端 diff 落入其中，但无法仅凭当前状态证明其中既有改动与本轮绝对无关。
