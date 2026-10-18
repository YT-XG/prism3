import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
} from "react";

import { MousePointerClick, Search } from "lucide-react";

import { ClipboardEditDialog } from "@/components/clipboard/clipboard-edit-dialog";
import { ClipboardList } from "@/components/clipboard/clipboard-list";
import { ClipboardStatus } from "@/components/clipboard/clipboard-status";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useClipboardList, type ClipboardFeedback } from "@/hooks/use-clipboard-list";
import type { ClipEntryMeta, ClipKindFilter } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** 面板一屏展示的条数（面板高度固定，靠 ↑↓ 与滚动容器浏览）。 */
const RESULT_LIMIT = 10;
/** 内联提示的停留时间。 */
const FEEDBACK_MS = 4000;

interface QuickPanelClipboardProps {
  /** 面板已展开且当前就是剪贴板页：决定是否发起查询。 */
  active: boolean;
  /** 是否已进入输入聚焦模式（窗口可聚焦并持有键盘焦点）。 */
  focused: boolean;
  /** 请求进入输入聚焦模式；返回是否成功。 */
  onRequestFocus: () => Promise<boolean>;
  /** 退出输入聚焦模式并把键盘焦点还给原应用。 */
  onReleaseFocus: () => Promise<void>;
  /** 收起面板（粘成功 / Esc）。 */
  onDismiss: () => void;
}

/**
 * 快捷小窗的「剪贴板」页。
 *
 * 与搜索页的关键差别：**点一条记录就是要「粘到刚才那个窗口」**，因此
 * 1. 列表行按下时 `preventDefault`（见 `ClipboardList`），绝不让面板抢走焦点；
 * 2. 只有点击输入框（或打开编辑弹窗）才临时接管键盘焦点；
 * 3. 粘贴成功后面板主动收起，让用户立刻看到粘好的结果。
 *
 * 面板平时 `focusable: false`，所以第 3 步之后焦点本来就在目标应用里——Rust 侧
 * 只在「前台是本应用」时才做焦点归还（见 `paste.rs`）。
 *
 * 提示一律走**页内一行文字**：面板点完就收起，toast 会随之消失，等于没提示。
 */
