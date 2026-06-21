# P5+.6 实现方案：`daedalus run` — 最小终端任务命令

> 基于 P5+.5。复用现有 `DaedalusClient.dispatch()`，新增 CLI `daedalus run`。
> 不新增协议、不修改 Rust daemon。

---

## 1. 命令设计

```bash
daedalus run "echo hello"
daedalus run "目标" --agent daedalus-desktop --socket /tmp/daedalusd.sock --timeout 300
```

| 参数 | 默认 | 说明 |
|------|------|------|
| `goal` | 必填 | 任务目标文本 |
| `--agent` | `daedalus-desktop` | Agent ID |
| `--socket` | `/tmp/daedalusd.sock` | UDS 路径 |
| `--timeout` | `300` | 超时秒数 |

---

## 2. TaskCard 构造

与 Desktop preload `buildTaskDispatch` 一致：

```python
def _build_task_card(goal: str, agent_id: str) -> dict:
    task_id = f"task-{uuid.uuid4().hex[:12]}"
    return {
        "schema_version": "2.8",
        "task_card_id": task_id,
        "project": "daedalus-cli",
        "created_at": _now_utc(),
        "status": "created",
        "goal": goal,
        "compiled_intent": {"action": goal},
        "context": {
            "user_preferences": {},
            "project_context": {"name": "cli", "data": {}, "global_must_avoid": []},
            "relevant_feedback": [],
        },
        "execution_plan": {"primary_agent": agent_id},
        "acceptance_criteria": {},
        "allowed_files": ["."],
        "safety": {"allowed_paths": ["."], "denied_commands": []},
        "output_contract": {},
        "review_gate_criteria": {},
    }
```

---

## 3. 终端权限交互

继承 `DaedalusClient.dispatch()` 的 `permission_handler` 回调。

```python
def _terminal_permission_handler(perm: dict) -> str:
    tool = perm.get("tool", "unknown")
    args = json.dumps(perm.get("args", {}), ensure_ascii=False)
    print(f"\n[Gate] {tool} 请求执行:\n  {args}")
    while True:
        ans = input("批准？[y/N] ").strip().lower()
        if ans in ("y", "yes"):
            return "approved"
        if ans in ("", "n", "no"):
            return "denied"
```

- 如果 stdin 不是 TTY（管道/重定向），直接返回 `"denied"`。
- 已有的 `on_permission` 参数继续可用。

---

## 4. 文件边界

| 文件 | 操作 | 职责 |
|------|:---:|------|
| `daedalus-orch/daedalus/orch/cli.py` | 修改 | 新增 `run` 子命令 + `_run()` async handler |
| `daedalus-orch/daedalus/orch/task_card.py` | **新增** | `build_task_card()` 构造逻辑 |
| `daedalus-orch/tests/test_cli_run.py` | **新增** | 5 个 CLI 测试 |

**不改**：`client.py`、`daedalusd`、IPC 协议

---

## 5. 输出

| 终态 | stdout | stderr | exit |
|------|--------|--------|:---:|
| task.done | `{"status":"done","summary":"...","outbox":{...}}` | — | 0 |
| task.error | — | `[taxonomy] detail` | 1 |
| connection failed | — | `connection failed: ...` | 1 |
| timeout | — | `timeout: ...` | 1 |

---

## 6. 测试计划

| # | 测试 | 场景 | 断言 |
|:--|------|------|------|
| 1 | `test_run_success` | mock daemon 返回 task.done | exit 0 + stdout 含 done |
| 2 | `test_run_task_error` | mock 返回 task.error | exit 1 + stderr 含 taxonomy |
| 3 | `test_run_permission_approved` | stdin="y\n" | permission.response approved |
| 4 | `test_run_permission_denied` | stdin="\n"（默认 N） | permission.response denied |
| 5 | `test_run_connection_failed` | socket 不存在 | exit 1 |
| 6 | `test_run_arg_parse` | `--agent custom --timeout 60` | 参数正确解析 |

Mock 策略：用 `asyncio.start_unix_server` 临时启动假的 daemon socket，返回预设 NDJSON 行。

---

## 7. README 更新

```bash
# 终端快速任务
daedalus run "列出当前目录文件"

# 指定 agent
daedalus run "修复 login bug" --agent daedalus-desktop
```

---

## 8. 不做

| 约束 | 状态 |
|------|:---:|
| 不新增 IPC 协议 | ✅ |
| 不修改 daedalusd | ✅ |
| 不修改 client.py API | ✅ |
| false provider / mock API key | ✅ |
