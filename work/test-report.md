# 测试报告：Narrative Backend Phase 4a

> developer 完成后填写验证命令和结果；reviewer 只读复核后追加验收结论。

## Developer 验证

[2026-07-10 15:27 +0800] Phase 4a developer 验证：

- `git fetch origin`：通过；当前分支 `codex/narrative-backend` 与 `origin/codex/narrative-backend` 对齐
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：32 passed
  - 覆盖 `end_session_marks_state_and_blocks_followup_messages`
  - 覆盖 `end_session_returns_409_while_processing`
  - 覆盖 `two_npc_sessions_keep_state_history_events_and_prompts_isolated`
  - 既有 JSON `/message`、SSE `/message/stream`、Phase 1-3 validator/prompt/stage/evidence/speak 回归继续通过
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- 新增 `POST /api/session/:session_id/end`，并在两个 router 注册；合法未结束、非 processing session 会写入 `session_end` game event，`/state.is_ended` 由现有事件派生为 `true`。
- 已结束 session 的 JSON `/message` 与 SSE `/message/stream` 都返回 `410 Gone`，重复 end 返回 `410 Gone`，不存在 session 返回 `404`，processing 中 end 返回 `409`。
- 新增两 NPC 隔离测试，使用 `zhang_san` 与测试 fixture `li_si` 分别创建 session，证明 prompt/knowledge、messages、confession stage、unlocked clues、game events 和 end 状态不串；结束 A 不影响 B。
- 本轮未改 DB schema、Agent Loop、Tool trait、Gate、provider、Godot 外部仓库或 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md`。

改动文件：

- `daedalusd/src/http/server.rs`
- `daedalusd/src/http/session.rs`
- `daedalusd/src/narrative/events.rs`
- `daedalusd/tests/http_session.rs`
- `work/task.md`
- `work/test-report.md`
- `work/callbacks.md`

剩余风险：

- end 状态仍按任务要求只通过 `session_end` event 派生，没有新增 DB flag 或索引。
- 第二 NPC 目前仅为测试临时 fixture，用于验证隔离；未引入多 NPC 编排、共享状态机或真实内容配置。
- Phase 4.2/4.4 history/context 截断与摘要、4.5 fallback 配置化、真实 provider/Godot/Gate 仍未进入本轮。

[2026-07-10 14:11 +0800] Phase 3b developer 验证：

- `git fetch origin`：通过；当前分支 `codex/narrative-backend` 与 `origin/codex/narrative-backend` 对齐
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test agent_loop`：23 passed
  - 覆盖 `lifecycle_emits_narrative_speak_after_speak_tool_success`
  - 证明 typed `narrative.speak` 仅在 `speak` 工具 execute 成功后从 lifecycle channel 发出
- `cargo test -p daedalusd --test http_session`：29 passed
  - 覆盖 `stream_message_forwards_safe_speak_before_task_done`
  - 覆盖 `stream_message_ignores_raw_task_stream_and_forbidden_speak`
  - 既有 JSON `/message`、Phase 2 validator/prompt/stage/evidence、Phase 3a SSE 完成事件、build / TaskError / timeout 恢复 processing 测试继续通过
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- 新增 typed `narrative.speak` IPC message，包含 `req_id`、`agent_id`、`task_id`、`text`、`emotion`。
- Agent Loop 仅在 narrative `speak` tool 成功 execute 且返回 `event_type = utterance_complete` 后发送 `narrative.speak`；raw `TaskStream { chunk }` 仍只代表 provider raw text，不会进入玩家 SSE。
- `/api/session/:session_id/message/stream` 现在会在任务运行期间消费 typed `narrative.speak`，按当前 session `game_state.forbidden_terms`、text 长度和 emotion enum 二次校验后，能在 `TaskDone` 前发送 `utterance_complete`。
- `TaskDone.outbox.summary` 仍是最终状态/持久化/stage/clue/done 权威；若已放行 speak 与最终 summary 的 utterance/emotion 不一致，会走安全 fallback，不追加第二段原始 LLM 文本。
- TaskCard prompt/输出契约已要求 NPC 先调用 `speak(text, emotion)`，再调用 `task_done`，且二者 text/emotion 一致。

