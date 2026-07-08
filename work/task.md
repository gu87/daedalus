# 当前任务：Narrative Backend Phase 1c - Session State 契约补齐

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Daedalus 后续作为《雾井焚痕》AI Narrative Runtime 的后端底座。前两轮已经完成：

```text
Phase 1a: 最小审讯 Session API
Phase 1b: game_events 事件日志
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

本轮只补 `GET /api/session/:session_id/state` 的游戏侧可用状态字段，不做 DB schema 迁移，不接 Tool / SSE / Godot / LLM / CharacterKnowledgeProvider / Agent Loop。

## 目标

让现有 state API 更接近游戏保存/恢复契约，基于已有 `sessions.messages_jsonl` 和 `game_events` 计算并返回：

```text
emotional_state
turn_count
unlocked_clues
is_ended
created_at
updated_at
```

本轮不新增 SQLite 字段。`turn_count`、`emotional_state`、`unlocked_clues`、`is_ended` 都从现有消息和事件推导，避免为还没稳定的游戏状态提前改表。

## 范围

允许修改：

- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- `work/callbacks.md`
- `work/test-report.md`

不做：

- 不改数据库迁移或表结构
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

### GET /api/session/:session_id/state

在现有响应基础上新增字段：

```json
{
  "session_id": "sess-...",
  "npc_id": "zhang_san",
  "case_id": "wujing_fenhen",
  "confession_stage": "denial",
  "emotional_state": "defensive",
  "turn_count": 1,
  "unlocked_clues": [],
  "messages": [],
  "events": [],
  "is_processing": false,
  "is_ended": false,
  "created_at": 123,
  "updated_at": 456
}
```

字段规则：

- `turn_count`：从 `messages` 中 `role == "npc"` 的条数计算。
- `emotional_state`：取最后一条 NPC message 的 `emotion`；没有 NPC message 时返回 `"calm"`。
- `unlocked_clues`：从 `events` 中 `event_type == "clue_unlocked"` 的 payload 推导；当前没有 clue 事件时返回空数组。
- `is_ended`：当前没有 session_end 持久状态，先从 `events` 中是否存在 `session_end` 推导。
- `created_at` / `updated_at`：直接返回 sessions 表已有时间戳。

`POST /api/session/start` 和 `POST /api/session/:session_id/message` 的请求/响应结构本轮保持不变。

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过，确认旧 task API 不被破坏
- [ ] `git diff --check` 通过
- [ ] `GET /api/session/:session_id/state` 测试证明初始 session 返回 `emotional_state = "calm"`、`turn_count = 0`、`unlocked_clues = []`、`is_ended = false`
- [ ] `POST /api/session/:session_id/message` 后，state 测试证明 `turn_count = 1`、`emotional_state` 来自 NPC 回复
- [ ] `created_at` / `updated_at` 字段存在且为数字
- [ ] 本轮未修改数据库迁移或表结构
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
