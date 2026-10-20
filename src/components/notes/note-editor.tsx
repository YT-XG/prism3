import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";

import { FilePlus2, Loader2, NotebookPen, Smile, Tag as TagIcon, Trash2, X } from "lucide-react";

import { NotesBacklinks } from "@/components/notes/notes-backlinks";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { deriveNoteTitle } from "@/lib/notes/markdown-text";
import type { Note, NoteEditorMode } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { useNotesStore } from "@/stores/notes-store";

/** 正文编辑器懒加载：把 Milkdown 全家桶隔离出首页 chunk（首帧硬约束）。 */
const MilkdownEditor = lazy(() =>
  import("@/components/notes/milkdown-editor").then((module) => ({
    default: module.MilkdownEditor,
  })),
);

/** 防抖落库延时（毫秒）。 */
const SAVE_DEBOUNCE_MS = 600;

const EDITOR_MODES: { value: NoteEditorMode; label: string }[] = [
  { value: "wysiwyg", label: "实时预览" },
  { value: "preview", label: "仅预览" },
  { value: "source", label: "源码" },
  { value: "split", label: "分屏" },
];

const ICON_CHOICES = [
  "📄", "📝", "📌", "⭐", "✅", "💡", "🔥", "📚", "🗂️", "🧠", "🎯", "🔖",
];

function normalizeMode(value: string | undefined): NoteEditorMode {
  return EDITOR_MODES.some((item) => item.value === value)
    ? (value as NoteEditorMode)
    : "wysiwyg";
}

interface NoteEditorProps {
  note: Note | null;
  notes: Note[];
  /** 速记变更 revision：反链面板据此重拉。 */
  revision: number;
  onOpenNote: (id: number) => void;
  onCreateNote: () => void;
}

/**
 * 速记编辑器：标题 + 图标 + 标签 + 模式切换 + 正文（Milkdown）+ 反链面板。
 * 正文以「本地缓冲 + 600ms 防抖落库」为准；切笔记 / 卸载时立即冲刷。
 */
