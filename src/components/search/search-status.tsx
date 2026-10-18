import { RefreshCw, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { formatTimeAgo } from "@/lib/format";
import type { FileIndexState } from "@/lib/ipc";
import { cn } from "@/lib/utils";

interface SearchIndexStatusProps {
  state: FileIndexState | null;
  /** 传入则显示「重建索引」按钮。 */
  onRebuild?: () => void;
  /** 传入则扫描中显示「取消」按钮。 */
  onCancel?: () => void;
  className?: string;
}

/**
 * 索引状态行（主窗口搜索页 / 快捷小窗搜索页 / 设置卡片共用）。
 *
 * 状态来自事件广播 `search:index-state`，窗口挂载时另有 `get_file_index_state` 兜底，
 * 因此这里只需渲染，不自己轮询。
 */
export function SearchIndexStatus({
  state,
  onRebuild,
  onCancel,
  className,
}: SearchIndexStatusProps) {
  const scanning = state?.phase === "scanning";

  return (
    <div className={cn("flex flex-wrap items-center gap-x-2 gap-y-1 text-xs", className)}>
      <span className="text-muted-foreground">{describe(state)}</span>

      {state?.truncated && (
        <span className="text-amber-600 dark:text-amber-500">
          已达条目上限，部分目录未索引
        </span>
      )}
      {state?.missingRoots.length ? (
        <span className="text-amber-600 dark:text-amber-500" title={state.missingRoots.join("\n")}>
          {state.missingRoots.length} 个索引目录已不存在
        </span>
      ) : null}

      {scanning && onCancel && (
        <Button variant="ghost" size="xs" onClick={onCancel}>
          <X data-icon="inline-start" />
          取消
        </Button>
      )}
      {!scanning && onRebuild && state?.enabled && (
        <Button variant="ghost" size="xs" onClick={onRebuild}>
          <RefreshCw data-icon="inline-start" />
          重建索引
        </Button>
      )}
    </div>
  );
}

function describe(state: FileIndexState | null): string {
  if (!state) return "正在读取索引状态…";
  if (!state.enabled) return "文件搜索未启用";

  switch (state.phase) {
    case "loading":
      return "正在载入上次的索引…";
    case "scanning":
      return state.indexed > 0
        ? `正在后台刷新索引（当前可用 ${state.indexed.toLocaleString()} 项，已扫描 ${state.scanned.toLocaleString()} 项）`
        : `正在建立索引：已扫描 ${state.scanned.toLocaleString()} 项`;
    case "ready": {
      const updated = state.builtAtSecs ? `更新于 ${formatTimeAgo(state.builtAtSecs)}` : "尚未建成";
      const stale = state.stale ? " · 后台刷新中" : "";
      return `已索引 ${state.indexed.toLocaleString()} 项 · ${updated}${stale}`;
    }
    case "error":
      return state.error ?? "索引构建失败";
    default:
      return "文件搜索未启用";
  }
}
