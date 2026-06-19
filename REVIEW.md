# P5+.2 完成报告：Desktop UDS bridge — task.dispatch 最小闭环

> commits: `a1a3adb`, fixup: 待提交

---

## 1. 修改文件

| 文件 | 操作 |
|------|:---:|
| `daedalus-desktop/electron/preload.js` | UDS NDJSON 客户端 + dispatchTask + ping |
| `daedalus-desktop/src/services/daedalusApi.ts` | TaskCallbacks 类型 |
| `daedalus-desktop/src/App.tsx` | createGeneratedTab("task") 真实 dispatch |

**不改**：daedalusd、IPC 协议、HTTP API、UI 布局

---

## 2. 返修记录

| # | 问题 | 修复 |
|---|------|------|
| 1 | task.done/error 后 `_close` 误报 `connection_lost` | 增加 `settled` 标志，终端事件先 `settle()` 再 `close()`；`_close`/`_error` 仅在 `!settled` 时回调 |
| 2 | `chunk.toString()` 不处理 UTF-8 跨 chunk | 引入 `StringDecoder("utf8")`，`decoder.write(chunk)` 安全拼接 |
| 3 | `ping` 可能 double-resolve | 增加 `resolved` 标志，`done()` wrapper 防止重复 resolve |

---

## 3. 环境变量

| 变量 | 默认值 | 用途 |
|------|--------|------|
| `DAEDALUSD_SOCK` | `/tmp/daedalusd.sock` | UDS socket 路径 |
| `DAEDALUSD_HTTP_ADDR` | `http://127.0.0.1:9800` | HTTP health API |
| `DAEDALUS_DESKTOP_AGENT_ID` | `daedalus-desktop` | Agent ID（需 managed-agents.yaml 中存在同名 agent，或启动时设置） |

---

## 4. 验证

```
npm run build  ✅ (219KB JS, 507ms)
```
