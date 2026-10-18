import { useEffect, useState } from "react";

import { Eraser, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { formatBytes } from "@/lib/format";
import { ipc, type ClipState } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** 二次确认的自动回退时间：走开一会儿就当作放弃，不必真的点「取消」。 */
const CONFIRM_RESET_MS = 4000;

interface ClipboardStatusProps {
  state: ClipState | null;
  /** 紧凑模式（面板窄）：只留状态文字 + 一个「清空」，不铺两个按钮。 */
  compact?: boolean;
  className?: string;
}

/**
 * 剪贴板历史状态行（主窗口剪贴板页 / 快捷小窗剪贴板页 / 设置卡片共用）。
 *
 * 状态来自事件广播 `clipboard:state`，窗口挂载时另有 `get_clipboard_state` 兜底，
 * 因此这里只渲染，不自己轮询。
 *
 * 「清空历史」是**不可撤销**的，因此走一次就地二次确认（按钮自身变成「确认清空」，
 * 4 秒不点自动回退），不弹额外对话框——面板窗口只有 520×520，能少一层就少一层。
 */
export function ClipboardStatus({ state, compact = false, className }: ClipboardStatusProps) {
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    if (!confirming) return;
    const timer = window.setTimeout(() => setConfirming(false), CONFIRM_RESET_MS);
    return () => window.clearTimeout(timer);
  }, [confirming]);

  // 历史空了 / 功能被关掉，确认态自然失效。
  const canAct = (state?.enabled ?? false) && (state?.count ?? 0) > 0;
  useEffect(() => {
    if (!canAct) setConfirming(false);
  }, [canAct]);

  const prune = async () => {
    try {
      await ipc.pruneClipboardHistory();
      toast.success("已按保留天数清理过期记录");
    } catch (cause) {
      toast.error(`清理失败：${String(cause)}`);
    }
  };

  const clear = async () => {
    try {
      await ipc.clearClipboardHistory();
      // 收藏的记录会被保留，因此文案跟着状态走（别让用户以为「什么都没了」）。
      toast.success(
        (state?.pinnedCount ?? 0) > 0 ? "已清空未收藏的记录（收藏的已保留）" : "已清空剪贴板历史",
      );
    } catch (cause) {
      toast.error(`清空失败：${String(cause)}`);
    } finally {
      setConfirming(false);
    }
  };

  return (
    <div className={cn("flex flex-wrap items-center gap-x-2 gap-y-1 text-xs", className)}>
      <span className="text-muted-foreground">{describe(state)}</span>

      {canAct && !compact && (
        <Button variant="ghost" size="xs" onClick={() => void prune()}>
          <Eraser data-icon="inline-start" />
          清理过期
        </Button>
      )}

      {canAct && (
        <Button
          variant={confirming ? "destructive" : "ghost"}
          size="xs"
          onClick={() => (confirming ? void clear() : setConfirming(true))}
        >
          <Trash2 data-icon="inline-start" />
          {confirming ? "确认清空" : "清空历史"}
        </Button>
      )}
    </div>
  );
}

function describe(state: ClipState | null): string {
  if (!state) return "正在读取剪贴板历史…";
  if (!state.enabled) return "剪贴板历史未启用";

  const retention = state.retentionDays === 0 ? "永久保留" : `保留 ${state.retentionDays} 天`;
  if (state.count === 0) return `暂无记录 · ${retention}`;

  const pinned = state.pinnedCount > 0 ? ` · 收藏 ${state.pinnedCount}` : "";
  return `${state.count} 条${pinned} · ${retention} · 占用 ${formatBytes(state.storageBytes)}`;
}
