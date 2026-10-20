import { useCallback, useEffect, useMemo, useState } from "react";

import { NoteEditor } from "@/components/notes/note-editor";
import { NotesFolderTree } from "@/components/notes/notes-folder-tree";
import { useNotesRevision } from "@/hooks/use-notes-state";
import { ipc, type Note } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { useNotesStore } from "@/stores/notes-store";

/**
 * 主窗口「速记」页：两栏（左 = 文件夹树 + 搜索 + 回收站，右 = 编辑器）。
 *
 * - 页面只在被选中时挂载（`AppShell` 按视图分支渲染），离开即卸载并停编辑器；
 * - 数据以 `notes:changed` 事件驱动重拉（`useNotesRevision`），跨窗口一致；
 * - 全局搜索 / 首页卡片 / 快捷小窗经 `openNote(id)` 落 `pendingNoteId`，本页挂载时消费一次。
 */
export function NotesPanel() {
  const load = useNotesStore((state) => state.load);
  const notes = useNotesStore((state) => state.notes);
  const folders = useNotesStore((state) => state.folders);
  const tags = useNotesStore((state) => state.tags);
  const noteTags = useNotesStore((state) => state.noteTags);
  const addNote = useNotesStore((state) => state.addNote);
  const setNoteFolder = useNotesStore((state) => state.setNoteFolder);
  const trashNote = useNotesStore((state) => state.trashNote);
  const restoreNote = useNotesStore((state) => state.restoreNote);
  const purgeNote = useNotesStore((state) => state.purgeNote);
  const purgeAllTrashed = useNotesStore((state) => state.purgeAllTrashed);
  const createFolder = useNotesStore((state) => state.createFolder);
  const renameFolder = useNotesStore((state) => state.renameFolder);
  const deleteFolder = useNotesStore((state) => state.deleteFolder);
  const reorderFolders = useNotesStore((state) => state.reorderFolders);

  const consumePendingNote = useAppStore((state) => state.consumePendingNote);
  const revision = useNotesRevision();

  const [selectedFolderId, setSelectedFolderId] = useState<number | null>(null);
  const [activeNoteId, setActiveNoteId] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [trashMode, setTrashMode] = useState(false);
  const [trashedNotes, setTrashedNotes] = useState<Note[]>([]);

  // 初次挂载与每次 revision 变化都重拉（写操作在本窗口也会广播回来）。
  useEffect(() => {
    void load();
  }, [load, revision]);

  // 外部入口（搜索 / 卡片 / 快捷小窗）请求打开某条笔记。
  useEffect(() => {
    const id = consumePendingNote();
    if (id != null) {
      setActiveNoteId(id);
      setSelectedFolderId(null);
      setTrashMode(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 回收站列表按需拉取。
  useEffect(() => {
    if (!trashMode) return;
    let active = true;
    void ipc
      .listNotes(true)
      .then((list) => {
        if (active) setTrashedNotes(list);
      })
      .catch(() => {
        if (active) setTrashedNotes([]);
      });
    return () => {
      active = false;
    };
  }, [trashMode, revision]);

  const activeNote = useMemo(() => {
    if (activeNoteId == null) return null;
    if (trashMode) return trashedNotes.find((note) => note.id === activeNoteId) ?? null;
    return notes.find((note) => note.id === activeNoteId) ?? null;
  }, [activeNoteId, trashMode, trashedNotes, notes]);

  const openNote = useCallback((id: number) => {
    setActiveNoteId(id);
    setSelectedFolderId(null);
    setTrashMode(false);
  }, []);

  const createNote = useCallback(
    async (folderId: number | null) => {
      setTrashMode(false);
      setQuery("");
      const note = await addNote("无标题笔记", folderId);
      setActiveNoteId(note.id);
      setSelectedFolderId(null);
    },
    [addNote],
  );

  return (
    <div className="flex h-full min-h-0 gap-4">
      <div className="glass-card flex w-64 shrink-0 flex-col rounded-xl bg-card p-3">
        <NotesFolderTree
          notes={notes}
          folders={folders}
          tags={tags}
          noteTags={noteTags}
          trashedNotes={trashedNotes}
          trashMode={trashMode}
          query={query}
          selectedFolderId={selectedFolderId}
          activeNoteId={activeNoteId}
          onQueryChange={setQuery}
          onTrashModeChange={setTrashMode}
          onSelectFolder={(id) => {
            setSelectedFolderId(id);
            setActiveNoteId(null);
            if (trashMode) setTrashMode(false);
          }}
          onSelectNote={(id) => {
            setActiveNoteId(id);
            setSelectedFolderId(null);
          }}
          onCreateNote={() => void createNote(selectedFolderId)}
          onCreateNoteIn={(folderId) => void createNote(folderId)}
          onCreateFolder={(parentId, name) => void createFolder(name, parentId)}
          onRenameFolder={(id, name) => void renameFolder(id, name)}
          onDeleteFolder={(id) => void deleteFolder(id)}
          onTrashNote={(id) => {
            void trashNote(id);
            if (activeNoteId === id) setActiveNoteId(null);
          }}
          onRestoreNote={(id) => {
            void restoreNote(id);
            setTrashedNotes((list) => list.filter((note) => note.id !== id));
            if (activeNoteId === id) setActiveNoteId(null);
          }}
          onPurgeNote={(id) => {
            void purgeNote(id);
            setTrashedNotes((list) => list.filter((note) => note.id !== id));
            if (activeNoteId === id) setActiveNoteId(null);
          }}
          onPurgeAll={() => {
            void purgeAllTrashed();
            setTrashedNotes([]);
            setActiveNoteId(null);
          }}
          onMoveNote={(noteId, folderId) => void setNoteFolder(noteId, folderId)}
          onReorderFolder={(id, parentId, sortOrder) =>
            void reorderFolders([{ id, parentId, sortOrder }])
          }
        />
      </div>

      <div className="glass-card flex min-w-0 flex-1 flex-col rounded-xl bg-card p-5">
        <NoteEditor
          note={activeNote}
          notes={notes}
          revision={revision}
          onOpenNote={openNote}
          onCreateNote={() => void createNote(selectedFolderId)}
        />
      </div>
    </div>
  );
}
