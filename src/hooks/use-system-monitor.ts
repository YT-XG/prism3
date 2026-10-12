import { useEffect, useRef, useState } from "react";

import { error as logError } from "@tauri-apps/plugin-log";

import {
  ipc,
  type DynamicSnapshot,
  type SystemSnapshot,
  type SystemStatics,
} from "@/lib/ipc";

/** 轮询间隔与历史窗口长度（约 60 秒）。 */
const POLL_INTERVAL = 1000;
const HISTORY_LENGTH = 60;

/** 图表历史数据：x 用自增样本序号，避免同一秒内出现重复 x。 */
export interface MonitorHistory {
  xs: number[];
  cpu: (number | null)[];
  mem: number[];
  rx: number[];
  tx: number[];
}

const createHistory = (): MonitorHistory => ({ xs: [], cpu: [], mem: [], rx: [], tx: [] });

function append<T>(list: T[], value: T): T[] {
  const next = [...list, value];
  return next.length > HISTORY_LENGTH ? next.slice(next.length - HISTORY_LENGTH) : next;
}

interface UseSystemMonitorOptions {
  /** 是否轮询。默认 `true`。 */
  enabled?: boolean;
  /** 是否维护 60 点历史缓冲（仅图表页需要）；面板只取当前值，可传 `false` 省去每秒数组克隆。默认 `true`。 */
  trackHistory?: boolean;
}

/**
 * 系统监控数据源：仅在监控页挂载、窗口可见且 `enabled` 为真时以 1s 轮询，
 * 离开页面、窗口隐藏或调用方显式关闭即停止，避免空闲开销。
 *
 * 静态信息（硬件型号/OS/主机名）只在首次启用时取一次并缓存，后续每秒只拉动态量。
 *
 * @param options.enabled 调用方额外开关（快捷小窗面板收起时传 `false`，窗口被隐藏但未失焦时也能停表）。
 * @param options.trackHistory 是否需要 60 点历史缓冲（面板不需要）。
 */
export function useSystemMonitor({
  enabled = true,
  trackHistory = true,
}: UseSystemMonitorOptions = {}) {
  const [dynamic, setDynamic] = useState<DynamicSnapshot | null>(null);
  const [statics, setStatics] = useState<SystemStatics | null>(null);
  const [history, setHistory] = useState<MonitorHistory>(createHistory);
  const sampleRef = useRef(0);
  const loggedErrorRef = useRef(false);
  const staticsRef = useRef<SystemStatics | null>(null);

  const reportError = (what: string, cause: unknown) => {
    // 失败只记一次，避免每秒刷日志。
    if (loggedErrorRef.current) return;
    loggedErrorRef.current = true;
    void logError(`${what}：${String(cause)}`);
  };

  // 静态信息只取一次：面板收起再展开、页面来回切换都复用缓存。
  useEffect(() => {
    if (!enabled || staticsRef.current) return;
    let active = true;

    void ipc
      .getSystemStatics()
      .then((next) => {
        if (!active) return;
        staticsRef.current = next;
        setStatics(next);
      })
      .catch((cause) => {
        if (active) reportError("读取系统硬件信息失败", cause);
      });

    return () => {
      active = false;
    };
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    let timer: number | null = null;

    const poll = async () => {
      try {
        const next = await ipc.getSystemSnapshot();
        setDynamic(next);
        if (!trackHistory) return;

        sampleRef.current += 1;
        const x = sampleRef.current;

        setHistory((prev) => ({
          xs: append(prev.xs, x),
          cpu: append(prev.cpu, next.cpuUsage),
          mem: append(prev.mem, next.memTotal > 0 ? (next.memUsed / next.memTotal) * 100 : 0),
          rx: append(prev.rx, next.netRxPerSec),
          tx: append(prev.tx, next.netTxPerSec),
        }));
      } catch (cause) {
        reportError("读取系统监控数据失败", cause);
      }
    };

    const start = () => {
      if (timer !== null) return;
      void poll();
      timer = window.setInterval(() => void poll(), POLL_INTERVAL);
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
  }, [enabled, trackHistory]);

  const snapshot: SystemSnapshot | null =
    dynamic && statics ? { ...statics, ...dynamic } : null;

  return { snapshot, history };
}