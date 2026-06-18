# P5.1 实现完成报告：DAEDALUS.md 项目指令源

> Memory Layer 最后一块。其他 7 个 Provider 已在 P3.1a/P3.1b 接入。
> P5.1 只新增 DAEDALUS.md。

---

## 1. 修改文件清单（6 个）

| 文件 | 操作 | 变更摘要 |
|------|:---:|------|
| `DAEDALUS.md` | **新增** | 项目根目录，项目级 Agent 指令模板 |
| `daedalusd/src/config.rs` | 修改 | DaedalusConfig + `daedalus_md_path`（env `DAEDALUS_MD_PATH`，默认 `./DAEDALUS.md`） |
| `daedalusd/src/agent/prompt_sources.rs` | 修改 | 新增 `DaedalusMdProvider`（`read_optional`，silent skip） |
| `daedalusd/src/agent/prompt.rs` | 修改 | PromptBuilder chain 插入 DaedalusMdProvider（soul 之后、memory 之前） |
| `daedalusd/tests/prompt.rs` | 修改 | +3 测试：链顺序、缺失 skip、自定义路径；+ 手动链补 DaedalusMdProvider |
| 10 个测试文件 | 修改 | DaedalusConfig struct literal 补齐 `daedalus_md_path` |

**未改**：IPC、schema、Gate、Agent Loop、daemon、HTTP

---

## 2. 核心行为

### Provider chain 顺序

```
[soul] → [daedalus] → [memory] → [user] → [prefs] → [agent]
→ [feedback] → [project] → [authority] → [history] → [skills]
```

### Silent skip

- DAEDALUS.md 不存在 → `read_optional` 返回 `None` → `[daedalus]` 段不注入
- DAEDALUS.md 存在但为空 → 同 None
- 不报错，不影响其他 provider

### Env override

- `DAEDALUS_MD_PATH` 环境变量覆盖默认路径 `./DAEDALUS.md`
- 自定义路径测试通过 `DaedalusConfig.daedalus_md_path` 直接构造

---

## 3. 验证

```
cargo fmt --all -- --check               ✅
cargo test --test prompt                 ✅ 38 passed (+3 P5.1)
cargo test --workspace                   ✅ 418 passed
  -- --skip long_line_returns_error_and_closes
cargo clippy --workspace -- -D warnings  ✅
```

### 新增测试（3 个）

| # | 测试 | 断言 |
|:--|------|------|
| 1 | `prompt_builder_new_includes_daedalus_between_soul_and_memory` | [soul] < [daedalus] < [memory] |
| 2 | `daedalus_md_missing_silent_skip` | 文件缺失 → prompt 不含 [daedalus]，不报错 |
| 3 | `daedalus_md_custom_path` | 自定义路径正确注入内容 |

---

## 4. 返回 Codex 复审

P5.1 实现完毕，418 测试全过。请 Codex 审查。
