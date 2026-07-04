# 多 Agent 协作规则

本项目当前使用 Codex thread 进行 standard（总控直管协作）：用户 → 总控 → developer → reviewer。

## 文件

| 文件 | 用途 |
|------|------|
| `work/registry.md` | Agent thread 注册表 |
| `work/task.md` | 当前任务 / PRD / 验收标准 |
| `work/log.md` | 工作日志 |
| `work/test-report.md` | 测试报告 |
| `work/callbacks.md` | Agent 主动回传兜底信箱 |
| `.codex/agents/*.md` | 当前模式所需角色提示词（developer / reviewer） |

## 通信

standard 模式下，当前主会话 `总控` 负责协调；project 内不再单设 manager thread。

Agent 间用 `codex_app__send_message_to_thread({ threadId, prompt })` 发消息，用 `codex_app__read_thread({ threadId })` 读取回复。
协作必须发生在侧边栏可见、可回读、可复用的 Codex thread 中。

回传规则：developer / reviewer 完成后必须优先用跨会话工具主动回传总控；同时把同一结论追加到 `work/callbacks.md`。如果某个线程没有跨会话发送工具，必须明确写“无法主动跨会话回传”，但 `work/callbacks.md` 仍然必须写入。总控以 `work/callbacks.md` 作为兜底信号，不再只靠轮询线程。
