# 当前任务：Narrative Backend Phase 2e - 多证据候选门槛

> standard 模式：总控分派；developer 实现；reviewer 验收。
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 2d 已完成并推送：

```text
568b485 feat: gate narrative stage transitions by evidence
```

当前能力：

- `/message` 通过 Agent Loop 获取 `TaskDone.outbox.summary`
- summary 必须是 NPC Reply JSON，否则 fallback
- `stage_delta.should_change = true` 只允许口供阶段单步前进：

```text
denial -> vague -> partial -> breakdown
```

- 当 `game_state.stage_requirements[new_stage].required_evidence_id` 存在且非空时，本轮 request `evidence_id` 必须精确匹配

当前剩余风险：

Phase 2d 只支持一个 `required_evidence_id`。剧情设计里常见情况是“几件等价证据任一命中即可推进”。本轮只补这个最小能力，不做复杂条件表达式。

## 目标

给 stage requirement 增加候选证据数组：

```text
game_state.stage_requirements
  -> target stage
  -> required_evidence_ids
```

当 LLM 想推进到某个 `new_stage` 时：

- 如果 `required_evidence_ids` 是非空字符串数组
- 则本轮 request `evidence_id` 命中数组任一项即可通过
- 否则 fallback
- 稳定错误 code 仍是：`missing_required_evidence`

## 数据形状

不改 DB，不改 API schema，只读现有 `game_state` JSON：

```json
{
  "stage_requirements": {
    "vague": {
      "required_evidence_ids": ["photo_1", "camera_2"]
    }
  }
}
```

兼容 Phase 2d 的旧字段：

```json
{
  "stage_requirements": {
    "vague": {
      "required_evidence_id": "photo_1"
    }
  }
}
```

如果两个字段同时存在，最小规则是“任一字段命中即可通过”。

## 行为规则

### 合法

当前阶段 `denial`，LLM 输出 `new_stage = "vague"`，且：

```json
game_state.stage_requirements.vague.required_evidence_ids = ["photo_1", "camera_2"]
request.evidence_id = "camera_2"
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
- `request.evidence_id` 不在 `required_evidence_ids` 中

应当：

- fallback
- 不暴露原始非法 summary
- 不采用非法 `utterance`
- 不更新 session stage
- `npc_reply.validation_status = "fallback"`
- `npc_reply.validation_error = "missing_required_evidence"`

### 无 requirement

如果目标阶段没有配置非空 `required_evidence_id` 或非空 `required_evidence_ids`，保持 Phase 2c 行为。

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
- 不实现 AND/OR/NOT 条件表达式
- 不实现“必须同时提交多份证据”

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

- 复用 Phase 2d 的 `has_required_stage_evidence(...)`
- 先读取 `required_evidence_id`
- 再读取 `required_evidence_ids`
- 空字符串、空数组、非字符串元素都忽略
- 如果最终没有任何有效 requirement，返回 true
- 如果有 requirement，则 request `evidence_id` trim 后命中任一项即可

不要引入新模块或新抽象。

## 验收标准

- [ ] `required_evidence_id` 旧字段仍保持 Phase 2d 行为
- [ ] `required_evidence_ids` 中任一候选证据命中时，合法单步推进通过
- [ ] `required_evidence_ids` 配置存在但 request 缺失 `evidence_id` 时 fallback
- [ ] `required_evidence_ids` 配置存在但 request 证据不在数组中时 fallback
- [ ] 空数组或只有空字符串时视为无 requirement，保持 Phase 2c 行为
- [ ] 同时存在 `required_evidence_id` 和 `required_evidence_ids` 时，任一字段命中即可通过
- [ ] fallback 不暴露原始非法 summary
- [ ] `npc_reply.validation_error` 返回稳定 code `missing_required_evidence`
- [ ] Phase 2a validator 测试仍通过
- [ ] Phase 2b prompt/knowledge tests 仍通过
- [ ] Phase 2c invalid stage transition tests 仍通过
- [ ] Phase 2d 单证据测试仍通过
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
