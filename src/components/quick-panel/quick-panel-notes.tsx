import { useEffect, useRef, useState, type MouseEvent } from "react";

import { emitTo } from "@tauri-apps/api/event";
import { NotebookPen, Save } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { useNotesRevision } from "@/hooks/use-notes-state";
import { formatTimeAgo } from "@/lib/format";
import { deriveNoteTitle, markdownPlainText } from "@/lib/notes/markdown-text";
import { cn } from "@/lib/utils";
import { useNotesStore } from "@/stores/notes-store";

/** 主窗口监听的事件名（与 `App.tsx` 中的订阅一致）。 */
const OPEN_NOTE_EVENT = "notes:open";
/** 最近速记列表展示条数。 */
const RECENT_LIMIT = 8;

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
 * 快捷小窗的「速记」页 = 快记：标题 + 源码正文 + 保存，下方是最近速记。
 *
 * **不挂 Milkdown**：面板页常驻挂载且窗口宽固定（内容 474px），把富文本编辑器常驻挂进去
 * 既挤又费（见实施方案 §二）。完整编辑在主窗口速记页。
 * 只有点输入框才临时接管键盘焦点；点列表条目绝不抢焦点。
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
  const revision = useNotesRevision();

  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [feedback, setFeedback] = useState<string | null>(null);

  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (active) void load();
  }, [active, load, revision]);

  // 收起后清掉残留的提示与草稿。
  useEffect(() => {
    if (active) return;
    setFeedback(null);
  }, [active]);

  useEffect(() => {
    if (active && focused) inputRef.current?.focus();
  }, [active, focused]);

  const handleInputMouseDown = (event: MouseEvent<HTMLDivElement>) => {
    if (focused) return;
    event.preventDefault();
    void onRequestFocus().then((ok) => {
      if (ok) inputRef.current?.focus();
    });
  };

  const save = async () => {
    const finalTitle = title.trim() || deriveNoteTitle(content) || "无标题笔记";
    if (!title.trim() && !content.trim()) {
      setFeedback("还没有内容可保存");
      return;
    }
    await addNote(finalTitle, null, content);
    setTitle("");
    setContent("");
    setFeedback("已保存为速记");
    if (focused) void onReleaseFocus();
  };

  const openNote = (id: number) => {
    // 打开动作交给主窗口（面板收起后自己看不到）。
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
          {focused ? "输入后点保存（Esc 退出）" : "点输入框开始记；点下方条目在主窗口打开"}
        </p>
      </div>

      <div className="flex flex-col gap-2" onMouseDown={handleInputMouseDown}>
        <Input
          ref={inputRef}
          value={title}
          readOnly={!focused}
          spellCheck={false}
          placeholder="标题（可留空，取首行）"
          onChange={(event) => setTitle(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onDismiss();
            }
          }}
          className={cn("h-8 text-xs", !focused && "cursor-pointer")}
        />
        <Textarea
          value={content}
          readOnly={!focused}
          spellCheck={false}
          placeholder="正文（Markdown）"
          onChange={(event) => setContent(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onDismiss();
            }
          }}
          className={cn("h-24 resize-none font-mono text-xs", !focused && "cursor-pointer")}
        />
      </div>

      <div className="flex items-center justify-between gap-2">
        <span className="text-[11px] text-muted-foreground">{feedback ?? " "}</span>
        <Button size="sm" className="gap-1.5" onClick={() => void save()}>
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
              <li key={note.id}>
                <button
                  type="button"
                  onClick={() => openNote(note.id)}
                  className="flex w-full flex-col rounded-md px-2 py-1 text-left hover:bg-muted/70"
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
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
