# P4.3 实现方案：cc-haha 本地只读仪表盘

> 基于 P4.2（commit `60703ce`），实现单 HTML 管理仪表盘。
> **只读展示，零操作按钮，零外部依赖。**

---

## 1. 目标

daedalusd HTTP server 直接 serving 一个单 HTML 管理仪表盘，展示：
- 健康状态（daemon uptime、socket 状态、DB 连通性）
- 任务数量快照（running / done / error 计数）
- 最近 20 条任务列表

数据来源：JS 通过 `fetch('/api/health')` + `fetch('/api/tasks')` 获取，5s 轮询刷新。

---

## 2. 设计

### 2.1 布局

```
┌──────────────────────────────────────────────┐
│  daedalusd cc-haha                           │
│  ────────────────────────────────────────────│
│  Daemon: up 2h34m  socket ✅  db ✅          │
│                                              │
│  [Running: 3]  [Done: 127]  [Error: 12]       │
│                                              │
│  Recent Tasks                                │
│  run-abc123  test-agent  error  tool_failure │
│  run-def456  designer   done   —             │
│  ...                                         │
└──────────────────────────────────────────────┘
```

### 2.2 技术约束

- **单文件**：一个 `dashboard.html`，内联 CSS + JS
- **零外部依赖**：无 npm/webpack/React/CDN/外部字体
- **编译时嵌入**：`include_str!("assets/dashboard.html")` 编译进二进制
- **数据获取**：`fetch('/api/health')` + `fetch('/api/tasks?limit=20')`
- **刷新**：`setInterval(fetchData, 5000)`
- **不实现**：WebSocket、SSE、任务操作按钮、登录、暗色模式

---

## 3. 文件边界

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/http/assets/dashboard.html` | **新增** | 单 HTML 仪表盘（~150 行，内联 CSS + JS） |
| `daedalusd/src/http/dashboard.rs` | **新增** | `GET /` handler — 返回内嵌 HTML |
| `daedalusd/src/http/server.rs` | 修改 | 注册 `GET /` 路由 |
| `daedalusd/tests/http_dashboard.rs` | **新增** | 集成测试（GET / → 200 + HTML 包含关键标签） |

**不改**：health.rs、tasks.rs、main.rs、daemon.rs、registry.rs、IPC、schema

---

## 4. 数据流

```
Browser GET /
  → dashboard handler → include_str!("assets/dashboard.html") → HTML
  → JS 加载后 setInterval:
      fetch('/api/health') → 渲染健康状态
      fetch('/api/tasks?limit=20') → 渲染任务列表 + 计数
  → 5s 后重复
```

---

## 5. 测试计划

| # | 测试 | 场景 | 断言 |
|:--|------|------|------|
| 1 | `dashboard_returns_html` | GET / | 200 + Content-Type: text/html + 包含 `daedalusd cc-haha` |
| 2 | `dashboard_js_can_fetch_data` | 启动 daemon（有预写入任务）+ GET / + 解析 HTML | 确认页面包含可用 JS tag（`<script>`、`fetch(`） |
| 3 | `dashboard_works_with_no_tasks` | 空 DB | GET / → 200，页面正常加载 |

---

## 6. 不做清单

| 约束 | 状态 |
|------|:---:|
| 多页面 / SPA 路由 | ✅ 不实现 |
| WebSocket / SSE 实时推送 | ✅ 不实现 |
| 任务操作按钮（取消/重试） | ✅ 不实现 |
| 用户认证 / 角色权限 | ✅ 不实现 |
| i18n / 暗色模式 / 移动端适配 | ✅ 不实现 |
| 外部 CDN / npm / React / 构建工具 | ✅ 不实现 |
| 修改任何后端 API | ✅ 只消费已有端点 |
