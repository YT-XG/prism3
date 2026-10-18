import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";

import { ClipboardList as ClipboardListIcon, Info, Search, X } from "lucide-react";

import { ClipboardEditDialog } from "@/components/clipboard/clipboard-edit-dialog";
import { ClipboardList } from "@/components/clipboard/clipboard-list";
import { ClipboardStatus } from "@/components/clipboard/clipboard-status";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useClipboardList } from "@/hooks/use-clipboard-list";
import type { ClipEntryMeta, ClipKindFilter } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";

/** 主窗口一屏最多展示多少条（Rust 侧另有硬上限）。 */
const RESULT_LIMIT = 100;

/**
 * 主窗口「剪贴板」页：完整形态的历史列表 + 类型筛选。
 *
 * 页面只在被选中时挂载（`AppShell` 按视图分支渲染），因此查询天然「离开页面即停」。
 */
export function ClipboardPanel() {
  const config = useAppStore((state) => state.config);
  const setActiveView = useAppStore((state) => state.setActiveView);

  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<ClipKindFilter>("all");
  const [editingId, setEditingId] = useState<number | null>(null);

  const {
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
  } = useClipboardList({ enabled: true, query, kind, limit: RESULT_LIMIT });

  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // ↑↓ 选中项始终滚进可视区。
  useEffect(() => {
    listRef.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, entries]);

  const enabled = state?.enabled ?? config?.clipboardHistory ?? true;
  const pasteOnClick = state?.pasteOnClick ?? config?.clipboardPasteOnClick ?? true;

  const handlePaste = useCallback(
    (entry: ClipEntryMeta) => {
      void paste(entry.id);
    },
    [paste],
  );

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const active = entries[activeIndex];
    const hasSelection =
      event.currentTarget.selectionStart !== null &&
      event.currentTarget.selectionStart !== event.currentTarget.selectionEnd;

    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveActive(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(-1);
    } else if (event.key === "Enter" && active) {
      event.preventDefault();
      handlePaste(active);
    } else if (event.key === "Delete" && active) {
      event.preventDefault();
      void remove(active.id);
    } else if (event.key === "Escape") {
      setQuery("");
      inputRef.current?.focus();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c") {
      // 有文本选中时让浏览器照常复制选中内容，否则把当前记录复制到系统剪贴板。
      if (!hasSelection && active) {
        event.preventDefault();
        void copy(active.id);
      }
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h1 className="font-heading text-lg font-medium">剪贴板</h1>
        <p className="text-sm text-muted-foreground">
          记录复制过的文本与图片。点击一条记录
          {pasteOnClick ? "即可回到上一个窗口并粘贴" : "即可复制到系统剪贴板"}
          ；↑↓ 选择 · Enter {pasteOnClick ? "粘贴" : "复制"} · Ctrl+C 仅复制 · Delete 删除。
        </p>
      </div>

      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={inputRef}
          value={query}
          autoFocus
          spellCheck={false}
          placeholder={enabled ? "搜索剪贴板里的文本…" : "剪贴板历史未启用"}
          disabled={!enabled}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={handleKeyDown}
          className="h-10 pr-9 pl-9"
        />
        {query && (
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="清空"
            onClick={() => {
              setQuery("");
              inputRef.current?.focus();
            }}
            className="absolute top-1/2 right-2 -translate-y-1/2"
          >
            <X />
          </Button>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <Tabs value={kind} onValueChange={(value) => setKind(value as ClipKindFilter)}>
          <TabsList>
            <TabsTrigger value="all">全部</TabsTrigger>
            <TabsTrigger value="text">文本</TabsTrigger>
            <TabsTrigger value="image">图片</TabsTrigger>
          </TabsList>
        </Tabs>
        <ClipboardStatus state={state} />
      </div>

      {!enabled ? (
        <Card>
          <CardHeader>
            <CardTitle>剪贴板历史还没有开启</CardTitle>
            <CardDescription>
              开启后会在后台记录复制过的文本与图片（只存在本机，不联网）；保留天数可自行设置。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Button variant="outline" onClick={() => setActiveView("settings")}>
              <Info data-icon="inline-start" />
              前往设置开启
            </Button>
          </CardContent>
        </Card>
      ) : (
        <div
          ref={listRef}
          className="max-h-[26rem] overflow-y-auto rounded-lg border border-border/60 p-1.5"
        >
          <ClipboardList
            entries={entries}
            activeIndex={activeIndex}
            onActivate={setActiveIndex}
            onPaste={handlePaste}
            onCopy={(entry) => void copy(entry.id)}
            onEdit={(entry) => setEditingId(entry.id)}
            onDelete={(entry) => void remove(entry.id)}
            onTogglePin={(entry) => void togglePin(entry.id, !entry.pinned)}
            empty={
              loading
                ? "读取中…"
                : query.trim()
                  ? `没有匹配「${query.trim()}」的记录`
                  : kind === "image"
                    ? "还没有记录过图片（复制截图或图片文件即可）"
                    : "还没有记录，复制点什么试试"
            }
          />
        </div>
      )}

      {!enabled && (
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <ClipboardListIcon className="size-3.5" aria-hidden />
          记录只保存在本机；文本可编辑，点击可回到上一个窗口并粘贴。
        </p>
      )}

      <ClipboardEditDialog id={editingId} onClose={() => setEditingId(null)} />
    </div>
  );
}
