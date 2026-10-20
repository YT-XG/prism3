import { useEffect, useMemo } from "react";

import { FilePlus2 } from "lucide-react";

import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { Button } from "@/components/ui/button";
import { useNotesRevision } from "@/hooks/use-notes-state";
import { formatTimeAgo } from "@/lib/format";
import { markdownPlainText } from "@/lib/notes/markdown-text";
import { useAppStore } from "@/stores/app-store";
import { useNotesStore } from "@/stores/notes-store";

/** 首页「速记」卡片：笔记数 / 标签数 / 最近编辑，可一键新建并跳转。 */
export function NotesCard() {
  const notes = useNotesStore((state) => state.notes);
  const tags = useNotesStore((state) => state.tags);
  const load = useNotesStore((state) => state.load);
  const addNote = useNotesStore((state) => state.addNote);
  const openNote = useAppStore((state) => state.openNote);
  const setActiveView = useAppStore((state) => state.setActiveView);
  const revision = useNotesRevision();

  useEffect(() => {
    void load();
  }, [load, revision]);

  const latest = useMemo(() => {
    if (notes.length === 0) return null;
    return notes.reduce((acc, note) => (note.updatedAt > acc.updatedAt ? note : acc));
  }, [notes]);

  const create = async () => {
    const note = await addNote("无标题笔记", null);
    openNote(note.id);
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      {notes.length === 0 ? (
        <HomeCardHint>还没有速记。新建一篇，随手记点东西。</HomeCardHint>
      ) : (
        <div className="grid grid-cols-2 gap-2">
          <div className="flex flex-col rounded-lg bg-muted/50 px-2.5 py-2">
            <span className="font-heading text-lg font-semibold tabular-nums">{notes.length}</span>
            <span className="text-[0.65rem] text-muted-foreground">笔记</span>
          </div>
          <div className="flex flex-col rounded-lg bg-muted/50 px-2.5 py-2">
            <span className="font-heading text-lg font-semibold tabular-nums">{tags.length}</span>
            <span className="text-[0.65rem] text-muted-foreground">标签</span>
          </div>
        </div>
      )}

      {latest && (
        <div className="min-h-0 flex-1 rounded-lg bg-muted/50 p-2.5">
          <span className="text-[0.65rem] text-muted-foreground">最近编辑</span>
          <p className="truncate text-xs font-medium">{latest.title || "无标题笔记"}</p>
          <p className="truncate text-[0.65rem] text-muted-foreground">
            {markdownPlainText(latest.content, 40) || "空白笔记"}
          </p>
          <span className="text-[0.65rem] text-muted-foreground">
            {formatTimeAgo(latest.updatedAt)}
          </span>
        </div>
      )}

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2">
        <Button variant="ghost" size="sm" className="gap-1.5 text-xs" onClick={() => void create()}>
          <FilePlus2 className="size-3.5" aria-hidden />
          新建
        </Button>
        <HomeCardLink onClick={() => setActiveView("notes")}>去速记</HomeCardLink>
      </div>
    </div>
  );
}
