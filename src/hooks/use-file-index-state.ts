import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type FileIndexState } from "@/lib/ipc";

/** 索引状态广播事件名，与 Rust `file_index::SEARCH_INDEX_EVENT` 一致。 */
export const FILE_INDEX_EVENT = "search:index-state";

/**
 * 订阅文件索引状态：主窗口搜索页、快捷小窗搜索页与设置卡片共用。
 *
 * 先订阅事件（后续变化），再 `get_file_index_state` 兜底拉一次——事件不缓存，
 * 窗口挂载 / 热重载时可能已错过最后一次广播（沿用 `useAppStatus` 的做法）。
 */
export function useFileIndexState(enabled: boolean): FileIndexState | null {
  const [state, setState] = useState<FileIndexState | null>(null);

  useEffect(() => {
    if (!enabled) return;

    let active = true;
    let unlisten: (() => void) | undefined;

    void (async () => {
      const off = await listen<FileIndexState>(FILE_INDEX_EVENT, (event) => {
        if (active) setState(event.payload);
      });
      if (!active) {
        off();
        return;
      }
      unlisten = off;

      const current = await ipc.getFileIndexState();
      if (active) setState(current);
    })().catch((cause) => {
      void logError(`读取文件索引状态失败：${String(cause)}`);
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [enabled]);

  return state;
}
