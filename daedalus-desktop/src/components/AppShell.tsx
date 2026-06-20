import { CenterTabs } from "./CenterTabs";
import { LeftRail } from "./LeftRail";
import { LeftSidebar } from "./LeftSidebar";
import { MainContent } from "./MainContent";
import { RightToolDrawer } from "./RightToolDrawer";
import type { ApprovalRequest, LayoutState, RightToolKind, WorkTab, Workspace } from "../types";

type StarterKind = "chat" | "meeting" | "task" | "search" | "skills" | "automation" | "settings";

type Props = {
  workspaces: Workspace[];
  activeWorkspaceId: string;
  tabs: WorkTab[];
  openTabs: WorkTab[];
  activeTab: WorkTab;
  layout: LayoutState;
  rightToolTabs: RightToolKind[];
  activeRightTool: RightToolKind | null;
  onOpenStarter: (kind: StarterKind) => void;
  onOpenWorkspace: (workspace: Workspace) => void;
  onAddWorkspace: () => void;
  onActivateTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  onToggleLeft: () => void;
  onToggleRight: () => void;
  onSetLeftCollapsed: (value: boolean) => void;
  onSetRightCollapsed: (value: boolean) => void;
  onCreateGeneratedTab: (kind: "chat" | "meeting" | "task", prompt: string) => void;
  onSendMessage: (tabId: string, body: string) => void;
  onDecideApproval: (tabId: string, approval: ApprovalRequest, decision: "approve" | "reject" | "changes") => void;
  onOpenRightTool: (tool: RightToolKind) => void;
  onActivateRightTool: (tool: RightToolKind | null) => void;
  onCloseRightTool: (tool: RightToolKind) => void;
  daemonOnline: boolean;
  historyTasks: WorkTab[];
  onOpenHistoryTask: (runId: string) => void;
};

export function AppShell(props: Props) {
  const classes = [
    "app",
    props.layout.leftCollapsed ? "left-collapsed" : "",
    props.layout.rightCollapsed ? "right-collapsed" : ""
  ].filter(Boolean).join(" ");

  return (
    <div className={classes}>
      <div style={{
        position: "fixed", top: 0, right: 12, zIndex: 999,
        fontSize: 12, padding: "2px 8px", borderRadius: 4,
        background: props.daemonOnline ? "#e6ffe6" : "#ffe6e6",
        color: props.daemonOnline ? "#1a7a1a" : "#a11",
      }}>
        {props.daemonOnline ? "🟢 daemon" : "🔴 daemon"}
      </div>
      <LeftSidebar
        workspaces={props.workspaces}
        activeWorkspaceId={props.activeWorkspaceId}
        tabs={props.tabs}
        activeTabId={props.activeTab.id}
        onOpenStarter={props.onOpenStarter}
        onOpenWorkspace={props.onOpenWorkspace}
        onAddWorkspace={props.onAddWorkspace}
        onActivateTab={props.onActivateTab}
        onCollapse={() => props.onSetLeftCollapsed(true)}
        historyTasks={props.historyTasks}
        onOpenHistoryTask={props.onOpenHistoryTask}
      />

      <LeftRail
        onExpand={() => props.onSetLeftCollapsed(false)}
        onOpenStarter={props.onOpenStarter}
      />

      <main className="center">
        <CenterTabs
          tabs={props.openTabs}
          activeTabId={props.activeTab.id}
          onActivateTab={props.onActivateTab}
          onCloseTab={props.onCloseTab}
          onNewTab={() => props.onOpenStarter("chat")}
          onToggleLeft={props.onToggleLeft}
          onToggleRight={props.onToggleRight}
        />

        <MainContent
          tab={props.activeTab}
          onCreateGeneratedTab={props.onCreateGeneratedTab}
          onSendMessage={props.onSendMessage}
          onDecideApproval={props.onDecideApproval}
        />
      </main>

      <RightToolDrawer
        activeTab={props.activeTab}
        rightToolTabs={props.rightToolTabs}
        activeRightTool={props.activeRightTool}
        onClose={() => props.onSetRightCollapsed(true)}
        onOpenTool={props.onOpenRightTool}
        onActivateTool={props.onActivateRightTool}
        onCloseTool={props.onCloseRightTool}
      />
    </div>
  );
}
