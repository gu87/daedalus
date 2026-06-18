const singleEvents = [
  {
    tag: "目标",
    agent: "用户",
    time: "09:42",
    message:
      "设计一个可以展示 Task、Run、审批 和 Evidence 的 Daedalus Desktop MVP，不接真实 daemon。",
  },
  {
    tag: "计划",
    agent: "Codex",
    time: "09:43",
    message:
      "新增 desktop-demo/ 静态页面，先做会话流、右侧工作区、审批条和浏览器只读上下文，避免影响现有 Rust/Python 后端。",
  },
  {
    tag: "命令",
    agent: "Codex",
    time: "09:45",
    message: "我会用最小真实调用验证这个原型能被加载，先做 JS 语法检查，再起静态服务请求入口页。",
    code: "$ node --check desktop-demo/app.js\n$ python3 -m http.server 5177 --directory desktop-demo",
  },
  {
    tag: "审批",
    agent: "Daedalus",
    time: "09:46",
    message:
      "Gate 正在等待你的决策。Codex 想在 desktop-demo/ 下写入文件。当前证据显示 daemon 和协议文件不在本次改动范围内。",
  },
];

const meetingEvents = [
  {
    tag: "目标",
    agent: "架构师",
    time: "09:42",
    message:
      "会议目标很清楚：把 Daedalus Desktop 从报告型 Dashboard 拉回 Agent Workbench。中间只保留会话和决策流，右侧承载执行现场。",
  },
  {
    tag: "提案",
    agent: "架构师",
    time: "09:44",
    message:
      "第一版不做复杂 dashboard。主对象是 Session / Run / 审批，布局采用左导航、中间事件流、右侧工作区。",
  },
  {
    tag: "Gate 检查",
    agent: "验证员",
    time: "09:45",
    message:
      "Browser 第一版只收集 URL、标题、选中文本、摘要和截图。自动操作会过早扩大权限边界。",
  },
  {
    tag: "分工",
    agent: "Codex",
    time: "09:46",
    message:
      "我会只改 desktop-demo，不碰 daemon/protocol。实现重点是 agent 会话流、会议 Brief、Gate 语义和 Evidence 验收清单。",
  },
  {
    tag: "决策",
    agent: "会议",
    time: "09:47",
    message:
      "决策 Summary: 采用三栏 Agent Workbench；中间是会话/会议决策流；右侧保留 Files、Diff、Terminal、Browser、Evidence；Gate 决策必须显式等待用户。",
    decision:
      "Codex 负责 Demo 迭代；验证员负责验收清单；文档员负责保持交付说明简洁。",
  },
];

const eventStream = document.querySelector("#eventStream");
const meetingBrief = document.querySelector("#meetingBrief");
const participantStrip = document.querySelector("#participantStrip");
const sessionStatus = document.querySelector("#sessionStatus");
const approvalBar = document.querySelector("#approvalBar");

function renderEvents(events) {
  eventStream.innerHTML = events
    .map((event) => {
      const codeBlock = event.code ? `<pre>${event.code}</pre>` : "";
      const decisionBlock = event.decision
        ? `<div class="decision-summary"><strong>决策 Summary</strong><span>${event.decision}</span></div>`
        : "";
      const avatar = event.agent
        .split(" ")
        .map((part) => part[0])
        .join("")
        .slice(0, 2);

      return `
        <article class="event-card ${event.tag === "决策" ? "decision" : ""}">
          <div class="event-meta">
            <span class="event-avatar">${avatar}</span>
          </div>
          <div class="event-content">
            <div class="event-head">
              <span class="event-agent-line">
                <span class="event-agent">${event.agent}</span>
                <span class="event-time">${event.time}</span>
              </span>
              <span class="event-kind">${event.tag}</span>
            </div>
            <p class="event-message">${event.message}</p>
            ${codeBlock}
            ${decisionBlock}
          </div>
        </article>
      `;
    })
    .join("");
}

function setMode(mode) {
  document.querySelectorAll(".mode-button").forEach((button) => {
    button.classList.toggle("active", button.dataset.mode === mode);
  });

  const is会议 = mode === "meeting";
  meetingBrief.hidden = !is会议;
  participantStrip.hidden = !is会议;
  renderEvents(is会议 ? meetingEvents : singleEvents);
}

function setPanel(panelName) {
  document.querySelectorAll(".panel-tab").forEach((tab) => {
    tab.classList.toggle("active", tab.dataset.panel === panelName);
  });

  document.querySelectorAll(".panel-view").forEach((view) => {
    view.classList.toggle("active", view.dataset.panelView === panelName);
  });
}

function add决策Event(decision) {
  const decisionText = {
    approve: "已批准。Codex 可以创建 Demo 文件并继续验证。",
    reject: "已拒绝。该运行已停止，不应创建本地 Demo 文件。",
    changes: "已要求修改。Codex 需要先调整范围，再写入文件。",
  };

  const decisionEvent = {
    tag: "决策",
    agent: "用户",
    time: "now",
    message: decisionText[decision],
    decision:
      decision === "approve"
        ? "Gate 已放行。该运行可以继续应用改动并执行验证。"
        : "Gate 保持关闭，直到范围被调整或该运行被停止。",
  };

  singleEvents.push(decisionEvent);
  meetingEvents.push(decisionEvent);
  renderEvents(
    document.querySelector(".mode-button.active").dataset.mode === "meeting"
      ? meetingEvents
      : singleEvents,
  );

  sessionStatus.textContent =
    decision === "approve"
      ? "正在应用改动"
      : decision === "reject"
        ? "已拒绝"
        : "已要求修改";
  approvalBar.hidden = true;
}

document.querySelectorAll(".mode-button").forEach((button) => {
  button.addEventListener("click", () => setMode(button.dataset.mode));
});

document.querySelectorAll(".panel-tab").forEach((tab) => {
  tab.addEventListener("click", () => setPanel(tab.dataset.panel));
});

document.querySelectorAll("[data-decision]").forEach((button) => {
  button.addEventListener("click", () => add决策Event(button.dataset.decision));
});

renderEvents(singleEvents);
