import { invoke } from "@tauri-apps/api/core";

export type Theme = "system" | "light" | "dark";

/** 与 Rust 侧 `AppConfig` 一一对应。 */
export interface AppConfig {
  theme: Theme;
  language: string;
  closeToTray: boolean;
  lastOpened: string | null;
}

export const ipc = {
  getConfig: () => invoke<AppConfig>("get_config"),
  setConfig: (config: AppConfig) => invoke<void>("set_config", { config }),
  resetConfig: () => invoke<AppConfig>("reset_config"),
};
