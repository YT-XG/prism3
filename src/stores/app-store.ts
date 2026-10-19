import { create } from "zustand";

import { ipc, type AppConfig } from "@/lib/ipc";

export type AppView = "home" | "settings" | "monitor" | "search" | "bluetooth" | "clipboard";

/** 侧栏收起状态：纯 UI 偏好，放 localStorage 即可（不进配置文件、不跨窗口）。 */
const SIDEBAR_KEY = "prism3:sidebar";

function readSidebarCollapsed(): boolean {
  try {
    return window.localStorage.getItem(SIDEBAR_KEY) === "collapsed";
  } catch {
    return false;
  }
}

interface AppState {
  config: AppConfig | null;
  loading: boolean;
  activeView: AppView;
  sidebarCollapsed: boolean;
  setActiveView: (view: AppView) => void;
  toggleSidebar: () => void;
  loadConfig: () => Promise<void>;
  saveConfig: (patch: Partial<AppConfig>) => Promise<void>;
  resetConfig: () => Promise<void>;
}

export const useAppStore = create<AppState>((set, get) => ({
  config: null,
  loading: true,
  activeView: "home",
  sidebarCollapsed: readSidebarCollapsed(),

  setActiveView: (view) => set({ activeView: view }),

  toggleSidebar: () => {
    const next = !get().sidebarCollapsed;
    set({ sidebarCollapsed: next });
    try {
      window.localStorage.setItem(SIDEBAR_KEY, next ? "collapsed" : "expanded");
    } catch {
      // 存储不可用时只是不记忆，不影响本次交互。
    }
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
