# P4.3 修订方案：daedalus-desktop Electron React UI Skeleton 收口

> 原 P4.3 计划为"单 HTML cc-haha 管理前端"。
> UI Skeleton 合并（commit `6dc9cd4`）后，前端形态已升级为 Electron + React + TypeScript 桌面项目。
> P4.3 修订为 **daedalus-desktop 收口任务**：确认边界、记录状态、不接真实后端。

---

## 1. 背景

原 P4.3 目标：daedalusd HTTP server serving 单 HTML 仪表盘（`GET /`）。

UI Skeleton 合并后现状：
- `daedalus-desktop/` 是独立 Electron + Vite + React 19 + TypeScript 项目
- 拥有完整的左中右三栏布局、工作 Tab、工具 Tab、Gate 审批条
- 三种核心视图（对话/会议/任务执行）均使用 mock data
- 零真实后端接入，零 IPC 接入

修订后的 P4.3 定位为**前端主线收口**：不再另外实现 cc-haha 单 HTML 仪表盘。
daedalus-desktop 是 Phase 4 及后续所有前端功能的统一载体。

---

## 2. 已完成 commit

| Commit | 内容 |
|--------|------|
| `6dc9cd4` | UI Skeleton 合并 + Electron wrapper + mock preload |
| `75d7df6` | 合并完成报告 |
| `a21b0df` | 返修：commit 说明修正 + Vite loopback 收紧 |

---

## 3. 文件边界（P4.3 涉及）

```
daedalus-desktop/
├── electron/
│   ├── main.js               # Electron 主进程（最小壳，不改）
│   └── preload.js            # contextBridge → window.daedalusAPI（mock，不改）
├── src/
│   ├── App.tsx               # 根组件（全部 UI 状态用 React state）
│   ├── styles.css            # 全局样式
│   ├── components/           # AppShell / LeftSidebar / CenterTabs / Composer /
│   │                           ApprovalBar / RightToolDrawer / MainContent / LeftRail
│   ├── views/                # ChatView / ProjectView / EmptyView
│   ├── data/mockData.ts      # Mock 工作区 + 标签
│   ├── types/index.ts        # TypeScript 类型
│   ├── utils/time.ts, view.ts
│   └── services/
│       └── daedalusApi.ts     # ★ IPC 接入层（当前 mock，后续只从这里改）
├── package.json              # Electron + Vite + React 依赖
├── vite.config.ts            # base: "./" 兼容 Electron file://
└── tsconfig.json
```

**不涉及的文件**：daedalusd/（Rust 后端）、daedalus-orch/（Python）、desktop-demo/（旧 JS demo）

---

## 4. 已验证的 UI 交互

| 功能 | 实现位置 | 确认方式 |
|------|------|:---:|
| 左侧展开 / 折叠 | App.tsx `leftCollapsed` + AppShell toggle | 源码审查 |
| 右侧显示 / 隐藏 | App.tsx `rightCollapsed` | 源码审查 |
| 中间工作 Tab（打开/切换/关闭） | App.tsx `openStarter` / `setActiveTabId` / `closeTab` | 源码审查 |
| 右侧工具 Tab（多开/切换/关闭） | App.tsx `openRightTool` / `closeRightTool` | 源码审查 |
| Gate 审批（approve/reject/changes） | ApprovalBar.tsx + App.tsx `decideApproval` | 源码审查 |
| 新对话 / 会议室 / 新建任务 → 新 Tab | App.tsx `createGeneratedTab` | 源码审查 |
| 构建 (`npm run build`) | tsc + vite → dist/ | ✅ 通过 |
| 开发 (`npm run dev`) | Vite → 127.0.0.1:5173 | ✅ 通过 |

---

## 5. 不做清单

| 约束 | 状态 |
|------|:---:|
| 不接真实 daedalusd 后端 | ✅ |
| 不接真实 Electron IPC | ✅ |
| 不重新设计 UI | ✅ |
| 不引入 Redux / Zustand（只用 React state） | ✅ |
| 不修改 Electron main process 业务逻辑 | ✅ |
| 不修改 Rust/Python 后端 | ✅ |
| 不删除 desktop-demo/ | ✅ |
| 不删除会议模式 / Gate 审批条 | ✅ |
| 不把右侧做成详情页再返回 | ✅ |
| 不把中间顶部做成固定功能 Tab | ✅ |
| 不新增 HTTP API 端点 | ✅ |

---

## 6. 后续真实 IPC 接入 TODO

| # | 事项 | 目标文件 | 依赖 |
|:--|------|------|:---:|
| 1 | `window.daedalusAPI.listSessions()` → 真实 `GET /api/tasks` | `electron/preload.js` | P4.2 已完成 |
| 2 | `window.daedalusAPI.getHealth()` → 真实 `GET /api/health` | `electron/preload.js` | P4.1 已完成 |
| 3 | `window.daedalusAPI.approveGate/rejectGate/requestChanges` → 真实 IPC permission response | `electron/preload.js` | daemon IPC 已有 |
| 4 | `daedalusApi.ts` 从 mock 改为调用 `window.daedalusAPI` | `src/services/daedalusApi.ts` | 1-3 |
| 5 | `App.tsx` 从 mock state 改为通过 `daedalusApi` 获取/更新数据 | `src/App.tsx` | 4 |
| 6 | 实时事件流 `subscribeEvents` → daedalusd event stream | `electron/preload.js` + `daedalusApi.ts` | daemon IPC |
| 7 | Electron 打包签名 / auto-update | `electron-builder.yml` | 1-6 稳定后 |

---

## 7. 与后续 P4.4–P4.6 的关系

| 后续任务 | 影响 daedalus-desktop？ |
|------|:---:|
| P4.4 配置诊断 + 模型摘要 API | 否（纯后端） |
| P4.5 模型连通性探测 API | 否（纯后端） |
| P4.6 Phase 4 验收收口 | 是（需确认 Electron app 可加载 + daedalusd HTTP API 可用） |
