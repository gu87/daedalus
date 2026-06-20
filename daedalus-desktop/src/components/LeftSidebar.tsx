import type { WorkTab, Workspace } from "../types";
import { tabIcon } from "../utils/view";

type StarterKind = "chat" | "meeting" | "task" | "search" | "skills" | "automation" | "settings";

type Props = {
  workspaces: Workspace[];
  activeWorkspaceId: string;
  tabs: WorkTab[];
  activeTabId: string;
  onOpenStarter: (kind: StarterKind) => void;
  onOpenWorkspace: (workspace: Workspace) => void;
  onAddWorkspace: () => void;
  onActivateTab: (tabId: string) => void;
  onCollapse: () => void;
  historyTasks: WorkTab[];
  onOpenHistoryTask: (runId: string) => void;
};

const primaryActions: Array<{ kind: StarterKind; icon: string; label: string }> = [
  { kind: "chat", icon: "✎", label: "新对话" },
  { kind: "meeting", icon: "▣", label: "会议室" },
  { kind: "task", icon: "✓", label: "新建任务" },
  { kind: "search", icon: "⌕", label: "搜索" },
  { kind: "skills", icon: "◇", label: "插件与技能" },
  { kind: "automation", icon: "◷", label: "自动化" }
];

export function LeftSidebar(props: Props) {
  const conversations = props.tabs.filter((tab) => ["meeting", "task", "chat"].includes(tab.kind));

  return (
    <aside className="left-sidebar">
      <div className="window-dots" aria-hidden="true">
        <span />
        <span />
        <span />
      </div>

      <div className="brand-block">
        <div className="brand-icon">D</div>
        <div className="brand-copy">
          <strong>Daedalus</strong>
          <small>Agent Workbench</small>
        </div>
        <button className="icon-button collapse-left" title="隐藏左侧栏" onClick={props.onCollapse}>
          ◧
        </button>
      </div>

      <nav className="primary-actions" aria-label="Primary actions">
        {primaryActions.map((action) => (
          <button className="left-action" key={action.kind} onClick={() => props.onOpenStarter(action.kind)}>
            <span className="action-icon">{action.icon}</span>
            <span>{action.label}</span>
          </button>
        ))}
      </nav>

      <section className="workspace-section">
        <div className="section-row">
          <span>工作区</span>
          <button className="mini-button" onClick={props.onAddWorkspace}>＋</button>
        </div>
        <div className="workspace-list">
          {props.workspaces.map((workspace) => (
            <button
              key={workspace.id}
              className={`workspace-item ${workspace.id === props.activeWorkspaceId ? "active" : ""}`}
              onClick={() => props.onOpenWorkspace(workspace)}
            >
              <span className="item-icon">{workspace.icon}</span>
              <span className="item-copy">
                <strong>{workspace.title}</strong>
              </span>
              <span />
            </button>
          ))}
        </div>
      </section>

      <section className="conversation-section">
        <div className="section-row">
          <span>对话</span>
        </div>
        <div className="conversation-list">
          {conversations.map((tab) => (
            <button
              key={tab.id}
              className={`conversation-item ${tab.id === props.activeTabId ? "active" : ""}`}
              onClick={() => props.onActivateTab(tab.id)}
            >
              <span className="item-icon">{tabIcon(tab.kind)}</span>
              <span className="item-copy">
                <strong>{tab.title}</strong>
                <small>{tab.status} · {tab.type}</small>
              </span>
              <span className="item-badge">{tab.type}</span>
            </button>
          ))}
        </div>
      </section>

      <section className="history-section">
        <div className="section-label">历史任务</div>
        <div className="tab-list">
          {props.historyTasks.length === 0 && (
            <div className="history-empty">暂无历史任务</div>
          )}
          {props.historyTasks.map((tab) => (
            <button
              key={tab.id}
              className={`tab-item ${tab.id === props.activeTabId ? "active" : ""}`}
              onClick={() => {
                const runId = tab.id.startsWith("history-") ? tab.id.slice(8) : tab.id;
                props.onOpenHistoryTask(runId);
              }}
            >
              <span className="item-label">{tab.title}</span>
              <span className={`item-badge ${tab.statusType}`}>{tab.status}</span>
            </button>
          ))}
        </div>
      </section>

      <button className="settings-button" onClick={() => props.onOpenStarter("settings")}>
        <span className="action-icon">⚙</span>
        <span>设置</span>
      </button>
    </aside>
  );
}
