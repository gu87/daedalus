# 当前任务：Narrative Backend Phase 2a - 结构化 NPC 输出与最小 Validator

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 1 已完成并推送：

```text
e82f392 feat: complete narrative session adapter phase 1
```

当前能力：

- `POST /api/session/start`
- `POST /api/session/:session_id/message`
- `GET /api/session/:session_id/state`
- `sessions` / `game_events`
- 最小 `game_events` 日志
- 5 个 narrative tools
- `check_knowledge` 读取 `knowledge.yaml`
- `DAEDALUS_NARRATIVE_ROOT`
- `/message` 已通过 `DaemonContext::spawn_task(...)` 走 Agent Loop，并等待 `TaskDone` / `TaskError`

现在进入 Phase 2：结构化输出 + 强约束。

本轮只做 Phase 2a 的最小可验收切片：**让 `/message` 不再把任意 task summary 当成 NPC 台词，而是先按 NPC JSON schema 解析和校验；校验失败时使用安全 fallback，不泄露非法内容。**

当前后端仓库：

```text
/Users/gu/Daedalus
branch: codex/narrative-backend
```

游戏 Demo、剧本、素材、Godot 仍在另一个仓库，不在本任务中修改：

```text
/Users/gu/daedalus-courtroom-demo
branch: main
```

## 目标

实现最小 `Narrative Output Validator`：

```text
TaskDone.outbox.summary
-> 必须解析为 NPC Reply JSON object
-> 校验 schema 和越界字段
-> 合规则用于 utterance/emotion/reveals
-> 不合规则使用当前阶段 fallback 台词
-> 不允许非法内容进入 response/messages/events
```

这是 Phase 2 的第一步。不要在本轮接 SSE、Godot 或完整 Gate AutoRevision。

## NPC Reply JSON schema

本轮支持并校验以下结构：

```json
{
  "utterance": "我...我不认识什么梁远山。",
  "emotion": "nervous",
  "stage_delta": {
    "should_change": false,
    "new_stage": null,
    "reason": null
  },
  "reveals": [],
  "debug_tags": ["withholding_known_fact"],
  "confidence": 0.85
}
```

### 必填规则

- `utterance`: string，非空，长度不超过 500 字符
- `emotion`: enum，只允许 `calm` / `defensive` / `nervous` / `anxious` / `angry` / `broken`
- `stage_delta`: object
- `stage_delta.should_change`: bool
- `reveals`: string array
- `confidence`: number，范围 `0.0..=1.0`

### 条件规则

- `stage_delta.should_change = true` 时：
  - `stage_delta.new_stage` 必须是 `denial` / `vague` / `partial` / `breakdown`
  - `stage_delta.reason` 必须是非空 string
- `stage_delta.should_change = false` 时：
  - `new_stage` / `reason` 可以为 `null` 或缺省

### 禁止规则

如果 summary JSON object 任一层级出现以下字段，判定无效：

- `inner_thought`
- `chain_of_thought`
- `forbidden_leak`

### debug_tags

本轮只允许以下 tag：

- `withholding_known_fact`
- `nervous_pause`
- `contradiction_pressure`
- `evidence_reaction`
- `fallback`

未知 tag 判定无效。

### forbidden terms

本轮先用最小来源，不做完整 `CharacterKnowledgeProvider`：

- 如果 `game_state.forbidden_terms` 是 string array，则 `utterance` 不得包含其中任一词。
- 没有该字段时，跳过 forbidden term 校验。

### reveals

本轮最小校验：

- `reveals` 里的每个 clue id 必须满足以下任一条件：
  - 已存在于 `game_state.unlocked_evidence_ids`
  - 等于本轮 request 的 `evidence_id`
- 否则判定无效。

## 行为要求

### 合法输出

合法 JSON summary 应该：

- response `utterance` 来自 JSON `utterance`
- response `emotion` 来自 JSON `emotion`
- response `revealed_clues` 合并：
  - 当前确定性 `evidence_id`
  - JSON `reveals`
- 写入 `messages_jsonl`
- 写入 `npc_reply` 事件
- `GET /state` 能读回

### 非法输出

非法 JSON summary 或非 JSON summary 应该：

- 不把原始 summary 暴露给玩家
- 使用当前阶段 fallback 台词
- emotion 使用安全默认值
- `revealed_clues` 不采用非法 reveals
- 写入 `npc_reply` 事件时记录最小验证状态，例如：
  - `validation_status = "fallback"`
  - `validation_error = "..."`

fallback 台词可复用现有 `deterministic_utterance(stage)`，不要新增复杂 fallback 配置。

## 非目标

本轮不做：

- 不接 SSE
- 不接 Godot
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不实现完整 `CharacterKnowledgeProvider`
- 不把 knowledge.yaml 注入 prompt
- 不接 Gate AutoRevision
- 不修改 Agent Loop 9 状态机
- 不改变 Tool trait
- 不做真实 LLM 联调
- 不做多 NPC
- 不做 context 压缩
- 不实现 `update_confession_stage` / `reveal_clue` 的 DB side effect
- 不改数据库表结构，除非绝对必要；如改，必须说明原因

## 允许修改

优先限制在：

- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- `daedalusd/tests/fixtures/phase1_zhang_san/...`
- `work/test-report.md`
- `work/callbacks.md`

如确实需要，为了避免 `session.rs` 继续膨胀，可新增一个小模块：

- `daedalusd/src/http/narrative_output.rs`

如果新增模块，需要只放 parser/validator，不放 HTTP handler。

## 建议实现

保持短 diff：

- 增加一个内部 `ValidatedNarrativeReply` / `NarrativeValidationError`
- 增加 `validate_task_summary(...)`
- 在现有 `reply_from_task_summary(...)` 或其调用点替换逻辑
- 测试用 fake Agent Loop 返回不同 summary：
  - 合法 JSON
  - 非 JSON
  - forbidden field
  - forbidden term
  - invalid reveal
  - invalid emotion

不要为了未来抽象出 Provider 层。

## 验收标准

- [ ] 合法 JSON summary 会成为 response/message/event 的 NPC 回复
- [ ] 非 JSON summary 不再直接成为 NPC 台词，而是 fallback
- [ ] `inner_thought` / `chain_of_thought` / `forbidden_leak` 会触发 fallback
- [ ] invalid emotion 会触发 fallback
- [ ] `game_state.forbidden_terms` 命中会触发 fallback
- [ ] 未解锁且非本轮 evidence 的 reveal 会触发 fallback
- [ ] 合法 reveal 会进入 `revealed_clues` 和 state `unlocked_clues`
- [ ] 现有 aggressive + evidence 阶段推进仍通过
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
