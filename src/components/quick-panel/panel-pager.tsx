import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { LayoutGrid, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";

import { usePanelPager } from "@/hooks/use-panel-pager";
import { cn } from "@/lib/utils";

export interface PanelPage {
  id: string;
  label: string;
  icon: LucideIcon;
  content: ReactNode;
  /** 该页期望的面板宽度；缺省即 `defaultPanelWidth`。 */
  panelWidth?: number;
}

interface PanelPagerProps {
  pages: PanelPage[];
  /** 未声明 `panelWidth` 的页面所用的面板宽度。 */
  defaultPanelWidth: number;
  /** 面板贴住灰条的一侧：`end` = 内容靠右（灰条在右 / 上），`start` = 靠左。 */
  anchor?: "start" | "end";
  /** 记忆当前页 id 的 localStorage 键。 */
  storageKey?: string;
  /** 当前激活页变化（含首次挂载恢复）时回调，用于按页调整面板宽度。 */
  onActiveChange?: (page: PanelPage) => void;
  className?: string;
}

/** 非当前页相对视口的轻微位移，滑页时形成层次感。 */
const PARALLAX = 10;

/** 左缘图标栏宽度（px）：图标栏样式与内容宽度换算共用，避免两处各写一份。 */
const RAIL_WIDTH = 44;
/** 面板本体左右各 1px 边框（`quick-panel-body.tsx` 的 `border`），换算内容宽度时扣除。 */
const PANEL_FRAME_WIDTH = 2;

/** 该页面板宽度下的内容宽度：面板宽度减去图标栏与本体边框。 */
function contentWidthOf(pageWidth: number) {
  return pageWidth - RAIL_WIDTH - PANEL_FRAME_WIDTH;
}

/**
 * 通用分页器：左缘图标栏（选中高亮胶囊垂直滑动）+ 纵向滑页内容轨道，
 * 支持点击 / 滚轮与触控板手势切换，底部宫格可一键直达任意页，按 id 记忆停留页。
 * 图标栏在页面增多时自动滚动，当前页始终保持在可视区内。
 *
 * 每页内容按**该页自身宽度**定宽布局（而非跟随视口），并靠灰条一侧对齐：
 * 面板换页时宽度会过渡，定宽内容不会随视口逐帧重排，切换瞬间不会出现文字折行闪动；
 * 宽度变化只表现为远侧被裁掉 / 揭开，与面板本体的宽度过渡方向一致。
 */
export function PanelPager({
  pages,
  defaultPanelWidth,
  anchor = "start",
  storageKey = "prism3:quick-panel:page",
  onActiveChange,
  className,
}: PanelPagerProps) {
  const ids = pages.map((page) => page.id);
  const { index, goTo, handleWheel } = usePanelPager({ ids, storageKey });

  /** 上一次上报的页面 id：`pages` 数组会因宿主重渲染重建，据此去重，避免同页重复上报。 */
  const reportedIdRef = useRef<string | null>(null);

  // 当前页变化（含首次恢复）时上报，宿主据此调整面板宽度并收回输入聚焦模式。
  // 用 layout effect：在浏览器绘制前完成宽度变更，避免新页以旧宽度闪一帧。
  // 按 id 去重：`pages` 会随宿主状态重建（如聚焦状态变化、系统监控每秒刷新），
  // 若按数组身份触发，同一页也会被反复上报，宿主会误判为「换了页」。
  useLayoutEffect(() => {
    const page = pages[index];
    if (!page || reportedIdRef.current === page.id) return;
    reportedIdRef.current = page.id;
    onActiveChange?.(page);
  }, [index, pages, onActiveChange]);

  const [indicator, setIndicator] = useState({ top: 0, height: 0, ready: false });
  const [gridOpen, setGridOpen] = useState(false);

  const railRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const sectionRefs = useRef<(HTMLElement | null)[]>([]);

  const measureIndicator = useCallback(() => {
    const tab = tabRefs.current[index];
    if (!tab) return;
    setIndicator({ top: tab.offsetTop, height: tab.offsetHeight, ready: true });
  }, [index]);

  useLayoutEffect(() => {
    measureIndicator();
  }, [measureIndicator]);

  // 图标栏尺寸/滚动条出现变化时重新量取胶囊位置。
  useLayoutEffect(() => {
    const rail = railRef.current;
    if (!rail || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measureIndicator());
    observer.observe(rail);
    window.addEventListener("resize", measureIndicator);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measureIndicator);
    };
  }, [measureIndicator]);

  // 页多时保证当前图标在可视区内。
  useLayoutEffect(() => {
    tabRefs.current[index]?.scrollIntoView({ block: "nearest", behavior: "auto" });
  }, [index]);

  const selectPage = useCallback(
    (i: number) => {
      goTo(i);
      setGridOpen(false);
    },
    [goTo],
  );

  return (
    <div className={cn("flex w-full min-h-0 min-w-0 flex-1 flex-col", className)}>
      {/*
       * `min-w-0` 这三层缺一不可——面板「窄页内容整体右移 + 右边被切掉」就是缺了它：
       * 各页内容按**自己**的页宽定宽（宽页 474），于是页轨的 min-content 宽度是 474；
       * flex 子项在交叉轴上默认不会收缩到内容宽度以下，面板盒子收窄到 360 后，
       * 分页器仍被撑到 44 + 474 = 518，内容又按 `self-end` 贴着这条「假想的宽边」排，
       * 结果整块内容右移一个页宽差（520 - 360 = 160），远侧还被面板盒子裁掉。
       * 显式给「分页器 / 行 / 内容视口」三层 `min-w-0`（分页器再加 `w-full`）后，
       * 宽度只由面板盒子决定，宽页多出来的部分交给视口的 `overflow-clip`。
       */}
      <div className="flex min-h-0 min-w-0 flex-1">
        <div
          className="flex shrink-0 flex-col border-r border-border/60 bg-muted/30"
          style={{ width: RAIL_WIDTH }}
        >
          <div
            ref={railRef}
            role="tablist"
            aria-orientation="vertical"
            aria-label="快捷小窗页面"
            className="relative flex min-h-0 flex-1 flex-col items-stretch gap-1 overflow-y-auto p-1.5"
          >
            <span
              aria-hidden
              className={cn(
                "absolute right-1 left-1 z-0 rounded-md bg-background shadow-sm ring-1 ring-black/5 transition-[top,height,opacity] duration-300 ease-out dark:ring-white/10",
                indicator.ready ? "opacity-100" : "opacity-0",
              )}
              style={{ top: indicator.top, height: indicator.height }}
            />
            {pages.map((page, i) => {
              const Icon = page.icon;
              const active = i === index;
              return (
                <button
                  key={page.id}
                  ref={(node) => {
                    tabRefs.current[i] = node;
                  }}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  title={page.label}
                  onClick={() => selectPage(i)}
                  className={cn(
                    "relative z-10 flex h-9 shrink-0 items-center justify-center rounded-md transition-colors focus-visible:outline-none",
                    active ? "text-foreground" : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <Icon className="size-4 shrink-0" />
                </button>
              );
            })}
          </div>

          <button
            type="button"
            title="全部页面"
            aria-label="全部页面"
            onClick={() => setGridOpen(true)}
            className="flex h-9 shrink-0 items-center justify-center border-t border-border/60 text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none"
          >
            <LayoutGrid className="size-4" />
          </button>
        </div>

        {/*
         * 内容视口用 `overflow-clip` 而非 `overflow-hidden`：hidden 仍是**可编程滚动的
         * 滚动容器**，任何一次 `focus()` / `scrollIntoView()`（浏览器会把目标滚进视口）
         * 落到当前看不见的那一页上，都会把整条页轨滚走；而 `clip` 完全不产生滚动容器，
         * 页轨位置只由 `transform` 决定，从结构上杜绝「面板内容整体错位」。
         * `min-w-0` 见上：不放行的话这一格也会被宽页的 474 顶住。
         */}
        <div className="relative min-h-0 min-w-0 flex-1 overflow-clip">
          <div
            className="flex h-full flex-col transition-transform duration-[360ms] ease-[cubic-bezier(0.32,0.72,0,1)]"
            style={{ transform: `translate3d(0, -${index * 100}%, 0)` }}
            onWheel={(event) => handleWheel(event, sectionRefs.current[index])}
          >
            {pages.map((page, i) => {
              const active = i === index;
              const offset = active ? 0 : i < index ? -PARALLAX : PARALLAX;
              return (
                <section
                  key={page.id}
                  ref={(node) => {
                    sectionRefs.current[i] = node;
                  }}
                  aria-hidden={!active}
                  style={{ width: contentWidthOf(page.panelWidth ?? defaultPanelWidth) }}
                  className={cn(
                    "h-full shrink-0 overflow-y-auto overflow-x-hidden px-3 py-3",
                    // 交叉轴对齐（而非 `ml-auto`）：内容比视口宽时 auto 外边距会退化成 0，
                    // 页宽被视口裁在灰条一侧、内容还会跟着视口左缘横移；`self-end/start`
                    // 是「不安全对齐」，超宽时溢出到远侧，页内容在屏幕上纹丝不动。
                    anchor === "end" ? "self-end" : "self-start",
                  )}
                >
                  <div
                    className="transition-transform duration-[360ms] ease-[cubic-bezier(0.32,0.72,0,1)] will-change-transform"
                    style={{ transform: `translate3d(0, ${offset}px, 0)` }}
                  >
                    {page.content}
                  </div>
                </section>
              );
            })}
          </div>

          {gridOpen && (
            <PageGrid pages={pages} onSelect={selectPage} onClose={() => setGridOpen(false)} />
          )}
        </div>
      </div>
    </div>
  );
}

/** 全部页面宫格：页面多时的直达入口。 */
function PageGrid({
  pages,
  onSelect,
  onClose,
}: {
  pages: PanelPage[];
  onSelect: (index: number) => void;
  onClose: () => void;
}) {
  return (
    <div className="absolute inset-0 z-20 flex animate-in flex-col bg-background fade-in-0 duration-150">
      <div className="flex shrink-0 items-center justify-between border-b border-border/60 px-3 py-2">
        <span className="text-xs font-medium text-muted-foreground">全部页面</span>
        <button
          type="button"
          aria-label="关闭"
          onClick={onClose}
          className="rounded p-0.5 text-muted-foreground transition-colors hover:text-foreground"
        >
          <X className="size-3.5" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        <div className="grid grid-cols-3 gap-1.5">
          {pages.map((page, i) => {
            const Icon = page.icon;
            return (
              <button
                key={page.id}
                type="button"
                onClick={() => onSelect(i)}
                className="flex flex-col items-center gap-1.5 rounded-lg border border-transparent px-1 py-2.5 transition-colors hover:border-border hover:bg-muted/60"
              >
                <Icon className="size-4" />
                <span className="max-w-full truncate text-[11px] leading-none text-muted-foreground">
                  {page.label}
                </span>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}