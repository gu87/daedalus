# 当前任务：DevSpace 增加 Daedalus 专用 MCP 工具

> standard 模式：总控分派；developer 实现；reviewer 验收。  
> 目标是让 ChatGPT 通过 DevSpace 调用本机 Daedalus runtime，而不是让 ChatGPT 直接用 `bash curl`。

## 背景

DevSpace 已经打通 ChatGPT MCP：

- MCP URL：`https://devspace.sevengu.top/mcp`
- DevSpace 版本：`1.0.3`
- 当前已暴露工具：`bash`、`edit`、`open_workspace`、`read`、`write`
- ChatGPT 已能打开 Obsidian vault 并做只读 Markdown 搜索
- DevSpace allowedRoots 已包含 `/Users/gu/Daedalus`

Daedalus 当前已有 runtime HTTP API：

- `POST http://127.0.0.1:9800/api/tasks`
- `GET http://127.0.0.1:9800/api/tasks/:run_id/wait`
- 返回 `outbox_json`

## 目标

在 DevSpace 中新增一个窄 MCP tool：

```text
daedalus_run
```

ChatGPT 调用该工具时，DevSpace 内部执行：

1. `POST http://127.0.0.1:9800/api/tasks`
2. 获取 `run_id`
3. `GET http://127.0.0.1:9800/api/tasks/{run_id}/wait`
4. 返回 `run_id`、`status`、`outbox_json`

## 边界

只新增独立工具，不改变现有 DevSpace 文件工具。

不要动：

- `bash`
- `edit`
- `open_workspace`
- `read`
- `write`
- DevSpace allowedRoots
- OAuth / auth.json
- Cloudflare tunnel 配置
- Daedalus 代码

不要做：

- 不把 Daedalus 合进 DevSpace 主启动流程
- 不让 DevSpace 启动强依赖 Daedalus
- 不暴露任意 URL 转发工具
- 不扩大本机 shell 权限

## 工具接口建议

输入：

```json
{
  "agent_id": "default-worker",
  "goal": "echo hello",
  "context": "可选上下文",
  "timeout_seconds": 300
}
```

行为：

- `agent_id` 默认 `default-worker`
- `timeout_seconds` 默认 300
- Daedalus 不在线时，返回清晰错误：`Daedalus daemon unavailable`
- 不让 DevSpace 进程崩溃
- 返回尽量短：

```json
{
  "run_id": "run-...",
  "status": "done",
  "outbox_json": "..."
}
```

## 验收标准

- [ ] DevSpace 原有工具列表仍包含 `bash/edit/open_workspace/read/write`
- [ ] 新增 `daedalus_run`
- [ ] Daedalus 不在线时，调用 `daedalus_run` 返回清晰错误，不影响其他工具
- [ ] Daedalus 在线时，`daedalus_run` 能提交 task 并等待结果
- [ ] 不修改 DevSpace allowedRoots / OAuth / Cloudflare 配置
- [ ] 不修改 Daedalus 代码
- [ ] 有最小验证记录

## 当前分工

- 总控：控范围、分派、读取开发/验收回报。
- developer：定位 DevSpace 源码/安装目录，做最小实现，验证工具注册和错误路径。
- reviewer：只读验收，确认旧功能不受影响、新工具可见、错误路径安全。
