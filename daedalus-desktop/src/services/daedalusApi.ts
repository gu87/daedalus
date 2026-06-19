/**
 * P5+.1: Real daemon API bridge.
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

export interface DaedalusApi {
  getHealth(): Promise<HealthStatus>;
  listSessions(): Promise<TaskSummary[]>;
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
  async approveGate() {},
  async rejectGate() {},
  async requestChanges() {},
  subscribeEvents() {
    return () => {};
  },
};

/** Real API when running inside Electron, mock otherwise. */
export function getApi(): DaedalusApi {
  if (window.daedalusAPI) return window.daedalusAPI;
  return mock;
}
