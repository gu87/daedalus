import type { ApprovalRequest } from "../types";

type Props = {
  approval: ApprovalRequest;
  onDecision: (decision: "approve" | "reject" | "changes") => void;
};

export function ApprovalBar(props: Props) {
  return (
    <section className="approval-bar">
      <div>
        <div className="approval-title">{props.approval.title}</div>
        <div className="approval-desc">{props.approval.desc}</div>
      </div>
      <div className="approval-actions">
        <button onClick={() => props.onDecision("changes")}>要求修改</button>
        <button className="reject" onClick={() => props.onDecision("reject")}>拒绝</button>
        <button className="approve" onClick={() => props.onDecision("approve")}>批准写入</button>
      </div>
    </section>
  );
}
