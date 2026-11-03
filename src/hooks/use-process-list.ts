import { useCallback, useEffect, useState } from "react";

import { ipc, type ProcessInfo, type ProcessSort } from "@/lib/ipc";

/** 进程列表轮询间隔：3 秒足够看到变化，又不会让 sysinfo 一直占 CPU。 */
const POLL_INTERVAL = 3000;

interface UseProcessListOptions {
  /** 是否轮询；离开监控页或窗口隐藏时传 `false`。默认 `true`。 */
  enabled?: boolean;
  /** 最多返回多少条（后端上限 60）。默认 20。 */
  limit?: number;
}

/**
 * 监控页的进程列表：按 CPU / 内存排序，3 秒轮询一次。
 *
 * 只在监控页可见时运行；与 `useSystemMonitor` 一样在窗口隐藏时停表，
 * 避免后台弹 Toast 或托盘常驻时还反复枚举进程。
 */
export function useProcessList({ enabled = true, limit = 20 }: UseProcessListOptions = {}) {
  const [processes, setProcesses] = useState<ProcessInfo[]>([]);
  const [sort, setSort] = useState<ProcessSort>("cpu");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let timer: number | null = null;
    let active = true;

    const poll = async () => {
      try {
        const next = await ipc.getProcessList(sort, limit);
        if (!active) return;
        setProcesses(next);
        setError(null);
      } catch (cause) {
        if (active) setError(String(cause));
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
      active = false;
      stop();
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, [enabled, limit, sort]);

  /** 结束进程后立即从列表里移除，下一轮轮询仍以系统实际结果为准。 */
  const kill = useCallback(async (pid: number) => {
    await ipc.killProcess(pid);
    setProcesses((current) => current.filter((process) => process.pid !== pid));
  }, []);

  return { processes, sort, setSort, kill, error };
}
