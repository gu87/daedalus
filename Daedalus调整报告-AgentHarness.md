# Daedalus 调整报告：从 Agent OS 收缩为个人后台 Agent Harness

## 0. 背景

Daedalus 目前已经完成了比较完整的 Agent runtime 工程实现，包括：

- Rust daemon `daedalusd`
- Python CLI `daedalus-orch`
- Electron / React Desktop
- UDS NDJSON 通信
- HTTP API
- Agent Loop
- LLM Router
- Tool Registry
- Permission 审批
- Gate 错误处理
- Pipeline 状态机
- Durable Execution / Ledger
- SQLite 持久化

这些能力说明 Daedalus 作为学习项目和工程验证项目已经成功。

但当前项目陷入瓶颈：继续扩展 Agent OS、大规模重构 Desktop、增加多 Agent、Ω-Agent、Browser Workspace 等，都不能直接证明 Daedalus 的日用价值。

因此下一阶段不应继续“做大系统”，而应收缩目标：

> Daedalus 不再优先定位为通用 Agent OS，而是先定位为“个人后台 Agent Harness”。

也就是：一个能让 Agent 在后台跑真实任务、留下记录、可恢复、可审计、可扩展的小底座。

---

## 1. 新定位

### 旧定位

Daedalus 是一个完整的 Agent OS：

- 多 Agent 调度
- 桌面工作台
- Browser Workspace
- 任务系统
- 记忆系统
- 插件系统
- 自动化系统
- 长期演化的智能体操作系统

这个方向太大，当前阶段容易继续陷入过度设计。

### 新定位

Daedalus 是个人后台 Agent Harness：

> 用户给一个任务，Daedalus 能启动 Agent 执行，记录完整过程，保存结果，必要时请求审批，并允许插件对任务结果做后续处理。

核心不是“做一个完整 OS”，而是跑通真实 loop。

---

## 2. 当前阶段的产品原则

### 原则一：暂停扩内核

当前不要新增这些大模块：

- Ω-Agent
- 多 Agent 会议室
- 完整 Browser Workspace
- 复杂 Desktop 重构
- 内置 Obsidian 记忆系统
- 内置 MCP 平台
- 完整插件市场
- 自动化大看板

原因：这些都会继续扩大项目，但不能立即验证真实价值。

---

### 原则二：先跑真实任务

下一阶段只验证一个真实闭环：

> Daedalus 执行一个任务 → 生成 transcript → 生成 summary → 写入 Obsidian inbox → 等用户 review。

这个闭环如果跑通，Daedalus 才从“工程实验平台”变成“每天能用的后台工具”。

---

### 原则三：功能尽量插件化

不要把 Obsidian、Feishu、GitHub、Browser、Cron 等都写进 core。

core 只提供：

- 任务生命周期事件
- run transcript
- tool call / tool result 记录
- task done / task error hook
- 插件注册点

具体能力通过插件做。

---

## 3. 参考 Pi Agent 的方向

Pi Agent 的关键启发不是功能，而是取舍：

- core 很小
- 默认只做好 Agent Loop
- 工具、权限、sub-agent、MCP、plan mode 等都不强行内置
- 扩展系统非常重要
- session 记录用 JSONL，容易导出、解析、回放、复用
- CLI / TUI / RPC / SDK 比 Desktop 更优先

Daedalus 应吸收这个思路：

> Daedalus 不要继续变成重型 Agent OS，而要先成为轻量、可扩展、可记录、可回放的个人 Agent Harness。

---

## 4. 本次调整目标

请实现一个小版本，不要大改架构。

### 目标名称

`Phase X: Run Transcript + Extension Hook MVP`

### 目标一句话

> 每个 Daedalus task 都自动生成可读、可解析、可回放的运行记录，并允许一个本地插件在 task 完成后处理结果。

---

## 5. 本次只做 3 件事

## 5.1 每个任务生成 transcript.jsonl

为每个 task / run 自动创建目录：

```text
~/.daedalus/runs/<run_id>/
├── transcript.jsonl
├── summary.md
└── artifacts/
```

如果项目当前已有类似 runtime 目录，可以沿用现有目录，不要强行新建重复结构。

### transcript.jsonl 记录内容

每行一个 JSON object，建议包含：

