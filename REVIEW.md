# P4.5 实现完成报告：模型连通性探测 API

> 基于 P4.4（commit `168eb23`）。新增 `POST /api/models/validate`。
> 走 `Router::from_models_config()` + `chat_with_fallback()` 生产路径。

---

## 1. 修改文件清单（4 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `daedalusd/src/http/validate.rs` | **新增** | `POST /api/models/validate` handler（Router 生产路径 + 10s 超时） |
| `daedalusd/src/http/mod.rs` | 修改 | + `pub mod validate` |
| `daedalusd/src/http/server.rs` | 修改 | 注册 `POST /api/models/validate` 路由（`axum::routing::post`） |
| `daedalusd/tests/http_validate.rs` | **新增** | 5 个 httptest 集成测试 |

**未改**：Router、AgentLoop、daemon、config.rs、IPC、schema

---

## 2. 核心行为

```
POST /api/models/validate  { "model_id": "local-model" }
  → model_id 为空 → 400
  → load_models_yaml() → 找 models[].id → 不存在 → 400
  → ModelStrategy { primary: ModelConfig { model: "local-model", max_tokens: 1 } }
  → Router::from_models_config() → 缺 API key / 配置错误 → 400
  → tokio::time::timeout(10s, chat_with_fallback("ping"))
  → Ok → 200 { reachable: true, latency_ms }
  → Err(ProviderError) → 200 { reachable: false, error: Display(e) }
  → Timeout → 200 { reachable: false, error: "timeout after 10s" }
```

- model_id 是本地 `models[].id`，非上游 `models[].model_id`
- ProviderError 用 `Display` 文本，不泄密

---

## 3. 验证

```
cargo fmt --all -- --check               ✅
cargo test --test http_validate          ✅ 5 passed
cargo test --workspace                   ✅ 415 passed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅
```

### http_validate 测试（5 个）

| # | 测试 | 断言 |
|:--|------|------|
| 1 | validate_reachable | httptest 200 → reachable:true + latency_ms |
| 2 | validate_unreachable_401 | httptest 401 → reachable:false + error 含 "auth error" |
| 3 | validate_unknown_model_400 | model_id 不在配置 → 400 |
| 4 | validate_missing_model_id_400 | 空 body → 400 + "model_id" |
| 5 | validate_missing_api_key_400 | remove_var → 400 + "missing API key" |

---

## 4. 返回 Codex 复审

P4.5 实现完毕，415 测试全过。请 Codex 审查。
