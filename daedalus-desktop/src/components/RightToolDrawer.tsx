import { rightTools } from "../data/mockData";
import type { RightToolKind, WorkTab } from "../types";

type Props = {
  activeTab: WorkTab;
  rightToolTabs: RightToolKind[];
  activeRightTool: RightToolKind | null;
  onClose: () => void;
  onOpenTool: (tool: RightToolKind) => void;
  onActivateTool: (tool: RightToolKind | null) => void;
  onCloseTool: (tool: RightToolKind) => void;
};

export function RightToolDrawer(props: Props) {
  const activeTool = rightTools.find((tool) => tool.id === props.activeRightTool);

  return (
    <aside className="right-drawer">
      <div className="drawer-top">
        <div>
          <strong>工作区工具</strong>
          <small>{activeTool ? `${activeTool.title} · 右侧标签页` : "点击工具打开为标签页"}</small>
        </div>
        <button className="icon-button" title="隐藏右侧工具区" onClick={props.onClose}>◨</button>
      </div>

      <section className={`tool-launcher ${props.rightToolTabs.length ? "compact" : ""}`}>
        {rightTools.map((tool) => (
          <button key={tool.id} className="tool-card" onClick={() => props.onOpenTool(tool.id)}>
            <span className="tool-icon">{tool.icon}</span>
            <span>
              <strong>{tool.title}</strong>
              <small>{tool.desc}</small>
            </span>
            {tool.shortcut ? <kbd>{tool.shortcut}</kbd> : <span />}
          </button>
        ))}
      </section>

      {props.rightToolTabs.length ? (
        <section className="right-tabs-area">
          <div className="right-tool-tabs">
            {props.rightToolTabs.map((toolId) => {
              const tool = rightTools.find((item) => item.id === toolId)!;
              return (
                <button
                  key={tool.id}
                  className={`right-tool-tab ${tool.id === props.activeRightTool ? "active" : ""}`}
                  onClick={() => props.onActivateTool(tool.id)}
                >
                  <span className="tool-tab-icon">{tool.icon}</span>
                  <span className="tool-tab-title">{tool.title}</span>
                  <span
                    className="tool-tab-close"
                    onClick={(event) => {
                      event.stopPropagation();
                      props.onCloseTool(tool.id);
                    }}
                  >
                    ×
                  </span>
                </button>
              );
            })}
          </div>
          <div className="right-tool-body">
            <RightToolBody tool={props.activeRightTool} activeTab={props.activeTab} />
          </div>
        </section>
      ) : null}
    </aside>
  );
}

function RightToolBody({ tool, activeTab }: { tool: RightToolKind | null; activeTab: WorkTab }) {
  if (!tool) {
    return <div className="right-empty-hint">选择一个右侧工具查看详情</div>;
  }

  if (tool === "review") {
    return (
      <>
        <div className="detail-card">
          <h3>当前状态</h3>
          <p>{activeTab.status} · {activeTab.title}</p>
        </div>
        <div className="detail-card">
          <h3>审查建议</h3>
          <p>仅当 Evidence 显示范围受控、终端检查通过、没有越权文件时，才建议批准。</p>
        </div>
        <div className="detail-card">
          <h3>变更摘要</h3>
          <pre>{`+ index.html
+ styles.css
+ app.js

未修改 daemon / protocol 文件。`}</pre>
        </div>
      </>
    );
  }

  if (tool === "evidence") {
    return (
      <div className="detail-card">
        <h3>验收清单</h3>
        <ul className="evidence-list">
          <li className="pass">JS 语法检查通过</li>
          <li className="pass">只修改 demo 文件</li>
          <li className="pass">Browser Workspace 为只读</li>
          <li className="todo">尚未接入 daedalusd 真实事件流</li>
          <li className="gate">Gate 需要人工确认</li>
        </ul>
      </div>
    );
  }

  if (tool === "terminal") {
    return (
      <div className="detail-card">
        <h3>最新终端</h3>
        <pre>{`$ node --check app.js
ok

$ python3 -m http.server 5177
Serving HTTP on 0.0.0.0 port 5177

$ daedalus verify
verify.pass`}</pre>
      </div>
    );
  }

  if (tool === "browser") {
    return (
      <div className="detail-card">
        <h3>Browser Workspace</h3>
        <p>当前为只读上下文模式。</p>
        <pre>{`URL: http://localhost:5177
Selected text: Task / Run / Approval / Evidence
Action permission: disabled`}</pre>
      </div>
    );
  }

  if (tool === "files") {
    return (
      <div className="detail-card">
        <h3>上下文文件</h3>
        <pre>{`desktop-demo/
├── index.html
├── styles.css
└── app.js

README.md`}</pre>
      </div>
    );
  }

  return (
    <div className="detail-card">
      <h3>侧边聊天</h3>
      <p>临时询问其他 Agent，不打断主会话。</p>
      <pre>Verifier: 当前变更范围可控，但仍需要 Gate 批准。</pre>
    </div>
  );
}