改动文件：

- `daedalusd/src/types.rs`
- `daedalusd/src/ipc/protocol.rs`
- `daedalusd/src/ipc/control.rs`
- `daedalusd/src/agent/loop.rs`
- `daedalusd/src/http/session.rs`
- `daedalusd/src/narrative/mod.rs`
- `daedalusd/src/narrative/session_adapter.rs`
- `daedalusd/src/narrative/stream_events.rs`
- `daedalusd/tests/agent_loop.rs`
- `daedalusd/tests/http_session.rs`
- `work/task.md`
- `work/test-report.md`
- `work/callbacks.md`

剩余风险：

- 本轮实现的是安全 tool 事件流基础，不是 provider token streaming，也不解析 tool-call argument delta。
- 如果某次尝试已放行安全 speak，但最终 summary 触发 revision/fallback，Session 会避免继续追加原始 LLM 文本；完整多尝试可撤回/替换语义仍留给后续更明确的事件协议。
- stream 端点现在以后台任务推送 SSE，任务启动后的错误会以 SSE `error` event 返回；JSON `/message` 错误语义保持不变。
- 未修改 `/Users/gu/daedalus-courtroom-demo`，未改 DB migration、Gate、provider、Tool trait。

[2026-07-09 13:42 +0800] Phase 2c developer 验证：

- `git fetch origin`：通过；当前分支 `codex/narrative-backend` 包含 `b4f59f7 chore: dispatch narrative phase 2c`
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：18 passed
  - 覆盖 `valid_stage_delta_advances_one_stage_and_uses_llm_reason`
  - 覆盖 `jump_stage_delta_falls_back_and_does_not_change_stage`
  - 覆盖 `same_stage_delta_change_falls_back`
  - Phase 2a validator、Phase 2b prompt/knowledge、aggressive + evidence、build / TaskError / timeout 恢复 processing 测试继续通过
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- `stage_delta.should_change = true` 现在只允许按 `denial -> vague -> partial -> breakdown` 向前推进 1 个阶段。
- 非法跳级、倒退、或同阶段伪变化都会整体 fallback，并返回稳定 `validation_error = "invalid_stage_transition"`。
- 合法单步推进会采用 JSON `utterance` / `emotion` / `stage_delta.reason`，并更新 session stage 与 `stage_change` 事件。

## Reviewer 验收

[2026-07-09 13:44 +0800] PASS

