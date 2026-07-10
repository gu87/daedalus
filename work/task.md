# 当前任务：Narrative Backend Phase 4a - 多 NPC Session 隔离 + 主动结束会话

> standard 模式：总控分派；developer 实现；reviewer 只读验收。
> 本轮只做多 NPC session 隔离验收和主动结束会话，不做 Phase 4 其他打磨项。

## 背景

Phase 3b 已完成安全 `speak` 工具事件流基础：

- raw `TaskStream { chunk }` 不进入玩家 SSE
- `narrative.speak` 只在 `speak` tool execute 成功后发出
- Session SSE 可在 `TaskDone` 前转发安全 `utterance_complete`
- `TaskDone.outbox.summary` 仍是状态/持久化权威

Phase 4 计划包含多 NPC 与打磨。本轮只取最小闭环：

- `/api/session/:session_id/end`
- 多 NPC session 之间状态、历史、prompt/knowledge、events、end 状态互不串

## 目标

新增端点：

```text
POST /api/session/:session_id/end
```

行为：

- session 存在、未结束、未 processing 时，写入 `session_end` game event
- `/api/session/:session_id/state` 通过现有 event 推导返回 `is_ended = true`
- 已结束 session 的 JSON `/message` 返回 `410 Gone`
- 已结束 session 的 `/message/stream` 返回 `410 Gone`，不 dispatch task
- 重复 end 返回 `410 Gone`
- 正在 processing 的 end 返回 `409 Conflict`
- 不存在 session 返回 `404 Not Found`
- 保持 DB schema 不变，结束状态只由 `session_end` event 派生

多 NPC 验收：

- 至少覆盖 `zhang_san` 和第二个测试 NPC
- 分别创建 session、发送不同状态/消息
- 证明 prompt 的 `npc_id` / knowledge boundary、messages、confession stage、unlocked clues、game events、end 状态均不串
- 证明结束 A 不影响 B

## 非目标

本轮不做：

- 不做多 NPC 编排
- 不做共享状态机
- 不做新的泛用 session 服务层
- 不做 history/context 截断或 LLM 摘要（Phase 4.2 / 4.4）
- 不做 fallback.yaml 配置化（Phase 4.5）
- 不改 Gate
- 不改 Agent Loop
- 不改 Tool trait
- 不改 DB migration
- 不接 SSE/Godot/真实 provider 新能力
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不 touch `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md`

## 允许修改

- `daedalusd/src/http/server.rs`
- `daedalusd/src/http/session.rs`
- `daedalusd/src/narrative/events.rs`
- `daedalusd/tests/http_session.rs`
- `work/task.md`
- `work/test-report.md`
- `work/callbacks.md`

## 验收标准

- [ ] `make_router(...)` 注册 `POST /api/session/:session_id/end`
- [ ] `run_http(...)` 注册 `POST /api/session/:session_id/end`
- [ ] end 成功后 state `is_ended = true`
- [ ] end 成功后 events 包含 `session_end`
- [ ] end 后 JSON `/message` 返回 `410 Gone`
- [ ] end 后 `/message/stream` 返回 `410 Gone`
- [ ] processing 时 end 返回 `409 Conflict`
- [ ] 不存在 session end 返回 `404 Not Found`
- [ ] 重复 end 返回 `410 Gone`
- [ ] 两 NPC session history/stage/clue/events/end 独立
- [ ] prompt/knowledge 不串
- [ ] 结束 A 不影响 B
- [ ] 原有 Phase 1-3 `http_session` 回归仍通过
- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] developer 完成后追加结论到 `work/callbacks.md` 并主动回传总控

## Phase 4a 完成定义

Reviewer PASS 后，Phase 4a 可标记完成：

```text
Daedalus Session API 支持主动结束审讯，并已验证多 NPC session 的状态、历史、知识边界与结束状态互相隔离。
```
