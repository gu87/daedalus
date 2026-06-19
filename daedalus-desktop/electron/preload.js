// Daedalus Desktop — preload script (IPC bridge)
// P5+.1: getHealth via HTTP API.
// P5+.2: UDS NDJSON client for task.dispatch + ping.

const { contextBridge } = require("electron");
const net = require("net");
const { StringDecoder } = require("string_decoder");

const DAEMON_HTTP_ADDR =
  process.env.DAEDALUSD_HTTP_ADDR || "http://127.0.0.1:9800";
const DAEMON_SOCK =
  process.env.DAEDALUSD_SOCK || "/tmp/daedalusd.sock";
const DESKTOP_AGENT_ID =
  process.env.DAEDALUS_DESKTOP_AGENT_ID || "daedalus-desktop";

// ═══════════════════════════════════════════════════════════════════════
// UDS NDJSON client
// ═══════════════════════════════════════════════════════════════════════

function udsConnect(socketPath) {
  const socket = net.createConnection(socketPath);
  const decoder = new StringDecoder("utf8");
  let buf = "";
  let closed = false;
  let settled = false;  // terminal received → no more _close/_error callbacks

  const handlers = {};

  function settle() { settled = true; }

  function onMessage(line) {
    let msg;
    try { msg = JSON.parse(line); }
    catch (e) { return { error: "protocol_error", detail: "JSON parse: " + e.message }; }
    const type = msg.type;
    if (!type) return { error: "protocol_error", detail: "missing type field" };
    if (handlers[type]) return handlers[type](msg);
    if (handlers["*"]) return handlers["*"](msg);
    return null;
  }

  socket.on("data", (chunk) => {
    buf += decoder.write(chunk);
    while (buf.includes("\n")) {
      const nl = buf.indexOf("\n");
      const line = buf.substring(0, nl);
      buf = buf.substring(nl + 1);
      const result = onMessage(line);
      if (result && result.error) {
        if (!settled && handlers._error) handlers._error(result.error, result.detail);
        if (!closed) { socket.destroy(); closed = true; }
      }
    }
  });

  socket.on("error", (err) => {
    if (!settled && !closed && handlers._error)
      handlers._error("connection_error", err.message);
    closed = true;
  });

  socket.on("close", () => {
    closed = true;
    if (!settled && handlers._close) handlers._close();
  });

  return {
    socket,
    on(type, fn) { handlers[type] = fn; },
    send(obj) { if (!closed) socket.write(JSON.stringify(obj) + "\n"); },
    close() { if (!closed) { socket.destroy(); closed = true; } },
    settle,
  };
}

// ═══════════════════════════════════════════════════════════════════════
// TaskCard builder
// ═══════════════════════════════════════════════════════════════════════

function nowISO() { return new Date().toISOString(); }

function buildTaskDispatch(goal) {
  const taskId = "task-" + Date.now() + "-" +
    Math.random().toString(36).slice(2, 8);
  const reqId = "req-" + Date.now();
  const agentId = DESKTOP_AGENT_ID;

  return {
    reqId,
    taskId,
    agentId,
    message: {
      type: "task.dispatch",
      ts: nowISO(),
      req_id: reqId,
      agent_id: agentId,
      task_id: taskId,
      task_card: {
        schema_version: "2.8",
        task_card_id: taskId,
        project: "daedalus-desktop",
        created_at: nowISO(),
        status: "created",
        goal: goal,
        compiled_intent: { action: goal },
        context: {
          user_preferences: {},
          project_context: { name: "daedalus", data: {}, global_must_avoid: [] },
          relevant_feedback: [],
        },
        execution_plan: { primary_agent: agentId },
        acceptance_criteria: {},
        allowed_files: [],
        safety: { allowed_paths: [], denied_commands: [] },
        output_contract: {},
        review_gate_criteria: {},
      },
    },
  };
}

// ═══════════════════════════════════════════════════════════════════════
// API exposed to renderer
// ═══════════════════════════════════════════════════════════════════════

contextBridge.exposeInMainWorld("daedalusAPI", {
  // ── Health (P5+.1) ────────────────────────────────────────────────────
  getHealth: async () => {
    try {
      const resp = await fetch(`${DAEMON_HTTP_ADDR}/api/health`);
      if (!resp.ok) return { status: "error", db_ok: false };
      return await resp.json();
    } catch {
      return { status: "unreachable", db_ok: false };
    }
  },

  listSessions: async () => {
    try {
      const resp = await fetch(`${DAEMON_HTTP_ADDR}/api/tasks?limit=20`);
      if (!resp.ok) return [];
      const body = await resp.json();
      return body.tasks || [];
    } catch {
      return [];
    }
  },

  // ── Ping (P5+.2) ──────────────────────────────────────────────────────
  ping: () => {
    return new Promise((resolve) => {
      let resolved = false;
      const done = (v) => { if (!resolved) { resolved = true; resolve(v); } };
      try {
        const conn = udsConnect(DAEMON_SOCK);
        const timer = setTimeout(() => { conn.close(); done(false); }, 3000);
        conn.on("system.pong", () => { clearTimeout(timer); conn.close(); done(true); });
        conn.on("_error", () => { clearTimeout(timer); done(false); });
        conn.on("_close", () => { clearTimeout(timer); done(false); });
        conn.send({ type: "system.ping", ts: nowISO(), req_id: "ping-" + Date.now() });
      } catch { done(false); }
    });
  },

  // ── Dispatch (P5+.2) ──────────────────────────────────────────────────
  dispatchTask: (goal, callbacks) => {
    return new Promise((resolve, reject) => {
      const td = buildTaskDispatch(goal);
      let conn;
      try {
        conn = udsConnect(DAEMON_SOCK);
      } catch (e) {
        reject(new Error("connection_error: " + e.message));
        return;
      }

      conn.on("task.stream", (msg) => {
        if (callbacks.onStream) callbacks.onStream(msg.chunk || "");
      });

      conn.on("task.done", (msg) => {
        conn.settle();
        if (callbacks.onDone) callbacks.onDone(msg.outbox);
        conn.close();
      });

      conn.on("task.error", (msg) => {
        conn.settle();
        if (callbacks.onError)
          callbacks.onError(msg.error_taxonomy || "unknown", msg.detail || "");
        conn.close();
      });

      conn.on("system.error", (msg) => {
        conn.settle();
        if (callbacks.onError)
          callbacks.onError("system_error", msg.detail || JSON.stringify(msg));
        conn.close();
      });

      conn.on("permission.request", (msg) => {
        // P5+.2: display only, do not reply.  P5+.3 will handle response.
        if (callbacks.onPermissionRequest) callbacks.onPermissionRequest(msg);
      });

      conn.on("_error", (code, detail) => {
        if (callbacks.onError) callbacks.onError(code, detail);
        conn.close();
      });

      conn.on("_close", () => {
        // If we get here without done/error, something went wrong.
        if (callbacks.onError)
          callbacks.onError("connection_lost", "daemon connection closed unexpectedly");
      });

      conn.send(td.message);
      resolve({ taskId: td.taskId, reqId: td.reqId });
    });
  },

  // ── Gate / Approval (future P5+.3) ────────────────────────────────────
  approveGate: async (requestId) => {
    console.log("[daedalusAPI] approveGate (P5+.3)", requestId);
  },
  rejectGate: async (requestId) => {
    console.log("[daedalusAPI] rejectGate (P5+.3)", requestId);
  },
  requestChanges: async (requestId) => {
    console.log("[daedalusAPI] requestChanges (P5+.3)", requestId);
  },
  subscribeEvents: async (_callback) => {
    return () => {};
  },
});
