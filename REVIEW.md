# P4.4 实现完成报告：配置诊断 + 模型摘要 API

> 基于 P4.2（commit `60703ce`）。不做 Router 热替换。
> (1) 证明 `AgentLoop::new()` 每次 build 读最新 models.yaml；
> (2) 新增 `GET /api/config/models`。

---

## 1. 修改文件清单（6 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/http/config.rs` | **新增** | `GET /api/config/models` handler（provider 不存在 → 500，YAML 错误 → 500，文件缺失 → []） |
| `daedalusd/src/http/mod.rs` | 修改 | + `pub mod config` |
| `daedalusd/src/http/server.rs` | 修改 | 注册 `/api/config/models` 路由 |
| `daedalusd/tests/config_reload.rs` | **新增** | 1 个 reload 证明测试（生产路径：AgentLoop::new） |
| `daedalusd/tests/http_config.rs` | **新增** | 5 个 HTTP 集成测试 |

**未改**：main.rs、daemon.rs、health.rs、tasks.rs、registry.rs、Router、IPC、schema、Gate

---

## 2. 核心行为

### 2.1 `/api/config/models`

- 文件存在且合法 → 200 + 模型列表（id / provider / type 三个字段）
- 文件不存在 → 200 + `{ "models": [] }`
- YAML 语法错误 → 500
- 模型引用不存在的 provider → 500
- 绝不返回 `api_key_env` / `base_url` / `model_id`

### 2.2 reload 证明

第一次 `AgentLoop::new()`：models.yaml 指向不存在 provider → Err
修改 models.yaml 为合法 → 第二次 `AgentLoop::new()` → Ok

证明生产路径（`AgentLoop::new → Router::from_models_config → load_models_yaml`）天然读取最新磁盘文件。

---

## 3. 验证

```
cargo fmt --all -- --check               ✅
cargo test --test config_reload          ✅ 1 passed
cargo test --test http_config            ✅ 5 passed
cargo test --workspace                   ✅ 410 passed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅
```

---

## 4. 返回 Codex 复审

P4.4 实现完毕。请 Codex 审查。
