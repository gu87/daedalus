# 当前任务：Narrative Backend Phase 2 Completion Audit

> standard 模式：总控分派；developer 做完成度自检；reviewer 只读验收。
> 本轮不写业务代码，只确认 Phase 2 是否可标记完成。

## 背景

Phase 2 已按小步完成并推送：

```text
0b4fd9b feat: validate narrative session replies
61ba51b feat: inject narrative task prompt context
54902e8 feat: constrain narrative stage transitions
568b485 feat: gate narrative stage transitions by evidence
b237b38 feat: allow candidate evidence for narrative stage gates
```

已具备能力：

- `/message` 使用 Agent Loop 的 `TaskDone.outbox.summary`
- NPC reply summary 必须是严格 JSON，非法输出 fallback
- fallback 不暴露非法原文
- prompt 注入输出契约、当前 session 状态、玩家输入和 knowledge 边界
- hidden knowledge content 不进入本轮 TaskCard prompt
- `stage_delta` 只允许 `denial -> vague -> partial -> breakdown` 单步前进
- 可用 `stage_requirements` 限制阶段推进所需证据
- 支持旧字段 `required_evidence_id`
- 支持候选数组 `required_evidence_ids`
- 缺证据或错误证据使用稳定错误码 `missing_required_evidence`

## 本轮目标

只做 Phase 2 完成度自检和记录：

- 确认 Phase 2a-2e 的能力仍在
- 确认当前分支业务代码可测试通过
- 确认没有本轮业务改动落入 `/Users/gu/daedalus-courtroom-demo`
- 追加 Phase 2 completion 结论到 `work/test-report.md`
- 追加同一结论到 `work/callbacks.md`

## 非目标

本轮不做：

- 不新增业务功能
- 不修改 `daedalusd/src/http/session.rs`
- 不修改 `daedalusd/tests/http_session.rs`
- 不接 Gate AutoRevision
- 不接 SSE
- 不接 Godot
- 不接真实 LLM
- 不改 Agent Loop
- 不改 Tool trait
- 不改数据库表结构
- 不做多 NPC
- 不做复杂条件表达式
- 不解决 timeout 测试真实等待问题

## 允许修改

只允许：

- `work/test-report.md`
- `work/callbacks.md`

不要修改：

- `/Users/gu/daedalus-courtroom-demo`
- `daedalusd/src/**`
- `daedalusd/tests/**`
- DB migrations
- HTTP 路由文件
- Agent Loop
- Tool trait

## 验收标准

- [ ] 当前分支包含 `b237b38 feat: allow candidate evidence for narrative stage gates`
- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo test -p daedalusd --test http_session` 通过
- [ ] `cargo test -p daedalusd --test http_tasks` 通过
- [ ] `cargo test -p daedalusd --test narrative_tools` 通过
- [ ] `cargo clippy -p daedalusd --all-targets -- -D warnings` 通过
- [ ] `git diff --check` 通过
- [ ] 只修改 `work/test-report.md` 与 `work/callbacks.md`
- [ ] developer 追加 Phase 2 completion 结论到 `work/callbacks.md`
- [ ] reviewer 只读复核后追加 PASS/FAIL 到 `work/callbacks.md`

## Phase 2 完成定义

Phase 2 可以标记完成，当且仅当 reviewer PASS：

```text
Daedalus Narrative Backend Phase 2 已完成：
结构化 NPC 输出、prompt/knowledge 边界、单步阶段机、证据门槛、候选证据门槛均已实现并通过回归。
```

## Phase 3 候选范围

以下仍是 Phase 3 或后续事项，不阻断 Phase 2 completion：

- SSE / Godot 接入
- 真实 LLM provider smoke
- Gate AutoRevision
- 多 NPC 状态机
- 多证据同时满足
- 复杂条件表达式
- timeout 测试加速
