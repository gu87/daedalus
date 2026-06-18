# P4.1 Codex 返修：HTTP handle + readonly DB

> P4.1 主实现（commit `5112720`）已通过。此返修修复两个问题。

---

## 返修项

### Fix 1: main.rs — 等待 HTTP task 完成

**问题**：HTTP server `tokio::spawn` 后未保存 handle。SIGINT/SIGTERM 后 UDS 返回，main 结束直接 drop runtime，HTTP 可能没完成 graceful shutdown。

**修复**：
- 保存 `http_handle`
- UDS server 返回后调用 `shutdown_token.cancel()`
- `tokio::time::timeout(Duration::from_secs(3), http_handle).await`
- Join error → `eprintln`，不覆盖 UDS 的业务返回
- UDS error 路径同样先 cancel + 等待 HTTP，再 exit(1)

### Fix 2: health.rs — 只读 DB 检查，不创建新文件

**问题**：`pool::open()` 使用 rusqlite bundled feature 会自动创建缺失的 SQLite 文件，导致 `db_ok` 误报 true 且凭空创建空 DB。

**修复**：
- 改用 `rusqlite::Connection::open_with_flags(path, SQLITE_OPEN_READ_ONLY)`
- 打开成功后验证 schema：`SELECT 1 FROM sqlite_master WHERE type='table' AND name='agent_runs'`
- 目录存在但 DB 文件不存在 → 返回 false，**不创建 DB 文件**
- 新增测试 `health_db_file_missing_dir_exists` 断言文件未被创建

---

## 修改文件

| 文件 | 变更 |
|------|------|
| `daedalusd/src/main.rs` | tokio::spawn → 保存 http_handle；UDS 返回后 cancel + timeout await HTTP |
| `daedalusd/src/http/health.rs` | pool::open → Connection::open_with_flags(READ_ONLY) + schema 验证；去除 db::pool 依赖 |
| `daedalusd/tests/http_health.rs` | + health_db_file_missing_dir_exists（4 个测试） |

---

## 验证

```
cargo fmt --all -- --check               ✅
cargo test --test http_health            ✅ 4 passed
cargo test --workspace                   ✅ 394 passed, 0 failed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅
```

---

## 返回 Codex 复审

返修完毕。请 Codex 审查确认。
