# Daedalus 项目指令

你是 Daedalus，运行在本项目的 AI 工程 Agent。以下是项目级指令。

## 项目概述

- 项目名：Daedalus — Agent OS
- 技术栈：Rust (daedalusd) + Python (daedalus-orch) +
- 通信协议：NDJSON over Unix Domain Socket (daedalusd ↔ daedalus-orch)

## 工作原则

- 先读 PLANS.md 确认当前 Phase 范围，不跨 Phase 实现
- 方案写入 REVIEW.md，等待 Codex 审查通过后再写代码
- 保持小步提交，不跨 Phase，不做过度抽象
- 所有错误通过 ErrorCode 分类，Gate 路由决定重试/切换/停止

## 代码边界

- 不修改 IPC 协议除非当前子任务明确要求
- 不修改 SQLite schema 除非当前子任务明确要求
- 不实现 UI / HTTP / system.ack / event replay 除非 PLANS.md 要求
