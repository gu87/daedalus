# 当前任务：Narrative Backend Phase 1f - check_knowledge 最小真实判定

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 1e 已新增并默认注册 5 个 narrative/game tool：

```text
speak
update_confession_stage
reveal_clue
check_knowledge
log_interrogation_event
```

剩余风险里最明确的一项是：`check_knowledge.allowed` 当前固定为 `true`，还不能约束 NPC 角色知识边界。

本轮只解决这个风险的最小可验证版本：让 `check_knowledge` 基于静态角色知识规则返回 true/false。先不接 LLM、不接 DB、不接 Godot、不改 Session API、不改 Agent Loop。

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

## 目标

实现最小 `CharacterKnowledge` 判定能力，并接入 `check_knowledge` tool：

- `check_knowledge` 输入从 `{fact_id}` 扩展为 `{npc_id, fact_id, confession_stage}`
- 对已知 NPC/事实/阶段返回确定性 `allowed`
- 对未知事实返回 `allowed = false`
- 对阶段未达到的事实返回 `allowed = false`
- 测试覆盖允许、禁止、未知、无效输入、旧工具回归

## 非目标

本轮不做：

- 不读外部 `knowledge.yaml`
- 不新增配置文件或 managed-agents 配置
- 不改数据库迁移或表结构
- 不改 `sessions` / `game_events` 写入逻辑
- 不改 `POST /api/session/:session_id/message`
- 不改 HTTP 路由
- 不改 Agent Loop 9 状态机
- 不接 SSE
- 不接 Godot
- 不接 LLM
- 不做 Output Validator / Gate AutoRevision 集成
- 不收紧 `allowed_agents()` 或权限弹窗
- 不改 `/Users/gu/daedalus-courtroom-demo`

## 最小规则

先内置一个 NPC 的最小知识规则，用于跑通判定形状：

```text
npc_id: zhang_san
case_id: wujing_fenhen
```

允许事实：

| fact_id | 最早允许阶段 |
|---------|--------------|
| `liang_is_neighbor` | `denial` |
| `liang_left_nov3` | `vague` |
| `saw_lu_jiping` | `partial` |
| `helped_cover` | `breakdown` |

阶段顺序：

```text
denial < vague < partial < breakdown
```

判定规则：

- `npc_id` 不是 `zhang_san`：`allowed = false`
- `fact_id` 不在规则表：`allowed = false`
- 当前 `confession_stage` 小于事实最早允许阶段：`allowed = false`
- 当前 `confession_stage` 大于或等于事实最早允许阶段：`allowed = true`

## Tool 契约

### check_knowledge 输入

```json
{
  "npc_id": "zhang_san",
  "fact_id": "liang_left_nov3",
  "confession_stage": "vague"
}
```

校验：

- `npc_id` 必须是非空字符串
- `fact_id` 必须是非空字符串
- `confession_stage` 必须是：`denial` / `vague` / `partial` / `breakdown`

输出至少包含：

```json
{
  "npc_id": "zhang_san",
  "fact_id": "liang_left_nov3",
  "confession_stage": "vague",
  "allowed": true,
  "reason": "stage_allows_fact"
}
```

`reason` 建议使用稳定字符串：

```text
stage_allows_fact
unknown_npc
unknown_fact
stage_blocks_fact
```

## 建议实现

保持最短 diff：

- 可在 `daedalusd/src/tools/narrative.rs` 内部先写私有规则函数，不要为了单个静态表新建复杂抽象。
- 如果为了测试清晰，需要暴露小函数，可以只暴露最小 `pub(crate)`/`pub` API。
- 不要引入新依赖。
- 不要把 YAML 解析提前做掉。

## 允许修改

- `daedalusd/src/tools/narrative.rs`
- `daedalusd/tests/narrative_tools.rs`
- 如确有必要，可改 `daedalusd/src/tools/mod.rs`
- `work/callbacks.md`
- `work/test-report.md`

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo test -p daedalusd --test tool_registry` 通过
- [ ] `cargo test -p daedalusd --test http_session` 仍通过
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] 测试证明 `zhang_san + liang_is_neighbor + denial` 返回 `allowed = true`
- [ ] 测试证明 `zhang_san + saw_lu_jiping + vague` 返回 `allowed = false`
- [ ] 测试证明未知 `fact_id` 返回 `allowed = false`
- [ ] 测试证明未知 `npc_id` 返回 `allowed = false`
- [ ] 测试证明无效 `confession_stage` 被拒绝为 `ToolError::InvalidInput`
- [ ] 本轮未修改数据库迁移或表结构
- [ ] 本轮未修改 Session API / HTTP 路由
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写业务代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
