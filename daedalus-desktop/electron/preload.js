// Daedalus Desktop — preload script (IPC bridge)
// Phase 1: mock implementations for UI skeleton development.
// Phase 2: replace with real Electron IPC calls to daedalusd.

const { contextBridge } = require("electron");

contextBridge.exposeInMainWorld("daedalusAPI", {
  // ── Sessions / Tabs ────────────────────────────────────────────────
  listSessions: async () => {
    // TODO: real IPC → daedalusd task.dispatch → task list
    return [];
  },

  subscribeEvents: async (callback) => {
    // TODO: real IPC → daedalusd event stream
    // Mock: no-op for now
    return () => {};
  },

  // ── Gate / Approval ────────────────────────────────────────────────
  approveGate: async (requestId) => {
    // TODO: real IPC → daedalusd permission.response approved
    console.log("[daedalusAPI] approveGate", requestId);
  },

  rejectGate: async (requestId) => {
    // TODO: real IPC → daedalusd permission.response denied
    console.log("[daedalusAPI] rejectGate", requestId);
  },

  requestChanges: async (requestId) => {
    // TODO: real IPC → daedalusd permission.response denied + feedback
    console.log("[daedalusAPI] requestChanges", requestId);
  },

  // ── Health ─────────────────────────────────────────────────────────
  getHealth: async () => {
    // TODO: real IPC → daedalusd GET /api/health
    return { status: "ok", uptime_seconds: 0, socket_path: "", db_ok: false };
  },
});
