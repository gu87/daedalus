# P5+.2 实现完成报告：Desktop UDS bridge — task.dispatch 最小闭环

> commit: 待提交

---

## 1. 修改文件清单

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalus-desktop/electron/preload.js` | 修改 | UDS NDJSON 客户端（`udsConnect` + `buildTaskDispatch`）；`ping()` / `dispatchTask()` 改为真实 UDS；`DAEDALUS_DESKTOP_AGENT_ID` 环境变量 |
| `daedalus-desktop/src/services/daedalusApi.ts` | 修改 | 类型更新：`TaskCallbacks`、`dispatchTask` 返回 `Promise<{taskId, reqId}>` |
| `daedalus-desktop/src/App.tsx` | 修改 | `createGeneratedTab("task")` 调真实 `dispatchTask` + `onStream/onDone/onError/onPermissionRequest` 回调 |
| `daedalus-desktop/src/components/AppShell.tsx` | 未改 | — |

**不改**：daedalusd、IPC 协议、HTTP API、UI 布局

---

## 2. API 变更

| 方法 | P5+.1 | P5+.2 |
|------|:---:|:---:|
| `getHealth()` | ✅ HTTP real | ✅ |
| `listSessions()` | ✅ HTTP real | ✅ |
| `ping()` | — | **✅ UDS** |
| `dispatchTask(goal, callbacks)` | — | **✅ UDS** → `{taskId, reqId}` |
| `approveGate/rejectGate/requestChanges` | mock | mock（P5+.3） |

---

## 3. 验证

```
npm run build  ✅ (219KB JS, 456ms)
```

- Agent ID from `DAEDALUS_DESKTOP_AGENT_ID` env var（默认 `daedalus-desktop`）
- TaskCard v2.8：`req_id`/`task_id` 由 preload 生成，`agent_id` 与 `execution_plan.primary_agent` 一致
- UDS NDJSON 客户端：按 `\n` 分帧，`JSON.parse` 错误 → `onError("protocol_error")`，socket error → `onError("connection_error")`，EOF 未终态 → `onError("connection_lost")`
- `permission.request` 只显示（`onPermissionRequest` 回调），**不回复**，提示 "P5+.3 接入"

---

## 4. 不做清单

| 约束 | 状态 |
|------|:---:|
| session.rejoin / ack / 重连 | ✅ |
| permission.response 发送 | ✅ — P5+.3 |
| 高级 TaskCard 编辑 | ✅ |
| 修改 daemon | ✅ |
| 修改 IPC 协议 | ✅ |
| 修改 UI 布局 | ✅ |
