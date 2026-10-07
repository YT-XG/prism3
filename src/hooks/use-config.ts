import { useEffect } from "react";

import { error as logError } from "@tauri-apps/plugin-log";
import { toast } from "sonner";

import { useAppStore } from "@/stores/app-store";

/** 启动时加载配置。 */
export function useConfig() {
  const loadConfig = useAppStore((state) => state.loadConfig);

  useEffect(() => {
    loadConfig().catch((cause) => {
      const message = `读取配置失败：${String(cause)}`;
      toast.error(message);
      void logError(message);
    });
  }, [loadConfig]);
}
