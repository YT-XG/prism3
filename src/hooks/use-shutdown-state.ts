import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";

import { ipc, type ShutdownState } from "@/lib/ipc";

/** 与 Rust 侧 `power::SHUTDOWN_EVENT` 一致。 */
const SHUTDOWN_EVENT = "shutdown:state";

/**
 * 定时关机状态 + 本地倒计时。
 *
 * 状态以 Rust 侧为准（`get_shutdown_state` 兜底 + `shutdown:state` 事件），
 * 剩余秒数在前端按截止时刻每秒重算——不需要后端推送秒数。
 */
export function useShutdownState() {
  const [state, setState] = useState<ShutdownState>({
    active: false,
    deadlineSecs: null,
    mode: null,
    dailyTime: null,
    weekdays: [],
  });
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<ShutdownState>(SHUTDOWN_EVENT, (event) => {
      if (active) setState(event.payload);
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch(() => {
        // 订不到事件不影响兜底拉取，下面还会主动读一次。
      });

    void ipc
      .getShutdownState()
      .then((next) => {
        if (active) setState(next);
      })
      .catch(() => {
        // 读不到就按空闲显示，不影响页面其余部分。
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!state.active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [state.active, state.deadlineSecs]);

  const remainingSecs =
    state.active && state.deadlineSecs != null
      ? Math.max(0, state.deadlineSecs - Math.floor(now / 1000))
      : null;

  // 倒计时归零后再问一次后端：让「已过期」立刻回到空闲态，而不是停在 00:00。
  useEffect(() => {
    if (!state.active || remainingSecs !== 0) return;
    void ipc
      .getShutdownState()
      .then((next) => setState(next))
      .catch(() => {
        // 读不到就保持现状，下次读取到截止时刻过期时后端会自行清空。
      });
  }, [state.active, remainingSecs]);

  return { state, remainingSecs };
}
