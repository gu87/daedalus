# Phase 5+ Dogfood 可用化拆解方案（Codex 修订版）

> Phase 1–5 已封板。Phase 5+ 目标：让我能日常使用 Daedalus。

---

## 0. 核心取舍（Codex 拍板）

**方案 A（采用）**：Desktop 使用 HTTP 读状态/health，使用 Electron preload 的 Node `net` 模块实现 UDS NDJSON 客户端来发送 `task.dispatch` 和处理 `permission.request`/`permission.response`。

- **不新增 daemon HTTP 写端点**
- 复用现有 UDS 协议和 daemon 行为
- Permission approve/reject 通过同一 UDS session 发送 `permission.response`

---

## 1. 子任务顺序

```
P5+.1 Desktop ↔ daemon 连接 + 在线状态（HTTP health）
  │
  └─→ P5+.2 Desktop UDS bridge（NDJSON 客户端 + task.dispatch）
        │
        └─→ P5+.3 Gate/Permission 审批可用化（permission.response over UDS）
              │
              └─→ P5+.4 Task state restore（HTTP tasks API 恢复状态）
                    │
                    └─→ P5+.5 Dogfood smoke + README
```

---

## 2. P5+.1 Desktop ↔ daemon 连接 + 在线状态

### 目标

Electron preload 中 `getHealth()` 改为真实 HTTP 请求 `GET /api/health`。
App.tsx 使用 `window.daedalusAPI.getHealth()` 显示 daemon online/offline。

### 文件清单

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalus-desktop/electron/preload.js` | 修改 | `getHealth()` → `fetch("http://127.0.0.1:9800/api/health")` |
| `daedalus-desktop/src/App.tsx` | 修改 | 新增 `daemonOnline` state，定时调 `getHealth()` |
| `daedalus-desktop/src/components/AppShell.tsx` | 修改 | 顶部状态栏显示 🟢/🔴 |

### 验证

- daemon 启动 → 桌面显示 🟢
- daemon 停止 → 桌面显示 🔴（5s 内）
- 不依赖 models.yaml 或 task dispatch

### 不做

- 不修改 daemon 代码
- 不新增 HTTP 端点
- 不做认证

---

## 3. P5+.2 Desktop UDS bridge

### 目标

在 Electron preload 中实现最小 UDS NDJSON 客户端，支持：
- `connect(socketPath)` — 连接 daemon
- `dispatchTask(taskCard)` → 发送 `task.dispatch`，循环读响应直到 `task.done`/`task.error`
- 接收 `task.stream` → 回调 onStream
- 接收 `permission.request` → 回调 onPermissionRequest（此时不回复，P5+.3 实现）

### 为什么不用 HTTP

`spawn_task` 需要 `writer_tx + SessionState`。HTTP 无 UDS session，无法接收 daemon 推送的 `task.stream`/`task.done`/`task.error`/`permission.request`。新增 HTTP session 管理层工作量远大于实现 UDS 客户端。

### 文件清单

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalus-desktop/electron/preload.js` | 修改 | 新增 `connect(socketPath)`、`dispatchTask(taskCard, onStream, onPermission)`；暴露到 `window.daedalusAPI` |
| `daedalus-desktop/src/services/daedalusApi.ts` | 修改 | TypeScript 类型更新，从 mock 改为调 `window.daedalusAPI` |
| `daedalus-desktop/src/App.tsx` | 修改 | `createGeneratedTab` 中调 `daedalusApi.dispatchTask()` |

### UDS NDJSON 客户端实现要点

```
// preload.js 新增
const net = require("net");

function connect(socketPath) {
  const socket = net.createConnection(socketPath);
  let buf = "";
  socket.on("data", (chunk) => {
    buf += chunk.toString();
    while (buf.includes("\n")) {
      const line = buf.substring(0, buf.indexOf("\n"));
      buf = buf.substring(buf.indexOf("\n") + 1);
      const msg = JSON.parse(line);
      handleMessage(msg);  // dispatch to onStream/onDone/onError/onPermission
    }
  });
  return { socket, send: (msg) => socket.write(JSON.stringify(msg) + "\n") };
}
```

### 验证

- daemon 运行 → preload 连接 `/tmp/daedalusd.sock` → 连接成功
- 发送 `system.ping` → 收到 `system.pong`
- 发送 `task.dispatch` → 收到 `task.done`/`task.error`

### 不做

- 不新增 daemon HTTP 写端点
- 不实现 `session.rejoin`（P5+.4 只做 HTTP 状态恢复）
- 不修改 daemon IPC 协议

---

## 4. P5+.3 Gate/Permission 审批可用化

### 目标

在 UDS bridge（P5+.2）基础上，`permission.request` 到达时触发 UI 审批条。
用户点击 approve/reject/changes → 通过同一 UDS session 发送 `permission.response`。

### 文件清单

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalus-desktop/electron/preload.js` | 修改 | dispatchTask 中 `onPermission` 回调挂起，等待用户决策后 `send(permission.response)` |
| `daedalus-desktop/src/App.tsx` | 修改 | `decideApproval` 调 `window.daedalusAPI.sendPermissionResponse()` |
| `daedalus-desktop/src/components/ApprovalBar.tsx` | 可能修改 | 审批条从 mock 数据改为真实 permission.request |

### 不做

- 不修改 Gate 路由规则
- 不修改 AgentLoop 状态机
- Permission 超时逻辑不变（daemon 侧 30s default deny）

---

## 5. P5+.4 Task state restore

### 目标

UI 启动/重连后通过 HTTP `GET /api/tasks` 恢复当前任务列表状态。
**不是 Durable event replay**（不用 session.rejoin，不走 event-by-event 回放）。

### 文件清单

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalus-desktop/electron/preload.js` | 修改 | `listSessions()` → `fetch(GET /api/tasks)` |
| `daedalus-desktop/src/App.tsx` | 修改 | 初始化时调 `listSessions()` 恢复 Tab |

### 不做

- 不实现 session.rejoin
- 不做 event-by-event 回放
- 不做 WebSocket

---

## 6. P5+.5 Dogfood smoke + README

### 目标

一键启动 daemon + desktop，完成一次任务。
README 更新真实使用步骤。

### 不做

- 不新增自动化测试
- 不修改 daemon 配置逻辑

---

## 7. 汇总

| 约束 | 状态 |
|------|:---:|
| 不新增 daemon HTTP 写端点 | ✅ — 用 UDS |
| 复用 NDJSON over UDS 协议 | ✅ — preload net 模块实现客户端 |
| 不重设计 UI | ✅ |
| 不做 Ω-Agent / MCP / DAG | ✅ |
| 不破坏 Phase 1–5 | ✅ |
| P5+.1 先实施 | ✅ |
