# P5+.4 实现完成报告：Task state restore

> commits: `92186c1`, fixup: `f17c549`

---

## 1. 修改文件（8 个）

| 文件 | 操作 |
|------|:---:|
| `daedalus-desktop/electron/preload.js` | + `getTaskDetail(runId)` |
| `daedalus-desktop/src/services/daedalusApi.ts` | + `TaskDetail` 类型 + `getTaskDetail()` + mock（run-1/run-2 真实详情） |
| `daedalus-desktop/src/types/index.ts` | `WorkTab` + `readonly?: boolean` |
| `daedalus-desktop/src/App.tsx` | `historyTasks` + `openHistoryTask` + `historyTab` helper |
| `daedalus-desktop/src/components/AppShell.tsx` | 传递 props |
| `daedalus-desktop/src/components/LeftSidebar.tsx` | 底部历史任务区域 + 过滤 readonly tab |
| `daedalus-desktop/src/views/ChatView.tsx` | 详情消息 pre-wrap + readonly 隐藏 Composer |
| `daedalus-desktop/src/styles.css` | history-section / task-detail-text 最小样式 |

**不改**：daedalusd、IPC、SQLite、UI 布局

---

## 2. 返修记录

| # | 修复 |
|---|------|
| 1 | history-section CSS（复用侧栏视觉规则） |
| 2 | mock getTaskDetail 对 run-1/run-2 返回真实详情 |
| 3 | conversations 列表过滤 `!tab.readonly` |
| 4 | 详情显示 heartbeat_at + pre-wrap 换行保留 |

---

## 3. 验证

```
npm run build  ✅ (223KB JS + 13.6KB CSS, 475ms)
```

- 历史任务列表正常显示
- 点击 → 只读详情（含 heartbeat_at，多行保留）
- 无 Composer
- 历史 Tab 不出现在对话列表
- 未知 runId → 加载失败

---

## 4. 不做

| 约束 | 状态 |
|------|:---:|
| session.rejoin / ack / event replay | ✅ |
| 修改 daemon / IPC / SQLite | ✅ |
| 新 UI 设计 | ✅ |
