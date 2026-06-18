# P5.1 实现完成报告：DAEDALUS.md 项目指令源

> Memory Layer 最后一块。其他 7 个 Provider 已在 P3.1a/P3.1b 接入。
> commits: `07fb756`, fixup: `72de9ac`, fixup: `e9f3a42`

---

## 1. 修改文件清单（6 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `DAEDALUS.md` | **新增** | 项目根目录，项目级 Agent 指令模板 |
| `daedalusd/src/config.rs` | 修改 | DaedalusConfig + `daedalus_md_path`（env `DAEDALUS_MD_PATH`，默认 `./DAEDALUS.md`）；+ `load_uses_daedalus_md_path_env_override` 测试 |
| `daedalusd/src/agent/prompt_sources.rs` | 修改 | 新增 `DaedalusMdProvider`（`read_optional`，silent skip） |
| `daedalusd/src/agent/prompt.rs` | 修改 | PromptBuilder chain 插入 DaedalusMdProvider（soul 之后、memory 之前） |
| `daedalusd/tests/prompt.rs` | 修改 | +3 测试：链顺序（走真实 `PromptBuilder::new(config)` + EnvGuard）、缺失 skip、自定义路径；+ EnvGuard util |
| 10 个测试文件 | 修改 | DaedalusConfig struct literal 补齐 `daedalus_md_path` |

**未改**：IPC 行为/协议、schema、Gate、Agent Loop、daemon、HTTP（`ipc/control.rs` 仅为 struct literal 补字段，无 IPC 行为变化）

---

## 2. 核心行为

### Provider chain 顺序

```
[soul] → [daedalus] → [memory] → [user] → [prefs] → [agent]
→ [feedback] → [project] → [authority] → [history] → [skills]
```

### Silent skip

- DAEDALUS.md 不存在 → `read_optional` 返回 `None` → `[daedalus]` 段不注入，不报错
- 测试 `daedalus_md_missing_silent_skip` 已恢复 `#[test]` 并实际运行 ✅

---

## 3. 验证

```
cargo fmt --all -- --check               ✅
cargo test --test prompt                 ✅ 38 passed
cargo test --workspace                   ✅ 419 passed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅
```

---

## 4. 返回 Codex 复审

P5.1 返修完毕。请 Codex 审查。
