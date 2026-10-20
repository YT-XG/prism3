import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import {
  ChevronDown,
  ChevronRight,
  FilePlus2,
  FolderClosed,
  FolderOpen,
  FolderPlus,
  Search,
  Trash2,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { Note, NoteFolder, NoteTagRow, Tag } from "@/lib/ipc";
import { markdownPlainText } from "@/lib/notes/markdown-text";
import { cn } from "@/lib/utils";

interface VisibleRow {
  kind: "folder" | "note";
  id: number;
  depth: number;
  /** 文件夹行：是否展开 / 是否为内置。 */
  expanded?: boolean;
  builtin?: boolean;
  label: string;
  sublabel?: string;
}

interface TreeProps {
  notes: Note[];
  folders: NoteFolder[];
  tags: Tag[];
  noteTags: NoteTagRow[];
  trashedNotes: Note[];
  trashMode: boolean;
  query: string;
  selectedFolderId: number | null;
  activeNoteId: number | null;
  onQueryChange: (value: string) => void;
  onTrashModeChange: (value: boolean) => void;
  onSelectFolder: (id: number | null) => void;
  onSelectNote: (id: number) => void;
  onCreateNote: () => void;
  onCreateNoteIn: (folderId: number | null) => void;
  onCreateFolder: (parentId: number | null, name: string) => void;
  onRenameFolder: (id: number, name: string) => void;
  onDeleteFolder: (id: number) => void;
  onTrashNote: (id: number) => void;
  onRestoreNote: (id: number) => void;
  onPurgeNote: (id: number) => void;
  onPurgeAll: () => void;
  onMoveNote: (noteId: number, folderId: number | null) => void;
  onReorderFolder: (id: number, parentId: number | null, sortOrder: number) => void;
}

interface DragState {
  kind: "folder" | "note";
  id: number;
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
}

/** 文件夹树（唯一导航面）：树 + 搜索（`#标签`） + 回收站 + 拖拽移动。 */
export function NotesFolderTree(props: TreeProps) {
  const {
    notes,
    folders,
    tags,
    noteTags,
    trashedNotes,
    trashMode,
    query,
    selectedFolderId,
    activeNoteId,
  } = props;

  const [collapsed, setCollapsed] = useState<Set<number>>(new Set());
  const [drag, setDrag] = useState<DragState | null>(null);
  const [dropFolderId, setDropFolderId] = useState<number | "root" | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const propsRef = useRef(props);
  propsRef.current = props;

  useEffect(() => {
    dragRef.current = drag;
  }, [drag]);

  const trimmed = query.trim();
  const tagFilter = trimmed.startsWith("#") ? trimmed.slice(1).toLowerCase() : null;

  // 标签筛选：`#xxx` 命中标签名的笔记
  const tagMatchedNoteIds = useMemo(() => {
    if (!tagFilter) return null;
    const tagIds = new Set(
      tags.filter((tag) => tag.name.toLowerCase().includes(tagFilter)).map((tag) => tag.id),
    );
    return new Set(
      noteTags.filter((row) => tagIds.has(row.tagId)).map((row) => row.noteId),
    );
  }, [tagFilter, tags, noteTags]);

  const searching = trimmed.length > 0;

  const searchResults = useMemo(() => {
    if (!searching) return [];
    if (tagMatchedNoteIds) {
      return notes.filter((note) => tagMatchedNoteIds.has(note.id));
    }
    const needle = trimmed.toLowerCase();
    return notes.filter(
      (note) =>
        note.title.toLowerCase().includes(needle) ||
        note.content.toLowerCase().includes(needle),
    );
  }, [searching, trimmed, tagMatchedNoteIds, notes]);

  const visibleRows = useMemo<VisibleRow[]>(() => {
    if (searching) {
      return searchResults.map((note) => ({
        kind: "note" as const,
        id: note.id,
        depth: 0,
        label: note.title || "无标题笔记",
        sublabel: noteLabel(note),
      }));
    }

    const rows: VisibleRow[] = [];
    const childrenOf = (parentId: number | null) =>
      folders
        .filter((folder) => folder.parentId === parentId)
        .sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt - b.createdAt);
    const notesOf = (folderId: number | null) =>
      notes.filter((note) => note.folderId === folderId);

    // 树根笔记（未归类）
    for (const note of notesOf(null)) {
      rows.push({
        kind: "note",
        id: note.id,
        depth: 0,
        label: note.title || "无标题笔记",
        sublabel: noteLabel(note),
      });
    }

    const walk = (parentId: number | null, depth: number) => {
      for (const folder of childrenOf(parentId)) {
        const expanded = !collapsed.has(folder.id);
        rows.push({
          kind: "folder",
          id: folder.id,
          depth,
          expanded,
          builtin: folder.builtin,
          label: folder.name,
        });
        if (!expanded) continue;
        for (const note of notesOf(folder.id)) {
          rows.push({
            kind: "note",
            id: note.id,
            depth: depth + 1,
            label: note.title || "无标题笔记",
            sublabel: noteLabel(note),
          });
        }
        walk(folder.id, depth + 1);
      }
    };
    walk(null, 0);
    return rows;
  }, [searching, searchResults, folders, notes, collapsed]);

  // ---- 拖拽（指针实现，避免 Tauri 原生 DnD 干扰） ----
  const beginDrag = (kind: "folder" | "note", id: number, event: ReactPointerEvent) => {
    if (trashMode) return;
    if (event.pointerType === "mouse" && event.button !== 0) return;
    setDrag({
      kind,
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      active: false,
    });
  };

  useEffect(() => {
    if (!drag) return;
    const move = (event: PointerEvent) => {
      const current = dragRef.current;
      if (!current || event.pointerId !== current.pointerId) return;
      const moved =
        Math.abs(event.clientX - current.startX) + Math.abs(event.clientY - current.startY) > 4;
      if (moved && !current.active) {
        const next = { ...current, active: true };
        dragRef.current = next;
        setDrag(next);
      }
      if (!dragRef.current?.active) return;
      const el = document.elementFromPoint(event.clientX, event.clientY) as HTMLElement | null;
      const folderRow = el?.closest<HTMLElement>("[data-folder-id]");
      if (folderRow) {
        setDropFolderId(Number(folderRow.dataset.folderId));
      } else if (el?.closest("[data-root-drop]")) {
        setDropFolderId("root");
      } else {
        setDropFolderId(null);
      }
    };
    const finish = () => {
      const current = dragRef.current;
      const target = dropFolderId;
      dragRef.current = null;
      setDrag(null);
      setDropFolderId(null);
      if (!current?.active || target == null) return;
      const handlers = propsRef.current;
      const parentId = target === "root" ? null : target;
      if (current.kind === "note") {
        handlers.onMoveNote(current.id, parentId);
      } else {
        // 文件夹改父级：后端会做环检测，非法时整批拒绝。
        const siblings = folders.filter((folder) => folder.parentId === parentId);
        const sortOrder = siblings.reduce((max, f) => Math.max(max, f.sortOrder), -1) + 1;
        if (parentId !== current.id) {
          handlers.onReorderFolder(current.id, parentId, sortOrder);
        }
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  }, [drag, dropFolderId, folders]);

  const dragging = drag?.active ?? false;

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="flex shrink-0 items-center gap-1.5">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            spellCheck={false}
            placeholder="搜索（#标签）"
            onChange={(event) => props.onQueryChange(event.target.value)}
            className="h-8 pr-7 pl-8 text-xs"
          />
          {query && (
            <button
              type="button"
              aria-label="清空"
              className="absolute top-1/2 right-2 -translate-y-1/2 text-muted-foreground"
              onClick={() => props.onQueryChange("")}
            >
              <X className="size-3.5" />
            </button>
          )}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          className="flex-1 justify-start gap-1.5 text-xs"
          onClick={() => props.onCreateNoteIn(selectedFolderId ?? null)}
          disabled={trashMode}
        >
          <FilePlus2 className="size-3.5" aria-hidden />
          新建笔记
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="新建文件夹"
          title="新建文件夹"
          disabled={trashMode}
          onClick={() => {
            const name = window.prompt("文件夹名称");
            if (name?.trim()) props.onCreateFolder(selectedFolderId, name.trim());
          }}
        >
          <FolderPlus />
        </Button>
        <Button
          variant={trashMode ? "default" : "ghost"}
          size="icon-sm"
          aria-label="回收站"
          title="回收站"
          onClick={() => props.onTrashModeChange(!trashMode)}
        >
          <Trash2 />
        </Button>
      </div>

      <div
        ref={rootRef}
        data-root-drop
        className={cn(
          "min-h-0 flex-1 overflow-y-auto rounded-lg border border-border/60 p-1 text-sm",
          dragging && dropFolderId === "root" && "border-brand/70 bg-brand-soft",
        )}
      >
        {trashMode ? (
          <TrashList
            trashedNotes={trashedNotes}
            activeNoteId={activeNoteId}
            onSelectNote={props.onSelectNote}
            onRestoreNote={props.onRestoreNote}
            onPurgeNote={props.onPurgeNote}
            onPurgeAll={props.onPurgeAll}
          />
        ) : visibleRows.length === 0 ? (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            {searching ? "没有匹配的速记" : "还没有速记，点「新建笔记」开始"}
          </p>
        ) : (
          <ul className="flex flex-col">
            {visibleRows.map((row) =>
              row.kind === "folder" ? (
                <li key={`folder-${row.id}`}>
                  <FolderRow
                    row={row}
                    selected={selectedFolderId === row.id}
                    dropTarget={dragging && dropFolderId === row.id}
                    onSelect={() => props.onSelectFolder(row.id)}
                    onToggle={() =>
                      setCollapsed((prev) => {
                        const next = new Set(prev);
                        if (next.has(row.id)) next.delete(row.id);
                        else next.add(row.id);
                        return next;
                      })
                    }
                    onRename={() => {
                      const name = window.prompt("重命名文件夹", row.label);
                      if (name?.trim()) props.onRenameFolder(row.id, name.trim());
                    }}
                    onDelete={() => props.onDeleteFolder(row.id)}
                    onCreateNote={() => props.onCreateNoteIn(row.id)}
                    onPointerDown={(event) => beginDrag("folder", row.id, event)}
                  />
                </li>
              ) : (
                <li key={`note-${row.id}`}>
                  <NoteRow
                    row={row}
                    active={activeNoteId === row.id}
                    dragging={dragging && drag?.kind === "note" && drag.id === row.id}
                    onSelect={() => props.onSelectNote(row.id)}
                    onTrash={() => props.onTrashNote(row.id)}
                    onPointerDown={(event) => beginDrag("note", row.id, event)}
                  />
                </li>
              ),
            )}
          </ul>
        )}
      </div>

      <Button
        variant="ghost"
        size="sm"
        className="shrink-0 justify-start gap-1.5 text-xs text-muted-foreground"
        disabled={trashMode}
        onClick={() => props.onSelectFolder(null)}
      >
        <FolderClosed className="size-3.5" aria-hidden />
        未归类（{notes.filter((note) => note.folderId === null).length}）
      </Button>
    </div>
  );
}

function noteLabel(note: Note): string {
  return markdownPlainText(note.content, 40);
}

function FolderRow({
  row,
  selected,
  dropTarget,
  onSelect,
  onToggle,
  onRename,
  onDelete,
  onCreateNote,
  onPointerDown,
}: {
  row: VisibleRow;
  selected: boolean;
  dropTarget: boolean;
  onSelect: () => void;
  onToggle: () => void;
  onRename: () => void;
  onDelete: () => void;
  onCreateNote: () => void;
  onPointerDown: (event: ReactPointerEvent) => void;
}) {
  return (
    <div
      data-folder-id={row.id}
      className={cn(
        "group flex items-center gap-0.5 rounded-md pr-1",
        selected ? "bg-brand-soft" : "hover:bg-muted/70",
        dropTarget && "ring-1 ring-brand/70",
      )}
      style={{ paddingLeft: row.depth * 12 }}
      onContextMenu={(event) => {
        event.preventDefault();
        if (row.builtin) return;
        if (window.confirm(`删除文件夹「${row.label}」？（子项会上移一级）`)) onDelete();
      }}
    >
      <button
        type="button"
        aria-label={row.expanded ? "收起" : "展开"}
        className="flex size-4 shrink-0 items-center justify-center text-muted-foreground"
        onClick={onToggle}
      >
        {row.expanded ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
      </button>
      <button
        type="button"
        onPointerDown={onPointerDown}
        onClick={onSelect}
        onDoubleClick={row.builtin ? undefined : onRename}
        className="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left"
      >
        {row.expanded ? (
          <FolderOpen className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        ) : (
          <FolderClosed className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        )}
        <span className="truncate text-xs">{row.label}</span>
      </button>
      <button
        type="button"
        aria-label="在此新建笔记"
        title="在此新建笔记"
        className="flex size-5 items-center justify-center text-muted-foreground opacity-0 group-hover:opacity-100"
        onClick={onCreateNote}
      >
        <FilePlus2 className="size-3.5" />
      </button>
    </div>
  );
}

function NoteRow({
  row,
  active,
  dragging,
  onSelect,
  onTrash,
  onPointerDown,
}: {
  row: VisibleRow;
  active: boolean;
  dragging: boolean;
  onSelect: () => void;
  onTrash: () => void;
  onPointerDown: (event: ReactPointerEvent) => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-center gap-1 rounded-md pr-1",
        active ? "bg-brand-soft" : "hover:bg-muted/70",
        dragging && "opacity-60",
      )}
      style={{ paddingLeft: row.depth * 12 + 18 }}
      onContextMenu={(event) => {
        event.preventDefault();
        onTrash();
      }}
    >
      <button
        type="button"
        onPointerDown={onPointerDown}
        onClick={onSelect}
        className="flex min-w-0 flex-1 flex-col py-0.5 text-left"
      >
        <span className="truncate text-xs">{row.label}</span>
        {row.sublabel && (
          <span className="truncate text-[0.65rem] text-muted-foreground">{row.sublabel}</span>
        )}
      </button>
      <button
        type="button"
        aria-label="移入回收站"
        title="移入回收站"
        className="flex size-5 items-center justify-center text-muted-foreground opacity-0 group-hover:opacity-100"
        onClick={onTrash}
      >
        <Trash2 className="size-3.5" />
      </button>
    </div>
  );
}

