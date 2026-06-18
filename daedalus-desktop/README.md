# Daedalus React UI Skeleton

这是从静态 Demo 工程化出来的 React + TypeScript UI 骨架，适合并入 Electron Renderer。

## 已包含

- 左侧：启动入口、工作区、历史会话、设置
- 中间：Chrome 式可关闭工作 Tab
- 右侧：工具 Tab，支持审查 / 证据 / 终端 / 浏览器 / 文件 / 侧边聊天
- 左侧可折叠，右侧可隐藏
- 对话 / 会议 / 任务 / 项目 / 空态视图
- Gate 审批条
- Mock data 与类型定义独立

## 运行

```bash
npm install
npm run dev
```

## 并入 Electron 的建议

先把 `src/` 作为 renderer UI 合并。  
后续把 `src/services/daedalusApi.ts` 中的 mock 实现替换为 `window.daedalusAPI` / IPC 调用。
