import { useEffect, useState } from "react";

import { Link2 } from "lucide-react";

import { ipc, type NoteLinks } from "@/lib/ipc";

/**
 * 反链面板：出链（本笔记引用了谁，含未链接提及）+ 入链（谁引用了本笔记）。
 * 数据是后端 `note_links` 的派生索引，笔记保存后会重建，因此每次 `noteId` 变化时重拉。
 */
export function NotesBacklinks({
  noteId,
  revision,
  onOpenNote,
}: {
  noteId: number;
  /** 速记变更 revision：变化即重拉（保存后双链索引可能已更新）。 */
  revision: number;
  onOpenNote: (id: number) => void;
}) {
  const [links, setLinks] = useState<NoteLinks | null>(null);

  useEffect(() => {
    let active = true;
    void ipc
      .getNoteLinks(noteId)
      .then((data) => {
        if (active) setLinks(data);
      })
      .catch(() => {
        if (active) setLinks(null);
      });
    return () => {
      active = false;
    };
  }, [noteId, revision]);

  if (!links || (links.outgoing.length === 0 && links.incoming.length === 0)) {
    return null;
  }

  return (
    <div className="flex shrink-0 flex-col gap-2 border-t border-border/50 pt-3 text-xs">
      <span className="flex items-center gap-1.5 font-medium text-muted-foreground">
        <Link2 className="size-3.5" aria-hidden />
        双链
      </span>
      {links.outgoing.length > 0 && (
        <div className="flex flex-col gap-1">
          <span className="text-[0.65rem] text-muted-foreground">出链</span>
          <ul className="flex flex-wrap gap-1.5">
            {links.outgoing.map((link) => (
              <li key={link.toTitle}>
                {link.toNoteId != null ? (
                  <button
                    type="button"
                    onClick={() => onOpenNote(link.toNoteId as number)}
                    className="rounded-full border border-border/60 px-2 py-0.5 hover:bg-muted"
                  >
                    {link.toTitle}
                  </button>
                ) : (
                  <span className="rounded-full border border-dashed border-border/60 px-2 py-0.5 text-muted-foreground">
                    {link.toTitle}（未创建）
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      {links.incoming.length > 0 && (
        <div className="flex flex-col gap-1">
          <span className="text-[0.65rem] text-muted-foreground">反向链接</span>
          <ul className="flex flex-wrap gap-1.5">
            {links.incoming.map((link) => (
              <li key={link.fromNoteId}>
                <button
                  type="button"
                  onClick={() => onOpenNote(link.fromNoteId)}
                  className="rounded-full border border-border/60 px-2 py-0.5 hover:bg-muted"
                >
                  {link.fromTitle || "无标题笔记"}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
