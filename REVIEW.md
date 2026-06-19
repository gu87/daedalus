# P5+.3 实现完成报告：Gate / Permission 审批可用化

> commits: `d6e01ed`, fixup: `35ef66d`

---

## 1. 修改文件

| 文件 | 操作 |
|------|:---:|
| `daedalus-desktop/electron/preload.js` | `permission.request` handler 创建 one-shot `reply` 对象（`used` 标志，`approve/reject/requestChanges`）；**不 settle**，连接保持打开 |
| `daedalus-desktop/src/services/daedalusApi.ts` | `PermissionReply` 接口 + `onPermissionRequest(perm, reply)` 签名更新 |
| `daedalus-desktop/src/App.tsx` | `useRef<Map>` 管理多 tab 审批；`decideApproval` 通过 `approval.id` 查 reply 并发送 response |

**不改**：daedalusd、IPC 协议、Gate/Agent Loop、UI 布局

---

## 2. 设计要点

| 约束 | 实现 |
|------|------|
| permission.request 不 settle | ✅ — 审批期间连接断开仍触发 `connection_lost` |
| Map 管理多 tab | ✅ — `useRef<Map<string, PermissionReply>>`，key = `permission_id` |
| one-shot reply | ✅ — `used` 标志，重复点击不发送 |
| requestChanges | ✅ — 映射为 `decision: "denied"`，UI 文案 "本轮按拒绝处理" |
| 发送后不关闭连接 | ✅ — 继续等待 task.done/task.error |

---

## 3. 返修记录

| # | 修复 |
|---|------|
| 1 | onDone/onError 回调中清理 `permissionReplies` Map + 设置 `approval: null`（终态后不残留可点击的审批条） |
| 2 | browser mock 改为纯 permission 流（不混发 onError + permission.request） |

---

## 4. 验证

```
npm run build  ✅ (219KB JS, 498ms)
```

---

## 4. 不做

| 约束 | 状态 |
|------|:---:|
| 修改 daedalusd / IPC / Gate | ✅ |
| session.rejoin / ack / reconnect | ✅ |
| 重设计 UI | ✅ |
