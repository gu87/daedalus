import type { WorkTab } from "../types";

type Props = {
  tab: WorkTab;
  onCreateTask: () => void;
};

export function ProjectView(props: Props) {
  return (
    <section className="project-view">
      <div className="form-card">
        <h1>{props.tab.title}</h1>
        <p className="muted-copy">
          这是工作区首页。这里聚合当前项目下的对话、会议、任务和自动化。
        </p>
        <div className="form-grid">
          <label>
            最近活动
            <select defaultValue="meeting">
              <option value="meeting">Browser Workspace API 评审会 · 会议 · 运行中</option>
              <option value="task">Desktop UI Demo 任务 · 等待审批</option>
              <option value="chat">权限中继设计 · 对话</option>
            </select>
          </label>
          <label>
            项目上下文
            <textarea
              defaultValue={`路径：~/Daedalus
默认 Agent：Architect / Codex / Verifier
规则：高风险写入必须经过 Gate 审批`}
            />
          </label>
        </div>
        <div className="form-actions">
          <button className="primary" onClick={props.onCreateTask}>新建任务</button>
        </div>
      </div>
    </section>
  );
}
