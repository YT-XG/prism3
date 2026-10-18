import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type ClipState } from "@/lib/ipc";

/** 剪贴板历史状态变化的事件名，与 Rust `clipboard::CLIPBOARD_EVENT` 一致。 */
export const CLIPBOARD_EVENT = "clipboard:state";

/**
 * 剪贴板历史状态订阅（主窗口剪贴板页 / 设置卡片 / 快捷小窗剪贴板页共用）。
 *
 * 与 `useFileIndexState` 同一套路：**先订阅再兜底拉取**——事件不缓存，
 * 窗口挂载或热重载时可能已经错过最后一次广播。
 */
export function useClipboardState(): ClipState | null {
  const [state, setState] = useState<ClipState | null>(null);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<ClipState>(CLIPBOARD_EVENT, (event) => {
      if (active) setState(event.payload);
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
        return ipc.getClipboardState();
      })
      .then((current) => {
        if (active) setState(current);
      })
      .catch((cause) => {
        void logError(`订阅剪贴板历史状态失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return state;
}
