# P5+.5 完成报告：Dogfood smoke + README

> commits: `a7ef253`, fixups: `4fb7461`, `2d3144f`, `b1bdcf1`, `0001697`

---

## 1. 修改文件

| 文件 | 操作 |
|------|:---:|
| `scripts/dogfood.sh` | 新增 — `NODE_ENV=production npm run electron:start` |
| `README.md` | Dogfood quickstart + 删除过期文案 |
| `daedalus-desktop/electron/main.cjs` | `sandbox: false`（preload 需 Node `net`） |
| `daedalus-desktop/electron/preload.cjs` | .js → .cjs（CommonJS） |

---

## 2. 返修记录

| # | 问题 | 修复 |
|---|------|------|
| 1 | STATE_DIR 临时 | 改为 `~/.daedalus/state` 持久 |
| 2 | ESM/CommonJS 冲突 | main.js/preload.js → .cjs |
| 3 | Vite 5173 冲突 | 改用 `electron:start`（`dist/` 直载） |
| 4 | CORS 误加 | 撤销 tower-http（Electron Node fetch 不需要） |
| 5 | sandbox preload 无法 require("net") | `sandbox: false` |

---

## 3. 验证

```
cargo clippy --workspace -- -D warnings  ✅
npm run build                            ✅
bash scripts/dogfood.sh (daemon health)  ✅
```

待用户实机确认：
- ⬜ 右上角 🟢
- ⬜ 新建任务 → task.done

非阻塞审计：Electron CSP warning → Release Candidate 处理

---

## 4. 不做

| 约束 | 状态 |
|------|:---:|
| CORS | ✅ 已撤销 |
| Vite dev server | ✅ 不依赖 |
| daemon CLI flag | ✅ env var only |
