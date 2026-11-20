import { ArrowRight, FileText, Folder, Search as SearchIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { useAppCatalog, useAppMeta } from "@/hooks/use-app-catalog";
import { usePresence } from "@/hooks/use-presence";
import { useFileSearch } from "@/hooks/use-file-search";
import type { SearchHit } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useAppStore, type AppView } from "@/stores/app-store";
import { useWindowStore } from "@/stores/window-store";

type Row = { kind: "app"; id: string } | { kind: "file"; hit: SearchHit };

/**
 * 桌面正中的搜索条：应用名 + 本机文件名一起搜。
 *
 * 参考站的搜索条是网址/搜索入口，这里换成「启动器」——回车打开第一个结果。
 * 文件结果复用 `useFileSearch`（只在聚焦且有词时启用，避免桌面空转查索引）。
 */
export function DesktopSearch() {
  const [query, setQuery] = useState("");
  const [focused, setFocused] = useState(false);
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const fileSearchEnabled = useAppStore((state) => state.config?.fileSearch ?? false);
  const setActiveView = useAppStore((state) => state.setActiveView);
  const openWindow = useWindowStore((state) => state.open);
  const catalog = useAppCatalog();

  const trimmed = query.trim();
  const { hits, indexState, activate } = useFileSearch({
    enabled: focused && fileSearchEnabled && trimmed.length > 0,
    limit: 5,
  });

  const appRows = useMemo(() => {
    const lower = trimmed.toLowerCase();
    const matched = catalog.filter((app) =>
      `${app.label}${app.title}${app.description}`.toLowerCase().includes(lower),
    );
    return (lower ? matched : catalog.slice(0, 4)).slice(0, 4);
  }, [catalog, trimmed]);

  const rows = useMemo<Row[]>(
    () => [
      ...appRows.map((app) => ({ kind: "app" as const, id: app.id })),
      ...hits.slice(0, 5).map((hit) => ({ kind: "file" as const, hit })),
    ],
    [appRows, hits],
  );

  useEffect(() => {
    setActive(0);
  }, [query]);

  const launch = (id: string) => {
    const meta = catalog.find((app) => app.id === id);
    if (!meta) return;
    if (meta.kind === "builtin") setActiveView(meta.id as AppView);
    else openWindow(meta.id, meta.size);
    setQuery("");
    inputRef.current?.blur();
    setFocused(false);
  };

  const openRow = (row: Row) => {
    if (row.kind === "app") {
      launch(row.id);
      return;
    }
    void activate(row.hit);
    setQuery("");
    inputRef.current?.blur();
    setFocused(false);
  };

  const panelOpen = focused && rows.length > 0;
  const panelPresence = usePresence(panelOpen, 130);
  const showPanel = panelPresence.rendered;

  return (
    <div
      className="absolute left-1/2 w-[min(560px,calc(100vw-460px))] -translate-x-1/2"
      style={{ top: "30%" }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          const row = rows[active];
          if (row) openRow(row);
        }}
        className={cn(
          "flex h-[54px] items-center gap-3 rounded-full border px-5 transition-all duration-200",
          "border-white/80 bg-white/78 shadow-[0_18px_44px_-24px_rgba(16,40,80,.55)] backdrop-blur-2xl",
          "dark:border-white/12 dark:bg-white/10",
          focused && "border-brand/45 bg-white/92 ring-4 ring-brand/12 dark:bg-white/14",
        )}
      >
        <SearchIcon className="size-[18px] shrink-0 text-muted-foreground" strokeWidth={2.1} />
        <input
          ref={inputRef}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onFocus={() => setFocused(true)}
          onBlur={() => window.setTimeout(() => setFocused(false), 120)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActive((current) => (rows.length ? (current + 1) % rows.length : 0));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((current) => (rows.length ? (current - 1 + rows.length) % rows.length : 0));
            } else if (event.key === "Escape") {
              event.preventDefault();
              setQuery("");
              inputRef.current?.blur();
            }
          }}
          placeholder="搜索应用或本机文件…"
          aria-label="搜索应用或本机文件"
          className="h-full min-w-0 flex-1 bg-transparent text-[15px] text-foreground placeholder:text-muted-foreground/80 focus:outline-none"
        />
        <button
          type="submit"
          aria-label="打开"
          className="flex size-[36px] shrink-0 items-center justify-center rounded-full bg-brand text-white shadow-[0_6px_14px_-6px_var(--brand)] transition-transform hover:scale-105 active:scale-95"
        >
          <ArrowRight className="size-[17px]" strokeWidth={2.4} />
        </button>
      </form>

      {showPanel ? (
        <div
          className={cn(
            "mt-2 overflow-hidden rounded-2xl border p-1.5",
            panelOpen ? "menu-in" : "pop-out",
            "border-white/80 bg-white/92 shadow-[0_28px_60px_-30px_rgba(16,40,80,.6)] backdrop-blur-2xl",
            "dark:border-white/12 dark:bg-[#1b1e26]/95",
          )}
        >
          {rows.map((row, index) => {
            const selected = index === active;
            if (row.kind === "app") {
              return (
                <AppRow
                  key={`app-${row.id}`}
                  id={row.id}
                  selected={selected}
                  onHover={() => setActive(index)}
                  onOpen={() => openRow(row)}
                />
              );
            }

            const hit = row.hit;
            const FileIcon = hit.isDir ? Folder : FileText;
            return (
              <button
                key={`file-${hit.path}`}
                type="button"
                onMouseEnter={() => setActive(index)}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => openRow(row)}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-xl px-2 py-1.5 text-left",
                  selected && "bg-brand/10",
                )}
              >
                <span className="flex size-[26px] items-center justify-center rounded-[8px] bg-black/5 text-muted-foreground dark:bg-white/10">
                  <FileIcon className="size-[14px]" strokeWidth={2} />
                </span>
                <span className="min-w-0">
                  <span className="block truncate text-[12.5px] font-medium text-foreground">
                    {hit.name}
                  </span>
                  <span className="block truncate text-[10.5px] text-muted-foreground">
                    {hit.parent}
                  </span>
                </span>
              </button>
            );
          })}

          <div className="mt-1 flex items-center justify-between border-t border-black/6 px-2 pt-1.5 pb-0.5 dark:border-white/8">
            <span className="text-[10px] text-muted-foreground">
              {fileSearchEnabled
                ? indexState
                  ? `已索引 ${indexState.indexed.toLocaleString()} 个文件`
                  : "索引准备中…"
                : "文件搜索已在设置里关闭"}
            </span>
            <span className="text-[10px] text-muted-foreground">↑↓ 选择 · Enter 打开 · Esc 关闭</span>
          </div>
        </div>
      ) : null}
    </div>
  );
}

/** 搜索结果里的一行应用：内置用 lucide 图标，自建用 emoji（标题栏 / 坞上一致）。 */
function AppRow({
  id,
  selected,
  onHover,
  onOpen,
}: {
  id: string;
  selected: boolean;
  onHover: () => void;
  onOpen: () => void;
}) {
  const app = useAppMeta(id);
  const Icon = app.icon;

  return (
    <button
      type="button"
      onMouseEnter={onHover}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onOpen}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-xl px-2 py-1.5 text-left",
        selected && "bg-brand/10",
      )}
    >
      <span
        className="flex size-[26px] items-center justify-center rounded-[8px] text-white"
        style={{ backgroundImage: `linear-gradient(150deg, ${app.tint[0]}, ${app.tint[1]})` }}
      >
        {Icon ? (
          <Icon className="size-[14px]" strokeWidth={2} />
        ) : (
          <span className="text-[12px] leading-none">{app.emoji ?? "📦"}</span>
        )}
      </span>
      <span className="text-[12.5px] font-medium text-foreground">{app.label}</span>
      <span className="truncate text-[11px] text-muted-foreground">{app.description}</span>
      <span className="ml-auto text-[10px] text-muted-foreground">打开</span>
    </button>
  );
}
