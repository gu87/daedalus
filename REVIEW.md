# P3.7 Codex 返修：测试真实性修复

> P3.7 主实现（commit `e20e17d`）已通过。此返修仅修复测试真实性问题 + 清理冗余文案。
> **主逻辑范围未扩大。**

---

## 返修项

### 1. `switch_agent_then_auto_revision` YAML first-match 修复

**问题**：switch_agent 规则无 `max_retries`，first-match 导致第二条 auto_revision 永远不可达。B 的第二次失败实际走了 switch_agent（再次切换到 B 自身），而非 auto_revision。

**修复**：switch_agent 加 `max_retries: 1`，auto_revision 加 `max_retries: 2`。

```yaml
# 修复前
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: test-agent-2         # ← 无 max_retries，永远匹配
  - error_code: tool_failure           # ← 不可达
    action: auto_revision

# 修复后
rules:
  - error_code: tool_failure
    action: switch_agent
    target_agent: test-agent-2
    max_retries: 1                     # ← 只允许 1 次 switch
  - error_code: tool_failure
    action: auto_revision
    max_retries: 2                     # ← switch 后 fallback 到 retry
```

### 2. 增强断言以区分 SwitchAgent vs AutoRevision

新增 provider_b 的 recorded messages 检查：

- B 第一次调用：包含 `"Previous agent 'test-agent' failed"`（证明是 SwitchAgent）
- B 第二次调用：**不**包含 `"Previous agent 'test-agent-2' failed"`（排除再次 SwitchAgent）
- B 第二次调用：包含 `"Previous attempt failed:"`（证明是 AutoRevision）

### 3. daemon.rs `set_retry_feedback` 去重

**问题**：SwitchAgent arm 自行拼接了 `"Please analyse the failure..."`，但 `set_retry_feedback` 内部已追加此句。

**修复**：只传 `format!("Previous agent '{}' failed: {}", old_agent, agent_error.detail)`。

---

## 修改文件

| 文件 | 变更 |
|------|------|
| `daedalusd/tests/gate_daemon.rs` | switch_agent_then_auto_revision：YAML max_retries + recorded messages 断言 |
| `daedalusd/src/daemon.rs` | SwitchAgent arm set_retry_feedback 去掉重复的 "Please analyse..." |

---

## 验证

```
cargo fmt --all -- --check               ✅
cargo test --test gate_daemon switch_agent_then_auto_revision  ✅
cargo test --workspace                   ✅ 385 passed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅
```

---

## 返回 Codex 复审

返修完毕，主逻辑范围未扩大。请 Codex 审查确认。
