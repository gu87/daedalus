# 测试报告：Narrative Backend Phase 1 completion

> developer 完成后填写验证命令和结果；reviewer 只读复核后追加验收结论。

## Developer 验证

[2026-07-09 10:46 +0800] Phase 1 completion developer 验证：

- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：7 passed
  - 覆盖 `message_uses_fake_agent_loop_utterance_and_updates_state`
  - 覆盖 `aggressive_message_advances_stage_and_unlocks_clue`
  - 覆盖 `build_failure_resets_processing_flag`
  - 覆盖 `task_error_resets_processing_flag`
  - 覆盖 `timeout_resets_processing_flag`
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- `/api/session/:session_id/message` 已通过 `DaemonContext::spawn_task` + mpsc wait 走最小 Session Adapter 闭环。
- `utterance` 在 HTTP 集成测试中来自 fake Agent Loop / fake provider 的 `task_done.summary`，不再来自旧 `deterministic_utterance()`。
- `build` / `task.error` / `timeout` 路径都验证了 `is_processing` 会恢复为 `0`。

## Reviewer 验收

[2026-07-09 11:48 +0800] PASS

- 只读复核 `work/task.md`、当前工作树和实际 diff；本轮业务改动落在 `daedalusd/src/http/session.rs`、`daedalusd/tests/http_session.rs`，并新增最小 `zhang_san` fixture 目录 `daedalusd/tests/fixtures/phase1_zhang_san/...`。
- `daedalusd/src/http/session.rs` 已实际构造 `TaskDispatch` 并调用 `state.ctx.spawn_task(...)`，随后通过 mpsc 等待 `Message::TaskDone` / `Message::TaskError`；非成功路径会调用 `reset_processing_after_error(...)`，其中 persist 失败恢复为代码复核确认。
- `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 7 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。
- `http_session` 已证明 `/message` 的 `utterance` 来自 fake Agent Loop / fake provider 的 `task_done.summary`：`message_uses_fake_agent_loop_utterance_and_updates_state` 断言返回 `[fake-loop] 我只说这一次。`，而非旧 `deterministic_utterance()` 默认文案；`GET /state` 仍能读回 messages / events / state。
- `build_failure_resets_processing_flag`、`task_error_resets_processing_flag`、`timeout_resets_processing_flag` 已证明 build / TaskError / timeout 路径会恢复 `is_processing = 0`；persist error 没有单独测试，但 handler 在持久化失败分支显式调用了同一个恢复函数。

剩余风险：

- `/Users/gu/daedalus-courtroom-demo` 当前仍是既有脏工作树；本次未修改该仓库，也未见本轮后端代码改动落入其中，但无法仅凭现状证明其中既有修改与本轮绝对无关。
- `persist error -> is_processing = 0` 目前只有代码路径复核，没有单独的集成测试命中。
- 当前仍是阻塞式 HTTP 闭环，未接 SSE / 真实 LLM；这与本轮任务卡范围一致，不构成阻断。
