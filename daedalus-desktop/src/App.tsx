import { useEffect, useMemo, useRef, useState } from "react";
import { AppShell } from "./components/AppShell";
import { initialTabs, workspaces as initialWorkspaces } from "./data/mockData";
import { getApi, type TaskSummary } from "./services/daedalusApi";
import type { ApprovalRequest, RightToolKind, StatusType, TimelineEvent, ViewKind, WorkTab, Workspace } from "./types";
import { createId, nowLabel } from "./utils/time";

function historyTab(t: TaskSummary): WorkTab {
  const st = ({ queued: "排队中", running: "运行中", done: "完成", error: "错误", cancelled: "已取消", orphaned: "已孤立" } as Record<string,string>)[t.status] || t.status;
  const stt = ({ queued: "waiting", running: "running", done: "done", error: "rejected", cancelled: "rejected", orphaned: "rejected" } as Record<string,string>)[t.status] as StatusType || "normal" as StatusType;
  return {
    id: `history-${t.run_id}`, kind: "task", type: "历史", readonly: true,
    title: t.task_id.slice(0, 24), status: st, statusType: stt,
    workspace: "Daedalus",
    meta: [`任务：${t.task_id}`, t.error_taxonomy ? `错误：${t.error_taxonomy}` : ""].filter(Boolean),
    goal: "", messages: [], approval: null,
  };
}

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

  // P5+.3: pending permission replies, keyed by permission_id.
  const permissionReplies = useRef<Map<string, { approve(): void; reject(): void; requestChanges(): void }>>(new Map());

  // P5+.4: history tasks from daemon.
  const [historyTasks, setHistoryTasks] = useState<WorkTab[]>([]);
  useEffect(() => {
    getApi().listSessions().then((sessions: TaskSummary[]) => {
      setHistoryTasks(sessions.map((t) => historyTab(t)));
    }).catch(() => {});
  }, []);

  function openHistoryTask(runId: string) {
    const existing = tabs.find((t) => t.id === `history-${runId}`);
    if (existing) { ensureOpen(existing.id); setActiveTabId(existing.id); return; }
    // Open detail tab immediately, then load detail async.
    const tab: WorkTab = {
      id: `history-${runId}`, kind: "task", type: "历史", readonly: true,
      title: runId.slice(0, 24), status: "加载中", statusType: "waiting",
      workspace: "Daedalus", meta: [], goal: "",
      messages: [], approval: null,
    };
    setTabs((items) => [...items, tab]);
    setOpenTabIds((ids) => [...ids, tab.id]);
    setActiveTabId(tab.id);
    getApi().getTaskDetail(runId).then((detail) => {
      if (!detail) {
        setTabs((items) => items.map((t) =>
          t.id !== tab.id ? t : { ...t, status: "加载失败", statusType: "rejected", messages: [{ id: createId("event"), agent: "系统", tag: "错误", time: nowLabel(), body: "无法加载任务详情" }] }
        ));
        return;
      }
      setTabs((items) => items.map((t) => {
        if (t.id !== tab.id) return t;
        const statusLabel = ({ queued: "排队中", running: "运行中", done: "完成", error: "错误", cancelled: "已取消", orphaned: "已孤立" } as Record<string,string>)[detail.status] || detail.status;
        const statusType = ({ queued: "waiting", running: "running", done: "done", error: "rejected", cancelled: "rejected", orphaned: "rejected" } as Record<string,string>)[detail.status] as StatusType || "normal" as StatusType;
        const lines = [
          `Run ID: ${detail.run_id}`,
          `Agent: ${detail.agent_id}`,
          `Task: ${detail.task_id}`,
          `Status: ${detail.status}`,
          detail.error_taxonomy ? `Error: ${detail.error_taxonomy}` : "",
          `Spawned: ${new Date(detail.spawned_at * 1000).toLocaleString()}`,
          detail.completed_at ? `Completed: ${new Date(detail.completed_at * 1000).toLocaleString()}` : "",
          detail.heartbeat_at ? `Heartbeat: ${new Date(detail.heartbeat_at * 1000).toLocaleString()}` : "",
          detail.parent_run_id ? `Parent: ${detail.parent_run_id}` : "",
          `Depth: ${detail.spawn_depth}`,
        ].filter(Boolean).join("\n");
        return { ...t, status: statusLabel, statusType, messages: [{ id: createId("event"), agent: "系统", tag: "详情", time: nowLabel(), body: lines }] };
      }));
    }).catch(() => {
      setTabs((items) => items.map((t) =>
        t.id !== tab.id ? t : { ...t, status: "加载失败", statusType: "rejected" }
      ));
    });
  }

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

    // P5+.2: real task dispatch via UDS.
    if (kind === "task") {
      const tabId = createId(kind);
      const baseEvent: TimelineEvent = {
        id: createId("event"),
        agent: "用户",
        tag: "目标",
        time: nowLabel(),
        body: prompt,
      };
      const tab: WorkTab = {
        id: tabId, kind, type: "任务", title,
        status: "运行中", statusType: "running", workspace: "Daedalus",
        meta: ["执行者：自动选择", "Gate：按风险触发"],
        goal: prompt,
        messages: [baseEvent, {
          id: createId("event"), agent: "Daedalus", tag: "回复", time: nowLabel(),
          body: "正在连接 daemon...",
        }],
        approval: null,
      };
      setTabs((items) => [...items, tab]);
      setOpenTabIds((ids) => [...ids, tabId]);
      setActiveTabId(tabId);

      const api = getApi();
      api.dispatchTask(prompt, {
        onStream(chunk) {
          setTabs((items) => items.map((t) => {
            if (t.id !== tabId) return t;
            const last = t.messages[t.messages.length - 1];
            if (last && last.tag === "stream") {
              return { ...t, messages: [...t.messages.slice(0, -1), { ...last, body: last.body + chunk }] };
            }
            return { ...t, messages: [...t.messages, { id: createId("event"), agent: "Daedalus", tag: "stream", time: nowLabel(), body: chunk }] };
          }));
        },
        onDone(_outbox) {
          setTabs((items) => items.map((t) => {
            if (t.id !== tabId) return t;
            if (t.approval?.id) permissionReplies.current.delete(t.approval.id);
            return { ...t, status: "完成", statusType: "done", approval: null };
          }));
        },
        onError(taxonomy, detail) {
          setTabs((items) => items.map((t) => {
            if (t.id !== tabId) return t;
            if (t.approval?.id) permissionReplies.current.delete(t.approval.id);
            return {
              ...t, status: taxonomy || "错误", statusType: "rejected", approval: null,
              messages: [...t.messages, { id: createId("event"), agent: "Daedalus", tag: "错误", time: nowLabel(), body: detail }],
            };
          }));
        },
        onPermissionRequest(perm: any, reply) {
          // P5+.3: store reply and show approval bar.
          permissionReplies.current.set(perm.permission_id, reply);
          setTabs((items) => items.map((t) =>
            t.id !== tabId ? t : {
              ...t,
              approval: {
                id: perm.permission_id,
                title: `Gate 审批: ${perm.tool || "tool"}`,
                desc: JSON.stringify(perm.args || {}),
              },
            }
          ));
        },
      }).catch(() => {});
      return;
    }

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
      type: kind === "meeting" ? "会议" : "对话",
      title,
      status: kind === "meeting" ? "运行中" : "对话",
      statusType: kind === "meeting" ? "running" : "normal",
      workspace: "Daedalus",
      meta: kind === "meeting" ? ["Agent 会议", "4 个 Agent"] : ["Agent：自动选择"],
      goal: prompt,
      messages: [
        baseEvent,
        {
          id: createId("event"),
          agent: kind === "meeting" ? "架构师" : "Daedalus",
          tag: kind === "meeting" ? "开场" : "回复",
          time: nowLabel(),
          body: kind === "meeting"
            ? "我会先定义边界，再让 Codex 判断实现路径，最后由验证员给出 Gate 建议。"
            : "收到。我会先按当前工作区上下文回答，也可以随时切换到会议或任务模式。"
        }
      ],
      approval: null
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
    // P5+.3: send permission.response via saved one-shot reply.
    const reply = permissionReplies.current.get(approval.id);
    if (reply) {
      permissionReplies.current.delete(approval.id);
      if (decision === "approve") reply.approve();
      else if (decision === "reject") reply.reject();
      else reply.requestChanges();
    }

    const body = decision === "approve"
      ? "已批准。Gate 放行，任务可以继续执行。"
      : decision === "reject"
        ? "已拒绝。任务停止，不再写入文件。"
        : "已要求修改（本轮按拒绝处理）。自动修订留到后续版本。";

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
      historyTasks={historyTasks}
      onOpenHistoryTask={openHistoryTask}
    />
  );
}

export default App;
