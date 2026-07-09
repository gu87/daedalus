# 测试报告：Narrative Backend Phase 2c

> developer 完成后填写验证命令和结果；reviewer 只读复核后追加验收结论。

## Developer 验证

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
