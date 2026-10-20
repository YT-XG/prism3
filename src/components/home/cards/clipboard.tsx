import { ClipboardPaste, Image as ImageIcon, Star, Type } from "lucide-react";

import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { useClipboardList } from "@/hooks/use-clipboard-list";
import { formatTimeAgo } from "@/lib/format";
import { useAppStore } from "@/stores/app-store";

/** 首页只露最近的几条，完整列表在「剪贴板」页。 */
const ENTRY_LIMIT = 4;

/** 首页「剪贴板」卡片：最近记录，点击复制、悬停可粘贴，收藏项置顶（与完整页同一数据源）。 */
export function ClipboardCard() {
  const config = useAppStore((state) => state.config);
  const setActiveView = useAppStore((state) => state.setActiveView);

  const enabled = config?.clipboardHistory ?? false;
  const { entries, state, paste, copy } = useClipboardList({
    enabled,
    query: "",
    kind: "all",
    limit: ENTRY_LIMIT,
  });

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      {!enabled ? (
        <HomeCardHint>
          剪贴板历史未开启。开启后会记录复制的文本与图片，支持收藏与按保留天数清理。
        </HomeCardHint>
      ) : entries.length === 0 ? (
        <HomeCardHint>还没有记录。复制一段文字或一张图片后会自动出现在这里。</HomeCardHint>
      ) : (
        <ul className="flex min-h-0 flex-1 flex-col gap-0.5">
          {entries.map((entry) => (
            <li
              key={entry.id}
              className="group flex items-center gap-1 rounded-md pr-1 transition-colors hover:bg-muted/70"
            >
              <button
                type="button"
                title={`复制：${entry.preview}`}
                onClick={() => void copy(entry.id)}
                className="flex min-w-0 flex-1 items-center gap-1.5 rounded-md px-1.5 py-1 text-left focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                {entry.kind === "image" ? (
                  <ImageIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
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
    </div>
  );
}
