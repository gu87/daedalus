export type Workspace = {
  id: string;
  title: string;
  icon: string;
};

export type ViewKind =
  | "chat"
  | "meeting"
  | "task"
  | "project"
  | "new-chat"
  | "new-meeting"
  | "new-task"
  | "search"
  | "skills"
  | "automation"
  | "settings";

export type StatusType = "normal" | "running" | "waiting" | "done" | "rejected";

export type TimelineEvent = {
  id: string;
  agent: string;
  tag: string;
  time: string;
  body: string;
  decision?: string;
};

export type ApprovalRequest = {
  id: string;
  title: string;
  desc: string;
};

export type WorkTab = {
  id: string;
  kind: ViewKind;
  type: string;
  title: string;
  status: string;
  statusType: StatusType;
  workspace: string;
  meta: string[];
  goal: string;
  messages: TimelineEvent[];
  approval?: ApprovalRequest | null;
};

export type RightToolKind =
  | "review"
  | "evidence"
  | "terminal"
  | "browser"
  | "files"
  | "sidechat";

export type RightToolConfig = {
  id: RightToolKind;
  title: string;
  icon: string;
  shortcut?: string;
  desc: string;
};

export type LayoutState = {
  leftCollapsed: boolean;
  rightCollapsed: boolean;
};
