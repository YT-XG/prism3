import { useEffect, useRef, useState } from "react";

import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type WirelessBattery } from "@/lib/ipc";

/**
 * 轮询间隔：电量变化很慢，60s 一次足够。
 *
 * 一轮会枚举全部 HID 接口，并对 Logitech 接收器逐槽位发请求，比 ML903 单条
 * feature report 重，所以间隔取 60s（鼠标卡片的 30s 只读一台设备）。
 */
const POLL_INTERVAL = 60_000;

/**
 * 多品牌无线外设电量数据源：挂载时先用 Rust 侧缓存渲染，再读一次真值；
 * 之后按可见性轮询。窗口隐藏或失焦即停表，不留常驻线程。
 */
export function useWirelessBatteries(enabled = true): WirelessBattery[] {
  const [batteries, setBatteries] = useState<WirelessBattery[]>([]);
  const loggedErrorRef = useRef(false);

  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let timer: number | null = null;

    const read = () => {
      void ipc
        .readWirelessBatteries()
        .then((next) => {
          if (!active) return;
          loggedErrorRef.current = false;
          setBatteries(next);
        })
        .catch((cause) => {
          // 失败只记一次，避免每分钟刷日志。
          if (loggedErrorRef.current) return;
          loggedErrorRef.current = true;
          void logError(`读取无线外设电量失败：${String(cause)}`);
        });
    };

    // 先用缓存值渲染（热重载 / 再次挂载时不必等一轮 HID 枚举）。
    void ipc
      .getWirelessBatteries()
      .then((cached) => {
        if (active && cached.length > 0) setBatteries(cached);
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

  return batteries;
}
