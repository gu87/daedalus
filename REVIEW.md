# P5+.7 实施完成报告

> 日期：2026-06-21
> 状态：✅ DONE

---

## 1. 根因与修复

| # | 问题 | 根因 | 修复 |
|:--|------|------|------|
| 1 | DeepSeek HTTP 400 | Thinking mode 要求 `reasoning_content` 传回 | `models.yaml` 加 `thinking: disabled` |
| 2 | Tool message 配对失败 | denied 路径跳过 `pending_tool_calls` | 继续处理剩余 pending 直到清空 |

两个 bug 互相独立，但 P5+.7 引入的 assistant.tool_calls 序列化暴露了 bug 2。

---

## 2. 变更文件

| # | 文件 | 变更 |
|:--|------|------|
| 1 | `daedalusd/src/types.rs` | `ChatMessage` + `tool_calls: Vec<ToolCall>` |
| 2 | `daedalusd/src/config.rs` | `ThinkingMode` enum + `ModelEntry.thinking` |
| 3 | `daedalusd/src/llm/openai_compat.rs` | 删 tool_choice WIP；`build_request` 方法化；tool_calls 序列化；thinking disabled；+4 单测 |
| 4 | `daedalusd/src/llm/router.rs` | 传递 `entry.thinking` 给 `OpenAICompatProvider::with_thinking()` |
| 5 | `daedalusd/src/agent/loop.rs` | 10 个 ChatMessage 显式填字段；assistant 带 tool_calls；denied 路径处理 pending；`pub fn messages()` |
| 6 | `daedalusd/src/http/validate.rs` | 1 个 ChatMessage 补齐字段 |
| 7 | `daedalusd/tests/provider.rs` | `text_msg()` 补齐 + 2 个 thinking:disabled HTTP mapping 测试 |
| 8 | `daedalusd/tests/agent_loop.rs` | + `scenario_21` 回归测试：denied 不截断 pending_tool_calls |
| 9 | `.gitignore` | + `.venv/` |
| 10 | `~/.daedalus/models.yaml` (仓库外) | 备份 + `thinking: disabled` |

---

## 3. 测试结果

### 单元测试
- llm::openai_compat: **10 passed** (+4 个新增)
- types / config / error / tools / agent: 全量通过

### 集成测试
- provider: **28 passed** (+2 个新增)
- agent_loop: **21 passed** (+1 个新增)
- protocol / tool_registry / db_registry / permission / gate_daemon / pipeline_db / prompt / durable: 全量通过

```
cargo test -p daedalusd --lib llm::openai_compat    ✅ 10 passed
cargo test -p daedalusd --test provider             ✅ 28 passed
cargo test -p daedalusd --test agent_loop           ✅ 21 passed
cargo clippy --workspace -- -D warnings             ✅ clean
```

### E2E
```
daedalus run --agent daedalus-desktop "confirm task done with message: hello world"
→ exit 0
→ {"type":"task.done",...,"status":"waiting_for_verification",...}
```

---

## 4. 关键设计决策

- `thinking: disabled` 用 enum 不用 bool/string 魔法值
- `ChatMessage.tool_calls` 用 `#[serde(default)]` 保证反序列化兼容
- `OpenAICompatProvider::with_thinking()` builder 不破坏 `new()` 签名
- `build_request` 改为方法访问 `self.thinking`
- `ToolCall.arguments` 序列化为 JSON 字符串（`tc.input.to_string()`）
- 本阶段不实现 `reasoning_content` passthrough

---

## 5. 不做

- `tool_choice: "auto"` — 已删除（不是根因）
- reasoning_content passthrough — 后续优化
- Anthropic 逻辑修改 — 不在影响范围
- ToolRegistry 按 AgentConfig.tools 过滤 — pre-existing
