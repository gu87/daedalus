# 当前任务：Narrative Backend Phase 3a - 最小 SSE Session Message 端点

> standard 模式：总控分派；developer 实现；reviewer 只读验收。
> 本轮目标是让 Godot 能先接 `text/event-stream` 形态，不做真正 token 级流式生成。

## 背景

Phase 2 已完成并推送：

```text
ad7176d chore: record narrative phase 2 completion
```

当前能力：

- `POST /api/session/start` 创建审讯 session
- `POST /api/session/:session_id/message` 返回阻塞 JSON
- `GET /api/session/:session_id/state` 返回当前 session 状态
- `/message` 已接 Agent Loop，校验 NPC Reply JSON，fallback 不泄露非法原文
- prompt/knowledge 边界、单步阶段机、证据门槛、候选证据门槛均已通过回归

Phase 3 的第一步是给游戏端一个 SSE 入口。先做最小可用的事件流封装，复用现有 message 逻辑，不碰 LLM provider token streaming。

## 目标

新增端点：

```text
POST /api/session/:session_id/message/stream
Content-Type: text/event-stream
```

请求体沿用现有 `/message`：

```json
{
  "player_text": "你认识梁远山吗？",
  "evidence_id": "photo_1",
  "pressure_level": "normal"
}
```

返回 SSE 事件：

```text
event: utterance_complete
data: {"session_id":"...","npc_id":"zhang_san","full_text":"...","emotion":"anxious"}

event: stage_change
data: {"session_id":"...","old_stage":"denial","new_stage":"vague","reason":"..."}

event: clue_unlocked
data: {"session_id":"...","clue_id":"photo_1"}

event: done
data: {"session_id":"...","confession_stage":"vague"}
```

## 行为规则

- stream 端点必须复用现有 `/message` 的核心处理逻辑
- JSON `/message` 行为不能变化
- 如果 NPC reply fallback，SSE 里的 `utterance_complete.full_text` 也必须是安全 fallback 文本，不能泄露非法 summary
- 如果阶段发生变化，发送 `stage_change`
- 如果有 `revealed_clues`，每个 clue 发送一个 `clue_unlocked`
- 最后总是发送 `done`
- 本轮可以是“处理完成后一次性发出多个 SSE event”，不要求 token/chunk 实时流

## 非目标

本轮不做：

- 不做真正 LLM token streaming
- 不接 Godot 项目代码
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不接真实 LLM provider smoke
- 不接 Gate AutoRevision
- 不改 Agent Loop
- 不改 Tool trait
- 不改数据库表结构
- 不做多 NPC 状态机
- 不做复杂条件表达式
- 不加新依赖，除非现有依赖无法完成

## 允许修改

优先限制在：

- `daedalusd/src/http/server.rs`
- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- `work/test-report.md`
- `work/callbacks.md`

不要修改：

- `/Users/gu/daedalus-courtroom-demo`
- DB migrations
- Agent Loop
- Tool trait
- LLM provider

## 建议实现

保持短 diff：

- 把现有 `message_session(...)` 内部主要逻辑抽成一个私有 helper，例如 `handle_session_message(...)`
- `message_session(...)` 继续返回 JSON
- 新增 `stream_session_message(...)` 调用同一个 helper，再把结果包装成 axum SSE
- 可以使用 `axum::response::sse::{Sse, Event}` 和 `futures_util::stream`
- SSE data 用 JSON 字符串
- 如果 helper 已能知道 old/new stage，直接生成 `stage_change`；否则只在 new stage != old stage 时生成

不要新增模块，不要做抽象层。

## 验收标准

- [ ] 新增 `POST /api/session/:session_id/message/stream`
- [ ] `make_router(...)` 和 `run_http(...)` 都注册该路由
- [ ] stream 端点返回 `text/event-stream`
- [ ] stream 成功时包含 `utterance_complete`
- [ ] stream 成功时最后包含 `done`
- [ ] 阶段变化时包含 `stage_change`
- [ ] revealed clues 非空时包含 `clue_unlocked`
- [ ] fallback 时 SSE 不泄露非法 summary
- [ ] 原有 JSON `/message` 测试仍通过
- [ ] Phase 2a-2e 的 `http_session` 回归仍通过
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## Phase 3a 完成定义

Reviewer PASS 后，Phase 3a 可标记完成：

```text
Godot 已可通过 Daedalus Session API 获得 SSE 格式的审讯结果事件流。
```
