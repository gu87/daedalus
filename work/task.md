# 当前任务：Narrative Backend Phase 3b - 安全 speak 工具事件流

> standard 模式：总控分派；developer 实现；reviewer 只读验收。
> 本轮只实现安全的 typed narrative speak 事件，不宣称 provider token streaming。

## 背景

Phase 3a 已提供 `POST /api/session/:session_id/message/stream` 的 SSE 端点，但它是完成后批量发送最终结果事件。原始 `task.stream` 是 provider raw text，不能直接作为玩家可见 SSE。

现有 narrative `speak` tool 已能校验：

- `text` 非空且最多 500 字符
- `emotion` 属于 `calm | defensive | nervous | anxious | angry | broken`
- execute 成功返回结构化 JSON：`event_type = utterance_complete`

因此本轮以“`speak` 工具 execute 成功”为最小安全提交边界。

## 目标

- 新增 typed IPC message：`narrative.speak`
- Agent Loop 仅在 `speak` 工具成功 execute 后发送 `narrative.speak`
- Session SSE 在 `TaskDone` 前消费 `narrative.speak`，再按当前 session `game_state.forbidden_terms`、长度和 emotion 做二次校验，通过后立即发送 `utterance_complete`
- `TaskDone.outbox.summary` 仍是状态推进、持久化、stage/clue/done 的权威
- prompt/输出契约要求 NPC 先调用 `speak(text, emotion)`，再调用 `task_done`，且二者 text/emotion 一致

## 非目标

本轮不做：

- 不把 raw `TaskStream { chunk }` 发给玩家 SSE
- 不做 provider token streaming
- 不做 tool-call argument delta streaming
- 不接 Godot 项目代码
- 不接真实 LLM provider smoke
- 不接 Gate AutoRevision
- 不改 DB migration
- 不改 Tool trait
- 不改 provider
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不做通用事件总线或多 NPC 抽象

## 行为规则

- `narrative.speak` 只代表已执行成功的 `speak` 工具输出，不代表 provider token
- Session SSE 必须忽略 raw `TaskStream`
- Session SSE 必须忽略包含 forbidden term 或非法 text/emotion 的 speak event
- 若已放行 speak，最终合法 `task_done.summary` 的 `utterance` / `emotion` 必须与 speak 一致
- 若不一致，Session 走安全 fallback，不追加第二段原始 LLM 文本
- JSON `/message` 行为保持回归稳定

## 允许修改

仅限完成本轮验收所需文件，预期包括：

- `daedalusd/src/types.rs`
- `daedalusd/src/ipc/protocol.rs`
- `daedalusd/src/ipc/control.rs`
- `daedalusd/src/agent/loop.rs`
- `daedalusd/src/http/session.rs`
- `daedalusd/src/narrative/mod.rs`
- `daedalusd/src/narrative/session_adapter.rs`
- `daedalusd/src/narrative/stream_events.rs`
- `daedalusd/tests/agent_loop.rs`
- `daedalusd/tests/http_session.rs`
- `work/task.md`
- `work/test-report.md`
- `work/callbacks.md`

不要 stage/revert 既有 `.codex/agents/*.md`、`AGENTS.md`、`work/registry.md` 脏改动。

## 验收标准

- [ ] 新的 typed narrative event 仅在 `speak` 工具成功 execute 后发出
- [ ] Agent Loop 单测覆盖 `narrative.speak`
- [ ] `/api/session/:id/message/stream` 能在 `TaskDone` 前从 typed event 发送 `utterance_complete`
- [ ] fake provider 测试先提交 `speak` ToolCall，延迟后再提交 `task_done`，并证明 event 到达时任务尚未结束
- [ ] forbidden term / 非法 emotion 不会发到 SSE
- [ ] raw `TaskStream` 不会发到 SSE
- [ ] 最终 state / persist / stage / clue / done 保持现有语义
- [ ] JSON `/message` 回归不变
- [ ] 不修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test agent_loop` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] developer 完成后追加结论到 `work/callbacks.md` 并主动回传总控

## Phase 3b 完成定义

Reviewer PASS 后，Phase 3b 可标记完成：

```text
Session SSE 已能基于已成功执行的安全 speak 工具事件，在 TaskDone 前推送完整玩家可见台词；raw provider stream 仍不会暴露给玩家。
```