- 只读复核 `work/task.md`、当前工作树与实际 diff；本轮业务改动仍落在 `daedalusd/src/http/session.rs`、`daedalusd/tests/http_session.rs`，工作树中的 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md` 既有脏改动未作为 Phase 2c 业务 diff 阻断处理。
- `validate_task_summary(...)` 已新增基于 `current_confession_stage` 的 `is_valid_stage_transition(...)` 校验，只允许 `new_rank == current_rank + 1`；因此单步前进合法，跳级、倒退和同阶段 `should_change=true` 都会返回稳定 `validation_error = "invalid_stage_transition"`。
- `valid_stage_delta_advances_one_stage_and_uses_llm_reason` 已证明合法 `denial -> vague` 会采用 JSON `utterance` / `emotion`，更新 session `confession_stage`，并把 JSON `stage_delta.reason` 写入 `stage_change` 事件。
- `jump_stage_delta_falls_back_and_does_not_change_stage` 与 `same_stage_delta_change_falls_back` 已证明非法跳级和同阶段伪变化会整体 fallback，不暴露原始非法 summary，也不会更新 session stage；`npc_reply.validation_error` 返回 `invalid_stage_transition`。
- Phase 2a validator、Phase 2b prompt/knowledge、aggressive + evidence、build failure、TaskError、timeout 恢复 `is_processing = 0` 的既有测试仍在 `http_session` 中通过。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 18 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。

剩余风险：

- 当前阶段跳转约束仍是 Phase 2c 的最小 validator 规则，只覆盖单步前进，不处理更复杂的剧情条件、证据门槛或多 NPC 状态机。
- `/Users/gu/daedalus-courtroom-demo` 当前仍是既有脏工作树；本次未修改该仓库，也未见本轮后端 diff 落入其中，但无法仅凭当前状态证明其中既有改动与本轮绝对无关。

[2026-07-09 13:50 +0800] [reviewer] PASS - Narrative Backend Phase 2c 限制 LLM 口供阶段跳转验收通过：只读复核 work/task.md、当前工作树与实际 diff；重跑 cargo fmt --all -- --check、cargo test -p daedalusd --test http_session、cargo test -p daedalusd --test http_tasks、cargo test -p daedalusd --test narrative_tools、cargo clippy -p daedalusd --all-targets -- -D warnings、git diff --check 全部通过。测试已证明合法单步 stage_delta 可推进并写入 reason，跳级和同阶段伪变化会以 invalid_stage_transition fallback 且不更新 session stage。非阻断风险：当前仍是 Phase 2c 最小 validator 规则，不覆盖复杂剧情条件或多 NPC 状态机；timeout 集成测试仍按真实 30 秒等待。

[2026-07-09 14:24 +0800] Phase 2d developer 验证：

- `git fetch origin`：已确认当前分支 `codex/narrative-backend` 包含 `4bd1385 chore: dispatch narrative phase 2d`
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：21 passed
  - 覆盖 `required_evidence_allows_valid_stage_delta`
  - 覆盖 `missing_required_evidence_falls_back_when_request_has_no_evidence`
  - 覆盖 `wrong_required_evidence_falls_back`
  - Phase 2a validator、Phase 2b prompt/knowledge、Phase 2c 单步阶段跳转、aggressive + evidence、build / TaskError / timeout 恢复 processing 测试继续通过
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- 在现有 Phase 2c 合法单步阶段推进之后，新增了 Phase 2d 证据门槛校验。
- 当 `game_state.stage_requirements[new_stage].required_evidence_id` 存在且非空时，本轮 request `evidence_id` 必须精确匹配；缺失或不匹配都会整体 fallback，并稳定返回 `validation_error = "missing_required_evidence"`。
- fallback 路径不会更新 session `confession_stage`，不会写入新的阶段推进结果，也不会把原始非法 summary 暴露给玩家。
- 若没有 `stage_requirements`，或目标阶段没有非空 `required_evidence_id`，则保持 Phase 2c 原有行为不变。

## Reviewer 验收

[2026-07-09 14:31 +0800] PASS

- 只读复核 `work/task.md`、当前工作树与实际 diff；本轮 Phase 2d 业务改动仍落在 `daedalusd/src/http/session.rs`、`daedalusd/tests/http_session.rs`，协作提示文件既有脏改动未作为本轮业务 diff 阻断处理。
- `validate_task_summary(...)` 已在 Phase 2c 合法单步阶段跳转校验之后新增证据门槛判断；目标阶段存在非空 `game_state.stage_requirements[new_stage].required_evidence_id` 时，本轮 request `evidence_id` 必须精确匹配。
- `required_evidence_allows_valid_stage_delta` 已证明命中必需证据时，合法单步 `stage_delta` 仍会采用 JSON `utterance` / `emotion` / `stage_delta.reason`，更新 session `confession_stage`，并写入 `stage_change` 事件。
- `missing_required_evidence_falls_back_when_request_has_no_evidence` 与 `wrong_required_evidence_falls_back` 已证明缺失/错误 evidence 会整体 fallback，稳定返回 `validation_error = "missing_required_evidence"`，不更新 session stage，也不暴露原始非法 summary。
- 既有 Phase 2a validator、Phase 2b prompt/knowledge、Phase 2c 单步阶段跳转、aggressive + evidence、build failure、TaskError、timeout 恢复 `is_processing = 0` 的测试仍通过，说明无 `stage_requirements` 或目标阶段无非空 `required_evidence_id` 时，Phase 2c 行为保持不变。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 21 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。

剩余风险：

- 当前仍是 Phase 2d 最小 validator 规则，只支持单个 `required_evidence_id` 的精确匹配，不覆盖多证据或条件表达式门槛。
- `timeout` 集成测试仍按真实 30 秒等待。

[2026-07-09 14:38 +0800] Phase 2e developer 验证：

- `git fetch origin`：已确认当前分支 `codex/narrative-backend` 包含 `25ae146 chore: dispatch narrative phase 2e`
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：24 passed
  - 覆盖 `required_evidence_ids_allow_any_matching_candidate`
  - 覆盖 `required_evidence_ids_missing_or_wrong_fall_back`
  - 覆盖 `empty_or_combined_stage_requirements_keep_expected_behavior`
  - Phase 2a validator、Phase 2b prompt/knowledge、Phase 2c 单步阶段跳转、Phase 2d 单证据门槛、aggressive + evidence、build / TaskError / timeout 恢复 processing 测试继续通过
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- 在 Phase 2d 的单证据门槛基础上，新增支持 `required_evidence_ids` 字符串数组候选。
- 目标阶段若配置了任一有效 requirement，则本轮 request `evidence_id` 命中旧字段 `required_evidence_id` 或新字段 `required_evidence_ids` 任一候选即可通过。
- 当 requirement 存在但 request 缺失 `evidence_id` 或证据不命中任一候选时，整体 fallback，稳定返回 `validation_error = "missing_required_evidence"`，不更新 session `confession_stage`，也不暴露原始非法 summary。
- 空数组、空字符串、缺字段、非字符串元素都会被忽略；若最终没有任何有效 requirement，则保持 Phase 2c / Phase 2d 既有行为不变。

## Reviewer 验收

[2026-07-09 14:47 +0800] PASS

- 只读复核 `work/task.md`、当前工作树与实际 diff；本轮 Phase 2e 业务改动仍落在 `daedalusd/src/http/session.rs`、`daedalusd/tests/http_session.rs`，工作树中的 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md` 是既有协作脏改动，未作为本轮业务 diff 阻断。
- `has_required_stage_evidence(...)` 已从单证据校验扩展为收集 `required_evidence_id` 与 `required_evidence_ids` 的有效字符串候选；空字符串、空数组、非字符串元素会被忽略；若最终没有任何有效 requirement，则直接放行，保持 Phase 2c / 2d 既有行为。
- `required_evidence_ids_allow_any_matching_candidate` 已证明 `required_evidence_ids = ["photo_1", "camera_2"]` 时，请求 `evidence_id = "camera_2"` 可以正常采用 JSON `utterance` / `emotion` / `stage_delta.reason`，推进 `confession_stage` 并写入 `stage_change.reason`。
- `required_evidence_ids_missing_or_wrong_fall_back` 已证明缺失 evidence 或证据不在候选数组中时会整体 fallback，稳定返回 `validation_error = "missing_required_evidence"`，不更新 session stage，也不暴露原始非法 summary。
- `empty_or_combined_stage_requirements_keep_expected_behavior` 已证明只有空字符串候选时视为无 requirement，仍保持 Phase 2c 行为；同时存在 `required_evidence_id` 与 `required_evidence_ids` 时，命中任一字段即可通过。
- 既有 Phase 2a validator、Phase 2b prompt/knowledge、Phase 2c invalid stage transition、Phase 2d 单证据门槛、aggressive + evidence、build failure、TaskError、timeout 恢复 `is_processing = 0` 的测试仍通过。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 24 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。

