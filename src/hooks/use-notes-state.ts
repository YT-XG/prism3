import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";

import type { NotesChanged } from "@/lib/ipc";

/** 速记数据变化的事件名，与 Rust `commands::NOTES_EVENT` 一致。 */
export const NOTES_EVENT = "notes:changed";

/**
 * 速记变更订阅（主窗口速记页 / 首页卡片 / 快捷小窗速记页共用）。
 *
 * 事件不缓存，窗口挂载或热重载时可能已错过最后一次广播，因此**先订阅再兜底拉取**
 * （兜底用 `listNotes` 的空返回只为触发一次 reload）。
 */
export function useNotesRevision(): number {
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<NotesChanged>(NOTES_EVENT, (event) => {
      if (active) setRevision(event.payload.revision);
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅速记变化失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return revision;
}
