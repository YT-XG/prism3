import { useEffect, useRef, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type AppStatus } from "@/lib/ipc";

/** 运行状态变化事件名，与 Rust 侧 `APP_STATUS_EVENT` 一致。 */
const APP_STATUS_EVENT = "app:status";
/** 负载采样间隔（毫秒）：状态灯不需要秒级跟随，放慢以压低常驻开销。 */
const POLL_INTERVAL = 3000;

/**
 * 订阅应用运行状态：监听 `app:status` 广播，并在挂载时主动拉一次
 * （事件不缓存，热重载或错过广播时也能拿到当前值）。
 */
export function useAppStatus(): AppStatus {
  const [status, setStatus] = useState<AppStatus>("normal");

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<AppStatus>(APP_STATUS_EVENT, (event) => {
      if (active) setStatus(event.payload);
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅运行状态失败：${String(cause)}`);
      });

    void ipc
      .getAppStatus()
      .then((next) => {
        if (active) setStatus(next);
      })
      .catch((cause) => {
        void logError(`读取运行状态失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return status;
}

/**
 * 周期采样系统负载并更新运行状态（`refresh_app_status` 只在档位变化时广播）。
 *
 * 挂在**灰条窗口**：该窗口恰好在灯可见时存在，因此禁用快捷小窗后零采样——
 * 不新增常驻采集线程，沿用 `MonitorState` 的按需模型。窗口被 `hide` 或
 * 最小化时靠 `visibilitychange` 自动停表。
 */
export function useAppStatusReporter() {
  const loggedErrorRef = useRef(false);

  useEffect(() => {
    let timer: number | null = null;

    const poll = () => {
      void ipc.refreshAppStatus().catch((cause) => {
        // 失败只记一次，避免每 3 秒刷日志。
        if (loggedErrorRef.current) return;
        loggedErrorRef.current = true;
        void logError(`上报运行状态失败：${String(cause)}`);
      });
    };

    const start = () => {
      if (timer !== null) return;
      poll();
      timer = window.setInterval(poll, POLL_INTERVAL);
    };

    const stop = () => {
      if (timer === null) return;
      window.clearInterval(timer);
      timer = null;
    };

    const handleVisibility = () => {
      if (document.visibilityState === "visible") start();
      else stop();
    };

    if (document.visibilityState === "visible") start();
    document.addEventListener("visibilitychange", handleVisibility);

    return () => {
      stop();
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, []);
}
