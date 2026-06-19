// Daedalus Desktop — preload script (IPC bridge)
// P5+.1: getHealth now calls the real daemon HTTP API.
// P5+.2–P5+.3 (future): UDS NDJSON client for task dispatch + permission.

const { contextBridge } = require("electron");

// Default daemon HTTP address, overridable via env.
const DAEMON_HTTP_ADDR =
  process.env.DAEDALUSD_HTTP_ADDR || "http://127.0.0.1:9800";

contextBridge.exposeInMainWorld("daedalusAPI", {
  // ── Health ─────────────────────────────────────────────────────────
  getHealth: async () => {
    try {
      const resp = await fetch(`${DAEMON_HTTP_ADDR}/api/health`);
      if (!resp.ok) return { status: "error", db_ok: false };
      return await resp.json();
    } catch {
      return { status: "unreachable", db_ok: false };
    }
  },

  // ── Sessions / Tabs (future) ───────────────────────────────────────
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

  // ── Gate / Approval (future) ───────────────────────────────────────
  approveGate: async (requestId) => {
    console.log("[daedalusAPI] approveGate", requestId);
  },

  rejectGate: async (requestId) => {
    console.log("[daedalusAPI] rejectGate", requestId);
  },

  requestChanges: async (requestId) => {
    console.log("[daedalusAPI] requestChanges", requestId);
  },

  // ── Subscribe (future) ─────────────────────────────────────────────
  subscribeEvents: async (callback) => {
    return () => {};
  },
});
