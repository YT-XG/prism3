import { useCallback, useEffect, useRef, type KeyboardEvent } from "react";

import { open as openDirectoryDialog } from "@tauri-apps/plugin-dialog";
import { FolderPlus, Info, Search, X } from "lucide-react";

import { SearchResultList } from "@/components/search/search-result-list";
import { SearchIndexStatus } from "@/components/search/search-status";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { useFileSearch } from "@/hooks/use-file-search";
import { useAppStore } from "@/stores/app-store";

import { toast } from "sonner";

/** 主窗口一屏最多展示多少条结果（Rust 侧硬上限 60）。 */
const RESULT_LIMIT = 40;

/**
 * 主窗口「搜索」页：完整形态的结果列表 + 索引范围管理。
 *
 * 页面只在被选中时挂载（`AppShell` 按视图分支渲染），因此查询天然「离开页面即停」。
 */
export function SearchPanel() {
  const config = useAppStore((state) => state.config);
  const saveConfig = useAppStore((state) => state.saveConfig);
  const setActiveView = useAppStore((state) => state.setActiveView);

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
    copyPath,
    rebuild,
    cancelScan,
  } = useFileSearch({ enabled: true, limit: RESULT_LIMIT });

  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // ↑↓ 选中项始终滚进可视区。
  useEffect(() => {
    listRef.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, hits]);

  const enabled = indexState?.enabled ?? config?.fileSearch ?? false;
  /** 自定义根为空 = 使用「桌面 / 文档 / 下载 / 图片」默认根。 */
  const customRoots = config?.fileSearchRoots ?? [];
  const effectiveRoots = customRoots.length > 0 ? customRoots : (indexState?.roots ?? []);

  const addRoot = useCallback(async () => {
    const picked = await openDirectoryDialog({
      directory: true,
      multiple: false,
      title: "选择要加入索引的目录",
    });
    if (typeof picked !== "string") return;

    // 首次添加时把默认根一并固化，避免「加了自定义目录却丢了常用目录」。
    const base = customRoots.length > 0 ? customRoots : (indexState?.roots ?? []);
    if (base.includes(picked)) {
      toast.info("该目录已在索引范围内");
      return;
    }
    try {
      await saveConfig({ fileSearchRoots: [...base, picked] });
      toast.success("已加入索引范围，正在重建索引");
    } catch (cause) {
      toast.error(`保存失败：${String(cause)}`);
    }
  }, [customRoots, indexState?.roots, saveConfig]);

  const removeRoot = useCallback(
    async (root: string) => {
      const next = effectiveRoots.filter((item) => item !== root);
      try {
        await saveConfig({ fileSearchRoots: next });
        toast.success("已移出索引范围，正在重建索引");
      } catch (cause) {
        toast.error(`保存失败：${String(cause)}`);
      }
    },
    [effectiveRoots, saveConfig],
  );

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const active = hits[activeIndex];
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
      void activate(active, event.shiftKey ? "reveal" : "open");
    } else if (event.key === "Escape") {
      setQuery("");
      inputRef.current?.focus();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c") {
      // 有文本选中时让浏览器照常复制选中内容，否则复制当前结果路径。
      if (!hasSelection && active) {
        event.preventDefault();
        void copyPath(active);
      }
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h1 className="font-heading text-lg font-medium">文件搜索</h1>
        <p className="text-sm text-muted-foreground">
          按文件名搜索本机已索引的目录。↑↓ 选择 · Enter 打开 · Shift+Enter 定位 · Ctrl+C 复制路径。
        </p>
      </div>

      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={inputRef}
          value={query}
          autoFocus
          spellCheck={false}
          placeholder={enabled ? "输入文件名关键词，例如 report、截图" : "文件搜索未启用"}
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

      <SearchIndexStatus state={indexState} onRebuild={rebuild} onCancel={cancelScan} />

      {!enabled ? (
        <Card>
          <CardHeader>
            <CardTitle>文件搜索还没有开启</CardTitle>
            <CardDescription>
              开启后会在后台为常用目录建立文件名索引，不联网、不读取文件内容。
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
        <>
          <div
            ref={listRef}
            className="max-h-[26rem] overflow-y-auto rounded-lg border border-border/60 p-1.5"
          >
            <SearchResultList
              hits={hits}
              query={query}
              activeIndex={activeIndex}
              onActivate={setActiveIndex}
              onOpen={(hit) => void activate(hit, "open")}
              onReveal={(hit) => void activate(hit, "reveal")}
              onCopyPath={(hit) => void copyPath(hit)}
              empty={
                searching
                  ? "搜索中…"
                  : query.trim()
                    ? `没有匹配「${query.trim()}」的文件`
                    : "索引为空或还没有扫描完成"
              }
            />
          </div>

          <Card>
            <CardHeader>
              <CardTitle>索引范围</CardTitle>
              <CardDescription>
                {customRoots.length > 0
                  ? "只索引下列目录；改动会立即触发一次重建。"
                  : "当前使用默认的常用目录（桌面 / 文档 / 下载 / 图片）；添加自定义目录后可自行增删。"}
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-col gap-2">
              {effectiveRoots.length === 0 ? (
                <p className="text-xs text-muted-foreground">正在读取索引范围…</p>
              ) : (
                <ul className="flex flex-col gap-1">
                  {effectiveRoots.map((root) => (
                    <li
                      key={root}
                      className="flex items-center justify-between gap-3 rounded-md bg-muted/50 px-2.5 py-1.5"
                    >
                      <span className="truncate text-xs" title={root}>
                        {root}
                      </span>
                      {customRoots.length > 0 && (
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          aria-label={`移除 ${root}`}
                          onClick={() => void removeRoot(root)}
                        >
                          <X />
                        </Button>
                      )}
                    </li>
                  ))}
                </ul>
              )}

              {indexState?.missingRoots.map((root) => (
                <p key={root} className="text-xs text-amber-600 dark:text-amber-500">
                  目录已不存在，已跳过：{root}
                </p>
              ))}

              <div>
                <Button variant="outline" size="sm" onClick={() => void addRoot()}>
                  <FolderPlus data-icon="inline-start" />
                  添加目录
                </Button>
              </div>
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
