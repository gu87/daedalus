# 当前任务：Narrative Backend Phase 2d - 阶段推进证据门槛

> standard 模式：总控分派；developer 实现；reviewer 验收。
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 2c 已完成并推送：

```text
ccee5fe chore: record narrative phase 2c review callback
```

当前能力：

- `/message` 通过 Agent Loop 获取 `TaskDone.outbox.summary`
- summary 必须是 NPC Reply JSON，否则 fallback
- TaskCard 已注入输出契约、当前 session 状态、玩家输入和 knowledge 边界
- `stage_delta.should_change = true` 只允许口供阶段单步前进：

```text
denial -> vague -> partial -> breakdown
```

当前剩余风险：

Phase 2c 只解决了“不能跳级/倒退”。但合法单步推进仍只依赖 LLM JSON，本轮补最小剧情条件：**某些阶段推进必须由 `game_state` 声明证据门槛，且本轮 request 必须带对应 `evidence_id`。**

## 目标

给 `stage_delta` validator 增加最小证据门槛：

```text
game_state.stage_requirements
  -> target stage
  -> required_evidence_id
```

当 LLM 想推进到某个 `new_stage` 时：

- 如果 `game_state.stage_requirements[new_stage].required_evidence_id` 存在且非空
- 则本轮 request 的 `evidence_id` 必须等于该值
- 否则整条 NPC Reply fallback
- 稳定错误 code：`missing_required_evidence`

## 建议数据形状

不改 DB，不改 API schema，只读现有 `game_state` JSON：

```json
{
  "stage_requirements": {
    "vague": {
      "required_evidence_id": "photo_1"
    },
    "partial": {
      "required_evidence_id": "note_2"
    }
  }
}
```

没有配置要求时保持 Phase 2c 行为。

## 行为规则

### 合法

当前阶段 `denial`，LLM 输出 `new_stage = "vague"`，且：

```json
game_state.stage_requirements.vague.required_evidence_id = "photo_1"
request.evidence_id = "photo_1"
```

应当：

- 使用 JSON `utterance`
- 使用 JSON `emotion`
- 更新 response/session `confession_stage`
- 写入 `stage_change`
- `stage_change.reason` 使用 JSON `stage_delta.reason`

### 非法

当前阶段 `denial`，LLM 输出 `new_stage = "vague"`，但：

- `request.evidence_id` 缺失；或
- `request.evidence_id != required_evidence_id`

应当：

- fallback
- 不暴露原始非法 summary
- 不采用非法 `utterance`
- 不更新 session stage
- `npc_reply.validation_status = "fallback"`
- `npc_reply.validation_error = "missing_required_evidence"`

### 无 requirement

如果目标阶段没有配置 `required_evidence_id`，保持 Phase 2c 行为。

## 非目标

本轮不做：

- 不接 Gate AutoRevision
- 不接 SSE
- 不接 Godot
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不接真实 LLM
- 不改 Agent Loop
- 不改 Tool trait
- 不改数据库表结构
- 不做多 NPC
- 不做缓存
- 不做完整 CharacterKnowledgeProvider
- 不拆 `session.rs` 做纯重构
- 不实现复杂条件表达式，只支持一个 `required_evidence_id`

## 允许修改

优先限制在：

- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- `work/test-report.md`
- `work/callbacks.md`

不要修改：

- `/Users/gu/daedalus-courtroom-demo`
- DB migrations
- HTTP 路由文件
- Agent Loop
- Tool trait

## 建议实现

保持短 diff：

- 在 `validate_task_summary(...)` 现有合法单步 stage transition 之后，加一个小 helper
- helper 从 `game_state.stage_requirements[new_stage].required_evidence_id` 读取字符串
- 空字符串或缺失视为无要求
- 有要求时比较 `request_evidence_id`
- 错误 code 用 `missing_required_evidence`

不要引入新模块或新抽象。

## 验收标准

- [ ] 没有 `stage_requirements` 时，Phase 2c 合法单步推进仍通过
- [ ] 配置 `required_evidence_id` 且 request 带匹配 `evidence_id` 时，合法单步推进通过
- [ ] 配置 `required_evidence_id` 但 request 缺失 `evidence_id` 时 fallback
- [ ] 配置 `required_evidence_id` 但 request 带错误 `evidence_id` 时 fallback
- [ ] fallback 不暴露原始非法 summary
- [ ] `npc_reply.validation_error` 返回稳定 code `missing_required_evidence`
- [ ] Phase 2a validator 测试仍通过
- [ ] Phase 2b prompt/knowledge tests 仍通过
- [ ] Phase 2c invalid stage transition tests 仍通过
- [ ] aggressive + evidence 确定性阶段推进仍通过
- [ ] build / TaskError / timeout 恢复 `is_processing = 0` 的测试仍通过
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写业务代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
