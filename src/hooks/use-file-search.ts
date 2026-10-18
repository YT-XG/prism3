import { useCallback, useEffect, useRef, useState } from "react";

import { error as logError } from "@tauri-apps/plugin-log";
import { toast } from "sonner";

import { useFileIndexState } from "@/hooks/use-file-index-state";
import { ipc, type OpenAction, type SearchHit } from "@/lib/ipc";

/** 输入防抖：按键到发起查询之间的等待（索引查询本身是毫秒级）。 */
const DEBOUNCE_MS = 120;

interface UseFileSearchOptions {
  /** 是否启用（面板收起 / 不在搜索页时置 false，避免无意义查询）。 */
  enabled: boolean;
  /** 返回条数上限，由 Rust 侧再夹到 60。 */
  limit?: number;
  /** 成功打开一条结果后的回调（快捷小窗用它收起面板）。 */
  onOpened?: () => void;
}

/**
 * 文件搜索的数据源：主窗口搜索页与快捷小窗搜索页共用。
 *
 * 关键词变化 → 防抖 → `search_files`；用请求序号丢弃过期响应，避免慢查询覆盖新结果。
 * 空关键词也会发查询：Rust 侧此时返回「最近修改的文件」，正好用作初始列表。
 * 索引建成（`indexed` 变化）后自动重查一次，这样扫描结束时列表自己是新的。
 */
export function useFileSearch({ enabled, limit, onOpened }: UseFileSearchOptions) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [searching, setSearching] = useState(false);

  const requestRef = useRef(0);
  const onOpenedRef = useRef(onOpened);
  onOpenedRef.current = onOpened;

  const indexState = useFileIndexState(enabled);
  const indexedCount = indexState?.indexed ?? 0;
  useEffect(() => {
    if (!enabled) return;

    const request = ++requestRef.current;
    setSearching(true);

    const timer = window.setTimeout(() => {
      void ipc
        .searchFiles(query, limit)
        .then((next) => {
          if (request !== requestRef.current) return;
          setHits(next);
          setActiveIndex(0);
        })
        .catch((cause) => {
          if (request !== requestRef.current) return;
          void logError(`文件搜索失败：${String(cause)}`);
        })
        .finally(() => {
          if (request === requestRef.current) setSearching(false);
        });
    }, DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
    // `indexedCount` 变化 = 索引刚建成 / 刚重建，此时重查一次拿新结果。
  }, [enabled, limit, query, indexedCount]);

  /** 打开或定位一条结果。 */
  const activate = useCallback(async (hit: SearchHit, action: OpenAction = "open") => {
    try {
      await ipc.openSearchResult(hit.path, action);
      onOpenedRef.current?.();
    } catch (cause) {
      toast.error(`打开失败：${String(cause)}`);
    }
  }, []);

  /** 复制完整路径。 */
  const copyPath = useCallback(async (hit: SearchHit) => {
    try {
      await navigator.clipboard.writeText(hit.path);
      toast.success("已复制完整路径");
    } catch (cause) {
      toast.error(`复制失败：${String(cause)}`);
    }
  }, []);

  const rebuild = useCallback(async () => {
    try {
      await ipc.rebuildFileIndex();
      toast.success("已开始重建索引");
    } catch (cause) {
      toast.error(`重建失败：${String(cause)}`);
    }
  }, []);

  const cancelScan = useCallback(async () => {
    try {
      await ipc.cancelFileIndex();
    } catch (cause) {
      void logError(`取消索引构建失败：${String(cause)}`);
    }
  }, []);

  /** 上下移动选中项（循环）。 */
  const moveActive = useCallback(
    (delta: number) => {
      setActiveIndex((current) => {
        if (hits.length === 0) return 0;
        return (current + delta + hits.length) % hits.length;
      });
    },
    [hits.length],
  );

  return {
    query,
    setQuery,
    hits,
    searching,
    indexState,
    activeIndex,
    setActiveIndex,
    moveActive,
    activate,
    copyPath,
    rebuild,
    cancelScan,
  };
}
