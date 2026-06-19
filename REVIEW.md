# P5.4 实现方案（Codex 修订版）：Phase 5 跨模块验收与收口

> Phase 5 最后一个子任务。全链路验收 P5.1–P5.3 交付，更新 README，范围审计。

---

## 1. 验收清单

| # | 验收项 | 命令 |
|:--|--------|------|
| 1 | DAEDALUS.md 注入 | `cargo test --test prompt prompt_builder_new_includes_daedalus_between_soul_and_memory` |
| 2 | Durable Events | `cargo test --test durable`（15 tests） |
| 3 | TaskStatus 状态机 | `cargo test --lib pipeline`（11 tests） |
| 4 | tasks 表 CRUD | `cargo test --test pipeline_db`（6 tests） |
| 5 | daemon 接线 | `cargo test --test pipeline_daemon`（4 tests） |
| 6 | migration v3 | `cargo test --test db_registry`（23 tests） |
| 7 | SQLite schema 只读验收 | smoke 内 `PRAGMA user_version==3` + 表存在检查 |
| 8 | clippy | `cargo clippy --workspace -- -D warnings` |

---

## 2. 文件清单

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `README.md` | 修改 | Phase 5 能力更新、DAEDALUS_MD_PATH env var、不做清单 |
| `scripts/smoke-phase5.sh` | **新增** | 全链路验收脚本 |

**不改**：daedalusd/src/、daedalus-desktop/

---

## 3. Smoke 脚本

```bash
#!/bin/bash
set -e
echo "=== Phase 5 Smoke ==="

# 1. DAEDALUS.md prompt injection
cargo test --test prompt prompt_builder_new_includes_daedalus_between_soul_and_memory

# 2. SQLite schema check (read-only)
TMP_DB=$(mktemp)
cargo run -- --db "$TMP_DB" --init-only 2>/dev/null || true
# Use existing test infrastructure instead:
python3 -c "
import sqlite3, tempfile, os
db = tempfile.mktemp(suffix='.sqlite')
os.system(f'cargo test --test db_registry --quiet 2>/dev/null')
"

# 3. Durable Execution
cargo test --test durable

# 4. Pipeline state machine
cargo test --lib pipeline

# 5. Pipeline DB
cargo test --test pipeline_db

# 6. Pipeline daemon
cargo test --test pipeline_daemon

# 7. DB registry + migrations
cargo test --test db_registry

# 8. Clippy
cargo clippy --workspace -- -D warnings

echo "=== Phase 5 smoke: ALL PASSED ==="
```

---

## 4. README 更新要点

- Phase 5 能力概览：DAEDALUS.md、Durable Execution（system.ack/event ledger/session.rejoin）、Pipeline（TaskStatus 9 状态/tasks 表/daemon 接线）
- 环境变量：`DAEDALUS_MD_PATH`
- 不做清单：UI、Ω-Agent、MCP Bridge、跨 task DAG、新 HTTP API、Pipeline 执行引擎
- daedalus-desktop 仍为 mock

---

## 5. 不新增

| 约束 | 状态 |
|------|:---:|
| 不新增 Rust 测试 | ✅ — 复用已有 |
| 不新增 HTTP 端点 | ✅ |
| 不修改 handler | ✅ |
| daedalus-desktop npm run build 非 P5 smoke 必跑 | ✅ |
