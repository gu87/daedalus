# P5+.6 完成报告：`daedalus run` — 终端任务命令

> commits: `e4a0cb1`, fixup: 待提交

---

## 1. 修改/新增文件

| 文件 | 操作 |
|------|:---:|
| `daedalus-orch/daedalus/orch/task_card.py` | **新增** |
| `daedalus-orch/daedalus/orch/cli.py` | 修改 |
| `daedalus-orch/tests/test_cli_run.py` | **新增** |
| `README.md` | 修改 |

**不改**：daedalusd、IPC 协议、`client.py` API

---

## 2. 返修记录

| # | 修复 |
|---|------|
| 1 | `permission_handler` 改为 `dispatch()` 显式参数（非 DaedalusClient 构造） |
| 2 | 不捕获 `KeyboardInterrupt`（让 Ctrl+C 中断退出） |
| 3 | 非 TTY 自动拒绝 + 集成流测试（mock daemon → perm → denied → task.done） |
| 4 | timeout `math.isfinite()` + nan/0/空 agent 测试 |
| 5 | README：pip install 顺序 + socket 提取 |

---

## 3. 测试

```
pytest daedalus-orch/tests/test_cli_run.py -v
============================== 14 passed in 1.27s ==============================
```

| # | 测试 | 类型 |
|:--|------|:---:|
| 1 | test_task_card_ids_consistent | 单元 |
| 2 | test_task_card_project | 单元 |
| 3 | test_run_task_done | 集成 |
| 4 | test_run_task_error | 集成 |
| 5 | test_run_connection_failed | 集成 |
| 6 | test_permission_tty_approved | 单元 |
| 7 | test_permission_tty_default_denied | 单元 |
| 8 | test_permission_non_tty_auto_denied | 单元 |
| 9 | test_arg_defaults | 集成 |
| 10 | test_arg_empty_goal_rejected | 集成 |
| 11 | test_arg_negative_timeout_rejected | 集成 |
| 12 | test_arg_timeout_nan_rejected | 集成 |
| 13 | test_arg_empty_agent_rejected | 集成 |
| 14 | test_run_permission_non_tty_auto_denied_flow | **集成流** |

---

## 4. 待完成

- ⬜ 真实 daemon 端到端 `daedalus run "echo hello"` 验收

---

等待 Codex 复审。
