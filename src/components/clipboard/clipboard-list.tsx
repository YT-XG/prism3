import type { ReactNode } from "react";

import {
  ClipboardPaste,
  Copy,
  Image as ImageIcon,
  Pencil,
  Star,
  StarOff,
  Trash2,
  Type,
} from "lucide-react";

import { ClipboardThumb } from "@/components/clipboard/clipboard-thumb";
import { formatBytes, formatTimeAgo } from "@/lib/format";
import type { ClipEntryMeta } from "@/lib/ipc";
import { cn } from "@/lib/utils";

interface ClipboardListProps {
  entries: ClipEntryMeta[];
  activeIndex: number;
  variant?: "full" | "compact";
  onActivate: (index: number) => void;
  /** 点击一行 / 「粘贴」：写剪贴板并（按配置）还原焦点后粘贴。 */
  onPaste: (entry: ClipEntryMeta) => void;
  /** 只复制。 */
  onCopy: (entry: ClipEntryMeta) => void;
  /** 编辑（仅文本记录）。 */
  onEdit: (entry: ClipEntryMeta) => void;
  onDelete: (entry: ClipEntryMeta) => void;
  /** 收藏 / 取消收藏（收藏的记录置顶，且不会被自动清理 / 「清空历史」删掉）。 */
  onTogglePin: (entry: ClipEntryMeta) => void;
  /** 无结果时展示的内容。 */
  empty?: ReactNode;
  className?: string;
}

/**
 * 剪贴板历史列表（主窗口剪贴板页与快捷小窗剪贴板页共用）。
 *
 * 与搜索结果列表同一约定：行本身**不接收 DOM 焦点**（`tabIndex={-1}` +
 * 按下时 `preventDefault`），键盘始终留在搜索输入框里，↑↓ 由输入框的 `onKeyDown` 驱动。
 * 这也是快捷小窗里「点一条就能粘回去」的前提——点行不会把焦点从目标应用抢到面板上。
 */
export function ClipboardList({
  entries,
  activeIndex,
  variant = "full",
  onActivate,
  onPaste,
  onCopy,
  onEdit,
  onDelete,
  onTogglePin,
  empty,
  className,
}: ClipboardListProps) {
  if (entries.length === 0) {
    return (
      <p className={cn("px-1 text-xs text-muted-foreground", className)}>
        {empty ?? "暂无记录"}
      </p>
    );
  }

  return (
    <div role="listbox" aria-label="剪贴板历史" className={cn("flex flex-col gap-0.5", className)}>
      {entries.map((entry, index) => (
        <ClipboardRow
          key={entry.id}
          entry={entry}
          variant={variant}
          active={index === activeIndex}
          onActivate={() => onActivate(index)}
          onPaste={() => onPaste(entry)}
          onCopy={() => onCopy(entry)}
          onEdit={() => onEdit(entry)}
          onDelete={() => onDelete(entry)}
          onTogglePin={() => onTogglePin(entry)}
        />
      ))}
    </div>
  );
}

interface ClipboardRowProps {
  entry: ClipEntryMeta;
  variant: "full" | "compact";
  active: boolean;
  onActivate: () => void;
  onPaste: () => void;
  onCopy: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onTogglePin: () => void;
}

function ClipboardRow({
  entry,
  variant,
  active,
  onActivate,
  onPaste,
  onCopy,
  onEdit,
  onDelete,
  onTogglePin,
}: ClipboardRowProps) {
  const compact = variant === "compact";
  const isText = entry.kind === "text";

  return (
    <div
      role="option"
      aria-selected={active}
      tabIndex={-1}
      title={entry.preview}
      onMouseEnter={onActivate}
      // 按下即阻止默认行为：不给行 DOM 焦点，输入框的键盘输入不被打断。
      onMouseDown={(event) => event.preventDefault()}
      onClick={onPaste}
      className={cn(
        "group flex cursor-default items-center gap-2 rounded-md px-2 transition-colors",
        compact ? "py-1" : "py-1.5",
        active ? "bg-muted" : "hover:bg-muted/60",
      )}
    >
      {isText ? (
        <Type
          className={cn("shrink-0 text-muted-foreground", compact ? "size-3.5" : "size-4")}
          aria-hidden
        />
      ) : (
        <ClipboardThumb
          id={entry.id}
          className={compact ? "h-7 w-11" : "h-10 w-16"}
        />
      )}

      <div className="min-w-0 flex-1">
        <div
          className={cn(
            "break-words text-muted-foreground",
            compact ? "line-clamp-1 text-xs" : "line-clamp-2 text-sm",
          )}
        >
          {entry.preview}
        </div>
        <div
          className={cn(
            "truncate text-muted-foreground",
            compact ? "text-[10px]" : "text-[11px]",
          )}
        >
          {isText ? `${entry.textLength} 字` : imageSizeLabel(entry)}
          {" · "}
          {formatTimeAgo(entry.createdAt)}
        </div>
      </div>

      {!compact && (
        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
          {formatBytes(entry.size)}
        </span>
      )}

      {entry.pinned && (
        <Star
          className={cn("shrink-0 fill-current text-amber-500", compact ? "size-3" : "size-3.5")}
          aria-hidden
        />
      )}

      <div className="hidden shrink-0 items-center gap-0.5 group-hover:flex">
        <RowAction label="粘贴到上一个窗口" onClick={onPaste}>
          <ClipboardPaste className="size-3.5" />
        </RowAction>
        <RowAction label="仅复制" onClick={onCopy}>
          <Copy className="size-3.5" />
        </RowAction>
        {isText && (
          <RowAction label="编辑文本" onClick={onEdit}>
            <Pencil className="size-3.5" />
          </RowAction>
        )}
        <RowAction label={entry.pinned ? "取消收藏" : "收藏"} onClick={onTogglePin}>
          {entry.pinned ? <StarOff className="size-3.5" /> : <Star className="size-3.5" />}
        </RowAction>
        <RowAction label="删除" onClick={onDelete}>
          <Trash2 className="size-3.5" />
        </RowAction>
      </div>

      {compact && !isText && (
        <ImageIcon className="size-3 shrink-0 text-muted-foreground/60 group-hover:hidden" aria-hidden />
      )}
    </div>
  );
}

function imageSizeLabel(entry: ClipEntryMeta): string {
  if (entry.imageWidth && entry.imageHeight) {
    return `${entry.imageWidth} × ${entry.imageHeight}`;
  }
  return "图片";
}

function RowAction({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      tabIndex={-1}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
      className="rounded p-1 text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
    >
      {children}
    </button>
  );
}