剩余风险：

- 当前仍是 Phase 2e 最小 validator 规则，只支持单次 request `evidence_id` 命中旧字段或候选数组中的任一项，不覆盖多证据同时满足或更复杂条件表达式。
- `timeout` 集成测试仍按真实 30 秒等待。

[2026-07-09 16:07 +0800] Phase 2 completion developer 审计：

- `git fetch origin`：通过
- 分支确认：当前分支 `codex/narrative-backend`
- commit 确认：当前分支包含 `b87f6bc chore: dispatch narrative phase 2 completion audit`
- commit 确认：当前分支包含 `b237b38 feat: allow candidate evidence for narrative stage gates`
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：24 passed
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- Daedalus Narrative Backend Phase 2 已完成：结构化 NPC 输出、prompt/knowledge 边界、单步阶段机、证据门槛、候选证据门槛均已实现并通过回归。
- Phase 2a-2e 能力在当前分支上仍可验证：非法 summary / 非法字段 / forbidden term / invalid reveal 会 fallback 且不泄露原文；prompt 注入输出契约、当前 session 状态与 knowledge 边界；`stage_delta` 仅允许 `denial -> vague -> partial -> breakdown` 单步推进；`required_evidence_id` 与 `required_evidence_ids` 都能约束阶段推进且错误证据返回稳定 `missing_required_evidence`。
- 本轮未做业务代码改动；按任务卡只更新了 `work/test-report.md` 与 `work/callbacks.md`，未修改 `/Users/gu/daedalus-courtroom-demo`。

