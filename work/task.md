# 当前任务：Narrative Backend Phase 1b - game_events 事件日志

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Daedalus 后续作为《雾井焚痕》AI Narrative Runtime 的后端底座。上一轮已经完成最小审讯 Session API：

```text
POST /api/session/start
POST /api/session/:session_id/message
GET  /api/session/:session_id/state
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

本轮只补“剧情运行时可追踪事件”的最小后端能力，不接 Godot，不做 SSE，不做完整角色知识系统，不改 Agent Loop。

## 目标

在现有 Session API 基础上新增 `game_events` 持久化事件日志，并让最小 API 闭环可验证：

```text
start session -> 写 session_start 事件
send player message -> 写 player_message / npc_reply 或等价 turn 事件
get state -> 能读回事件结果
```

事件内容第一版可以是确定性 JSON，不接 LLM，不实现游戏 Tool。

## 范围

允许修改：

- `daedalusd/src/db/migrations.rs`
- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- `daedalusd/tests/db_registry.rs`（仅在迁移版本断言必须更新时）
- `work/callbacks.md`
- `work/test-report.md`

不做：

- 不改 Agent Loop 9 状态机
- 不接 SSE
- 不接 Godot
- 不做多 NPC
- 不做 CharacterKnowledgeProvider
- 不做 5 个游戏 Tool
- 不做 Gate AutoRevision 集成
- 不做 managed-agents 配置
- 不改 `/Users/gu/daedalus-courtroom-demo`

## 数据模型

新增最小 `game_events` 表，字段以能支撑本轮验收为准：

```text
event_id
session_id
event_type
payload_json
created_at
```

建议约束：

```text
event_id TEXT PRIMARY KEY
session_id TEXT NOT NULL
event_type TEXT NOT NULL
payload_json TEXT NOT NULL
created_at INTEGER NOT NULL
FOREIGN KEY(session_id) REFERENCES sessions(session_id)
```

建议索引：

```text
game_events(session_id, created_at)
```

`payload_json` 必须是 JSON 字符串，第一版只放本轮已有字段，例如 `npc_id`、`case_id`、`player_text`、`utterance`、`confession_stage`、`emotion`、`revealed_clues`。

## API 契约

### POST /api/session/start

保持上一轮请求和响应结构不变。

新增行为：

- 创建 session 成功后，写入 `session_start` 事件。
- 事件 payload 至少包含 `session_id`、`npc_id`、`case_id`、`confession_stage`。

### POST /api/session/:session_id/message

保持上一轮请求和响应结构不变。

新增行为：

- 成功处理玩家消息后，写入可验证的事件。
- 可以选择写两条事件：`player_message`、`npc_reply`。
- 也可以选择写一条事件：`message_turn`。
- 选择哪种由 developer 根据现有代码最小改动决定，但测试必须明确断言事件类型和数量。

同一 session 正在处理时仍应返回 `409 Conflict`。本轮不要求为冲突写事件。

### GET /api/session/:session_id/state

在上一轮返回当前 session 状态和 message history 的基础上，新增最小事件可读结果。

可接受两种实现之一：

```json
{
  "events": [
    {
      "event_id": "evt-...",
      "event_type": "session_start",
      "payload": {},
      "created_at": 123
    }
  ]
}
```

或：

```json
{
  "event_count": 3
}
```

优先返回 `events`，因为后续会接 Godot 和剧情调试面板；如果实现成本明显更高，可以先返回 `event_count`。

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过，确认旧 task API 不被破坏
- [ ] `cargo test -p daedalusd migration` 通过
- [ ] `POST /api/session/start` 测试证明会写入 `session_start` 事件
- [ ] `POST /api/session/:session_id/message` 测试证明会写入 message/turn 相关事件
- [ ] `GET /api/session/:session_id/state` 测试证明能读回事件列表或事件数量
- [ ] 旧 Session API 请求/响应结构保持兼容
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
