# 测试报告

当前任务：Narrative Backend Phase 1h - narrative root 环境变量契约

验收时间：2026-07-08 16:43 +0800

结论：PASS

验收范围复核：
- `git status --short --branch` 显示本轮工作树改动仅涉及 `daedalusd/src/tools/narrative.rs`、`daedalusd/tests/narrative_tools.rs`、`work/callbacks.md`。
- `git diff --stat` / `git diff` 复核结果与任务卡一致：业务代码只改 narrative root resolver 和对应测试。
- 未见数据库迁移/表结构、Session API/HTTP 路由、Agent Loop、SSE、Godot、LLM、Gate 文件改动。
- `work/callbacks.md` 已存在 developer 本轮回传：`[2026-07-08 16:40 +0800] [developer] ...`

测试与验证：
- `cargo fmt --all -- --check`：通过
- `cargo test -p daedalusd --test narrative_tools`：通过，`7 passed`
- `cargo test -p daedalusd --test tool_registry`：通过，`5 passed`
- `cargo test -p daedalusd --test http_session`：通过，`4 passed`
- `cargo test -p daedalusd --test http_tasks`：通过，`15 passed`
- `cargo clippy -p daedalusd --all-targets -- -D warnings`：通过
- `git diff --check`：通过

验收结论依据：
- `check_knowledge` 已在 `DAEDALUS_NARRATIVE_ROOT` 非空时优先使用该 root；未设置、空字符串、全空白时回退 `ToolContext.work_dir`。
- 知识文件路径仍是 `{resolved_root}/narrative/characters/{npc_id}/knowledge.yaml`。
- `daedalusd/tests/narrative_tools.rs` 已覆盖：
  - 未设置 env 时使用 `ToolContext.work_dir`；
  - env 为空字符串或全空白时回退 `ToolContext.work_dir`；
  - env 非空时优先读取 env root 下的 `knowledge.yaml`；
  - env root 指向不存在目录时返回 `allowed = false` 且 `reason = "knowledge_file_missing"`；
  - 测试内通过全局 mutex 串行化 env 修改，并在 `Drop` 中恢复原值。

风险与说明：
- `/Users/gu/daedalus-courtroom-demo` 当前位于 `main` 且存在既有脏工作树；本次验收未修改该仓库，也未见本轮 diff 落入该仓库，但无法仅凭当前状态证明其中既有改动与本轮绝对无关。
- 当前实现仍是第一版文件直读方案，每次调用同步读取单个 YAML；这与本轮任务卡范围一致，不构成阻断。
- 当前线程无法主动跨会话回传，总控结论已按要求写入 `work/callbacks.md` 兜底。