剩余风险：

- Phase 3 事项仍未进入本轮：SSE / Godot 接入、真实 LLM provider smoke、Gate AutoRevision、多 NPC 状态机、多证据同时满足、复杂条件表达式、timeout 测试加速。
- `timeout` 集成测试仍按真实约 30 秒等待，但不阻断 Phase 2 completion。

## Reviewer 验收

[2026-07-09 16:14 +0800] PASS

- 只读复核 `work/task.md`、当前工作树与实际 diff；本轮 completion audit 未见任何 `daedalusd/src/**`、`daedalusd/tests/**` 或 `/Users/gu/daedalus-courtroom-demo` 改动，当前未提交 diff 仅落在 `work/test-report.md` 与 `work/callbacks.md`，工作树中的 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md` 为既有协作脏改动，未作为本轮业务 diff 阻断。
- `git log --oneline` 已确认当前分支 `codex/narrative-backend` 包含 `b87f6bc chore: dispatch narrative phase 2 completion audit` 与 `b237b38 feat: allow candidate evidence for narrative stage gates`，满足任务卡的分支/完成定义前提。
- 当前 Phase 2a-2e 能力仍可由现有测试矩阵覆盖：`http_session` 24 passed，继续覆盖非法 summary fallback、不泄露原文、prompt/knowledge 边界、单步阶段机、单证据门槛、候选证据门槛、aggressive + evidence、build failure、TaskError、timeout 恢复 `is_processing = 0`。
- `http_tasks` 15 passed、`narrative_tools` 7 passed、`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过，说明本轮在不改业务代码的前提下，Phase 2 回归状态保持稳定。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 24 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。
- 结论成立：Daedalus Narrative Backend Phase 2 已完成，结构化 NPC 输出、prompt/knowledge 边界、单步阶段机、证据门槛、候选证据门槛均已实现并通过回归。

剩余风险：

- Phase 3 范围仍未进入本轮：SSE / Godot 接入、真实 LLM provider smoke、Gate AutoRevision、多 NPC 状态机、多证据同时满足、复杂条件表达式。
- `timeout` 集成测试仍按真实约 30 秒等待，但不阻断 Phase 2 completion。

[2026-07-09 16:34 +0800] Phase 3a developer 验证：

- `git fetch origin`：已确认当前分支 `codex/narrative-backend` 包含 `59c1b15 chore: dispatch narrative phase 3a`
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：26 passed
  - 覆盖 `stream_message_returns_sse_events_with_stage_and_clue_changes`
  - 覆盖 `stream_message_fallback_does_not_leak_invalid_summary`
  - 既有 JSON `/message`、Phase 2a-2e validator/prompt/stage/evidence 回归、build / TaskError / timeout 恢复 processing 测试继续通过
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- 新增 `POST /api/session/:session_id/message/stream`，返回 `text/event-stream`，并复用现有 `/message` 核心处理逻辑。
- stream 成功时会发送 `utterance_complete`，阶段变化时发送 `stage_change`，每个已解锁线索发送一个 `clue_unlocked`，最后总是发送 `done`。
- 本轮仍是“处理完成后一次性发出多个 SSE 事件”的最小实现，不做真正 LLM token streaming，但 Godot 已可通过 Daedalus Session API 获得 SSE 格式的审讯结果事件流。
- fallback 时 SSE 里的 `utterance_complete.full_text` 仍是安全 fallback 文本，不会泄露非法 summary；现有 JSON `/message` 行为保持不变。

