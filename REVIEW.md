# P5+.5 完成报告：Dogfood smoke + README

> commits: `a7ef253`, fixup: 待提交

---

## 1. 修改文件

| 文件 | 操作 |
|------|:---:|
| `scripts/dogfood.sh` | **新增** — 一键启动 daemon + Electron desktop |
| `README.md` | 修改 — Dogfood quickstart、删除过期 "mock/zero backend" 文案 |

---

## 2. 脚本行为

| 特性 | 实现 |
|------|------|
| daemon 启动 | env var only（DAEDALUSD_SOCK/STATE_DIR/HTTP_ADDR），无 CLI flags |
| STATE_DIR | `${DAEDALUSD_STATE_DIR:-$HOME/.daedalus/state}`，持久保留任务历史 |
| 端口检测 | `lsof -nP -iTCP:9800 -sTCP:LISTEN`，仅检测 LISTEN 状态 |
| daemon 存活检测 | `kill -0` 轮询，提前退出则报错 |
| Electron 前台 | `npm run electron:dev`，Ctrl+C 自然终止 |
| cleanup | 删除临时 socket + TMP_DIR，不删持久 DB |

---

## 3. 验证

```
bash -n scripts/dogfood.sh        ✅ 语法检查
```

验收边界：

| 验收项 | 状态 |
|--------|:---:|
| daemon 编译 + health | 自动（脚本） |
| Electron 启动 | 自动 |
| Desktop 显示 🟢 | **待用户人工确认** |
| 新建任务 → task.done | **待用户人工确认**（需真实模型 + API key） |
| Ctrl+C 后无残留进程 | **待用户人工确认** |

---

## 4. 不做

| 约束 | 状态 |
|------|:---:|
| 不新增 daemon CLI flag | ✅ |
| 不修改 daemon 代码 | ✅ |
| API key 不硬编码 | ✅ — env var 引用 |
