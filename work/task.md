# 当前任务：Narrative Backend Phase 1g - check_knowledge 读取 knowledge.yaml

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 1f 已让 `check_knowledge` 不再固定返回 `true`，但仍有一个剩余风险：事实表硬编码在 `daedalusd/src/tools/narrative.rs`。

本轮只解决这个风险的最小可验证版本：让 `check_knowledge` 从当前任务工作目录下的 `knowledge.yaml` 文件加载规则。先不接 DB、不接 Session API、不接 Godot、不接 LLM、不做完整 `CharacterKnowledgeProvider` prompt 注入。

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

让 `check_knowledge` 使用文件规则，而不是内置单 NPC 事实表：

- `check_knowledge` 按 `ToolContext.work_dir` 查找知识文件
- 从 `knowledge.yaml` 加载 `npc_id`、`knows`、`hides`
- 根据 `fact_id` 和 `confession_stage` 返回 `allowed/reason`
- 文件缺失、NPC 不匹配、未知事实、阶段不足都稳定返回 `allowed = false`
- 保留 Phase 1f 的输入契约：`{npc_id, fact_id, confession_stage}`

## 知识文件路径

固定为：

```text
{ToolContext.work_dir}/narrative/characters/{npc_id}/knowledge.yaml
```

例：

```text
/some/game/root/narrative/characters/zhang_san/knowledge.yaml
```

说明：

- 本轮不新增全局配置项。
- 本轮不读取 `/Users/gu/daedalus-courtroom-demo`。
- 测试里用临时目录创建这个文件。

## YAML 最小契约

支持以下字段即可：

```yaml
npc_id: zhang_san
case_id: wujing_fenhen

knows:
  - fact_id: liang_is_neighbor
    content: "梁远山是我的邻居，住在302"
  - fact_id: liang_left_nov3
    content: "梁远山11月3日晚上带着行李出门了"
    unlock_condition: "stage >= vague"
  - fact_id: saw_lu_jiping
    content: "我看到卢继平在11月4日早上来过"
    unlock_condition: "stage >= partial"

hides:
  - fact_id: helped_cover
    content: "我帮忙处理了现场"
    reveal_stage: breakdown
```

字段规则：

- `npc_id` 必填，必须等于输入 `npc_id` 才允许继续判定
- `case_id` 本轮可解析但不参与判定
- `knows[].fact_id` 必填
- `knows[].unlock_condition` 可选；缺省等同 `stage >= denial`
- `hides[].fact_id` 必填
- `hides[].reveal_stage` 可选；缺省等同 `breakdown`

`unlock_condition` 本轮只需要支持这种格式：

```text
stage >= denial
stage >= vague
stage >= partial
stage >= breakdown
```

遇到其他格式，可把该 fact 当作不可用并返回 `allowed = false` / `reason = invalid_rule`。

## 判定规则

阶段顺序：

```text
denial < vague < partial < breakdown
```

输出 reason 使用稳定字符串：

```text
stage_allows_fact
knowledge_file_missing
npc_mismatch
unknown_fact
stage_blocks_fact
invalid_rule
knowledge_file_invalid
```

建议规则：

- knowledge 文件不存在：`allowed = false`，`reason = knowledge_file_missing`
- YAML 解析失败：`allowed = false`，`reason = knowledge_file_invalid`
- 文件内 `npc_id` 与输入不一致：`allowed = false`，`reason = npc_mismatch`
- `fact_id` 不在 `knows` 或 `hides`：`allowed = false`，`reason = unknown_fact`
- 当前阶段达到 `knows.unlock_condition` 或 `hides.reveal_stage`：`allowed = true`，`reason = stage_allows_fact`
- 当前阶段不足：`allowed = false`，`reason = stage_blocks_fact`
- 规则字段格式不支持：`allowed = false`，`reason = invalid_rule`

## 非目标

本轮不做：

- 不实现完整 `CharacterKnowledgeProvider`
- 不把知识内容注入 system prompt
- 不做 Output Validator / Gate AutoRevision 集成
- 不改数据库迁移或表结构
- 不改 `sessions` / `game_events` 写入逻辑
- 不改 Session API / HTTP 路由
- 不改 Agent Loop 9 状态机
- 不接 SSE
- 不接 Godot
- 不接 LLM
- 不新增全局配置项
- 不收紧 `allowed_agents()` 或权限弹窗
- 不改 `/Users/gu/daedalus-courtroom-demo`

## 允许修改

- `daedalusd/src/tools/narrative.rs`
- `daedalusd/tests/narrative_tools.rs`
- `work/callbacks.md`
- `work/test-report.md`

如确实需要，可在不引入新依赖的前提下改：

- `daedalusd/src/tools/mod.rs`

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo test -p daedalusd --test tool_registry` 通过
- [ ] `cargo test -p daedalusd --test http_session` 仍通过
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] 测试证明从临时目录的 `narrative/characters/zhang_san/knowledge.yaml` 读取规则
- [ ] 测试证明 `knows` 无 `unlock_condition` 时 `denial` 阶段允许
- [ ] 测试证明 `knows` 有 `unlock_condition: "stage >= partial"` 时，`vague` 阶段禁止、`partial` 阶段允许
- [ ] 测试证明 `hides` 的 `reveal_stage: breakdown` 在 `partial` 阶段禁止、`breakdown` 阶段允许
- [ ] 测试证明文件缺失返回 `allowed = false` / `reason = knowledge_file_missing`
- [ ] 测试证明未知 `fact_id` 返回 `allowed = false` / `reason = unknown_fact`
- [ ] 测试证明无效 `confession_stage` 仍被拒绝为 `ToolError::InvalidInput`
- [ ] 本轮未修改数据库迁移或表结构
- [ ] 本轮未修改 Session API / HTTP 路由
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