剩余风险：

- 当前 SSE 是完成后批量发送的最小封装，不是 token/chunk 实时流。
- Phase 3 后续事项仍未进入本轮：Godot 项目代码接线、真实 LLM provider smoke、Gate AutoRevision、多 NPC 状态机、复杂条件表达式。

## Reviewer 验收

[2026-07-10 14:18 +0800] PASS

- 只读复核 `work/task.md`、当前分支提交与实际代码路径；`git fetch origin` 后确认当前分支 `codex/narrative-backend` 已包含 `52a0579 feat: stream safe narrative speak events`。本轮业务改动落在 `daedalusd/src/types.rs`、`src/ipc/{protocol,control}.rs`、`src/agent/loop.rs`、`src/http/session.rs`、`src/narrative/{mod,session_adapter,stream_events}.rs`、`tests/{agent_loop,http_session}.rs`；工作树中的 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md` 为既有协作脏改动，未作为本轮业务 diff 阻断。
- `Message::NarrativeSpeak` / `type = "narrative.speak"` 已作为新 typed IPC message 加入 `types.rs` 与 `ipc/protocol.rs`；`ipc/control.rs` 明确把 `Message::NarrativeSpeak(_)` 与 `TaskStream(_)` 一样排除在 control 路径处理之外，避免控制分发误吞业务事件。
- `agent_loop::emit_narrative_speak(...)` 只在 `tool_name == "speak"` 且 `tool_result.is_error == false`，并且 `tool_result.output` 可解析且 `event_type == "utterance_complete"` 时发出 typed event；`lifecycle_emits_narrative_speak_after_speak_tool_success` 已覆盖成功路径。
- Session stream 侧在 `run_session_task(...)` 中只消费 `Message::NarrativeSpeak`，对 `text` 长度、`emotion` 枚举、`game_state.forbidden_terms` 做二次校验；`Some(_) => continue` 也证明 raw `TaskStream { chunk }` 不会进玩家 SSE。`stream_message_ignores_raw_task_stream_and_forbidden_speak` 已证明 raw provider 文本和带 forbidden term 的 speak 都不会出现在 SSE 中。
- `stream_message_forwards_safe_speak_before_task_done` 已证明 `utterance_complete` 可以早于 `TaskDone` 到达：测试在收到 speak 事件时断言还没有 `done` 事件，且 `is_processing` 仍为真，不是靠最终 summary 文本切片伪造的“提前显示”。
- 最终 state / persist / stage / clue / done 仍以 `TaskDone.summary` validator 为权威：`handle_session_message(...)` 仍先对 `summary` 做 Phase 2 validator / revision / stage / evidence / clue 流程；若 speak 与最终 `summary.utterance/emotion` 不一致，则以 `speak_summary_mismatch` 走安全 fallback，不追加第二段原始 LLM 文本。普通 JSON `/message` 路径仍复用同一 helper，`http_session` 29 passed 证明 Phase 2a-2e 与 3a 回归未破坏。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test agent_loop` 23 passed；`cargo test -p daedalusd --test http_session` 29 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。

剩余风险：

- 本轮仍不是 provider token streaming，也不解析 tool-call argument delta；`/Users/gu/daedalus-courtroom-demo/docs/daedalus-game-backend-plan.md` 的旧 Phase 3 段落仍写着 raw `task.stream -> utterance_chunk` 方案，和当前安全 typed `narrative.speak` 设计存在文档漂移。
- 当前只接受首个通过校验的 `speak` 事件；多次 `speak` 的撤回/替换语义仍未定义，应作为后续事件协议设计项诚实保留。

## Reviewer 验收

[2026-07-09 16:42 +0800] PASS

