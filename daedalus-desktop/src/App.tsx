import { useEffect, useMemo, useState } from "react";
import { AppShell } from "./components/AppShell";
import { initialTabs, workspaces as initialWorkspaces } from "./data/mockData";
import { getApi } from "./services/daedalusApi";
import type { ApprovalRequest, RightToolKind, TimelineEvent, ViewKind, WorkTab, Workspace } from "./types";
import { createId, nowLabel } from "./utils/time";

const starterMap: Record<string, { kind: ViewKind; title: string; type: string }> = {
  chat: { kind: "new-chat", title: "新对话", type: "对话" },
  meeting: { kind: "new-meeting", title: "会议室", type: "会议" },
  task: { kind: "new-task", title: "新建任务", type: "任务" },
  search: { kind: "search", title: "搜索", type: "搜索" },
  skills: { kind: "skills", title: "插件与技能", type: "插件" },
  automation: { kind: "automation", title: "自动化", type: "自动化" },
  settings: { kind: "settings", title: "设置", type: "设置" }
};

function App() {
  const [workspaces, setWorkspaces] = useState<Workspace[]>(initialWorkspaces);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState("daedalus");

  const [tabs, setTabs] = useState<WorkTab[]>(initialTabs);
  const [openTabIds, setOpenTabIds] = useState<string[]>(["meeting-browser", "task-demo", "chat-permission"]);
  const [activeTabId, setActiveTabId] = useState("chat-permission");

  const [leftCollapsed, setLeftCollapsed] = useState(false);
  const [rightCollapsed, setRightCollapsed] = useState(false);

  const [rightToolTabs, setRightToolTabs] = useState<RightToolKind[]>([]);
  const [activeRightTool, setActiveRightTool] = useState<RightToolKind | null>(null);

  // P5+.1: daemon online status.
  const [daemonOnline, setDaemonOnline] = useState(false);

  useEffect(() => {
    const api = getApi();
    let stop = false;
    const check = async () => {
      const health = await api.getHealth();
      if (!stop) setDaemonOnline(health.status === "ok");
    };
    check();
    const interval = setInterval(check, 5000);
    return () => { stop = true; clearInterval(interval); };
  }, []);

  const activeTab = useMemo(() => tabs.find((tab) => tab.id === activeTabId) ?? tabs[0], [activeTabId, tabs]);
  const openTabs = useMemo(() => openTabIds.map((id) => tabs.find((tab) => tab.id === id)).filter(Boolean) as WorkTab[], [openTabIds, tabs]);

  function ensureOpen(tabId: string) {
    setOpenTabIds((ids) => (ids.includes(tabId) ? ids : [...ids, tabId]));
  }

  function openStarter(kind: keyof typeof starterMap) {
    const config = starterMap[kind];
    const existing = tabs.find((tab) => tab.kind === config.kind);

    if (existing) {
      ensureOpen(existing.id);
      setActiveTabId(existing.id);
      return;
    }

    const tab: WorkTab = {
      id: createId(config.kind),
      kind: config.kind,
      type: config.type,
      title: config.title,
      status: "入口",
      statusType: "normal",
      workspace: "Daedalus",
      meta: [],
      goal: "",
      messages: [],
      approval: null
    };

    setTabs((items) => [...items, tab]);
    setOpenTabIds((ids) => [...ids, tab.id]);
    setActiveTabId(tab.id);
  }

  function openWorkspace(workspace: Workspace) {
    setActiveWorkspaceId(workspace.id);
    const id = `project-${workspace.id}`;
    const existing = tabs.find((tab) => tab.id === id);

    if (existing) {
      ensureOpen(id);
      setActiveTabId(id);
      return;
    }

    const tab: WorkTab = {
      id,
      kind: "project",
      type: "项目",
      title: workspace.title,
      status: "工作区",
      statusType: "normal",
      workspace: workspace.title,
      meta: ["项目首页"],
      goal: `${workspace.title} 工作区`,
      messages: [],
      approval: null
    };

    setTabs((items) => [...items, tab]);
    setOpenTabIds((ids) => [...ids, id]);
    setActiveTabId(id);
  }

  function closeTab(tabId: string) {
    setOpenTabIds((ids) => {
      const next = ids.filter((id) => id !== tabId);
      if (activeTabId === tabId) {
        setActiveTabId(next[next.length - 1] ?? "chat-permission");
      }
      return next.length ? next : ["chat-permission"];
    });
  }

  function createGeneratedTab(kind: "chat" | "meeting" | "task", prompt: string) {
    const title = prompt.slice(0, 24) || (kind === "meeting" ? "新会议" : kind === "task" ? "新任务" : "新对话");
    const baseEvent: TimelineEvent = {
      id: createId("event"),
      agent: "用户",
      tag: "目标",
      time: nowLabel(),
      body: prompt
    };

    const tab: WorkTab = {
      id: createId(kind),
      kind,
      type: kind === "meeting" ? "会议" : kind === "task" ? "任务" : "对话",
      title,
      status: kind === "task" ? "运行中" : kind === "meeting" ? "运行中" : "对话",
      statusType: kind === "task" || kind === "meeting" ? "running" : "normal",
      workspace: "Daedalus",
      meta: kind === "meeting" ? ["Agent 会议", "4 个 Agent"] : kind === "task" ? ["执行者：自动选择", "Gate：按风险触发"] : ["Agent：自动选择"],
      goal: prompt,
      messages: [
        baseEvent,
        {
          id: createId("event"),
          agent: kind === "meeting" ? "架构师" : kind === "task" ? "Daedalus" : "Daedalus",
          tag: kind === "meeting" ? "开场" : "回复",
          time: nowLabel(),
          body: kind === "meeting"
            ? "我会先定义边界，再让 Codex 判断实现路径，最后由验证员给出 Gate 建议。"
            : kind === "task"
              ? "已根据任务目标选择 Codex 执行，Verifier 负责验收。"
              : "收到。我会先按当前工作区上下文回答，也可以随时切换到会议或任务模式。"
        }
      ],
      approval: kind === "task"
        ? {
            id: createId("gate"),
            title: "Gate 暂未触发",
            desc: "当前任务还没有高风险写入请求。"
          }
        : null
    };

    setTabs((items) => [...items, tab]);
    setOpenTabIds((ids) => [...ids, tab.id]);
    setActiveTabId(tab.id);
  }

  function sendMessage(tabId: string, body: string) {
    setTabs((items) =>
      items.map((tab) => {
        if (tab.id !== tabId) return tab;
        const userEvent: TimelineEvent = {
          id: createId("event"),
          agent: "用户",
          tag: "补充",
          time: nowLabel(),
          body
        };
        const replyEvent: TimelineEvent = {
          id: createId("event"),
          agent: tab.kind === "meeting" ? "架构师" : tab.kind === "task" ? "Codex" : "Daedalus",
          tag: "回复",
          time: nowLabel(),
          body: tab.kind === "meeting"
            ? "我会把这个补充纳入会议结论，并让验证员同步检查风险。"
            : "已记录。下一步我会基于当前上下文继续推进。"
        };
        return { ...tab, messages: [...tab.messages, userEvent, replyEvent] };
      })
    );
  }

  function decideApproval(tabId: string, approval: ApprovalRequest, decision: "approve" | "reject" | "changes") {
    const body = decision === "approve"
      ? "已批准。Gate 放行，任务可以继续执行。"
      : decision === "reject"
        ? "已拒绝。任务停止，不再写入文件。"
        : "已要求修改。Codex 需要调整范围后重新提交。";

    setTabs((items) =>
      items.map((tab) => {
        if (tab.id !== tabId) return tab;
        return {
          ...tab,
          approval: null,
          status: decision === "approve" ? "运行中" : decision === "reject" ? "已拒绝" : "要求修改",
          statusType: decision === "approve" ? "running" : decision === "reject" ? "rejected" : "waiting",
          messages: [
            ...tab.messages,
            {
              id: createId("event"),
              agent: "用户",
              tag: "决策",
              time: nowLabel(),
              body,
              decision: decision === "approve" ? "进入应用改动和验证阶段。" : "Gate 保持关闭。"
            }
          ]
        };
      })
    );

    void approval;
  }

  function addWorkspace() {
    const workspace: Workspace = {
      id: createId("workspace"),
      title: `新工作区 ${workspaces.length + 1}`,
      icon: "▱"
    };
    setWorkspaces((items) => [...items, workspace]);
    openWorkspace(workspace);
  }

  function openRightTool(tool: RightToolKind) {
    setRightCollapsed(false);
    setRightToolTabs((items) => (items.includes(tool) ? items : [...items, tool]));
    setActiveRightTool(tool);
  }

  function closeRightTool(tool: RightToolKind) {
    setRightToolTabs((items) => {
      const next = items.filter((item) => item !== tool);
      if (activeRightTool === tool) {
        setActiveRightTool(next[next.length - 1] ?? null);
      }
      return next;
    });
  }

  return (
    <AppShell
      workspaces={workspaces}
      activeWorkspaceId={activeWorkspaceId}
      tabs={tabs}
      openTabs={openTabs}
      activeTab={activeTab}
      layout={{ leftCollapsed, rightCollapsed }}
      rightToolTabs={rightToolTabs}
      activeRightTool={activeRightTool}
      onOpenStarter={openStarter}
      onOpenWorkspace={openWorkspace}
      onAddWorkspace={addWorkspace}
      onActivateTab={setActiveTabId}
      onCloseTab={closeTab}
      onToggleLeft={() => setLeftCollapsed((value) => !value)}
      onToggleRight={() => setRightCollapsed((value) => !value)}
      onSetLeftCollapsed={setLeftCollapsed}
      onSetRightCollapsed={setRightCollapsed}
      onCreateGeneratedTab={createGeneratedTab}
      onSendMessage={sendMessage}
      onDecideApproval={decideApproval}
      onOpenRightTool={openRightTool}
      onActivateRightTool={setActiveRightTool}
      onCloseRightTool={closeRightTool}
      daemonOnline={daemonOnline}
    />
  );
}

export default App;
