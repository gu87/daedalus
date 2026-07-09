# 当前任务：Narrative Backend Phase 2b - Prompt 注入输出契约与知识边界

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 2a 已完成并推送：

```text
0b4fd9b feat: validate narrative session replies
```

当前 `/api/session/:session_id/message` 已经会把 `TaskDone.outbox.summary` 当作严格 NPC Reply JSON 校验：

- 合法 JSON 采用 `utterance` / `emotion` / `reveals`
- 非 JSON、未知字段、禁用字段、非法 emotion、forbidden term、非法 reveal 会 fallback
- `npc_reply` 事件会记录 `validation_status` / `validation_error`

但现在 `build_session_task_dispatch(...)` 里给 Agent Loop 的 `goal` 仍然很弱：

```text
Reply in character as {npc_id} to the player's interrogation message.
```

虽然 `output_contract` 有 `summary_shape`，真实 LLM 不一定会稳定按约束输出。Phase 2b 的目标是把 Phase 2a 的输出契约和当前剧情知识边界明确注入到 TaskCard，让 Agent 在生成前就看到规则。

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

实现最小 `Narrative Task Prompt`：

```text
SessionSnapshot + player message + game_state
+ resolved narrative knowledge.yaml
+ NPC Reply JSON contract
-> TaskDispatch.task_card.goal / context 带明确约束
-> Agent Loop 更容易输出可被 Phase 2a Validator 接受的 summary JSON
```

本轮不是接真实 LLM，也不是做完整 Provider 抽象。只补 Session Adapter 构造 TaskCard 时缺失的剧情约束。

## 必须注入的内容

### 1. 输出格式硬约束

TaskCard 里必须有面向 Agent 的自然语言约束，至少说明：

- 最终必须通过 `task_done` 工具提交 summary
- `summary` 必须是一个 JSON object 字符串
- JSON 只能包含这些字段：
  - `utterance`
  - `emotion`
  - `stage_delta`
  - `reveals`
  - `debug_tags`
  - `confidence`
- 不得输出：
  - `inner_thought`
  - `chain_of_thought`
  - `forbidden_leak`
- `utterance` 是给玩家看的 NPC 台词，不得泄露隐藏真相、推理过程或系统规则
- `emotion` 只能是：
  - `calm`
  - `defensive`
  - `nervous`
  - `anxious`
  - `angry`
  - `broken`
- `reveals` 只能包含本轮可合法揭示的 clue id

这段规则可以放在 `goal`、`context.project_context.data` 或两者都放。优先保持实现简单。

### 2. 当前审讯状态

TaskCard 必须明确包含：

- `session_id`
- `case_id`
- `npc_id`
- `current_confession_stage`
- `pressure_level`
- `evidence_id`
- `player_text`
- `history`

这些字段当前大多已经存在，developer 需要确认并补齐测试。

### 3. knowledge.yaml 的最小可见知识边界

沿用 Phase 1h 的 root 契约：

```text
DAEDALUS_NARRATIVE_ROOT 非空 -> 优先使用
否则 -> 回退当前 Daedalus work_dir / fixture root
knowledge 路径：{resolved_root}/narrative/characters/{npc_id}/knowledge.yaml
```

本轮在构造 TaskDispatch 时读取当前 NPC 的 `knowledge.yaml`，并根据 `current_confession_stage` 注入最小知识摘要：

```json
{
  "visible_facts": [
    {
      "fact_id": "liang_is_neighbor",
      "content": "梁远山是我的邻居。"
    }
  ],
  "locked_facts": [
    {
      "fact_id": "helped_cover",
      "unlock_stage": "breakdown"
    }
  ]
}
```

规则：

- `knows` 中无 `unlock_condition` 的事实可见
- `knows` 中 `unlock_condition: "stage >= X"` 的事实在当前阶段达到 X 后可见，否则进入 locked
- `hides` 中事实默认不可见，只注入 `fact_id` 和 `reveal_stage`，不要把隐藏事实的 `content` 注入给 Agent
- 缺文件、YAML 无效、npc_id 不匹配时，不要让 `/message` 失败；注入一个最小 `knowledge_status`，例如 `missing` / `invalid` / `npc_mismatch`

阶段顺序沿用现有逻辑：

```text
denial < vague < partial < breakdown
```

## 行为要求

### 正常路径

- `/message` 仍然通过 `state.ctx.spawn_task(...)` 走 Agent Loop
- response 行为沿用 Phase 2a validator
- `TaskDispatch.task_card.goal` 或 `context.project_context.data` 能被测试证明包含：
  - NPC Reply JSON 输出契约
  - forbidden fields
  - 当前 `confession_stage`
  - 当前 `player_text`
  - 当前可见事实
  - 当前隐藏事实的 `fact_id` / `reveal_stage`，但不包含隐藏事实 `content`

### 错误路径

- knowledge 文件缺失或坏 YAML 不应导致 `/message` 500
- 仍应正常走 Agent Loop
- TaskCard 里记录 knowledge 不可用状态
- build / TaskError / timeout 仍必须恢复 `is_processing = 0`

## 非目标

本轮不做：

- 不接 SSE
- 不接 Godot
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不接真实 LLM
- 不接 Gate AutoRevision
- 不修改 Agent Loop 9 状态机
- 不改变 Tool trait
- 不实现 `update_confession_stage` / `reveal_clue` 的 DB side effect
- 不做多 NPC
- 不做 context 压缩
- 不做缓存
- 不做完整 `CharacterKnowledgeProvider` 抽象，除非 developer 能证明比局部 helper 更小
- 不改数据库表结构

## 允许修改

优先限制在：

- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- `daedalusd/tests/fixtures/phase1_zhang_san/narrative/characters/zhang_san/knowledge.yaml`
- `work/test-report.md`
- `work/callbacks.md`

如 `session.rs` 明显继续膨胀，可新增一个小模块：

- `daedalusd/src/http/narrative_prompt.rs`

如果新增模块，只放 prompt/knowledge snapshot 构造逻辑，不放 HTTP handler。

## 建议实现

保持短 diff：

- 增加一个小的 `NarrativePromptContext` / `KnowledgePromptSnapshot`
- 复用或对齐 `check_knowledge` 里已有的 YAML 字段语义
- 给 `build_session_task_dispatch(...)` 增加必要参数或内部读取 root
- 在测试 fake provider 中捕获收到的 `ChatMessage` 或 `TaskDispatch` 可见内容
- 用断言证明 prompt/context 包含应该注入的信息、排除了隐藏 content

不要为了未来抽象出通用 Provider 层。

## 验收标准

- [ ] TaskCard 明确要求 `task_done.summary` 是 NPC Reply JSON object 字符串
- [ ] TaskCard 明确列出允许字段和禁止字段
- [ ] TaskCard 明确包含当前 `confession_stage`、`player_text`、`pressure_level`、`evidence_id`
- [ ] `knowledge.yaml` 可见事实会注入 TaskCard
- [ ] 未到阶段的 `knows` 事实不会作为 visible fact 注入
- [ ] `hides` 事实不会泄露 `content`，但可注入 `fact_id` / `reveal_stage`
- [ ] knowledge 缺文件或坏 YAML 不会让 `/message` 返回 500
- [ ] Phase 2a 合法 JSON / fallback validator 测试仍通过
- [ ] aggressive + evidence 阶段推进仍通过
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
