import { create } from "zustand";

import { ipc, type AppConfig } from "@/lib/ipc";

export type AppView = "home" | "settings" | "monitor" | "search" | "bluetooth";

interface AppState {
  config: AppConfig | null;
  loading: boolean;
  activeView: AppView;
  setActiveView: (view: AppView) => void;
  loadConfig: () => Promise<void>;
  saveConfig: (patch: Partial<AppConfig>) => Promise<void>;
  resetConfig: () => Promise<void>;
}

export const useAppStore = create<AppState>((set, get) => ({
  config: null,
  loading: true,
  activeView: "home",

  setActiveView: (view) => set({ activeView: view }),

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
