# 当前任务：Narrative Backend Phase 1e - 游戏 Tool 最小注册

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Daedalus 后续作为《雾井焚痕》AI Narrative Runtime 的后端底座。已完成：

```text
Phase 1a: 最小审讯 Session API
Phase 1b: game_events 事件日志
Phase 1c: state 响应补齐 emotional_state / turn_count / unlocked_clues / is_ended
Phase 1d: 阻塞 JSON /message 的确定性阶段/线索事件
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

本轮只让 daemon 的 ToolRegistry 能提供 5 个游戏专用 Tool 的最小定义和输入校验。先不接 LLM 对话、不写 session DB、不改 Session API、不改 Agent Loop。

## 目标

新增并注册 5 个 narrative/game tool：

```text
speak
update_confession_stage
reveal_clue
check_knowledge
log_interrogation_event
```

这些工具本轮只完成：

- provider-facing `ToolDef` 定义
- `validate()` 输入校验
- `execute()` 返回确定性 JSON 结果
- `DefaultAgentLoopFactory` 默认注册
- 测试覆盖工具定义、校验、执行和注册可见性

这是临时工具外壳，用来让后续 NPC agent 能拿到稳定工具名和 schema。真正写 DB、校验角色知识、触发 Gate AutoRevision 的逻辑后置。

## 范围

允许修改：

- `daedalusd/src/tools/mod.rs`
- `daedalusd/src/tools/narrative.rs`（建议新增）
- `daedalusd/src/daemon.rs`
- `daedalusd/tests/tool_registry.rs`
- 如需要可新增一个 focused test：`daedalusd/tests/narrative_tools.rs`
- `work/callbacks.md`
- `work/test-report.md`

不做：

- 不改数据库迁移或表结构
- 不改 `daedalusd/src/http/session.rs`
- 不改 HTTP 路由
- 不改 Agent Loop 9 状态机
- 不接 SSE
- 不接 Godot
- 不做 CharacterKnowledgeProvider
- 不做 Output Validator / Gate AutoRevision 集成
- 不改 managed-agents 配置文件
- 不改 `/Users/gu/daedalus-courtroom-demo`

## Tool 契约

### 公共要求

- 5 个工具都实现现有 `Tool` trait。
- `risk_level()` 返回 `RiskLevel::R1`。
- `allowed_agents()` 暂时返回 `["*"]`，后续 managed-agents 阶段再收紧。
- `needs_permission()` 返回 `false`。
- `execute()` 内部先调用 `validate()`；校验失败按现有模式映射为 `ToolError::InvalidInput`。
- `execute()` 返回 JSON 字符串，`is_error = false`。

### speak

输入：

```json
{
  "text": "我不知道你在说什么。",
  "emotion": "defensive"
}
```

校验：

- `text` 必须是非空字符串，trim 后长度不超过 500 字符
- `emotion` 必须是：`calm` / `defensive` / `nervous` / `anxious` / `angry` / `broken`

输出至少包含：

```json
{
  "event_type": "utterance_complete",
  "text": "...",
  "emotion": "defensive"
}
```

### update_confession_stage

输入：

```json
{
  "new_stage": "vague",
  "reason": "aggressive_pressure"
}
```

校验：

- `new_stage` 必须是：`denial` / `vague` / `partial` / `breakdown`
- `reason` 必须是非空字符串

输出至少包含：

```json
{
  "event_type": "stage_change",
  "new_stage": "vague",
  "reason": "aggressive_pressure"
}
```

### reveal_clue

输入：

```json
{
  "clue_id": "photo_1"
}
```

校验：

- `clue_id` 必须是非空字符串

输出至少包含：

```json
{
  "event_type": "clue_unlocked",
  "clue_id": "photo_1"
}
```

### check_knowledge

输入：

```json
{
  "fact_id": "liang_is_neighbor"
}
```

校验：

- `fact_id` 必须是非空字符串

输出至少包含：

```json
{
  "fact_id": "liang_is_neighbor",
  "allowed": true
}
```

说明：本轮还没有 CharacterKnowledgeProvider，`allowed` 先固定为 `true`，只用于打通工具调用形状。

### log_interrogation_event

输入：

```json
{
  "type": "player_pressure",
  "payload": {
    "pressure_level": "aggressive"
  }
}
```

校验：

- `type` 必须是非空字符串
- `payload` 必须是 JSON object

输出至少包含：

```json
{
  "event_type": "player_pressure",
  "payload": {
    "pressure_level": "aggressive"
  }
}
```

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test tool_registry` 通过
- [ ] 如新增 `narrative_tools` 测试，`cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo test -p daedalusd --test http_session` 仍通过，确认 Session API 未被破坏
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过，确认旧 task API 未被破坏
- [ ] `git diff --check` 通过
- [ ] 测试证明 5 个工具的 definition name 与 schema 可见
- [ ] 测试证明 5 个工具都不需要权限，risk 为 `R1`
- [ ] 测试证明无效输入会被拒绝
- [ ] 测试证明 `execute()` 返回可解析 JSON
- [ ] 测试证明 `DefaultAgentLoopFactory` 默认注册后，这 5 个工具可被 registry 找到或出现在 definitions 中
- [ ] 本轮未修改数据库迁移或表结构
- [ ] 本轮未修改 Session API / HTTP 路由
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