function TrashList({
  trashedNotes,
  activeNoteId,
  onSelectNote,
  onRestoreNote,
  onPurgeNote,
  onPurgeAll,
}: {
  trashedNotes: Note[];
  activeNoteId: number | null;
  onSelectNote: (id: number) => void;
  onRestoreNote: (id: number) => void;
  onPurgeNote: (id: number) => void;
  onPurgeAll: () => void;
}) {
  if (trashedNotes.length === 0) {
    return (
      <p className="px-2 py-6 text-center text-xs text-muted-foreground">回收站是空的</p>
    );
  }
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between px-1">
        <span className="text-[0.65rem] text-muted-foreground">{trashedNotes.length} 条</span>
        <Button
          variant="ghost"
          size="sm"
          className="text-xs text-destructive"
          onClick={() => {
            if (window.confirm(`清空回收站？${trashedNotes.length} 条将被永久删除。`)) {
              onPurgeAll();
            }
          }}
        >
          清空
        </Button>
      </div>
      <ul className="flex flex-col">
        {trashedNotes.map((note) => (
          <li key={note.id}>
            <div
              className={cn(
                "group flex items-center gap-1 rounded-md px-1.5 py-1",
                activeNoteId === note.id ? "bg-brand-soft" : "hover:bg-muted/70",
              )}
            >
              <button
                type="button"
                className="min-w-0 flex-1 truncate text-left text-xs line-through"
                onClick={() => onSelectNote(note.id)}
              >
                {note.title || "无标题笔记"}
              </button>
              <button
                type="button"
                className="text-[0.65rem] text-muted-foreground hover:text-foreground"
                onClick={() => onRestoreNote(note.id)}
              >
                还原
              </button>
              <button
                type="button"
                className="text-[0.65rem] text-destructive"
                onClick={() => {
                  if (window.confirm("永久删除这条笔记？此操作不可撤销。")) onPurgeNote(note.id);
                }}
              >
                删除
              </button>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
