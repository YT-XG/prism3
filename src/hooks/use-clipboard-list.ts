import { useCallback, useEffect, useRef, useState } from "react";

import { error as logError } from "@tauri-apps/plugin-log";
import { toast } from "sonner";

import { useClipboardState } from "@/hooks/use-clipboard-state";
import { ipc, type ClipEntryMeta, type ClipKindFilter } from "@/lib/ipc";

/** 输入防抖：按键到发起查询之间的等待（纯内存扫描，本身是毫秒级）。 */
const DEBOUNCE_MS = 120;

/** 一条给用户的反馈（成功 / 提示 / 失败）。 */
export interface ClipboardFeedback {
  tone: "ok" | "info" | "error";
  text: string;
}

interface UseClipboardListOptions {
  /** 是否启用（面板收起 / 不在剪贴板页时置 false，避免无意义查询）。 */
  enabled: boolean;
  query: string;
  kind: ClipKindFilter;
  /** 返回条数上限，由 Rust 侧再夹到硬上限。 */
  limit?: number;
  /**
   * 反馈通道。不传就按主窗口的习惯弹 toast；快捷小窗传自己的一行内联提示——
   * **面板点一条就收起，toast 会跟着面板一起消失**，等于没提示。
   */
  onFeedback?: (feedback: ClipboardFeedback) => void;
}

/**
 * 剪贴板历史的数据源：主窗口剪贴板页与快捷小窗剪贴板页共用。
 *
 * 关键词 / 筛选变化 → 防抖 → `search_clipboard`；另外订阅状态事件，
 * `revision` 变化（采集到新内容 / 编辑 / 删除 / 清空）后自动重查一次。
 * 用请求序号丢弃过期响应，避免慢查询覆盖新结果。
 */
export function useClipboardList({
  enabled,
  query,
  kind,
  limit,
  onFeedback,
}: UseClipboardListOptions) {
  const [entries, setEntries] = useState<ClipEntryMeta[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [loading, setLoading] = useState(false);

  const requestRef = useRef(0);
  /** 用 ref 持有回调：它不必参与依赖，也不会因为父组件重渲染而让动作失效。 */
  const feedbackRef = useRef(onFeedback);
  useEffect(() => {
    feedbackRef.current = onFeedback;
  }, [onFeedback]);

  const state = useClipboardState();
  const revision = state?.revision ?? 0;

  const notify = useCallback((feedback: ClipboardFeedback) => {
    const handler = feedbackRef.current;
    if (handler) {
      handler(feedback);
      return;
    }
    if (feedback.tone === "error") toast.error(feedback.text);
    else if (feedback.tone === "info") toast.info(feedback.text);
    else toast.success(feedback.text);
  }, []);

  useEffect(() => {
    if (!enabled) return;

    const request = ++requestRef.current;
    setLoading(true);

    const timer = window.setTimeout(() => {
      void ipc
        .searchClipboard(query, kind, limit)
        .then((next) => {
          if (request !== requestRef.current) return;
          setEntries(next);
          setActiveIndex(0);
        })
        .catch((cause) => {
          if (request !== requestRef.current) return;
          void logError(`查询剪贴板历史失败：${String(cause)}`);
        })
        .finally(() => {
          if (request === requestRef.current) setLoading(false);
        });
    }, DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
    // `revision` 变化 = 历史刚被改写（采集 / 编辑 / 删除），此时重查一次拿新列表。
  }, [enabled, query, kind, limit, revision]);

  /** 上下移动选中项（循环）。 */
  const moveActive = useCallback(
    (delta: number) => {
      setActiveIndex((current) => {
        if (entries.length === 0) return 0;
        return (current + delta + entries.length) % entries.length;
      });
    },
    [entries.length],
  );

  /**
   * 写入剪贴板 + 还原上一个焦点并粘贴（是否真的粘贴由后端配置决定）。
   *
   * 返回结果供调用方决定是否要收起面板——**没粘上时内容也已经在剪贴板里**，
   * 因此这里只提示原因，绝不静默失败。
   */
  const paste = useCallback(
    async (id: number) => {
      try {
        const outcome = await ipc.pasteClipboardEntry(id);
        if (outcome.pasted) {
          notify({ tone: "ok", text: "已粘贴" });
        } else if (outcome.reason) {
          notify({ tone: "info", text: outcome.reason });
        } else {
          notify({ tone: "ok", text: "已复制到剪贴板" });
        }
        return outcome;
      } catch (cause) {
        notify({ tone: "error", text: `粘贴失败：${String(cause)}` });
        return null;
      }
    },
    [notify],
  );

  /** 只复制，不粘贴。 */
  const copy = useCallback(
    async (id: number) => {
      try {
        await ipc.copyClipboardEntry(id);
        notify({ tone: "ok", text: "已复制到剪贴板" });
      } catch (cause) {
        notify({ tone: "error", text: `复制失败：${String(cause)}` });
      }
    },
    [notify],
  );

  /** 删除一条记录。 */
  const remove = useCallback(
    async (id: number) => {
      try {
        await ipc.deleteClipboardEntry(id);
      } catch (cause) {
        notify({ tone: "error", text: `删除失败：${String(cause)}` });
      }
    },
    [notify],
  );

  /** 收藏 / 取消收藏一条记录（收藏的不会被自动清理，也不被「清空历史」删掉）。 */
  const togglePin = useCallback(
    async (id: number, pinned: boolean) => {
      try {
        await ipc.setClipboardPinned(id, pinned);
      } catch (cause) {
        notify({ tone: "error", text: `操作失败：${String(cause)}` });
      }
    },
    [notify],
  );

  return {
    entries,
    loading,
    state,
    activeIndex,
    setActiveIndex,
    moveActive,
    paste,
    copy,
    remove,
    togglePin,
  };
}