```json
{
  "type": "task.started",
  "run_id": "...",
  "task_id": "...",
  "agent_id": "...",
  "goal": "...",
  "timestamp": "..."
}
```

```json
{
  "type": "llm.message",
  "role": "assistant",
  "content": "...",
  "model": "...",
  "timestamp": "..."
}
```

```json
{
  "type": "tool.call",
  "tool_name": "file_read",
  "args": {},
  "timestamp": "..."
}
```

```json
{
  "type": "tool.result",
  "tool_name": "file_read",
  "ok": true,
  "summary": "...",
  "timestamp": "..."
}
```

```json
{
  "type": "task.done",
  "run_id": "...",
  "outbox": {},
  "timestamp": "..."
}
```

```json
{
  "type": "task.error",
  "run_id": "...",
  "error_code": "...",
  "message": "...",
  "timestamp": "..."
}
```

要求：

- append-only
- 不覆盖历史
- 写入失败不能导致任务主流程崩溃
- transcript 应尽量不记录敏感环境变量
- 如果 tool result 很长，可以截断，并把完整内容放到 artifacts

---

## 5.2 每个任务生成 summary.md

任务结束时生成一个简单 Markdown 摘要：

```markdown
# Run Summary

- Run ID:
- Task ID:
- Agent:
- Goal:
- Status:
- Started:
- Finished:
- Model:
- Tools Used:

## Result

...

## Errors / Gate Decisions

...

## Artifacts

...
```

第一版可以不调用 LLM 生成高质量摘要。

可以先用规则生成：

- goal
- final outbox
- task status
- tool calls 列表
- error / gate 信息
- artifact 路径

后续再考虑 LLM summary。

---

## 5.3 增加最小 Extension Hook

先不要做完整插件系统，只做最小本地 hook。

建议支持一个配置：

```yaml
hooks:
  task_done:
    - ~/.daedalus/hooks/task_done.sh
  task_error:
    - ~/.daedalus/hooks/task_error.sh
```

或者如果项目已有配置系统，则放入现有 config。

hook 执行时传入环境变量：

```bash
DAEDALUS_RUN_ID
DAEDALUS_TASK_ID
DAEDALUS_RUN_DIR
DAEDALUS_TRANSCRIPT_PATH
DAEDALUS_SUMMARY_PATH
DAEDALUS_STATUS
```

第一版 hook 只需要支持：

- task_done
- task_error

要求：

- hook 超时，比如 30 秒
- hook 失败不影响主任务状态
- hook stdout / stderr 写入 run 目录下的 hook log
- 默认不启用 hook
- 不要引入复杂插件生命周期

---

## 6. 示例插件：Obsidian Inbox Hook

实现一个示例脚本即可，不要内置进 daemon。

路径建议：

```text
examples/hooks/obsidian-inbox-task-done.sh
```

功能：

```text
读取 DAEDALUS_SUMMARY_PATH
追加到 Obsidian inbox 文件
```

示例目标文件：

```text
~/个人知识库/0-inbox/daedalus-runs.md
```

或者通过环境变量配置：

```bash
OBSIDIAN_DAEDALUS_INBOX="$HOME/个人知识库/0-inbox/daedalus-runs.md"
```

写入格式：

```markdown
## <date> <run_id>

Source: <summary path>

<summary content>
```

要求：

- 只 append，不删除，不改写旧内容
- 如果目标文件不存在则创建
- 如果没有配置 `OBSIDIAN_DAEDALUS_INBOX`，脚本应直接退出并提示
- 不要自动提交 git
- 不要自动写入 canon 正典区
- 不要自动改已有知识库内容

---

## 7. 不要做的事

本轮明确不要做：

- 不要重构 Desktop UI
- 不要做 Browser Workspace
- 不要做完整插件市场
- 不要做多 Agent 会议室
- 不要做 Ω-Agent
- 不要把 Obsidian 变成内置模块
- 不要做复杂 Memory Layer
- 不要接 Feishu / Discord
- 不要引入 MCP
- 不要改 LLM Router 大结构
- 不要大改 Gate
- 不要重写 Pipeline
- 不要迁移 SQLite schema，除非绝对必要

本轮只做运行记录和 hook。

---

