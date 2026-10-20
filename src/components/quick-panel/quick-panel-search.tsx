import { useEffect, useRef, type KeyboardEvent, type MouseEvent } from "react";

import { MousePointerClick, Search } from "lucide-react";

import { SearchResultList } from "@/components/search/search-result-list";
import { Input } from "@/components/ui/input";
import { useFileSearch } from "@/hooks/use-file-search";
import type { FileIndexState } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** 面板一屏展示的结果数（面板高度固定，靠 ↑↓ 而非滚动浏览）。 */
const RESULT_LIMIT = 8;

interface QuickPanelSearchProps {
  /** 面板已展开且当前就是搜索页：决定是否发起查询。 */
  active: boolean;
  /** 是否已进入输入聚焦模式（窗口可聚焦并持有键盘焦点）。 */
  focused: boolean;
  /** 请求进入输入聚焦模式；返回是否成功。 */
  onRequestFocus: () => Promise<boolean>;
  /** Esc：退出聚焦模式并收起面板。 */
  onDismiss: () => void;
}

/**
 * 快捷小窗的「搜索」页。
 *
 * 关键约束：面板窗口平时 `focusable: false`（悬停划过不抢焦点），所以**点击输入框时**
 * 才由 Rust 侧临时把窗口切成可聚焦并索要焦点（见 `hooks/use-quick-panel-body.ts` 与
 * Rust 的 `focus.rs`）；成功之前输入框只读，避免"看起来能打字其实打不进去"。
 * 聚焦期间面板被 pin 住，鼠标移开也不会自动收起。
 *
 * 结果列表直接铺在页面里（不套内层滚动容器）：滚动统一交给分页器的页面容器，
 * 这样滚轮翻页的既有手势不会被嵌套滚动打乱。
 */
export function QuickPanelSearch({
  active,
  focused,
  onRequestFocus,
  onDismiss,
}: QuickPanelSearchProps) {
  const {
    query,
    setQuery,
    hits,
    indexState,
    activeIndex,
    setActiveIndex,
    moveActive,
    activate,
    copyPath,
  } = useFileSearch({ enabled: active, limit: RESULT_LIMIT, onOpened: onDismiss });

  const inputRef = useRef<HTMLInputElement>(null);

  // 窗口拿到焦点后再把 DOM 焦点给输入框，否则键盘事件根本进不来。
  // 且必须要求「本页可见」：所有页常驻挂载，若后台页也去 focus() 自己的输入框，
  // 浏览器会把整条页轨滚到那个看不见的输入框上（面板内容整体错位），
  // 键盘输入也会落进别的页的输入框。
  useEffect(() => {
    if (active && focused) inputRef.current?.focus();
  }, [active, focused]);

  const handleMouseDown = (event: MouseEvent<HTMLDivElement>) => {
    // 已聚焦：窗口已持有键盘焦点，不要再拦默认行为——否则点击无法把光标移到点击处、
    // 拖动也无法选择文字（结果行自身仍会 preventDefault，键盘继续留在输入框里）。
    if (focused) return;
    // 未聚焦：先拦下「点击即给输入框 DOM 焦点」，让 Rust 把窗口切成可聚焦并索要焦点，
    // 成功后再把 DOM 焦点给输入框。
    event.preventDefault();
    void onRequestFocus().then((ok) => {
      if (ok) inputRef.current?.focus();
    });
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const current = hits[activeIndex];
    const hasSelection =
      event.currentTarget.selectionStart !== null &&
      event.currentTarget.selectionStart !== event.currentTarget.selectionEnd;

    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveActive(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveActive(-1);
    } else if (event.key === "Enter" && current) {
      event.preventDefault();
      void activate(current, event.shiftKey ? "reveal" : "open");
    } else if (event.key === "Escape") {
      event.preventDefault();
      onDismiss();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c") {
      if (!hasSelection && current) {
        event.preventDefault();
        void copyPath(current);
      }
    }
  };

  const enabled = indexState?.enabled ?? false;

  return (
    <div className="flex flex-col gap-2" onMouseDown={handleMouseDown}>
      <div className="flex flex-col gap-0.5">
        <h2 className="font-heading text-sm font-medium">文件搜索</h2>
        <p className="text-xs text-muted-foreground">
          {focused
            ? "↑↓ 选择 · Enter 打开 · Shift+Enter 定位 · Esc 收起"
            : "点击输入框开始打字（会临时接管键盘焦点）"}
        </p>
      </div>

      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={inputRef}
          value={query}
          readOnly={!focused}
          spellCheck={false}
          placeholder={enabled ? "文件名关键词…" : "文件搜索未启用"}
          disabled={!enabled}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={handleKeyDown}
          className={cn("h-8 pr-2 pl-8 text-xs", !focused && "cursor-pointer")}
        />
      </div>

      {!enabled ? (
        <p className="text-xs text-muted-foreground">
          还没有开启文件搜索，请到主窗口的「设置 → 文件搜索」里启用。
        </p>
      ) : (
        <>
          <PanelStatus state={indexState} focused={focused} />
          <SearchResultList
            hits={hits}
            query={query}
            activeIndex={activeIndex}
            variant="compact"
            onActivate={setActiveIndex}
            onOpen={(hit) => void activate(hit, "open")}
            onReveal={(hit) => void activate(hit, "reveal")}
            onCopyPath={(hit) => void copyPath(hit)}
            empty={query.trim() ? `没有匹配「${query.trim()}」的文件` : "索引为空或还没有扫描完成"}
          />
        </>
      )}
    </div>
  );
}

/** 面板内的极简状态行：面板窄，只保留「能不能搜 / 索引到哪一步」这一条信息。 */
function PanelStatus({ state, focused }: { state: FileIndexState | null; focused: boolean }) {
  if (!state) return <p className="text-[11px] text-muted-foreground">正在读取索引状态…</p>;

  const text =
    state.phase === "scanning"
      ? state.indexed > 0
        ? `后台刷新中 · 可用 ${state.indexed.toLocaleString()} 项`
        : `正在建立索引 · 已扫描 ${state.scanned.toLocaleString()} 项`
      : state.phase === "ready"
        ? `已索引 ${state.indexed.toLocaleString()} 项${state.stale ? " · 刷新中" : ""}`
        : state.phase === "error"
          ? (state.error ?? "索引构建失败")
          : "正在载入索引…";

  return (
    <p className="flex items-center gap-1 text-[11px] text-muted-foreground">
      {!focused && <MousePointerClick className="size-3 shrink-0" aria-hidden />}
      <span className="truncate">{text}</span>
    </p>
  );
}
