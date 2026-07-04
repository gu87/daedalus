# Agent 注册表

- 当前模式：standard（总控直管协作）
- 协作流转：用户 → 总控 → developer → reviewer

| 角色 | Codex Thread ID | 状态 | 备注 |
|------|-----------------|------|------|
| 总控 | 019f1b89-44f8-72e3-8abb-e5003f38abba | 在线 | 当前主会话 fork，负责协调 |
| developer | 019f1b89-5603-7921-876e-c5821e4abf4c | 在线 | developer fork，接收总控分派 |
| reviewer | 019f1b89-674b-7663-8fbb-5d1f922cf142 | 在线 | reviewer fork，验证并回报总控 |
| manager（已退役） | 019f1174-e5fb-76c2-91ff-ac79f554315d | 已退役 | 为避免双层协调，已停用 |

## 通信协议

- 发消息：`codex_app__send_message_to_thread({ threadId, prompt })`
- 读回复：`codex_app__read_thread({ threadId })`
- standard 模式默认回报给 `总控`
- 只使用可见 Codex thread 做跨会话协作
