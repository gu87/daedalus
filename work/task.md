# 当前任务：Narrative Backend Phase 2c - 限制 LLM 口供阶段跳转

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 2b 已完成并推送：

```text
61ba51b feat: inject narrative task prompt context
```

当前能力：

- `/message` 通过 Agent Loop 获取 `TaskDone.outbox.summary`
- summary 必须是 NPC Reply JSON，否则 fallback
- TaskCard 已注入 NPC Reply JSON 输出契约、当前 session 状态、玩家输入和 knowledge 边界
- `stage_delta` 的 JSON 结构已经会被校验

当前风险：

`stage_delta.should_change = true` 时，validator 只检查 `new_stage` 是合法枚举，但没有检查它是否符合当前口供状态机。LLM 理论上可以在 `denial` 阶段直接输出 `breakdown`，导致剧情过早崩盘。

本轮只补这个洞：**LLM 可以建议阶段变化，但不能跳级、倒退或保持原阶段伪装成变化。**

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

给 Phase 2a 的 `stage_delta` validator 增加最小状态机约束：

```text
current_confession_stage + stage_delta.new_stage
-> 只允许向前推进 1 个阶段
-> 不允许倒退
-> 不允许跳级
-> 不允许 should_change=true 但 new_stage 等于当前阶段
```

阶段顺序：

```text
denial -> vague -> partial -> breakdown
```

## 行为规则

### 合法

当前阶段到下一阶段：

```text
denial -> vague
vague -> partial
partial -> breakdown
```

合法输出应：

- 使用 JSON `utterance`
- 使用 JSON `emotion`
- 更新 response `confession_stage`
- 更新 session 表 `confession_stage`
- 写入 `stage_change` 事件
- `stage_change.reason` 使用 JSON `stage_delta.reason`

### 非法

以下都判定为 invalid stage transition，并走现有 fallback：

```text
denial -> partial
denial -> breakdown
vague -> denial
partial -> vague
breakdown -> partial
denial -> denial 且 should_change=true
```

非法输出应：

- 不暴露原始非法 summary
- 不采用非法 `utterance`
- 不采用非法 `emotion`
- 不采用非法 `new_stage`
- 使用当前阶段 fallback 台词和安全默认 emotion
- `npc_reply` 事件记录 `validation_status = "fallback"`
- `validation_error` 使用稳定 code，例如 `invalid_stage_transition`

### should_change=false

保持 Phase 2a 行为：

- `new_stage` / `reason` 可以缺省或 null
- 不写 `stage_change`

## 非目标

本轮不做：

- 不接 Gate AutoRevision
- 不接 SSE
- 不接 Godot
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不接真实 LLM
- 不改 Agent Loop 状态机
- 不改 Tool trait
- 不改数据库表结构
- 不做多 NPC
- 不做缓存
- 不做完整 CharacterKnowledgeProvider
- 不拆 `session.rs` 做纯重构，除非是为了让本任务 diff 更小

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

- 让 `validate_task_summary(...)` 能拿到当前 `confession_stage`
- 复用现有 `confession_stage_rank(...)`
- 增加一个很小的 transition check
- 错误 code 用 `invalid_stage_transition`
- 补 2-3 个 integration tests：
  - 合法 `denial -> vague` 会更新 state 并写 `stage_change.reason`
  - 非法 `denial -> breakdown` 会 fallback 且 state 仍是 `denial`
  - 非法倒退或同阶段 `should_change=true` 至少覆盖一个

不要引入新抽象或新模块。

## 验收标准

- [ ] 合法 `stage_delta` 只允许推进到下一阶段
- [ ] 合法 `stage_delta.reason` 会进入 `stage_change` 事件
- [ ] 非法跳级会 fallback，且不会更新 session stage
- [ ] 非法倒退或同阶段变化会 fallback
- [ ] fallback 不暴露原始非法 summary
- [ ] `npc_reply.validation_error` 对非法阶段跳转返回稳定 code
- [ ] Phase 2a validator 测试仍通过
- [ ] Phase 2b prompt/knowledge tests 仍通过
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
