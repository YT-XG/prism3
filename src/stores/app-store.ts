import { create } from "zustand";

import { ipc, type AppConfig } from "@/lib/ipc";

export type AppView =
  | "home"
  | "settings"
  | "monitor"
  | "search"
  | "bluetooth"
  | "clipboard"
  | "notes"
  | "projects";

interface AppState {
  config: AppConfig | null;
  loading: boolean;
  activeView: AppView;
  /**
   * 待打开的速记 id：全局搜索 / 首页卡片 / 快捷小窗「打开某条笔记」时先落这里，
   * 速记页挂载后 consume 一次（`AppShell` 按视图分支渲染，速记页离开即卸载）。
   */
  pendingNoteId: number | null;
  /**
   * 待打开的项目运行记录：首页项目卡点「查看输出」时先落这里，
   * 项目页挂载后 consume 一次（选中对应项目并展开控制台）。
   */
  pendingProjectRun: { projectId: string; runId: string | null } | null;
  setActiveView: (view: AppView) => void;
  /** 切到速记页并请求打开某条笔记。 */
  openNote: (id: number) => void;
  /** 取走待打开笔记 id（命中即清空，避免重复跳转）。 */
  consumePendingNote: () => number | null;
  /** 切到项目页并选中某项目；给了 `runId` 就同时展开那条运行记录的控制台。 */
  openProjectRun: (projectId: string, runId?: string | null) => void;
  /** 取走待打开的项目运行记录（命中即清空）。 */
  consumePendingProjectRun: () => { projectId: string; runId: string | null } | null;
  loadConfig: () => Promise<void>;
  saveConfig: (patch: Partial<AppConfig>) => Promise<void>;
  resetConfig: () => Promise<void>;
}

export const useAppStore = create<AppState>((set, get) => ({
  config: null,
  loading: true,
  activeView: "home",
  pendingNoteId: null,
  pendingProjectRun: null,

  setActiveView: (view) => set({ activeView: view }),

  openNote: (id) => set({ activeView: "notes", pendingNoteId: id }),

  consumePendingNote: () => {
    const id = get().pendingNoteId;
    if (id != null) set({ pendingNoteId: null });
    return id;
  },

  openProjectRun: (projectId, runId = null) =>
    set({ activeView: "projects", pendingProjectRun: { projectId, runId } }),

  consumePendingProjectRun: () => {
    const pending = get().pendingProjectRun;
    if (pending) set({ pendingProjectRun: null });
    return pending;
  },

  loadConfig: async () => {
    set({ loading: true });
    try {
      set({ config: await ipc.getConfig() });
    } finally {
      set({ loading: false });
    }
  },

  saveConfig: async (patch) => {
    const previous = get().config;
    if (!previous) return;

    const next = { ...previous, ...patch };
    set({ config: next });

    try {
      await ipc.setConfig(next);
    } catch (cause) {
      set({ config: previous });
      throw cause;
    }
  },

  resetConfig: async () => {
    set({ config: await ipc.resetConfig() });
  },
}));