## 8. 建议检查的代码位置

先阅读项目文档和关键文件，再动手。

重点看：

```text
daedalusd/src/daemon.rs
daedalusd/src/agent/loop.rs
daedalusd/src/db/ledger.rs
daedalusd/src/pipeline/status.rs
daedalusd/src/pipeline/db.rs
daedalusd/src/types.rs
daedalusd/src/config.rs
daedalusd/src/ipc/control.rs
daedalus-orch/daedalus/orch/cli.py
```

历史 Desktop 前端已移除，本节不再保留前端文件入口：

```text
removed frontend API adapter
removed frontend app shell
```

---

## 9. 技术设计建议

### 9.1 新增 RunArtifactWriter / TranscriptWriter

可以在 Rust daemon 里新增一个轻量模块，例如：

```text
daedalusd/src/run_artifacts/
├── mod.rs
├── transcript.rs
├── summary.rs
└── hooks.rs
```

或者如果现有模块结构更适合，也可以放到 `db/ledger` 附近。

职责：

- 确保 run 目录存在
- append JSONL event
- 写 summary.md
- 调用 hooks

注意：不要让 artifact 写入污染核心 AgentLoop。

---

### 9.2 事件来源

优先复用已有事件：

- task.dispatch
- AgentLoop lifecycle
- tool call
- tool result
- task.done
- task.error
- Gate decision
- Pipeline status transition

如果当前某些事件不容易拿到，第一版可以先记录最容易拿到的：

- task.started
- task.done
- task.error
- final outbox
- pipeline status
- run metadata

不要为了 transcript 完整性大改 AgentLoop。

---

### 9.3 错误处理

artifact / hook 系统必须是旁路能力。

原则：

```text
Agent 主任务 > transcript > hook
```

即：

- 主任务不能因为 transcript 写失败而失败
- 主任务不能因为 hook 失败而失败
- transcript 写失败应该记录 warning
- hook 失败应该写入 hook log

---

## 10. 验收标准

### 10.1 CLI 验收

启动 daemon 后运行：

```bash
daedalus run "用一句话介绍 Daedalus"
```

完成后应该能看到：

```text
~/.daedalus/runs/<run_id>/transcript.jsonl
~/.daedalus/runs/<run_id>/summary.md
```

`transcript.jsonl` 至少包含：

- task.started
- task.done 或 task.error

`summary.md` 至少包含：

- run_id
- goal
- status
- result

---

### 10.2 Hook 验收

配置 task_done hook 后，再运行一个任务。

预期：

- hook 被调用
- hook log 写入 run 目录
- 如果配置了 `OBSIDIAN_DAEDALUS_INBOX`，summary 被 append 到对应 Markdown 文件
- hook 失败不影响 daedalus run 的最终状态

---

### 10.3 测试要求

至少新增或更新测试覆盖：

- transcript writer append JSONL
- summary writer 生成 Markdown
- hook disabled 时不执行
- hook enabled 时接收正确环境变量
- hook 超时 / 失败不影响主流程

如果项目已有测试规范，遵守现有规范。

---

## 11. 最终交付物

请交付：

1. 代码实现
2. 新增测试
3. README 或 docs 更新
4. 一个 example Obsidian hook
5. 简短说明：
   - 如何启用 run transcript
   - 如何启用 hook
   - 如何配置 Obsidian inbox
   - 当前限制是什么

---

## 12. 重要取舍

这次调整的重点不是让 Daedalus 更强，而是让 Daedalus 更容易产生真实价值。

不要继续追求“完整 Agent OS”。

先做到：

```text
任务可记录
结果可回放
产物可进入 Obsidian
hook 可扩展
```

只要这个闭环跑通，Daedalus 就从“工程项目”变成“个人后台工具”的起点。

---

## 13. 总结

本轮目标：

> 把 Daedalus 从重型 Agent OS 收缩为可扩展的个人后台 Agent Harness。

核心实现：

```text
Run Transcript JSONL
+ Summary Markdown
+ task_done / task_error Hook
+ Obsidian Inbox 示例
```

不要做大改，不要继续扩架构，不要重构 Desktop。

先让 Daedalus 每跑一次任务，都能留下可读、可解析、可复用的记录，并能把结果流入用户的知识库。
