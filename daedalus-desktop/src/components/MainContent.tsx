import { ChatView } from "../views/ChatView";
import { EmptyView } from "../views/EmptyView";
import { ProjectView } from "../views/ProjectView";
import type { ApprovalRequest, WorkTab } from "../types";

type Props = {
  tab: WorkTab;
  onCreateGeneratedTab: (kind: "chat" | "meeting" | "task", prompt: string) => void;
  onSendMessage: (tabId: string, body: string) => void;
  onDecideApproval: (tabId: string, approval: ApprovalRequest, decision: "approve" | "reject" | "changes") => void;
};

export function MainContent(props: Props) {
  if (props.tab.kind === "project") {
    return <ProjectView tab={props.tab} onCreateTask={() => props.onCreateGeneratedTab("task", "在当前工作区中新建任务")} />;
  }

  if (["new-chat", "new-meeting", "new-task", "search", "skills", "automation", "settings"].includes(props.tab.kind)) {
    return <EmptyView tab={props.tab} onCreateGeneratedTab={props.onCreateGeneratedTab} />;
  }

  return (
    <ChatView
      tab={props.tab}
      onSendMessage={props.onSendMessage}
      onDecideApproval={props.onDecideApproval}
    />
  );
}
