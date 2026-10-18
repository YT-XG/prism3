import type { ReactNode } from "react";

import { Copy, ExternalLink, File as FileIcon, Folder, FolderOpen } from "lucide-react";

import { formatBytes, formatTimeAgo } from "@/lib/format";
import type { SearchHit } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** 结果占据的左侧图标区宽度之外，路径展示的最大字符数（超出则中间省略）。 */
const PARENT_WIDTH = { full: 76, compact: 40 } as const;

interface SearchResultListProps {
  hits: SearchHit[];
  /** 当前关键词（用于命中高亮；由前端计算，避免依赖后端的字节偏移）。 */
  query: string;
  activeIndex: number;
  variant?: "full" | "compact";
  onActivate: (index: number) => void;
  onOpen: (hit: SearchHit) => void;
  onReveal: (hit: SearchHit) => void;
  onCopyPath: (hit: SearchHit) => void;
  /** 无结果时展示的内容。 */
  empty?: ReactNode;
  className?: string;
}

/**
 * 搜索结果列表（主窗口搜索页与快捷小窗搜索页共用）。
 *
 * 行本身不接收 DOM 焦点（`tabIndex={-1}` + 按下时 `preventDefault`）：键盘始终留在搜索
 * 输入框里，↑↓ 由输入框的 `onKeyDown` 驱动。否则点一下结果键盘就跑到行上，
 * 继续打字会失效（快捷小窗里尤其明显）。
 */
export function SearchResultList({
  hits,
  query,
  activeIndex,
  variant = "full",
  onActivate,
  onOpen,
  onReveal,
  onCopyPath,
  empty,
  className,
}: SearchResultListProps) {
  if (hits.length === 0) {
    return (
      <p className={cn("px-1 text-xs text-muted-foreground", className)}>
        {empty ?? "没有匹配的文件"}
      </p>
    );
  }

  return (
    <div role="listbox" aria-label="搜索结果" className={cn("flex flex-col gap-0.5", className)}>
      {hits.map((hit, index) => (
        <SearchResultRow
          key={hit.path}
          hit={hit}
          query={query}
          variant={variant}
          active={index === activeIndex}
          onActivate={() => onActivate(index)}
          onOpen={() => onOpen(hit)}
          onReveal={() => onReveal(hit)}
          onCopyPath={() => onCopyPath(hit)}
        />
      ))}
    </div>
  );
}

interface SearchResultRowProps {
  hit: SearchHit;
  query: string;
  variant: "full" | "compact";
  active: boolean;
  onActivate: () => void;
  onOpen: () => void;
  onReveal: () => void;
  onCopyPath: () => void;
}

function SearchResultRow({
  hit,
  query,
  variant,
  active,
  onActivate,
  onOpen,
  onReveal,
  onCopyPath,
}: SearchResultRowProps) {
  const compact = variant === "compact";
  const Icon = hit.isDir ? Folder : FileIcon;

  return (
    <div
      role="option"
      aria-selected={active}
      tabIndex={-1}
      title={hit.path}
      onMouseEnter={onActivate}
      // 按下即阻止默认行为：不给行 DOM 焦点，输入框的键盘输入不被打断。
      onMouseDown={(event) => event.preventDefault()}
      onClick={onOpen}
      className={cn(
        "group flex cursor-default items-center gap-2 rounded-md px-2 transition-colors",
        compact ? "py-1" : "py-1.5",
        active ? "bg-muted" : "hover:bg-muted/60",
      )}
    >
      <Icon
        className={cn("shrink-0 text-muted-foreground", compact ? "size-3.5" : "size-4")}
        aria-hidden
      />

      <div className="min-w-0 flex-1">
        <div className={cn("truncate", compact ? "text-xs" : "text-sm")}>
          <HighlightedName name={hit.name} query={query} />
        </div>
        <div
          className={cn(
            "truncate text-muted-foreground",
            compact ? "text-[11px]" : "text-xs",
          )}
        >
          {shortenPath(hit.parent, PARENT_WIDTH[variant])}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2 text-[11px] text-muted-foreground tabular-nums">
        {!compact && !hit.isDir && <span>{formatBytes(hit.size)}</span>}
        <span>{formatTimeAgo(hit.modifiedSecs)}</span>
      </div>

      <div className="hidden shrink-0 items-center gap-0.5 group-hover:flex">
        <RowAction label="打开" onClick={onOpen}>
          <ExternalLink className="size-3.5" />
        </RowAction>
        <RowAction label="在资源管理器中定位" onClick={onReveal}>
          <FolderOpen className="size-3.5" />
        </RowAction>
        <RowAction label="复制完整路径" onClick={onCopyPath}>
          <Copy className="size-3.5" />
        </RowAction>
      </div>
    </div>
  );
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

/** 文件名中的命中片段高亮（大小写不敏感）。 */
function HighlightedName({ name, query }: { name: string; query: string }) {
  const needle = query.trim().toLowerCase();
  const at = needle ? name.toLowerCase().indexOf(needle) : -1;
  if (at < 0) return <>{name}</>;

  const end = Math.min(at + needle.length, name.length);
  return (
    <>
      {name.slice(0, at)}
      <mark className="rounded-sm bg-primary/20 px-0 text-foreground">
        {name.slice(at, end)}
      </mark>
      {name.slice(end)}
    </>
  );
}

/** 路径超长时从中间省略，保留盘符与末级目录（两端都是定位信息）。 */
function shortenPath(path: string, max: number): string {
  if (path.length <= max) return path;
  const head = Math.ceil((max - 1) / 3);
  const tail = max - 1 - head;
  return `${path.slice(0, head)}…${path.slice(path.length - tail)}`;
}
