import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent,
} from "react";

import { emitTo } from "@tauri-apps/api/event";
import { ExternalLink, NotebookPen, Save } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useNotesRevision } from "@/hooks/use-notes-state";
import { formatTimeAgo } from "@/lib/format";
import { ipc } from "@/lib/ipc";
import { deriveNoteTitle, markdownPlainText } from "@/lib/notes/markdown-text";
import { cn } from "@/lib/utils";
import { useNotesStore } from "@/stores/notes-store";

/** 正文编辑器与主窗口同款（Milkdown Crepe）；懒加载，避免常驻面板白白拖进大 chunk。 */
const MilkdownEditor = lazy(() =>
  import("@/components/notes/milkdown-editor").then((module) => ({
    default: module.MilkdownEditor,
  })),
);

/** 主窗口监听的事件名（与 `App.tsx` 中的订阅一致）。 */
const OPEN_NOTE_EVENT = "notes:open";
/** 最近速记列表展示条数。 */
const RECENT_LIMIT = 8;
/** 输入停止多久后自动保存（与主窗口速记编辑器的 600ms 防抖一致）。 */
const AUTOSAVE_DELAY_MS = 600;

interface QuickPanelNotesProps {
  /** 面板已展开且当前就是速记页。 */
  active: boolean;
  /** 是否已进入输入聚焦模式。 */
  focused: boolean;
  onRequestFocus: () => Promise<boolean>;
  onReleaseFocus: () => Promise<void>;
  /** 收起面板。 */
  onDismiss: () => void;
}

/**
 * 快捷小窗的「速记」页 = 快记：标题 + Markdown 正文（与主窗口同款 Milkdown 编辑器）。
 * 正文输入 600ms 防抖自动保存；点「保存」表示这一条写完了：立即落库并清空，开始下一条。
 *
 * 编辑器只在页面可见时挂载（懒加载 chunk），收起即卸载，草稿留在本地 state 里；
 * 面板平时不可聚焦，只有点输入区才临时接管键盘焦点。
 */
