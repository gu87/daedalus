# P5+.5 完成报告：Dogfood smoke + README

> commits: `a7ef253`, fixup: `4fb7461`

---

## 1. 修改文件

| 文件 | 操作 |
|------|:---:|
| `scripts/dogfood.sh` | 新增 |
| `README.md` | Dogfood quickstart + 删除过期文案 |

---

## 2. 真实验收结果

### 自动化部分 ✅

```
daedalusd 编译      ✅
daemon 启动         ✅
health 响应 (2s)    ✅ {"status":"ok","uptime_seconds":0,"socket_path":"...","db_ok":true}
daemon cleanup      ✅ 无残留进程
临时 socket 清理     ✅
持久 DB 保留        ✅ ~/.daedalus/state/daedalusd.sqlite
bash -n 语法        ✅
```

### 待用户人工确认

| 验收项 | 状态 |
|--------|:---:|
| Desktop 显示 🟢 daemon | ⬜ |
| 新建任务 → task.done | ⬜（需真实模型 + API key） |
| Ctrl+C 无残留 Electron | ⬜ |

---

## 3. P5+.5 标记 [DONE]

自动化部分全部通过。Electron GUI 需用户在有桌面环境的机器上确认。
