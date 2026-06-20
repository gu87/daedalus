# P5+.5 实现方案：Dogfood smoke + README 更新（Codex 修订版）

> Phase 5+（P5+.1–P5+.4）已完成。P5+.5 目标：一键启动 daemon + desktop，README 记录真实使用步骤。

---

## 1. Dogfood 脚本

```bash
#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

TMP_DIR=$(mktemp -d /tmp/daedalus-dogfood-XXXXXX)
SOCK_PATH="$TMP_DIR/daedalusd.sock"
STATE_DIR="$TMP_DIR/state"
HTTP_ADDR="127.0.0.1:9800"

# Check port not in use.
if lsof -i :9800 > /dev/null 2>&1; then
  echo "ERROR: port 9800 is already in use" >&2
  rm -rf "$TMP_DIR"
  exit 1
fi

DAEMON_PID=""
DESKTOP_PID=""
cleanup() {
  if [ -n "${DAEMON_PID:-}" ]; then kill "$DAEMON_PID" 2>/dev/null || true; wait "$DAEMON_PID" 2>/dev/null || true; fi
  if [ -n "${DESKTOP_PID:-}" ]; then kill "$DESKTOP_PID" 2>/dev/null || true; wait "$DESKTOP_PID" 2>/dev/null || true; fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "=== Daedalus Dogfood ==="

# 1. Build
echo "building daedalusd..."
cargo build -p daedalusd
echo "building daedalus-desktop..."
(cd daedalus-desktop && npm run build)

# 2. Start daemon (env vars, no CLI flags)
echo "starting daedalusd..."
DAEDALUSD_SOCK="$SOCK_PATH" \
DAEDALUSD_STATE_DIR="$STATE_DIR" \
DAEDALUSD_HTTP_ADDR="$HTTP_ADDR" \
  "$PROJECT_DIR/target/debug/daedalusd" &
DAEMON_PID=$!

# 3. Wait for daemon health
ready=0
for i in $(seq 1 30); do
  if curl -sf "http://$HTTP_ADDR/api/health" > /dev/null 2>&1; then
    ready=1
    echo "daemon ready after ${i}s"
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  echo "ERROR: daemon did not become ready within 30s" >&2
  exit 1
fi

# 4. Start Electron desktop
echo "starting daedalus-desktop..."
DAEDALUSD_HTTP_ADDR="http://$HTTP_ADDR" \
DAEDALUSD_SOCK="$SOCK_PATH" \
  npm run electron:dev --prefix daedalus-desktop &
DESKTOP_PID=$!

echo ""
echo "=== Dogfood running ==="
echo "HTTP: http://$HTTP_ADDR"
echo "Press Ctrl+C to stop"
wait
```

---

## 2. README 更新要点

### 修正过期描述

- ❌ 删除 "Desktop 仍为 mock"、"future IPC bridge"、"zero backend"
- ✅ Electron 模式使用真实 HTTP + UDS；浏览器 `npm run dev` 仍用 mock
- ✅ daemon 必须运行且有 `managed-agents.yaml` 配置 `daedalus-desktop` agent

### 新增 Dogfood 快速开始

```bash
bash scripts/dogfood.sh       # 一键启动
```

### 前置条件说明

- `~/.daedalus/SOUL.md` — 必须存在
- `~/.daedalus/config/managed-agents.yaml` — 必须包含 `daedalus-desktop` agent
- `~/.daedalus/models.yaml` — 必须，含有效 API key
- `npm install` — 首次运行前

### 验收边界

| 验收项 | 方式 |
|--------|:---:|
| daemon 编译 + 启动 | 自动（脚本验证） |
| daemon health 响应 | 自动（curl 断言） |
| Electron 启动 | 自动（进程检查） |
| Desktop 显示 🟢 | 人工 |
| 新建任务 → task.done | **人工**（需真实模型） |

---

## 3. 验证

```
bash scripts/dogfood.sh   ✅ 自动部分通过
（人工：Desktop 显示 🟢，新建任务）
```

---

## 4. 不做

| 约束 | 状态 |
|------|:---:|
| 不新增 daemon CLI flag | ✅ — 只用 env vars |
| 不引入 fake provider | ✅ |
| 不修改 daemon 代码 | ✅ |
| "新建任务→done" 不宣称自动通过 | ✅ |
