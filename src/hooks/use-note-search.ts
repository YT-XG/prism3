import { useEffect, useState } from "react";

import { ipc, type Note } from "@/lib/ipc";

/** 输入防抖（毫秒）。 */
const DEBOUNCE_MS = 120;

/**
 * 速记搜索：关键词变化 → 防抖 → `search_notes`。空关键词返回空数组（不查询）。
 * 与 `useFileSearch` 同一套路；用 `active` 标志丢弃过期响应。
 */
export function useNoteSearch(keyword: string, limit = 5): Note[] {
  const [notes, setNotes] = useState<Note[]>([]);

  useEffect(() => {
    const trimmed = keyword.trim();
    if (!trimmed) {
      setNotes([]);
      return;
    }
    let active = true;
    const timer = window.setTimeout(() => {
      void ipc
        .searchNotes(trimmed, limit)
        .then((list) => {
          if (active) setNotes(list);
        })
        .catch(() => {
          if (active) setNotes([]);
        });
    }, DEBOUNCE_MS);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [keyword, limit]);

  return notes;
}