- 只读复核 `work/task.md`、当前工作树与实际 diff；本轮 Phase 3a 业务改动落在 `daedalusd/src/http/server.rs`、`daedalusd/src/http/session.rs`、`daedalusd/tests/http_session.rs`，工作树中的 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md` 为既有协作脏改动，未作为本轮业务 diff 阻断。
- `make_router(...)` 与 `run_http(...)` 都已注册 `POST /api/session/:session_id/message/stream`；`stream_session_message(...)` 通过共享 `handle_session_message(...)` 复用现有 `/message` 核心处理逻辑，JSON `/message` 返回路径仍由同一 helper 产出 `SessionMessageResponse`。
- `stream_message_returns_sse_events_with_stage_and_clue_changes` 已证明 stream 端点返回 `text/event-stream`，包含 `utterance_complete`、`stage_change`、`clue_unlocked`、`done`，且会把阶段推进与线索解锁同步反映到 `/state`。
- `stream_message_fallback_does_not_leak_invalid_summary` 已证明 fallback 时 SSE 的 `utterance_complete.full_text` 返回安全 fallback 文本，不暴露非法 summary，且不会伪造 `stage_change`。
- 原有 JSON `/message` 行为和 Phase 2a-2e 回归仍通过：`http_session` 26 passed，继续覆盖非法 summary fallback、不泄露原文、prompt/knowledge 边界、单步阶段机、单证据门槛、候选证据门槛、aggressive + evidence、build failure、TaskError、timeout 恢复 `is_processing = 0`。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 26 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。
- 结论成立：Godot 已可通过 Daedalus Session API 获得 SSE 格式的审讯结果事件流。

剩余风险：

- 当前 SSE 仍是“处理完成后一次性发出多个事件”的最小封装，不是 token/chunk 级实时流。
- Godot 项目接线、真实 LLM provider smoke、Gate AutoRevision、多 NPC 状态机、复杂条件表达式仍未进入本轮。

[2026-07-09 19:04 +0800] Phase 2 completion developer 验证：

- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test http_session`：27 passed
  - 新增覆盖 `invalid_first_summary_retries_and_uses_revised_reply`
  - 证明第一次 NPC summary 未通过 validator 时，会注入 `revision_feedback` 重新 dispatch，同一 session 最终采用第二次合法回复
  - 证明第一次非法内容不会写入 `messages` 或 `game_events`
  - 证明 `npc_reply` 事件记录 `validation_status = "revised"`、`revision_error` 与 `revision_attempts`
- `cargo test -p daedalusd --test prompt`：40 passed
- `cargo test -p daedalusd --test narrative_tools`：7 passed
- `cargo test -p daedalusd --test agent_loop`：22 passed
- `cargo test -p daedalusd --test http_tasks`：15 passed
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

结论：

- Phase 2 现在补齐了结构化 NPC 输出失败后的最小 AutoRevision 闭环：第一次非法输出不再直接 fallback，而是把 validator 错误码作为 `revision_feedback` 注入第二次同 NPC 任务；第二次合法则采用修正版，第二次仍非法才安全 fallback。
- `TaskDispatch` 现在在 revision 场景中携带 `revision_feedback`，同时保留原有输出契约、session 状态、证据、阶段和 knowledge boundary。
- `npc_reply` 事件现在额外记录 `revision_error` 与 `revision_attempts`，便于游戏侧和调试侧区分 `validated`、`revised`、`fallback`。

剩余风险：

- 这仍是 Phase 2 范围内的单次修正闭环，不是完整 Gate Router 策略系统；现有 Gate AutoRevision 仍主要处理 Agent 执行失败。
- 真实 LLM provider smoke、Godot 接线、多 NPC 状态机、多证据同时满足、复杂条件表达式仍属于后续 Phase。

## Reviewer 验收

[2026-07-10 15:31 +0800] PASS

