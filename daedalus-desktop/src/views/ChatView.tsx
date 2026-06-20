import { useState } from "react";
import { ApprovalBar } from "../components/ApprovalBar";
import { Composer } from "../components/Composer";
import type { ApprovalRequest, WorkTab } from "../types";

type Props = {
  tab: WorkTab;
  onSendMessage: (tabId: string, body: string) => void;
  onDecideApproval: (tabId: string, approval: ApprovalRequest, decision: "approve" | "reject" | "changes") => void;
};

export function ChatView(props: Props) {
  const [attached, setAttached] = useState(false);

  return (
    <div className="view">
      <header className="view-header">
        <div className="title-row">
          <h1>{props.tab.title}</h1>
          <span className={`status-chip ${props.tab.statusType}`}>{props.tab.status}</span>
        </div>
        <div className="meta-row">
          {props.tab.meta.map((item) => <span key={item}>{item}</span>)}
        </div>
        <div className="goal-box">{props.tab.goal}</div>
      </header>

      <section className="event-stream">
        {props.tab.messages.map((message) => (
          <article className="message" key={message.id}>
            <div className="avatar">{message.agent.slice(0, 1)}</div>
            <div>
              <div className="message-head">
                <span className="message-agent">{message.agent}</span>
                <span className="message-time">{message.time}</span>
                <span className="message-tag">{message.tag}</span>
              </div>
              <div className={`message-body${message.tag === "详情" ? " task-detail-text" : ""}`}>{message.body}</div>
              {message.decision ? (
                <div className="decision-box">
                  <strong>决策摘要：</strong>{message.decision}
                </div>
              ) : null}
            </div>
          </article>
        ))}
        {attached ? (
          <article className="message">
            <div className="avatar">D</div>
            <div>
              <div className="message-head">
                <span className="message-agent">Daedalus</span>
                <span className="message-time">刚刚</span>
                <span className="message-tag">上下文</span>
              </div>
              <div className="message-body">已模拟附加当前工作区上下文：文件、浏览器选中文本和最近命令结果。</div>
            </div>
          </article>
        ) : null}
      </section>

      {props.tab.approval ? (
        <ApprovalBar
          approval={props.tab.approval}
          onDecision={(decision) => props.onDecideApproval(props.tab.id, props.tab.approval!, decision)}
        />
      ) : null}

      {!props.tab.readonly && (
        <Composer
          placeholder={`给 ${props.tab.kind === "meeting" ? "会议" : props.tab.kind === "task" ? "任务" : props.tab.title} 发送消息...`}
          onAttach={() => setAttached(true)}
          onSend={(value) => props.onSendMessage(props.tab.id, value)}
        />
      )}
    </div>
  );
}
