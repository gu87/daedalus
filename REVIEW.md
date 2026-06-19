# P5+.3 实现方案：Gate / Permission 审批可用化

> 基于 P5+.2（commit `9d1b854`）。目标：Desktop UDS dispatch 链路中，
> `permission.request` 到达时触发 UI 审批条，用户 approve/reject/changes 后
> 通过同一 UDS 连接发送 `permission.response`，AgentLoop 继续执行。

---

## 1. 核心设计

### 1.1 连接生命周期

```
──── dispatchTask(goal) ────→
  task.stream / task.stream / ...
  permission.request ← daemon 需要审批
  （UDS 连接保持打开，等待用户决策）
  permission.response → user clicked approve
  task.stream / task.stream / ...
  task.done ← daemon 完成
  UDS 连接关闭
```

- **同一 UDS 连接**：`task.dispatch` 和 `permission.request`/`permission.response` 共享
- **permission.request 到达时连接不关闭**
- **用户决策后在同一连接上发送 permission.response**

### 1.2 回调设计

```typescript
// onPermissionRequest 签名变更：
onPermissionRequest(perm: PermissionRequest, reply: PermissionReply): void

interface PermissionReply {
  approve(): void;
  reject(): void;
  requestChanges(): void;
}
```

- `onPermissionRequest` 被调用时，接收 `perm` 数据 + `reply` 对象
- UI 显示审批条（复用现有 `ApprovalBar` 组件）
- 用户点击 → 调用 `reply.approve()` / `reply.reject()` / `reply.requestChanges()`
- `reply` 内部通过保存的 UDS socket 引用发送 `permission.response`

---

## 2. 文件边界

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalus-desktop/electron/preload.js` | 修改 | `dispatchTask` 中 `onPermissionRequest` 回调改为 `(msg, reply)` 双参数；`reply` 对象持有 conn 引用发送 `permission.response`；发送后不关闭连接 |
| `daedalus-desktop/src/services/daedalusApi.ts` | 修改 | `TaskCallbacks.onPermissionRequest` 签名更新 |
| `daedalus-desktop/src/App.tsx` | 修改 | `decideApproval` 调 `reply.approve/reject/changes`；`onPermissionRequest` 保存 `reply` 引用 |
| `daedalus-desktop/src/components/ApprovalBar.tsx` | 可能不改 | 已有 approve/reject/request changes 三个按钮 |

**不改**：daedalusd、IPC 协议、Gate/Agent Loop、SQLite schema、UI 布局

---

## 3. preload.js 实现要点

### 3.1 dispatchTask 改造

```javascript
dispatchTask: (goal, callbacks) => {
  return new Promise((resolve) => {
    const td = buildTaskDispatch(goal);
    let pendingReply = null;  // { approve, reject, requestChanges }

    const conn = udsConnect(DAEMON_SOCK);

    conn.on("permission.request", (msg) => {
      conn.settle();  // 阻止 close/error 回调在审批期间误报
      const reply = {
        approve() {
          conn.send({
            type: "permission.response",
            ts: nowISO(),
            permission_id: msg.permission_id,
            req_id: msg.req_id,
            decision: "approved",
          });
        },
        reject() {
          conn.send({
            type: "permission.response",
            ts: nowISO(),
            permission_id: msg.permission_id,
            req_id: msg.req_id,
            decision: "denied",
          });
        },
        requestChanges() {
          conn.send({
            type: "permission.response",
            ts: nowISO(),
            permission_id: msg.permission_id,
            req_id: msg.req_id,
            decision: "denied",  // daemon 不需要专门的 changes 变体
          });
        },
      };
      pendingReply = reply;
      if (callbacks.onPermissionRequest) callbacks.onPermissionRequest(msg, reply);
    });

    // task.done / task.error 先 settle + close（不变）
    // ...
  });
},
```

### 3.2 字段映射

| permission.response 字段 | 来源 |
|-------------------------|------|
| `permission_id` | `msg.permission_id`（daemon 生成） |
| `req_id` | `msg.req_id`（daemon dispatch req_id） |
| `decision` | `"approved"` / `"denied"` |

- `requestChanges` 发送 `decision: "denied"`（daemon 当前 decision 只有 approved/denied）
- `ts` 由 preload 生成

---

## 4. App.tsx 接入

### 4.1 保存 reply 引用

```typescript
// 新增 state：当前活跃的 permission reply
const [pendingPermissionReply, setPendingPermissionReply] = useState<PermissionReply | null>(null);
```

### 4.2 onPermissionRequest 回调

```typescript
onPermissionRequest(perm, reply) {
  setPendingPermissionReply(reply);
  setTabs((items) => items.map((t) =>
    t.id !== tabId ? t : {
      ...t,
      approval: {
        id: perm.permission_id,
        title: `Gate 审批: ${perm.tool}`,
        desc: JSON.stringify(perm.args),
      },
    }
  ));
},
```

### 4.3 decideApproval 发送 response

```typescript
function decideApproval(tabId: string, approval: ApprovalRequest, decision: "approve" | "reject" | "changes") {
  // P5+.3: send permission.response via saved reply.
  if (pendingPermissionReply) {
    if (decision === "approve") pendingPermissionReply.approve();
    else if (decision === "reject") pendingPermissionReply.reject();
    else pendingPermissionReply.requestChanges();
    setPendingPermissionReply(null);
  }
  // 更新 tab 状态（清除 approval，标记决策）
  setTabs((items) => items.map((t) =>
    t.id !== tabId ? t : {
      ...t,
      approval: null,
      status: decision === "approve" ? "运行中" : decision === "reject" ? "已拒绝" : "要求修改",
      statusType: decision === "approve" ? "running" : decision === "reject" ? "rejected" : "waiting",
    }
  ));
}
```

## 5. 失败路径

| 场景 | 行为 |
|------|------|
| `permission_id` 缺失 | preload 不调用 `onPermissionRequest`，记录 console.warn |
| 用户点击时连接已断开 | `conn.send()` 静默失败（socket 已 destroy），UI 回退到错误态 |
| 重复点击 | `reply` 可多次调用但 `conn.send()` 第二次在 closed socket 上静默失败 |
| daemon 返回 system.error | 连接 close，与 task.error 处理相同 |
| 审批超时（daemon 侧 30s） | daemon 默认 deny，连接继续（task.error 或继续执行） |

---

## 6. 验证

| # | 场景 | 预期 |
|:--|------|------|
| 1 | daemon 推送 permission.request → UI 显示审批条 | ✅ |
| 2 | 用户点击 approve → `permission.response { decision: "approved" }` 发送 | ✅ |
| 3 | 用户点击 reject → `permission.response { decision: "denied" }` 发送 | ✅ |
| 4 | 审批后 task 继续执行 → 最终 task.done/task.error | ✅ |
| 5 | npm run build | ✅ |

---

## 7. 不做

| 约束 | 状态 |
|------|:---:|
| 修改 daedalusd IPC 协议 | ✅ |
| 修改 Gate/Agent Loop/SQLite | ✅ |
| session.rejoin / ack / reconnect | ✅ |
| 重设计 UI / ApprovalBar 组件 | ✅ |
