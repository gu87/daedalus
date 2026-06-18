import { useState } from "react";
import type { WorkTab } from "../types";
import { tabIcon } from "../utils/view";

type Props = {
  tab: WorkTab;
  onCreateGeneratedTab: (kind: "chat" | "meeting" | "task", prompt: string) => void;
};

const config: Record<string, { title: string; desc: string; placeholder: string; action: string; createKind: "chat" | "meeting" | "task" }> = {
  "new-chat": {
    title: "新对话",
    desc: "选择一个 Agent，开始沟通、分析或执行轻量任务。",
    placeholder: "给单个 Agent 发送消息...",
    action: "开始",
    createKind: "chat"
  },
  "new-meeting": {
    title: "会议室",
    desc: "组织多个 Agent 围绕一个问题讨论，形成结论和下一步。",
    placeholder: "输入会议目标...",
    action: "开始会议",
    createKind: "meeting"
  },
  "new-task": {
    title: "新建任务",
    desc: "描述明确目标，Daedalus 会选择 Agent 执行、验证，并在需要时请求审批。",
    placeholder: "描述任务目标...",
    action: "运行任务",
    createKind: "task"
  },
  search: {
    title: "搜索",
    desc: "搜索工作区、历史会话、任务、文件和 Agent。",
    placeholder: "搜索会话、任务或文件...",
    action: "搜索",
    createKind: "chat"
  },
  skills: {
    title: "插件与技能",
    desc: "管理 MCP、Skills、工具权限和 Agent 可用能力。",
    placeholder: "搜索插件或技能...",
    action: "搜索",
    createKind: "chat"
  },
  automation: {
    title: "自动化",
    desc: "创建定时任务、条件检查和长期观察。",
    placeholder: "描述要自动执行或持续检查的事情...",
    action: "创建",
    createKind: "task"
  },
  settings: {
    title: "设置",
    desc: "管理工作区、模型、Agent、权限和界面偏好。",
    placeholder: "搜索设置项...",
    action: "搜索",
    createKind: "chat"
  }
};

export function EmptyView(props: Props) {
  const c = config[props.tab.kind] ?? config["new-chat"];
  const [value, setValue] = useState("");

  function submit() {
    props.onCreateGeneratedTab(c.createKind, value.trim() || c.placeholder.replace("...", ""));
  }

  return (
    <section className="empty-view">
      <div className="hero-card">
        <div className="hero-icon">{tabIcon(props.tab.kind)}</div>
        <h1>{c.title}</h1>
        <p>{c.desc}</p>
      </div>
      <div className="start-box">
        <input
          value={value}
          placeholder={c.placeholder}
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") submit();
          }}
        />
        <button onClick={submit}>{c.action}</button>
      </div>
    </section>
  );
}