export function QuickPanelClipboard({
  active,
  focused,
  onRequestFocus,
  onReleaseFocus,
  onDismiss,
}: QuickPanelClipboardProps) {
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<ClipKindFilter>("all");
  const [editingId, setEditingId] = useState<number | null>(null);
  const [feedback, setFeedback] = useState<ClipboardFeedback | null>(null);

  const { entries, state, activeIndex, setActiveIndex, moveActive, paste, copy, remove, togglePin } =
    useClipboardList({
      enabled: active,
      query,
      kind,
      limit: RESULT_LIMIT,
      onFeedback: setFeedback,
    });

  const inputRef = useRef<HTMLInputElement>(null);
  /** 这次编辑是不是「为了让弹窗能打字」才临时接管焦点的（是则关闭时得还回去）。 */
  const askedFocusRef = useRef(false);

  // 窗口拿到焦点后再把 DOM 焦点给输入框，否则键盘事件根本进不来。
  // 且必须要求「本页可见」：所有页常驻挂载，若后台页也去 focus() 自己的输入框，
  // 浏览器会把整条页轨滚到那个看不见的输入框上（面板内容整体错位），
  // 键盘输入也会落进别的页的输入框。
  useEffect(() => {
    if (active && focused) inputRef.current?.focus();
  }, [active, focused]);

  // 收起 / 翻到别页：关掉弹窗、清掉上一次的提示，免得下次展开看到残留。
  useEffect(() => {
    if (active) return;
    setEditingId(null);
    setFeedback(null);
    askedFocusRef.current = false;
  }, [active]);

  useEffect(() => {
    if (!feedback) return;
    const timer = window.setTimeout(() => setFeedback(null), FEEDBACK_MS);
    return () => window.clearTimeout(timer);
  }, [feedback]);

  const enabled = state?.enabled ?? true;
  const pasteOnClick = state?.pasteOnClick ?? true;

  /** 粘成功了才收起面板；没粘上（上一个窗口已关、焦点抢不回来、调用失败）就把原因留在面板里。 */
  const handlePaste = useCallback(
    async (entry: ClipEntryMeta) => {
      const outcome = await paste(entry.id);
      if (outcome && (outcome.pasted || !outcome.reason)) {
        onDismiss();
      }
    },
    [onDismiss, paste],
  );

  const handleEdit = useCallback(
    async (entry: ClipEntryMeta) => {
      if (focused) {
        askedFocusRef.current = false;
      } else {
        const ok = await onRequestFocus();
        if (!ok) {
          setFeedback({ tone: "error", text: "没拿到键盘焦点，请再点一次编辑" });
          return;
        }
        askedFocusRef.current = true;
      }
      setEditingId(entry.id);
    },
    [focused, onRequestFocus],
  );

  const handleCloseDialog = useCallback(() => {
    setEditingId(null);
    if (askedFocusRef.current) {
      askedFocusRef.current = false;
      void onReleaseFocus();
    }
  }, [onReleaseFocus]);

  /** 点输入框：只有这里才需要键盘焦点（点列表行绝不能接管焦点）。 */
  const handleInputMouseDown = (event: MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (focused) {
      inputRef.current?.focus();
      return;
    }
    void onRequestFocus().then((ok) => {
      if (ok) inputRef.current?.focus();
    });
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const current = entries[activeIndex];
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
      void handlePaste(current);
    } else if (event.key === "Delete" && current) {
      event.preventDefault();
      void remove(current.id);
    } else if (event.key === "Escape") {
      event.preventDefault();
      onDismiss();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c") {
      if (!hasSelection && current) {
        event.preventDefault();
        void copy(current.id);
      }
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-col gap-0.5">
        <h2 className="font-heading text-sm font-medium">剪贴板</h2>
        <p className="text-xs text-muted-foreground">
          {focused
            ? pasteOnClick
              ? "↑↓ 选择 · Enter 粘贴 · Esc 收起"
              : "↑↓ 选择 · Enter 复制 · Esc 收起"
            : pasteOnClick
              ? "点一条记录即可粘到刚才那个窗口；点输入框可搜索"
              : "点一条记录即复制到剪贴板；点输入框可搜索"}
        </p>
      </div>

      <div className="relative" onMouseDown={handleInputMouseDown}>
        <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          ref={inputRef}
          value={query}
          readOnly={!focused}
          spellCheck={false}
          placeholder={enabled ? "搜索剪贴板文本…" : "剪贴板历史未启用"}
          disabled={!enabled}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={handleKeyDown}
          className={cn("h-8 pr-2 pl-8 text-xs", !focused && "cursor-pointer")}
        />
      </div>

      {!enabled ? (
        <p className="text-xs text-muted-foreground">
          还没有开启剪贴板历史，请到主窗口的「设置 → 剪贴板历史」里启用。
        </p>
      ) : (
        <>
          <div className="flex items-center justify-between gap-2">
            <Tabs value={kind} onValueChange={(value) => setKind(value as ClipKindFilter)}>
              <TabsList>
                <TabsTrigger value="all">全部</TabsTrigger>
                <TabsTrigger value="text">文本</TabsTrigger>
                <TabsTrigger value="image">图片</TabsTrigger>
              </TabsList>
            </Tabs>
            {!focused && (
              <MousePointerClick className="size-3 shrink-0 text-muted-foreground" aria-hidden />
            )}
          </div>

          <ClipboardStatus state={state} compact />

          {feedback && (
            // 两行而不是单行截断：失败原因（尤其后端带上来的具体错误）单行会被切掉一半，
            // 只剩「粘贴失败：剪贴板错误: Unknown error while intera…」这种没法用的信息。
            <p
              className={cn(
                "line-clamp-2 break-words text-[11px] leading-snug",
                feedback.tone === "error" ? "text-destructive" : "text-muted-foreground",
              )}
            >
              {feedback.text}
            </p>
          )}

          <ClipboardList
            entries={entries}
            activeIndex={activeIndex}
            variant="compact"
            onActivate={setActiveIndex}
            onPaste={(entry) => void handlePaste(entry)}
            onCopy={(entry) => void copy(entry.id)}
            onEdit={(entry) => void handleEdit(entry)}
            onDelete={(entry) => void remove(entry.id)}
            onTogglePin={(entry) => void togglePin(entry.id, !entry.pinned)}
            empty={
              query.trim()
                ? `没有匹配「${query.trim()}」的记录`
                : kind === "image"
                  ? "还没有记录过图片"
                  : "还没有记录，复制点什么试试"
            }
          />
        </>
      )}

      <ClipboardEditDialog id={editingId} onClose={handleCloseDialog} />
    </div>
  );
}
