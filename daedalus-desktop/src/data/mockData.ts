import type { RightToolConfig, WorkTab, Workspace } from "../types";

export const workspaces: Workspace[] = [
  { id: "daedalus", title: "Daedalus", icon: "▱" },
  { id: "hermes", title: "Hermes", icon: "▱" },
  { id: "marketing", title: "营销工具", icon: "▱" },
  { id: "ai-tools", title: "AI 工具", icon: "▱" },
  { id: "monorepo", title: "Monorepo", icon: "▱" },
  { id: "obsidian", title: "Obsidian 知识库", icon: "▱" }
];

export const initialTabs: WorkTab[] = [
  {
    id: "meeting-browser",
    type: "会议",
    kind: "meeting",
    title: "Browser Workspace API 评审会",
    status: "运行中",
    statusType: "running",
    workspace: "Daedalus",
    meta: ["4 个 Agent", "阶段：方案评审", "创建于刚刚"],
    goal: "评审 Browser Workspace API 设计，确认第一版只读上下文边界。",
    messages: [
      {
        id: "m1",
        agent: "架构师",
        tag: "方案建议",
        time: "09:41",
        body: "Browser 第一版应该只做读取，不做操作。先暴露 URL、标题、选中文本、截图和页面摘要。"
      },
      {
        id: "m2",
        agent: "Codex",
        tag: "实现判断",
        time: "09:42",
        body: "实现上可以先做 getPageContext()，不暴露 click / type / scroll，避免过早扩大权限边界。"
      },
      {
        id: "m3",
        agent: "验证员",
        tag: "Gate 检查",
        time: "09:44",
        body: "preload API 变更属于中等风险。即便是只读，也需要用户批准后才能合并。"
      },
      {
        id: "m4",
        agent: "会议",
        tag: "决策",
        time: "09:46",
        body: "结论：Browser Workspace MVP 只读；后续再引入可审批的单步操作。",
        decision: "Codex 负责 Demo；验证员负责风险清单；文档员记录 API contract。"
      }
    ],
    approval: null
  },
  {
    id: "task-demo",
    type: "任务",
    kind: "task",
    title: "Desktop UI Demo 任务",
    status: "等待审批",
    statusType: "waiting",
    workspace: "Daedalus",
    meta: ["执行者：Codex", "风险：中等", "目标路径：desktop-demo/"],
    goal: "生成一个可交互的 Daedalus Desktop Demo，包含左中右三栏、可关闭 Tab、右侧工具抽屉和 Gate 审批。",
    messages: [
      {
        id: "t1",
        agent: "用户",
        tag: "目标",
        time: "10:20",
        body: "按照刚才确定的左中右结构，做一个可以交互的 Demo。"
      },
      {
        id: "t2",
        agent: "Codex",
        tag: "执行",
        time: "10:21",
        body: "我会生成 index.html / styles.css / app.js 三个静态文件，并模拟会话切换、审批、右侧工具和左右侧折叠。"
      },
      {
        id: "t3",
        agent: "验证员",
        tag: "验证",
        time: "10:24",
        body: "检查范围仅限 demo 文件，没有修改 daemon 或 protocol，建议进入人工审批。"
      },
      {
        id: "t4",
        agent: "Daedalus",
        tag: "Gate",
        time: "10:25",
        body: "Gate 需要你的决策。Codex 想在 desktop-demo/ 下写入文件。"
      }
    ],
    approval: {
      id: "gate-demo",
      title: "Gate 需要你的决策",
      desc: "Codex 申请写入 demo 文件。证据显示未修改 daemon / protocol 文件。"
    }
  },
  {
    id: "chat-permission",
    type: "对话",
    kind: "chat",
    title: "权限中继设计",
    status: "对话",
    statusType: "normal",
    workspace: "Daedalus",
    meta: ["Agent：Architect", "模型：Claude 3.5 Sonnet", "工作区：Daedalus"],
    goal: "讨论 Desktop、Daedalus 和 Browser 之间的权限申请流程。",
    messages: [
      {
        id: "c1",
        agent: "用户",
        tag: "问题",
        time: "10:32",
        body: "我想让 Daedalus 作为中继，Desktop 向 Browser 申请权限的流程，应该怎么设计比较合理？"
      },
      {
        id: "c2",
        agent: "Architect",
        tag: "回答",
        time: "10:32",
        body: "我建议采用三段式设计：Desktop 发起权限申请，Daedalus 展示给用户审批，Browser 只接收短期、最小权限的授权令牌。关键点是所有敏感操作必须人工确认。"
      }
    ],
    approval: null
  }
];

export const rightTools: RightToolConfig[] = [
  { id: "review", title: "审查", icon: "☑", shortcut: "⌘G", desc: "查看变更、审批和任务状态" },
  { id: "evidence", title: "证据", icon: "♧", desc: "查看验收依据和检查结果" },
  { id: "terminal", title: "终端", icon: "›_", shortcut: "⌘T", desc: "查看命令执行结果和日志" },
  { id: "browser", title: "浏览器", icon: "◎", shortcut: "⌘B", desc: "打开 Browser Workspace" },
  { id: "files", title: "文件", icon: "▱", shortcut: "⌘P", desc: "查看和管理项目文件" },
  { id: "sidechat", title: "侧边聊天", icon: "＋", shortcut: "⌥⌘S", desc: "与其他 Agent 临时沟通" }
];
