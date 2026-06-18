import type { ViewKind } from "../types";

export function tabIcon(kind: ViewKind): string {
  if (kind === "meeting" || kind === "new-meeting") return "👥";
  if (kind === "task" || kind === "new-task") return "✓";
  if (kind === "project") return "▱";
  if (kind === "settings") return "⚙";
  if (kind === "automation") return "◷";
  if (kind === "skills") return "◇";
  if (kind === "search") return "⌕";
  return "💬";
}

export function viewLabel(kind: ViewKind): string {
  if (kind === "meeting" || kind === "new-meeting") return "会议";
  if (kind === "task" || kind === "new-task") return "任务";
  if (kind === "project") return "项目";
  if (kind === "settings") return "设置";
  if (kind === "automation") return "自动化";
  if (kind === "skills") return "插件";
  if (kind === "search") return "搜索";
  return "对话";
}
