type StarterKind = "chat" | "meeting" | "task" | "search" | "skills" | "automation" | "settings";

type Props = {
  onExpand: () => void;
  onOpenStarter: (kind: StarterKind) => void;
};

export function LeftRail(props: Props) {
  return (
    <aside className="rail">
      <button className="rail-icon" title="展开左侧栏" onClick={props.onExpand}>D</button>
      <button className="rail-icon" title="新对话" onClick={() => props.onOpenStarter("chat")}>✎</button>
      <button className="rail-icon" title="会议室" onClick={() => props.onOpenStarter("meeting")}>▣</button>
      <button className="rail-icon" title="新建任务" onClick={() => props.onOpenStarter("task")}>✓</button>
      <button className="rail-icon" title="搜索" onClick={() => props.onOpenStarter("search")}>⌕</button>
      <button className="rail-icon" title="设置" onClick={() => props.onOpenStarter("settings")}>⚙</button>
    </aside>
  );
}
