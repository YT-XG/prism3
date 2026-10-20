import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";

import { ChevronRight, MoveDiagonal2, X, type LucideIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import type { HomePlacement } from "./home-layout";

interface HomeCardProps {
  placement: HomePlacement;
  icon: LucideIcon;
  title: string;
  editing: boolean;
  /** 正在被拖动 / 缩放：原卡片半透明，位置由虚线预览块指示。 */
  dragging: boolean;
  onRemove: () => void;
  onDragStart: (event: ReactPointerEvent<HTMLElement>) => void;
  onResizeStart: (event: ReactPointerEvent<HTMLElement>) => void;
  children: ReactNode;
}

/**
 * 首页卡片外壳：玻璃底 + 标题行 + 可滚动内容区。
 *
 * 编辑态下标题行就是拖拽把手，右下角出现缩放把手；内容区整体
 * `pointer-events-none`，避免拖动卡片时误触里面的输入框 / 按钮。
 */
export function HomeCard({
  placement,
  icon: Icon,
  title,
  editing,
  dragging,
  onRemove,
  onDragStart,
  onResizeStart,
  children,
}: HomeCardProps) {
  return (
    <section
      className={cn(
        "group/card relative flex h-full min-h-0 flex-col overflow-hidden rounded-xl bg-card text-sm text-card-foreground shadow-[var(--shadow-card)] ring-1 ring-[color:var(--frost-border)] [background-image:var(--frost-surface)]",
        editing && "ring-brand/35",
        dragging && "opacity-35",
      )}
      style={{
        gridColumn: `${placement.x + 1} / span ${placement.w}`,
        gridRow: `${placement.y + 1} / span ${placement.h}`,
      }}
    >
      <header
        className={cn(
          "flex shrink-0 items-center gap-2 px-4 pt-3 pb-2",
          editing && "cursor-grab touch-none select-none active:cursor-grabbing",
        )}
        onPointerDown={editing ? onDragStart : undefined}
      >
        <Icon className="size-3.5 shrink-0 text-brand" aria-hidden="true" />
        <h2 className="min-w-0 truncate font-heading text-[0.8125rem] font-semibold tracking-[-0.01em]">
          {title}
        </h2>

        <div
          className="ml-auto flex shrink-0 items-center gap-0.5"
          onPointerDown={(event) => event.stopPropagation()}
        >
          {editing && (
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label={`移除${title}`}
              title="从首页移除"
              onClick={onRemove}
            >
              <X />
            </Button>
          )}
        </div>
      </header>

      <div
        inert={editing ? true : undefined}
        className={cn(
          "min-h-0 flex-1 overflow-y-auto px-4 pb-3",
          editing && "pointer-events-none opacity-80 select-none",
        )}
      >
        {children}
      </div>

      {editing && (
        <button
          type="button"
          aria-label={`调整${title}尺寸`}
          title="拖动调整尺寸"
          className="absolute right-1 bottom-1 flex size-5 cursor-nwse-resize touch-none items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
          onPointerDown={onResizeStart}
        >
          <MoveDiagonal2 className="size-3.5" />
        </button>
      )}
    </section>
  );
}

/** 卡片内容底部的「查看全部」入口。 */
export function HomeCardLink({
  children,
  onClick,
  ariaLabel,
}: {
  children: ReactNode;
  onClick: () => void;
  ariaLabel?: string;
}) {
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      onClick={onClick}
      className="inline-flex items-center gap-0.5 text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
    >
      {children}
      <ChevronRight className="size-3" aria-hidden="true" />
    </button>
  );
}

/** 卡片内的弱化空状态 / 说明文字。 */
export function HomeCardHint({ children }: { children: ReactNode }) {
  return <p className="py-2 text-xs leading-relaxed text-muted-foreground">{children}</p>;
}

/** 细进度条：用 `transform` 走合成器，避免每秒轮询引发布局重排。 */
export function HomeBar({
  percent,
  warn = false,
}: {
  percent: number;
  warn?: boolean;
}) {
  const value = Number.isFinite(percent) ? Math.min(100, Math.max(0, percent)) : 0;
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
      <div
        className={cn(
          "h-full origin-left rounded-full transition-transform duration-500 ease-out",
          warn
            ? "bg-[linear-gradient(90deg,#f59e0b,#ef4444)]"
            : "bg-[linear-gradient(90deg,var(--brand-600),var(--brand))]",
        )}
        style={{ transform: `scaleX(${value / 100})` }}
      />
    </div>
  );
}