export function NoteEditor({ note, notes, revision, onOpenNote, onCreateNote }: NoteEditorProps) {
  const config = useAppStore((state) => state.config);
  const saveConfig = useAppStore((state) => state.saveConfig);
  const store = useNotesStore();

  const mode = normalizeMode(config?.noteEditorMode);

  const noteId = note?.id ?? null;
  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [iconOpen, setIconOpen] = useState(false);
  const [tagInput, setTagInput] = useState("");

  const pendingRef = useRef<{ id: number; title: string; content: string } | null>(null);
  const timerRef = useRef<number | null>(null);

  const tagIds = useMemo(
    () =>
      noteId == null
        ? []
        : store.noteTags.filter((row) => row.noteId === noteId).map((row) => row.tagId),
    [noteId, store.noteTags],
  );

  const flush = () => {
    if (timerRef.current != null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    const pending = pendingRef.current;
    pendingRef.current = null;
    if (pending) void store.saveNote(pending.id, pending.title, pending.content);
  };

  const scheduleSave = (id: number, nextTitle: string, nextContent: string) => {
    pendingRef.current = { id, title: nextTitle, content: nextContent };
    if (timerRef.current != null) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(flush, SAVE_DEBOUNCE_MS);
  };

  // 切笔记 / 卸载：先冲刷上一条未落库的内容，再装载新笔记。
  useEffect(() => {
    return () => {
      flush();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [noteId]);

  useEffect(() => {
    setTitle(note?.title ?? "");
    setContent(note?.content ?? "");
    setIconOpen(false);
    setTagInput("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [noteId]);

  const effectiveTitle = (rawTitle: string, rawContent: string) => {
    const trimmed = rawTitle.trim();
    if (trimmed && trimmed !== "无标题笔记") return trimmed;
    return deriveNoteTitle(rawContent) || "无标题笔记";
  };

  const onTitleChange = (value: string) => {
    setTitle(value);
    if (noteId != null) scheduleSave(noteId, effectiveTitle(value, content), content);
  };

  const onContentChange = (value: string) => {
    setContent(value);
    if (noteId != null) scheduleSave(noteId, effectiveTitle(title, value), value);
  };

  const switchMode = (next: NoteEditorMode) => {
    void saveConfig({ noteEditorMode: next });
  };

  const toggleTag = (tagId: number) => {
    if (noteId == null) return;
    const next = tagIds.includes(tagId)
      ? tagIds.filter((id) => id !== tagId)
      : [...tagIds, tagId];
    void store.setNoteTags(noteId, next);
  };

  const addTag = async () => {
    const name = tagInput.trim();
    if (!name || noteId == null) return;
    const tag = await store.createTag(name);
    if (tag) {
      await store.setNoteTags(noteId, [...tagIds, tag.id]);
    }
    setTagInput("");
  };

  if (!note || noteId == null) {
    return (
      <div className="flex h-full min-h-0 flex-col items-center justify-center gap-3 text-center">
        <NotebookPen className="size-7 text-muted-foreground" aria-hidden />
        <div>
          <p className="text-sm font-medium">还没有打开速记</p>
          <p className="mt-1 text-xs text-muted-foreground">
            从左侧选一条，或新建一篇。
          </p>
        </div>
        <Button size="sm" onClick={onCreateNote}>
          <FilePlus2 data-icon="inline-start" />
          新建速记
        </Button>
      </div>
    );
  }

  const selectedTags = store.tags.filter((tag) => tagIds.includes(tag.id));

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex shrink-0 items-start gap-2">
        <div className="relative">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="设置图标"
            title="设置图标"
            onClick={() => setIconOpen((open) => !open)}
          >
            {note.icon ? <span className="text-base leading-none">{note.icon}</span> : <Smile />}
          </Button>
          {iconOpen && (
            <div className="absolute top-9 left-0 z-30 grid w-56 grid-cols-6 gap-1 rounded-md border border-border bg-popover p-2 shadow-md">
              {ICON_CHOICES.map((emoji) => (
                <button
                  key={emoji}
                  type="button"
                  className="rounded p-1 text-base hover:bg-muted"
                  onClick={() => {
                    void store.setNoteIcon(noteId, emoji);
                    setIconOpen(false);
                  }}
                >
                  {emoji}
                </button>
              ))}
              <button
                type="button"
                className="col-span-6 mt-1 rounded px-2 py-1 text-xs text-muted-foreground hover:bg-muted"
                onClick={() => {
                  void store.setNoteIcon(noteId, null);
                  setIconOpen(false);
                }}
              >
                清除图标
              </button>
            </div>
          )}
        </div>

        <Input
          value={title}
          placeholder="标题"
          onChange={(event) => onTitleChange(event.target.value)}
          className="h-9 flex-1 border-none bg-transparent px-0 font-heading text-lg font-medium shadow-none focus-visible:ring-0"
        />

        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="移入回收站"
          title="移入回收站"
          onClick={() => void store.trashNote(noteId)}
        >
          <Trash2 />
        </Button>
      </div>

      <div className="flex shrink-0 flex-wrap items-center gap-2">
        {EDITOR_MODES.map((item) => (
          <Button
            key={item.value}
            variant={mode === item.value ? "default" : "ghost"}
            size="sm"
            onClick={() => switchMode(item.value)}
          >
            {item.label}
          </Button>
        ))}
        <span className="mx-1 h-4 w-px bg-border" />
        <div className="flex flex-wrap items-center gap-1">
          {selectedTags.map((tag) => (
            <Badge key={tag.id} variant="secondary" className="gap-1">
              <TagIcon className="size-3" aria-hidden />
              {tag.name}
              <button
                type="button"
                aria-label={`移除标签 ${tag.name}`}
                onClick={() => toggleTag(tag.id)}
              >
                <X className="size-3" />
              </button>
            </Badge>
          ))}
          <Input
            value={tagInput}
            placeholder="添加标签…"
            onChange={(event) => setTagInput(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void addTag();
              }
            }}
            className="h-7 w-28 text-xs"
          />
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        {mode === "source" ? (
          <Textarea
            value={content}
            spellCheck={false}
            onChange={(event) => onContentChange(event.target.value)}
            className="min-h-0 flex-1 resize-none font-mono text-sm"
          />
        ) : mode === "split" ? (
          <div className="grid min-h-0 flex-1 grid-cols-2 gap-3">
            <Textarea
              value={content}
              spellCheck={false}
              onChange={(event) => onContentChange(event.target.value)}
              className="min-h-0 resize-none font-mono text-sm"
            />
            <div className="min-h-0 overflow-hidden rounded-md border border-border/60">
              <Suspense fallback={<EditorLoading />}>
                <MilkdownEditor
                  noteId={noteId}
                  initialContent={content}
                  editable={false}
                  onChange={onContentChange}
                  onOpenNote={onOpenNote}
                  notes={notes}
                />
              </Suspense>
            </div>
          </div>
        ) : (
          <Suspense fallback={<EditorLoading />}>
            <MilkdownEditor
              noteId={noteId}
              initialContent={content}
              editable={mode === "wysiwyg"}
              onChange={onContentChange}
              onOpenNote={onOpenNote}
              notes={notes}
            />
          </Suspense>
        )}
      </div>

      <NotesBacklinks noteId={noteId} revision={revision} onOpenNote={onOpenNote} />

      <p className="shrink-0 truncate text-[0.65rem] text-muted-foreground">
        {note.sourceUrl ? `来源：${note.sourceUrl}` : "输入即自动保存（600ms 防抖）"}
      </p>
    </div>
  );
}

function EditorLoading() {
  return (
    <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
      <Loader2 className="size-4 animate-spin" aria-hidden />
      正在载入编辑器…
    </div>
  );
}
