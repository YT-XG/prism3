import type { KeyboardEvent } from "react";

import { FileText, Folder, Search } from "lucide-react";

import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { Input } from "@/components/ui/input";
import { useFileSearch } from "@/hooks/use-file-search";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/** 首页只展示前几条，避免把整页变成搜索页。 */
const RESULT_LIMIT = 5;

/** 首页「快速搜索」卡片：文件名搜索的紧凑版，Enter 打开、↑↓ 选择。 */
export function SearchCard() {
  const config = useAppStore((state) => state.config);
  const setActiveView = useAppStore((state) => state.setActiveView);

  const enabled = config?.fileSearch ?? false;
  const {
    query,
    setQuery,
    hits,
    searching,
    indexState,
    activeIndex,
    setActiveIndex,
    moveActive,
    activate,
  } = useFileSearch({ enabled, limit: RESULT_LIMIT });

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveActive(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(-1);
    } else if (event.key === "Enter") {
      const hit = hits[activeIndex];
      if (hit) {
        event.preventDefault();
        void activate(hit);
      }
    } else if (event.key === "Escape") {
      setQuery("");
    }
  };

  const indexLabel = indexState
    ? indexState.phase === "ready"
      ? `${indexState.indexed.toLocaleString("zh-CN")} 个文件`
      : `${indexState.scanned.toLocaleString("zh-CN")} 个条目`
    : "索引状态读取中…";

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="relative">
        <Search
          className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
          aria-hidden="true"
        />
        <Input
          value={query}
          spellCheck={false}
          disabled={!enabled}
          placeholder={enabled ? "搜索文件名…" : "文件搜索未启用"}
          aria-label="搜索文件名"
          className="h-8 pr-2 pl-8 text-xs"
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={handleKeyDown}
        />
      </div>

      {!enabled ? (
        <HomeCardHint>
          开启后会在后台为常用目录建立文件名索引，不联网、不读取文件内容。
        </HomeCardHint>
      ) : (
        <div className="min-h-0 flex-1">
          {hits.length === 0 ? (
            <HomeCardHint>
              {searching
                ? "搜索中…"
                : query.trim()
                  ? `没有匹配「${query.trim()}」的文件`
                  : "输入关键词；空关键词会列出最近修改的文件。"}
            </HomeCardHint>
          ) : (
            <ul className="flex flex-col gap-0.5">
              {hits.map((hit, index) => (
                <li key={hit.path}>
                  <button
                    type="button"
                    title={hit.path}
                    onMouseEnter={() => setActiveIndex(index)}
                    onFocus={() => setActiveIndex(index)}
                    onClick={() => void activate(hit)}
                    className={cn(
                      "flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left transition-colors focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                      index === activeIndex ? "bg-muted" : "hover:bg-muted/70",
                    )}
                  >
                    {hit.isDir ? (
                      <Folder className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                    ) : (
                      <FileText className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                    )}
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-xs font-medium">{hit.name}</span>
                      <span className="block truncate text-[0.65rem] text-muted-foreground">
                        {hit.parent || "—"}
                      </span>
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2">
        <span className="truncate text-[0.65rem] text-muted-foreground">{indexLabel}</span>
        {enabled ? (
          <HomeCardLink onClick={() => setActiveView("search")}>搜索页</HomeCardLink>
        ) : (
          <HomeCardLink onClick={() => setActiveView("settings")}>去设置</HomeCardLink>
        )}
      </div>
    </div>
  );
}
