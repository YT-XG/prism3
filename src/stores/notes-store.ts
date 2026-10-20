import { create } from "zustand";

import { ipc, type Note, type NoteFolder, type NoteTagRow, type Tag } from "@/lib/ipc";

interface NotesState {
  /** 活笔记（不含回收站），创建时间正序。 */
  notes: Note[];
  folders: NoteFolder[];
  tags: Tag[];
  /** 笔记-标签关联（构建 `tagId → noteIds` 筛选映射）。 */
  noteTags: NoteTagRow[];
  loading: boolean;

  /** 拉取速记全量数据（笔记 / 文件夹 / 标签 / 关联）。 */
  load: () => Promise<void>;

  addNote: (
    title: string,
    folderId: number | null,
    content?: string,
    sourceUrl?: string,
  ) => Promise<Note>;
  saveNote: (id: number, title: string, content: string) => Promise<Note | null>;
  setNoteFolder: (noteId: number, folderId: number | null) => Promise<void>;
  setNoteIcon: (id: number, icon: string | null) => Promise<void>;
  trashNote: (id: number) => Promise<void>;
  restoreNote: (id: number) => Promise<void>;
  purgeNote: (id: number) => Promise<void>;
  purgeAllTrashed: () => Promise<number>;

  createFolder: (name: string, parentId: number | null) => Promise<NoteFolder | null>;
  renameFolder: (id: number, name: string) => Promise<void>;
  deleteFolder: (id: number) => Promise<void>;
  reorderFolders: (
    moves: { id: number; parentId: number | null; sortOrder: number }[],
  ) => Promise<void>;

  createTag: (name: string) => Promise<Tag | null>;
  renameTag: (id: number, name: string) => Promise<void>;
  deleteTag: (id: number) => Promise<void>;
  setNoteTags: (noteId: number, tagIds: number[]) => Promise<void>;
}

/**
 * 速记数据源（主窗口与快捷小窗各持一份实例）。
 *
 * **跨窗口一致性由 `notes:changed` 事件驱动 `load()`**：写操作后端会广播 revision，
 * 各窗口用 `useNotesRevision` 订阅后重拉。这里的动作在成功后也主动 `load()` 一次，
 * 让同窗口的界面立刻反映结果（不等广播回来）。
 */
export const useNotesStore = create<NotesState>((set, get) => ({
  notes: [],
  folders: [],
  tags: [],
  noteTags: [],
  loading: false,

  load: async () => {
    set({ loading: true });
    try {
      const [notes, folders, tags, noteTags] = await Promise.all([
        ipc.listNotes(false),
        ipc.listNoteFolders(),
        ipc.listTags(),
        ipc.listNoteTags(),
      ]);
      set({ notes, folders, tags, noteTags });
    } finally {
      set({ loading: false });
    }
  },

  addNote: async (title, folderId, content = "", sourceUrl = "") => {
    const note = await ipc.createNoteIn(title, content, folderId, sourceUrl);
    await get().load();
    return note;
  },

  saveNote: async (id, title, content) => {
    const updated = await ipc.updateNote(id, title, content);
    // 只把返回的这条写回本地数组（避免整份重拉打断正在编辑的光标），标题可能被后端改写。
    set((state) => ({
      notes: state.notes.map((note) => (note.id === id ? updated : note)),
    }));
    return updated;
  },

  setNoteFolder: async (noteId, folderId) => {
    await ipc.setNoteFolder(noteId, folderId);
    await get().load();
  },

  setNoteIcon: async (id, icon) => {
    await ipc.setNoteIcon(id, icon);
    await get().load();
  },

  trashNote: async (id) => {
    await ipc.trashNote(id);
    await get().load();
  },

  restoreNote: async (id) => {
    await ipc.restoreNote(id);
    await get().load();
  },

  purgeNote: async (id) => {
    await ipc.purgeNote(id);
  },

  purgeAllTrashed: () => ipc.purgeAllTrashedNotes(),

  createFolder: async (name, parentId) => {
    const folder = await ipc.createNoteFolder(name, parentId);
    await get().load();
    return folder;
  },

  renameFolder: async (id, name) => {
    await ipc.renameNoteFolder(id, name);
    await get().load();
  },

  deleteFolder: async (id) => {
    await ipc.deleteNoteFolder(id);
    await get().load();
  },

  reorderFolders: async (moves) => {
    await ipc.reorderNoteFolders(moves);
    await get().load();
  },

  createTag: async (name) => {
    const tag = await ipc.createTag(name);
    await get().load();
    return tag;
  },

  renameTag: async (id, name) => {
    await ipc.renameTag(id, name);
    await get().load();
  },

  deleteTag: async (id) => {
    await ipc.deleteTag(id);
    await get().load();
  },

  setNoteTags: async (noteId, tagIds) => {
    await ipc.setNoteTags(noteId, tagIds);
    await get().load();
  },
}));
