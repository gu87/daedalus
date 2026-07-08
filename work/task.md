# 当前任务：Narrative Backend Phase 1h - narrative root 环境变量契约

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 总控不写业务代码。developer 只做本任务最小实现；reviewer 只做只读验收。

## 背景

Phase 1g 已让 `check_knowledge` 从：

```text
{ToolContext.work_dir}/narrative/characters/{npc_id}/knowledge.yaml
```

读取角色知识文件。

剩余风险是：真实游戏内容仓库可能不等于 agent task 的 `work_dir`。如果 Daedalus 后端运行目录和游戏内容目录分离，当前实现还缺一个明确的外部接线契约。

本轮只解决这个风险的最小版本：增加一个环境变量覆盖根目录。

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

为 narrative 知识文件增加明确 root 解析规则：

```text
DAEDALUS_NARRATIVE_ROOT 优先
否则回退 ToolContext.work_dir
```

最终读取路径仍然是：

```text
{resolved_root}/narrative/characters/{npc_id}/knowledge.yaml
```

这样后续运行 daemon 时，可以用：

```bash
DAEDALUS_NARRATIVE_ROOT=/Users/gu/daedalus-courtroom-demo
```

让 Daedalus 读取游戏内容仓库里的 `narrative/characters/.../knowledge.yaml`。

## 规则

- `DAEDALUS_NARRATIVE_ROOT` 未设置：继续使用 `ToolContext.work_dir`
- `DAEDALUS_NARRATIVE_ROOT` 设置为空字符串或全空白：继续使用 `ToolContext.work_dir`
- `DAEDALUS_NARRATIVE_ROOT` 设置为非空字符串：使用该路径作为 root
- 不要求本轮 canonicalize root
- 不要求 root 必须存在；不存在时沿用 Phase 1g 的 `knowledge_file_missing`
- 不新增配置文件
- 不新增依赖

## 非目标

本轮不做：

- 不接 Godot
- 不接 `/Users/gu/daedalus-courtroom-demo` 的真实文件
- 不改 `/Users/gu/daedalus-courtroom-demo`
- 不实现完整 `CharacterKnowledgeProvider`
- 不把知识内容注入 system prompt
- 不做 Output Validator / Gate AutoRevision 集成
- 不做缓存
- 不做 Provider 抽象
- 不改数据库迁移或表结构
- 不改 Session API / HTTP 路由
- 不改 Agent Loop 9 状态机
- 不接 SSE
- 不接 LLM
- 不收紧 `allowed_agents()` 或权限弹窗

## 允许修改

- `daedalusd/src/tools/narrative.rs`
- `daedalusd/tests/narrative_tools.rs`
- `work/callbacks.md`
- `work/test-report.md`

如确实需要，可在不引入新依赖的前提下改：

- `daedalusd/src/tools/mod.rs`

## 建议实现

保持最短 diff：

- 在 `narrative.rs` 里给 knowledge path 增加一个小 resolver。
- 只让 `check_knowledge` 的读文件路径使用 resolver。
- 测试里用临时目录分别放 `work_dir` 和 `env_root` 两套 `knowledge.yaml`，证明 env root 优先。
- 如果测试需要修改环境变量，用测试内 mutex 串行化，避免并发污染。

## 验收标准

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo test -p daedalusd --test tool_registry` 通过
- [ ] `cargo test -p daedalusd --test http_session` 仍通过
- [ ] `cargo test -p daedalusd --test http_tasks` 仍通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] 测试证明未设置 `DAEDALUS_NARRATIVE_ROOT` 时使用 `ToolContext.work_dir`
- [ ] 测试证明 `DAEDALUS_NARRATIVE_ROOT` 为空或全空白时回退 `ToolContext.work_dir`
- [ ] 测试证明 `DAEDALUS_NARRATIVE_ROOT` 非空时优先读取 env root 下的 `knowledge.yaml`
- [ ] 测试证明 env root 指向不存在目录时返回 `knowledge_file_missing`
- [ ] 本轮未修改数据库迁移或表结构
- [ ] 本轮未修改 Session API / HTTP 路由
- [ ] 未修改 `/Users/gu/daedalus-courtroom-demo`
- [ ] developer 完成后追加结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## 当前分工

- 总控：只协调、分派、汇总；不写代码。
- developer：做最小实现和最小有效验证。
- reviewer：基于本任务验收标准只读验收，输出 PASS/FAIL。