- 只读复核 `work/task.md`、`daedalusd/src/http/{server,session}.rs`、`daedalusd/src/narrative/events.rs`、`daedalusd/tests/http_session.rs` 与当前工作树；`git fetch origin` 后确认 `HEAD`/`origin/codex/narrative-backend` 均已包含 `577b896 feat: add narrative session end lifecycle`。当前脏文件仅见 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md` 等既有协作改动，未作为本轮业务 diff 阻断。
- `make_router(...)` 与 `run_http(...)` 都已注册 `POST /api/session/:session_id/end`；`end_session_once(...)` 只在 session 存在、未 processing、未结束时写入 `session_end` 事件并返回新的 state。`events::session_end(...)` payload 也已补齐 `session_id` / `npc_id` / `reason` / `final_stage`。
- `end_session_marks_state_and_blocks_followup_messages` 已证明正常 end 后 `/state.is_ended = true`、`events` 中出现 `session_end`；重复 end 返回 `410 Gone`，后续 JSON `/message` 与 SSE `/message/stream` 也都返回 `410 Gone`。`end_session_returns_409_while_processing` 已证明 processing 中 end 返回 `409 Conflict`；同用例和 `sess-missing` 断言覆盖了 `404 Not Found`。
- `ensure_session_can_message(...)` 会在 JSON `/message` 与 SSE `/message/stream` 两条路径上统一拦截已结束 session，且在 `410` 前不 dispatch task；`load_session_for_message(...)` 也再次用 `session_end` event 做守门，因此 end 后不会继续处理消息。
- `two_npc_sessions_keep_state_history_events_and_prompts_isolated` 已证明 `zhang_san` 与 `li_si` 的 `npc_id`、`case_id`、messages、`confession_stage`、`unlocked_clues`、`game_events`、prompt/knowledge boundary 都不串；结束 A 后仅 A 的 `is_ended` 变 `true`，B 保持未结束，且 B 的 events 中没有 `session_end`。
- 本轮未改 DB migration、Gate、Agent Loop、Tool trait，也未修改 `/Users/gu/daedalus-courtroom-demo`；`http_session` 32 passed 说明 Phase 1-3 既有 JSON `/message`、SSE、typed speak、revision、stage/evidence 回归仍通过。
- 重跑 `cargo fmt --all -- --check` 通过；`cargo test -p daedalusd --test http_session` 32 passed；`cargo test -p daedalusd --test http_tasks` 15 passed；`cargo test -p daedalusd --test narrative_tools` 7 passed；`cargo clippy -p daedalusd --all-targets -- -D warnings` 通过；`git diff --check` 通过。

剩余风险：

- end 状态仍按本轮要求只由 `session_end` event 派生，没有新增 DB flag 或索引；如果后续需要更高频查询或跨表统计，再考虑结构化字段。
- Phase 4.2 / 4.4 的 history/context 截断与摘要、Phase 4.5 的 fallback 配置化、以及真实 provider / Godot / Gate 集成仍未进入本轮，不应被本次 PASS 掩盖。

## 回归修复记录（2026-08-17）

[2026-08-17] 修复 `codex/narrative-backend` 分支 11 个回归测试，全量测试恢复全绿：

- 根因：`5f17905 feat: emit task stream chunks from agent loop` 让 Agent Loop 在成功路径上先发 `TaskStream` 再发 `TaskDone`/`TaskError`；`gate_daemon` / `pipeline_daemon` 的旧断言仍期望直接收到终止消息，报 `expected TaskDone, got TaskStream`。
- 修复：两个测试文件各新增 `recv_terminal()` helper，循环接收直到终止消息（跳过 TaskStream 等中间消息），替换 11 个失败断言点（gate_daemon 9 处 + pipeline_daemon 2 处）。
- 验证：
  - `cargo test -p daedalusd --test gate_daemon`：23 passed（原 14 passed / 9 failed）
  - `cargo test -p daedalusd --test pipeline_daemon`：4 passed（原 2 passed / 2 failed）
  - `cargo test -p daedalusd`：全量 520+ passed，0 failed
  - 全量运行中 "readonly database" 报错计数为 0（原为断言 panic 提前销毁 TempDir、后台 `tokio::spawn` 仍写库的次生效应，随断言修复消失）
  - `cargo fmt --all -- --check`、`cargo clippy -p daedalusd --all-targets -- -D warnings`、`git diff --check` 均通过
- 附带发现（未处理）：`daedalusd/src/run_artifacts/` 模块在 `daemon.rs` 中无任何调用点（死代码），共享 `/tmp/runs` 因此无实际写入。
- 教训：验收基线应从「本轮涉及文件的测试」改为「`cargo test -p daedalusd` 全量必须全绿」，避免漏掉未更新断言的套件。
