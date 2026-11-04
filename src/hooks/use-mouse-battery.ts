import { useEffect, useRef, useState } from "react";

import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type MouseBattery } from "@/lib/ipc";

/**
 * 轮询间隔：鼠标电量变化很慢，30s 一次足够。
 * 每次读数是一次 HID feature report 往返（毫秒级），没有设备时也只是枚举一下接口。
 */
const POLL_INTERVAL = 30_000;

/** 还没读到任何值时先用它占位（不显示「0%」，那会像是真的没电）。 */
const UNKNOWN: MouseBattery = {
  connected: false,
  wireless: false,
  percent: null,
  charging: false,
};

/**
 * 鼠标电量数据源：挂载时先用 Rust 侧缓存渲染，再读一次真值；之后按可见性轮询。
 *
 * 轮询**只在这个 hook 挂载期间且 `enabled` 为真时**存在（首页卡片 / 有焦点的组合窗口），
 * 窗口隐藏或失焦即停表——与 `useAppStatusReporter` 同一套「按需采集」模型，不留常驻线程。
 * `enabled` 变化只收表、**不清空当前值**：重新可见时先显示上一份读数再刷新。
 */
export function useMouseBattery(enabled = true): MouseBattery {
  const [battery, setBattery] = useState<MouseBattery>(UNKNOWN);
  const loggedErrorRef = useRef(false);

  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let timer: number | null = null;

    const read = () => {
      void ipc
        .readMouseBattery()
        .then((next) => {
          if (!active) return;
          loggedErrorRef.current = false;
          setBattery(next);
        })
        .catch((cause) => {
          // 失败只记一次，避免每 30 秒刷日志。
          if (loggedErrorRef.current) return;
          loggedErrorRef.current = true;
          void logError(`读取鼠标电量失败：${String(cause)}`);
        });
    };

    // 先用缓存值渲染（热重载 / 再次挂载时不必等一次 HID 往返）。
    void ipc
      .getMouseBattery()
      .then((cached) => {
        if (active && cached.connected) setBattery(cached);
      })
      .catch(() => {
        /* 缓存读不到就直接等下面的真实读数 */
      });

    const start = () => {
      if (timer !== null) return;
      read();
      timer = window.setInterval(read, POLL_INTERVAL);
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
      active = false;
      stop();
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, [enabled]);

  return battery;
}
