import type { WorkTab } from "../types";

/**
 * 这里是后续接入 Electron preload / IPC / daedalusd 的预留层。
 *
 * 当前 UI Skeleton 先使用 mock state。
 * 合并进 Electron 后，可以改成：
 *
 * const api = window.daedalusAPI
 */
export type DaedalusApi = {
  listSessions(): Promise<WorkTab[]>;
  approveGate(requestId: string): Promise<void>;
  rejectGate(requestId: string): Promise<void>;
  requestChanges(requestId: string): Promise<void>;
};

export const mockDaedalusApi: DaedalusApi = {
  async listSessions() {
    return [];
  },
  async approveGate() {},
  async rejectGate() {},
  async requestChanges() {}
};
