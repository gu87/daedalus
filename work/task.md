# 当前任务：Narrative Backend Phase 1 - 最小审讯 Session API

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Daedalus 后续作为《雾井焚痕》AI Narrative Runtime 的后端底座。当前分支：

```text
/Users/gu/Daedalus
branch: codex/narrative-backend
```

游戏 Demo、剧本、素材、Godot 仍在另一个仓库，不在本任务中修改：

```text
/Users/gu/daedalus-courtroom-demo
branch: main
```

本轮只做后端最小闭环，不接 Godot，不做 SSE，不做完整角色知识系统。

## 目标

实现最小审讯 session 后端：

```text
POST /api/session/start
POST /api/session/:session_id/message
GET  /api/session/:session_id/state
```

第一版 `/message` 返回阻塞 JSON。可以先用确定性假 NPC 回复跑通 DB/API/session 状态；如复用现有 Agent Loop 成本过高，本轮不强求接 LLM。

## 范围

允许修改：

- `daedalusd/src/db/migrations.rs`
- `daedalusd/src/http/server.rs`
- `daedalusd/src/http/mod.rs`
- `daedalusd/src/http/session.rs`
- `daedalusd/src/session/mod.rs`
- `daedalusd/src/lib.rs`
- 相关最小测试文件

不做：

- 不改 Agent Loop 9 状态机
- 不接 SSE
- 不接 Godot
- 不做多 NPC
- 不做 CharacterKnowledgeProvider
- 不做 5 个游戏 Tool
- 不做 Gate AutoRevision 集成
- 不改 `/Users/gu/daedalus-courtroom-demo`

## 数据模型

新增最小 `sessions` 表，字段以能支撑本轮验收为准：

```text
session_id
npc_id
case_id
confession_stage
game_state_json
messages_jsonl
is_processing
created_at
updated_at
```

`messages_jsonl` append-only，每行一条 JSON。

## API 契约

### POST /api/session/start

输入示例：

```json
{
  "npc_id": "zhang_san",
  "scene_id": "police_office",
  "game_state": {
    "case_id": "wujing_fenhen",
    "unlocked_evidence_ids": [],
    "player_reputation": 50,
    "time_pressure": 0.3
  },
  "initial_confession_stage": "denial"
}
```

返回 `201`，包含：

```json
{
  "session_id": "sess-...",
  "npc_id": "zhang_san",
  "confession_stage": "denial"
}
```

### POST /api/session/:session_id/message

输入示例：

```json
{
  "player_text": "你认识梁远山吗？",
  "evidence_id": null,
  "pressure_level": "normal"
}
```

返回 `200`，包含最小 NPC 输出：

```json
{
  "session_id": "sess-...",
  "npc_id": "zhang_san",
  "utterance": "我不知道你在说什么。",
  "emotion": "defensive",
  "confession_stage": "denial",
  "revealed_clues": []
}
```

同一 session 正在处理时，应返回 `409 Conflict`。

### GET /api/session/:session_id/state

返回当前 session 状态和 message history。

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd session` 或同等 targeted session/http_session 测试通过
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过，确认旧 task API 不被破坏
- [ ] `POST /api/session/start` 有测试覆盖，能返回有效 session_id
- [ ] `POST /api/session/:session_id/message` 有测试覆盖，能追加 player/npc message 并返回 utterance JSON
- [ ] `GET /api/session/:session_id/state` 有测试覆盖，能读回 session 和 history
- [ ] 同一 session 并发/processing 状态有最小测试或明确实现，返回 409
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
