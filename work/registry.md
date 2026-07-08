# Agent 注册表

- 当前模式：standard（总控直管协作）
- 协作流转：用户 -> 总控 -> developer -> reviewer
- 当前项目：`/Users/gu/Daedalus`
- 当前后端分支：`codex/narrative-backend`

| 角色 | Codex Thread ID | 状态 | 备注 |
|------|-----------------|------|------|
| 总控 | 当前主会话 | 在线 | 只负责协调、分派、汇总；不写业务代码 |
| developer | 019f3f9c-2d57-72b1-93db-e832db3de66d | 在线 | Daedalus 后端研发，本轮实现 `work/task.md` |
| reviewer | 019f3f9c-382a-7833-b71e-e47463b05b8b | 在线 | Daedalus 后端验收，等 developer 完成后只读验收 |

## 旧线程记录

| 角色 | Codex Thread ID | 状态 | 备注 |
|------|-----------------|------|------|
| developer（旧） | 019f1b89-5603-7921-876e-c5821e4abf4c | 已退役 | DevSpace / 旧 HTTP 任务阶段线程 |
| reviewer（旧） | 019f1b89-674b-7663-8fbb-5d1f922cf142 | 已退役 | DevSpace / 旧 HTTP 任务阶段线程 |
| manager（旧） | 019f1174-e5fb-76c2-91ff-ac79f554315d | 已退役 | 为避免双层协调，已停用 |

## 通信协议

- 发消息：`codex_app__send_message_to_thread({ threadId, prompt })`
- 读回复：`codex_app__read_thread({ threadId })`
- developer / reviewer 完成后必须追加同一结论到 `work/callbacks.md`
- 若无法主动跨会话回传总控，必须在 `work/callbacks.md` 明确写“无法主动跨会话回传”
- 总控以 `work/callbacks.md` 作为兜底完成信号，不只依赖轮询线程
