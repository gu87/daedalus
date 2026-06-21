# P5+.6 完成报告：`daedalus run` — 终端任务命令

> commit: `e4a0cb1`

---

## 1. 修改/新增文件

| 文件 | 操作 |
|------|:---:|
| `daedalus-orch/daedalus/orch/task_card.py` | **新增** — `build_task_card()` v2.8 构造器 |
| `daedalus-orch/daedalus/orch/cli.py` | 修改 — 新增 `run` 子命令 + `_run()` + `_terminal_permission_handler()` |
| `daedalus-orch/tests/test_cli_run.py` | **新增** — 11 个测试 |
| `README.md` | 修改 — `daedalus run` 示例 + pip install 说明 |

**不改**：`daedalusd`、IPC 协议、`client.py` API

---

## 2. 命令接口

```
daedalus run "echo hello"
daedalus run "目标" --agent daedalus-desktop --socket /tmp/daedalusd.sock --timeout 300
```

| 参数 | 默认 | 说明 |
|------|------|------|
| `goal` | 必填 | 非空 |
| `--agent` | `daedalus-desktop` | Agent ID |
| `--socket` | `$DAEDALUSD_SOCK` 或 `/tmp/daedalusd.sock` | UDS 路径 |
| `--timeout` | `300` | 正数秒 |

### 输出

| 终态 | stdout | stderr | exit |
|------|--------|--------|:---:|
| task.done | `task.done` JSON（原样） | — | 0 |
| task.error | — | `[taxonomy] detail` | 1 |
| connection failed | — | `connection failed: ...` | 1 |
| timeout | — | `timeout: ...` | 1 |
| 参数错误 | — | `error: ...` | 2 |

---

## 3. TaskCard 一致性

- `task_id == task_card["task_card_id"]`（`build_task_card` 返回 `(task_id, task_card)`）
- `agent == task_card["execution_plan"]["primary_agent"]`
- `allowed_files` / `safety.allowed_paths` 均为 `[]`，与 Desktop 一致

---

## 4. 权限交互

| 场景 | 行为 |
|------|------|
| TTY + 输入 `y`/`yes` | `approved` |
| TTY + 输入空/`n`/其他 | `denied` |
| 非 TTY（管道/重定向） | `denied`（stderr 提示） |
| EOFError / KeyboardInterrupt | `denied` |

---

## 5. 测试结果

```
PYTHONPATH=daedalus-orch pytest daedalus-orch/tests/test_cli_run.py -v
============================== 11 passed in 0.63s ==============================
```

| # | 测试 | 类型 |
|:--|------|:---:|
| 1 | test_task_card_ids_consistent | 单元 |
| 2 | test_task_card_project | 单元 |
| 3 | test_run_task_done | 集成（mock daemon） |
| 4 | test_run_task_error | 集成（mock daemon） |
| 5 | test_run_connection_failed | 集成 |
| 6 | test_permission_tty_approved | 单元 |
| 7 | test_permission_tty_default_denied | 单元 |
| 8 | test_permission_non_tty_auto_denied | 单元 |
| 9 | test_arg_defaults | 集成 |
| 10 | test_arg_empty_goal_rejected | 集成 |
| 11 | test_arg_negative_timeout_rejected | 集成 |

---

## 6. 待完成

- ⬜ 真实 daemon 端到端 CLI 验收（`daedalus run "echo hello"` 完整闭环）

---

等待 Codex 复审。
