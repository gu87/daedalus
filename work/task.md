# 当前任务：Narrative Backend Phase 1d - 确定性阶段/线索事件

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Daedalus 后续作为《雾井焚痕》AI Narrative Runtime 的后端底座。已完成：

```text
Phase 1a: 最小审讯 Session API
Phase 1b: game_events 事件日志
Phase 1c: state 响应补齐 emotional_state / turn_count / unlocked_clues / is_ended
```

当前分支：

```text
/Users/gu/Daedalus
branch: codex/narrative-backend
```

游戏 Demo、剧本、素材、Godot 仍在另一个仓库，不在本任务中修改：

```text
/Users/gu/daedalus-courtroom-demo
branch: main
```

本轮只让现有阻塞 JSON `/message` 能确定性地产生最小游戏状态变化。不接 Tool / SSE / Godot / LLM / CharacterKnowledgeProvider / Agent Loop，不改 DB schema。

## 目标

在 `POST /api/session/:session_id/message` 中加入最小确定性状态效果：

```text
pressure_level = "aggressive" 且当前 confession_stage = "denial"
  -> confession_stage 推进到 "vague"
  -> 写 stage_change 事件

evidence_id 非空
  -> 写 clue_unlocked 事件
  -> GET state 的 unlocked_clues 能读回该 clue_id
```

这是临时确定性规则，用来跑通游戏状态闭环。后续接 Tool/LLM 后，这些事件会由工具或结构化输出驱动。

## 范围

允许修改：

- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- `work/callbacks.md`
- `work/test-report.md`

不做：

- 不改数据库迁移或表结构
- 不改 HTTP 路由
- 不改 Agent Loop 9 状态机
- 不接 SSE
- 不接 Godot
- 不做多 NPC
- 不做 CharacterKnowledgeProvider
- 不做 5 个游戏 Tool
- 不做 Gate AutoRevision 集成
- 不做 managed-agents 配置
- 不改 `/Users/gu/daedalus-courtroom-demo`

## API 契约

### POST /api/session/:session_id/message

请求/响应结构保持不变。

新增行为：

- `pressure_level == "aggressive"` 且当前 `confession_stage == "denial"` 时：
  - 更新 sessions 表中的 `confession_stage` 为 `"vague"`。
  - NPC 回复 response 中的 `confession_stage` 返回 `"vague"`。
  - NPC message 中的 `confession_stage` 写 `"vague"`。
  - 写入一条 `stage_change` 事件，payload 至少包含：

```json
{
  "session_id": "sess-...",
  "npc_id": "zhang_san",
  "old_stage": "denial",
  "new_stage": "vague",
  "reason": "aggressive_pressure"
}
```

- `evidence_id` 非空字符串时：
  - 写入一条 `clue_unlocked` 事件。
  - payload 至少包含 `clue_id`，值先等于 `evidence_id`。
  - response 的 `revealed_clues` 返回该 clue_id。

### GET /api/session/:session_id/state

保持 Phase 1c 结构不变。

新增可观察结果：

- aggressive message 后，state 的 `confession_stage` 为 `"vague"`。
- evidence message 后，state 的 `unlocked_clues` 包含该 clue_id。
- events 列表能读到 `stage_change` / `clue_unlocked`。

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过，确认旧 task API 不被破坏
- [ ] `git diff --check` 通过
- [ ] 测试证明 normal message 不改变 `confession_stage`
- [ ] 测试证明 aggressive message 将 `denial` 推进到 `vague`，response 和 state 都返回 `"vague"`
- [ ] 测试证明 aggressive message 写入 `stage_change` 事件
- [ ] 测试证明带 `evidence_id` 的 message 写入 `clue_unlocked` 事件，response `revealed_clues` 与 state `unlocked_clues` 都包含该 id
- [ ] 本轮未修改数据库迁移或表结构
- [ ] 本轮未修改 HTTP 路由
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