export function QuickPanelNotes({
  active,
  focused,
  onRequestFocus,
  onReleaseFocus,
  onDismiss,
}: QuickPanelNotesProps) {
  const notes = useNotesStore((state) => state.notes);
  const load = useNotesStore((state) => state.load);
  const addNote = useNotesStore((state) => state.addNote);
  const saveNote = useNotesStore((state) => state.saveNote);
  const revision = useNotesRevision();

  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [feedback, setFeedback] = useState<string | null>(null);
  /** 编辑器重挂载代号：点「保存」清空草稿后 +1，让编辑器以空内容重建。 */
  const [draftKey, setDraftKey] = useState(0);
  /** 正在接着改的已有速记 id；null = 这是一条还没落库的新草稿。 */
  const [editingId, setEditingId] = useState<number | null>(null);

  const inputRef = useRef<HTMLInputElement>(null);
  const editorRef = useRef<HTMLDivElement>(null);
  /** 自动保存建好的速记 id：第一条防抖落库后，后续输入改为更新这一条。 */
  const noteIdRef = useRef<number | null>(null);
  /** 与输入框同步的最新值：防抖回调 / 收起时的兜底保存都读这里。 */
  const titleRef = useRef("");
  const contentRef = useRef("");
  const timerRef = useRef<number | null>(null);
  /** 串行化保存：连发的两次不会都走「新建」分支。 */
  const queueRef = useRef<Promise<unknown>>(Promise.resolve());

  /** 真正落库：没有 noteId 就新建一条并记住 id，之后都更新同一条。 */
  const persist = useCallback(
    async (nextTitle: string, nextContent: string): Promise<boolean> => {
      const trimmedTitle = nextTitle.trim();
      if (!trimmedTitle && !nextContent.trim()) return true;

      const finalTitle =
        trimmedTitle || deriveNoteTitle(nextContent) || "无标题笔记";
      try {
        if (noteIdRef.current === null) {
          const note = await addNote(finalTitle, null, nextContent);
          noteIdRef.current = note.id;
        } else {
          await saveNote(noteIdRef.current, finalTitle, nextContent);
        }
        setFeedback("已自动保存");
        return true;
      } catch (cause) {
        setFeedback(`自动保存失败：${String(cause)}`);
        return false;
      }
    },
    [addNote, saveNote],
  );

  /** 立即保存当前草稿（取消在途防抖）。返回是否成功。 */
  const flush = useCallback((): Promise<boolean> => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    const nextTitle = titleRef.current;
    const nextContent = contentRef.current;
    if (!nextTitle.trim() && !nextContent.trim()) return Promise.resolve(true);

    const task = queueRef.current.then(() => persist(nextTitle, nextContent));
    queueRef.current = task.catch(() => false);
    return task;
  }, [persist]);

  /** 输入变化：更新 ref 并重置防抖计时。 */
  const scheduleAutosave = (nextTitle: string, nextContent: string) => {
    titleRef.current = nextTitle;
    contentRef.current = nextContent;
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
    }
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      void flush();
    }, AUTOSAVE_DELAY_MS);
  };

  useEffect(() => {
    if (active) void load();
  }, [active, load, revision]);

  // 收起时清掉提示，并把还没到点的草稿立刻落库——否则面板一收就丢最后一次输入。
  useEffect(() => {
    if (active) return;
    setFeedback(null);
    void flush();
  }, [active, flush]);

  // 卸载时只取消防抖计时：此时窗口正在销毁，异步保存已来不及完成。
  useEffect(
    () => () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    },
    [],
  );

  useEffect(() => {
    if (active && focused) inputRef.current?.focus();
  }, [active, focused]);

  const handleInputMouseDown = (event: MouseEvent<HTMLDivElement>) => {
    if (focused) return;
    event.preventDefault();
    // 面板平时不可聚焦：先让 Rust 把窗口切成可聚焦，再把焦点交给被点的输入区。
    const inEditor = editorRef.current?.contains(event.target as Node) ?? false;
    void onRequestFocus().then((ok) => {
      if (!ok) return;
      if (inEditor) {
        editorRef.current?.querySelector<HTMLElement>(".ProseMirror")?.focus();
      } else {
        inputRef.current?.focus();
      }
    });
  };

  /** 「保存」按钮：把在途草稿立刻落库，然后清空开始下一条。 */
  const finishDraft = async () => {
    if (!titleRef.current.trim() && !contentRef.current.trim()) {
      setFeedback("还没有内容可保存");
      return;
    }
    // 失败时保留内容，方便用户直接重试。
    if (!(await flush())) return;

    titleRef.current = "";
    contentRef.current = "";
    setTitle("");
    setContent("");
    // 编辑器以空内容重建（它不监听 initialContent 变化）。
    setDraftKey((value) => value + 1);
    noteIdRef.current = null;
    setEditingId(null);
    setFeedback("已保存为速记");
    if (focused) void onReleaseFocus();
  };

  /**
   * 点「最近速记」里的一条：把正文读回来填进上面的输入框，就地接着改
   * （不再跳到主窗口——那样面板一收，用户还得再切回来）。
   *
   * 换一条之前先 `flush()`：否则当前草稿会连同它刚建好的 noteId 一起被覆盖掉。
   */
  const loadNote = async (id: number) => {
    if (!(await flush())) return;
    try {
      const note = await ipc.getNote(id);
      if (!note) {
        setFeedback("这条速记已经不在了，列表可能还没刷新");
        return;
      }
      noteIdRef.current = note.id;
      titleRef.current = note.title;
      contentRef.current = note.content;
      setTitle(note.title);
      setContent(note.content);
      setEditingId(note.id);
      // 编辑器不监听 `initialContent` 变化，换内容必须让它重建实例。
      setDraftKey((value) => value + 1);
      setFeedback(`正在接着改《${note.title || "无标题笔记"}》，改动会存回这一条`);
    } catch (cause) {
      setFeedback(`读取速记失败：${String(cause)}`);
    }
  };

  /** 在主窗口打开某条速记（面板/悬浮窗收起后自己看不到它）。 */
  const openInMainWindow = (id: number) => {
    // 切走前先把草稿落库（防抖可能还没到点）。
    void flush();
    void emitTo("main", OPEN_NOTE_EVENT, id);
    onDismiss();
  };

  const recent = notes
    .slice()
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, RECENT_LIMIT);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-col gap-0.5">
        <h2 className="font-heading text-sm font-medium">速记</h2>
        <p className="text-xs text-muted-foreground">
          {focused
            ? "输入即自动保存（600ms 防抖）· 点保存开始下一条 · Esc 退出"
            : "点输入框开始记；点下方条目把它读上来接着改（↗ 在主窗口打开）"}
        </p>
      </div>

      <div className="flex flex-col gap-2" onMouseDown={handleInputMouseDown}>
        <Input
          ref={inputRef}
          value={title}
          readOnly={!focused}
          spellCheck={false}
          placeholder="标题（可留空，取首行）"
          onChange={(event) => {
            const next = event.target.value;
            setTitle(next);
            scheduleAutosave(next, contentRef.current);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              void flush();
              onDismiss();
            }
          }}
          className={cn("h-8 border-0 bg-muted/40 text-xs", !focused && "cursor-pointer")}
        />
        {/* 无边框（整屏已有蒙版，浅底色足够圈出编辑区），聚焦环仍保留做状态提示。 */}
        <div
          ref={editorRef}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              void flush();
              onDismiss();
            }
          }}
          className="flex h-48 min-h-32 resize-y flex-col overflow-hidden rounded-lg bg-muted/25 select-text focus-within:ring-3 focus-within:ring-ring/50 xl:h-64"
        >
          {active ? (
            <Suspense
              fallback={
                <p className="px-2.5 py-2 text-xs text-muted-foreground">正在载入编辑器…</p>
              }
            >
              <MilkdownEditor
                key={draftKey}
                noteId={editingId ?? 0}
                initialContent={content}
                editable={focused}
                onChange={(markdown) => {
                  setContent(markdown);
                  scheduleAutosave(titleRef.current, markdown);
                }}
                // 正文里的 `[标题](note/12)` 双链：跳去主窗口打开，不要覆盖当前草稿。
                onOpenNote={openInMainWindow}
                notes={notes}
              />
            </Suspense>
          ) : (
            <p className="px-2.5 py-2 text-xs text-muted-foreground">
              点击开始记，支持 Markdown（标题可留空，取正文首行）
            </p>
          )}
        </div>
      </div>

      <div className="flex items-center justify-between gap-2">
        <span className="text-[11px] text-muted-foreground">{feedback ?? " "}</span>
        <Button size="sm" className="gap-1.5" onClick={() => void finishDraft()}>
          <Save className="size-3.5" aria-hidden />
          保存
        </Button>
      </div>

      <div className="flex flex-col gap-1">
        <span className="flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground">
          <NotebookPen className="size-3" aria-hidden />
          最近速记
        </span>
        {recent.length === 0 ? (
          <p className="text-xs text-muted-foreground">还没有速记</p>
        ) : (
          <ul className="flex max-h-40 flex-col gap-0.5 overflow-y-auto">
            {recent.map((note) => (
              <li
                key={note.id}
                className={cn(
                  "group/note flex items-center gap-0.5 rounded-md",
                  editingId === note.id && "bg-brand-soft",
                )}
              >
                <button
                  type="button"
                  title="点一下把这篇读进上面的输入框接着改"
                  // 按下不抢焦点：键盘留在输入框里，接着打字不会断。
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => void loadNote(note.id)}
                  className="flex min-w-0 flex-1 flex-col rounded-md px-2 py-1 text-left hover:bg-muted/70"
                >
                  <span className="flex items-center gap-1">
                    <span className="truncate text-xs font-medium">
                      {note.title || "无标题笔记"}
                    </span>
                    <span className="ml-auto shrink-0 text-[0.65rem] text-muted-foreground">
                      {formatTimeAgo(note.updatedAt)}
                    </span>
                  </span>
                  <span className="truncate text-[0.65rem] text-muted-foreground">
                    {markdownPlainText(note.content, 40) || "空白笔记"}
                  </span>
                </button>
                <button
                  type="button"
                  title="在主窗口打开"
                  aria-label={`在主窗口打开 ${note.title || "无标题笔记"}`}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => openInMainWindow(note.id)}
                  className="shrink-0 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/note:opacity-100"
                >
                  <ExternalLink className="size-3.5" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
