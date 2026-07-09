# 当前任务：Narrative Backend Phase 1 completion - 最小审讯闭环

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 1a-1h 已完成：

- `POST /api/session/start`
- `POST /api/session/:session_id/message`
- `GET /api/session/:session_id/state`
- `sessions` / `game_events` 持久化
- state 契约字段
- 确定性阶段推进和线索事件
- 5 个 narrative/game tool
- `check_knowledge` 从 narrative root 读取 `knowledge.yaml`

现在 Phase 1 还差最小闭环：

- repo 内有一个可用于 Phase 1 的 `zhang_san` NPC 配置/人格/知识样例
- `/message` 不再只是纯 HTTP 层假回复，而是通过一个薄的 Session Adapter 走 `DaemonContext::spawn_task`
- `POST /api/session/:session_id/message` 继续返回当前阻塞 JSON，能手动一轮跑通

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

把 Daedalus Narrative Backend Phase 1 做到“最小可玩后端闭环”：

```text
start session
-> message
-> Session Adapter 构造 task
-> Agent Loop 执行并返回结果
-> Session API 写 messages/game_events/session state
-> state 可读回
```

本轮仍然不是完整 AI 剧情引擎，只要求后端底座最小闭环真实接上 Agent Loop。

## 必做

### 1. 补 repo 内 Phase 1 NPC 样例

新增最小 `zhang_san` NPC 样例文件，用于测试和手动验证。

要求：

- 不写入用户家目录配置
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 可以放在 `daedalusd/tests/fixtures/...`、`examples/...` 或更符合现有仓库风格的位置
- 至少包含：
  - `SOUL.md` 或等价人格提示样例
  - `managed-agents.yaml` 中的 `zhang_san`
  - `narrative/characters/zhang_san/knowledge.yaml`

### 2. 实现最小 Session Adapter

`POST /api/session/:session_id/message` 需要改成：

1. 校验 `player_text`
2. 原子地把 session 标记为 `is_processing = 1`
3. 读取 session 当前状态和历史消息
4. 构造一个 `TaskDispatch`
5. 调用 `state.ctx.spawn_task(...)`
6. 等待 `TaskDone` 或 `TaskError`
7. 从 Agent Loop 结果生成 NPC 回复
8. 写回：
   - player message
   - npc message
   - `game_events`
   - `confession_stage`
   - `is_processing = 0`
9. 返回现有阻塞 JSON response

保留当前 URL 和响应格式：

```text
POST /api/session/:session_id/message
Content-Type: application/json

Response: application/json
```

本轮不接 SSE。

### 3. Agent Loop 结果解析规则

保持最小实现即可：

- 如果 `TaskDone.outbox.summary` 是 JSON object，可尝试读取：
  - `utterance`
  - `emotion`
  - `confession_stage`
  - `revealed_clues`
- 如果不是 JSON，就把 summary 当作 `utterance`
- 缺失字段使用现有默认值
- 口供阶段推进和 evidence 解锁可继续沿用 daemon 侧确定性规则
- 不能因为模型输出不是 JSON 就让整个接口失败

### 4. 错误和状态恢复

必须处理：

- session 不存在：404
- session 正在处理：409
- `spawn_task` build 失败：返回 500，并恢复 `is_processing = 0`
- `TaskError`：返回 500，并恢复 `is_processing = 0`
- 等待超时：返回 504 或 500，并恢复 `is_processing = 0`

不能留下卡死在 `is_processing = 1` 的路径。

### 5. 测试证明真的接了 Agent Loop

更新或新增测试，必须证明 `/message` 的 utterance 来自 fake Agent Loop / fake provider 的输出，而不是旧的 `deterministic_utterance()`。

建议复用现有测试样板：

- `daedalusd/tests/full_dispatch.rs` 的 `TestAgentLoopFactory`
- `daedalusd/tests/http_session.rs` 的 HTTP server helper
- `daedalusd/src/http/tasks.rs` 的 `TaskDispatch` 构造方式

测试不要依赖真实 LLM 或外部网络。

### 6. 手动一轮验证

完成后需要用本地 HTTP server 或等价集成测试证明：

```text
POST /api/session/start
POST /api/session/:session_id/message
GET /api/session/:session_id/state
```

这一轮完整闭环可跑通。

如果用自动化测试覆盖了同样路径，也要在回传里写清楚对应测试名。

## 非目标

本轮不做：

- 不接 Godot
- 不接 `/Users/gu/daedalus-courtroom-demo`
- 不修改 `/Users/gu/daedalus-courtroom-demo`
- 不接 SSE
- 不实现完整 `CharacterKnowledgeProvider`
- 不做 Output Validator / Gate AutoRevision 集成
- 不改 Agent Loop 9 状态机
- 不改 Tool trait
- 不做多 NPC
- 不做缓存或 Provider 抽象
- 不接真实 LLM
- 不做 UI
- 不做数据库表结构大改；只有确实必要时才允许迁移，并必须说明原因

## 允许修改

优先限制在：

- `daedalusd/src/http/session.rs`
- `daedalusd/tests/http_session.rs`
- repo 内 Phase 1 NPC fixture / example 文件
- `work/callbacks.md`
- `work/test-report.md`

如确实需要，可最小修改：

- `daedalusd/src/http/mod.rs`
- `daedalusd/src/http/server.rs`
- `daedalusd/src/daemon.rs`
- `daedalusd/src/types.rs`
- `daedalusd/tests/http_tasks.rs`

每个超出优先范围的文件都要在回传里说明原因。

## 验收标准

- [ ] `POST /api/session/start` 仍可创建 session
- [ ] `POST /api/session/:session_id/message` 会构造 task 并调用 `DaemonContext::spawn_task`
- [ ] `/message` 返回的 utterance 在测试中来自 fake Agent Loop / fake provider 输出
- [ ] `/message` 仍写入 player/npc messages
- [ ] `/message` 仍写入 `player_message`、`npc_reply`、必要的 `stage_change`、`clue_unlocked`
- [ ] `GET /api/session/:session_id/state` 能读回 messages/events/state
- [ ] 409 processing 分支仍通过
- [ ] spawn/build/error/timeout 分支不会遗留 `is_processing = 1`
- [ ] repo 内有最小 `zhang_san` NPC fixture/example
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
