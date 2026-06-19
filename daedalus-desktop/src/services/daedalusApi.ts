/**
 * P5+.1/P5+.2: Daemon API bridge.
 *
 * In production the Electron preload exposes `window.daedalusAPI`.
 * In browser dev mode (`npm run dev` without Electron), fall back to mock.
 */

export interface HealthStatus {
  status: string;
  uptime_seconds?: number;
  socket_path?: string;
  db_ok: boolean;
}

export interface TaskSummary {
  run_id: string;
  agent_id: string;
  task_id: string;
  status: string;
  spawned_at: number;
  completed_at?: number;
  error_taxonomy?: string;
}

/** P5+.3: one-shot reply for permission requests. */
export interface PermissionReply {
  approve(): void;
  reject(): void;
  requestChanges(): void;
}

/** P5+.2/P5+.3: callbacks for dispatchTask. */
export interface TaskCallbacks {
  onStream?(chunk: string): void;
  onDone?(outbox: unknown): void;
  onError?(taxonomy: string, detail: string): void;
  onPermissionRequest?(perm: unknown, reply: PermissionReply): void;
}

export interface DaedalusApi {
  getHealth(): Promise<HealthStatus>;
  listSessions(): Promise<TaskSummary[]>;
  ping(): Promise<boolean>;
  dispatchTask(
    goal: string,
    callbacks: TaskCallbacks,
  ): Promise<{ taskId: string; reqId: string }>;
  approveGate(requestId: string): Promise<void>;
  rejectGate(requestId: string): Promise<void>;
  requestChanges(requestId: string): Promise<void>;
  subscribeEvents(callback: (event: unknown) => void): () => void;
}

declare global {
  interface Window {
    daedalusAPI?: DaedalusApi;
  }
}

const mock: DaedalusApi = {
  async getHealth() {
    return { status: "mock", db_ok: false };
  },
  async listSessions() {
    return [];
  },
  async ping() {
    return false;
  },
  async dispatchTask(_goal, callbacks) {
    setTimeout(() => callbacks.onError?.("mock", "daemon not connected"), 100);
    // Mock permission to test reply object: after 2s, fire permission, then after
    // user clicks approve (within 10s), simulate task done.
    setTimeout(() => {
      if (callbacks.onPermissionRequest) {
        const reply = {
          approve() { setTimeout(() => callbacks.onDone?.("mock-done"), 500); },
          reject() { callbacks.onError?.("mock-rejected", "user rejected"); },
          requestChanges() { callbacks.onError?.("mock-changes", "changes requested (denied)"); },
        };
        callbacks.onPermissionRequest({ permission_id: "mock-perm-1", req_id: "mock-req", tool: "test", args: {} }, reply);
      }
    }, 2000);
    return { taskId: "mock-task", reqId: "mock-req" };
  },
  async approveGate() {},
  async rejectGate() {},
  async requestChanges() {},
  subscribeEvents() {
    return () => {};
  },
};

export function getApi(): DaedalusApi {
  if (window.daedalusAPI) return window.daedalusAPI;
  return mock;
}
