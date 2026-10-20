import { useState } from "react";

import { ClipboardPaste, Search, Star, Type, X } from "lucide-react";

import { ClipboardImagePreview } from "@/components/clipboard/clipboard-image-preview";
import { ClipboardThumb } from "@/components/clipboard/clipboard-thumb";
import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { Input } from "@/components/ui/input";
import { useClipboardList } from "@/hooks/use-clipboard-list";
import { formatTimeAgo } from "@/lib/format";
import { useAppStore } from "@/stores/app-store";

/** 空关键词时首页只露最近的几条，完整列表在「剪贴板」页。 */
const ENTRY_LIMIT = 4;
/** 有关键词时多给几条（首页卡片空间有限，仍给上限）。 */
const SEARCH_LIMIT = 8;

/** 首页「剪贴板」卡片：最近记录，点击复制、悬停可粘贴，收藏项置顶；带关键词搜索框。 */
export function ClipboardCard() {
  const config = useAppStore((state) => state.config);
  const setActiveView = useAppStore((state) => state.setActiveView);

  const [query, setQuery] = useState("");
  /** 正在悬停预览的图片记录（缩略图太小看不清，悬停时放大）；`anchor` 是被悬停那一行的位置。 */
  const [preview, setPreview] = useState<{ id: number; anchor: DOMRect } | null>(null);
  const enabled = config?.clipboardHistory ?? false;
  const searching = query.trim().length > 0;

  const { entries, state, paste, copy } = useClipboardList({
    enabled,
    query,
    kind: "all",
    limit: searching ? SEARCH_LIMIT : ENTRY_LIMIT,
  });

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      {enabled && (
        <div className="relative shrink-0">
          <Search className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            spellCheck={false}
            placeholder="搜索记录…"
            onChange={(event) => setQuery(event.target.value)}
            className="h-7 pr-6 pl-7 text-xs"
          />
          {query && (
            <button
              type="button"
              aria-label="清空搜索"
              className="absolute top-1/2 right-1.5 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              onClick={() => setQuery("")}
            >
              <X className="size-3.5" />
            </button>
          )}
        </div>
      )}

      {!enabled ? (
        <HomeCardHint>
          剪贴板历史未开启。开启后会记录复制的文本与图片，支持收藏与按保留天数清理。
        </HomeCardHint>
      ) : entries.length === 0 ? (
        <HomeCardHint>
          {searching
            ? `没有匹配「${query.trim()}」的记录`
            : "还没有记录。复制一段文字或一张图片后会自动出现在这里。"}
        </HomeCardHint>
      ) : (
        <ul className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto">
          {entries.map((entry) => (
            <li
              key={entry.id}
              className="group flex items-center gap-1 rounded-md pr-1 transition-colors hover:bg-muted/70"
              // 悬停图片记录即放大预览（文本记录不预览）；浮层挂在 body 上，见组件内注释。
              onMouseEnter={
                entry.kind === "image"
                  ? (event) =>
                      setPreview({
                        id: entry.id,
                        anchor: event.currentTarget.getBoundingClientRect(),
                      })
                  : undefined
              }
              onMouseLeave={entry.kind === "image" ? () => setPreview(null) : undefined}
            >
              <button
                type="button"
                title={`复制：${entry.preview}`}
                onClick={() => void copy(entry.id)}
                className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1.5 py-1 text-left focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                {entry.kind === "image" ? (
                  // 图片条目直接给缩略图（懒加载 + 跨列表缓存，图片经命令回传 data URL）
                  <ClipboardThumb id={entry.id} className="size-6" />
                ) : (
                  <Type className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                )}
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1">
                    <span className="truncate text-xs">{entry.preview || "（空）"}</span>
                    {entry.pinned && (
                      <Star
                        className="size-2.5 shrink-0 fill-brand text-brand"
                        aria-label="已收藏"
                      />
                    )}
                  </span>
                  <span className="block text-[0.65rem] text-muted-foreground">
                    {entry.kind === "image" &&
                      `${entry.imageWidth ?? 0}×${entry.imageHeight ?? 0} · `}
                    {formatTimeAgo(entry.createdAt)}
                  </span>
                </span>
              </button>

              <button
                type="button"
                title="粘贴到上一个窗口"
                aria-label="粘贴到上一个窗口"
                onClick={() => void paste(entry.id)}
                className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                <ClipboardPaste className="size-3" />
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2">
        <span className="truncate text-[0.65rem] text-muted-foreground">
          {enabled
            ? `${state?.count ?? 0} 条${state?.pinnedCount ? ` · 收藏 ${state.pinnedCount}` : ""}`
            : "未启用"}
        </span>
        {enabled ? (
          <HomeCardLink onClick={() => setActiveView("clipboard")}>全部记录</HomeCardLink>
        ) : (
          <HomeCardLink onClick={() => setActiveView("settings")}>去设置</HomeCardLink>
        )}
      </div>

      {preview && <ClipboardImagePreview id={preview.id} anchor={preview.anchor} />}
    </div>
  );
}
