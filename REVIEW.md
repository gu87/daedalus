# P5+.4 实现完成报告：Task state restore

> commit: 待提交

---

## 1. 修改文件（6 个）

| 文件 | 操作 |
|------|:---:|
| `daedalus-desktop/electron/preload.js` | + `getTaskDetail(runId)` → `fetch(/api/tasks/:run_id)` |
| `daedalus-desktop/src/services/daedalusApi.ts` | + `TaskDetail` 类型 + `getTaskDetail()` + mock |
| `daedalus-desktop/src/types/index.ts` | `WorkTab` + `readonly?: boolean` |
| `daedalus-desktop/src/App.tsx` | `historyTasks` state + `openHistoryTask()` + `historyTab()` helper |
| `daedalus-desktop/src/components/AppShell.tsx` | 传递 `historyTasks` / `onOpenHistoryTask` 到 LeftSidebar |
| `daedalus-desktop/src/components/LeftSidebar.tsx` | 底部新增 "历史任务" 区域 |
| `daedalus-desktop/src/views/ChatView.tsx` | `readonly` tab 隐藏 Composer |

**不改**：daedalusd、IPC 协议、SQLite schema、UI 布局

---

## 2. 验证

```
npm run build  ✅ (222KB JS, 512ms)
```

- 启动 Desktop → 左侧显示历史任务列表（mock 2 条）
- 点击历史任务 → 打开只读 Tab（无 Composer）
- daemon 离线/空列表 → 显示 "暂无历史任务"

---

## 3. 不做

| 约束 | 状态 |
|------|:---:|
| session.rejoin / ack / event replay | ✅ |
| 修改 daemon / IPC / SQLite | ✅ |
| 新 UI 设计 | ✅ |
