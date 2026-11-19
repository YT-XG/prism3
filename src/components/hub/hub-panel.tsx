import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Activity,
  ClipboardList,
  Clock,
  FolderGit2,
  ListChecks,
  MonitorSmartphone,
  NotebookPen,
  Search,
  X,
  type LucideIcon,
} from "lucide-react";

import { PrismMark } from "@/components/app/prism-mark";
import { ClockCard } from "@/components/home/cards/clock";
import { MonitorCard } from "@/components/home/cards/monitor";
import { HubDevicesWidget } from "@/components/hub/hub-widgets";
import { QuickPanelClipboard } from "@/components/quick-panel/quick-panel-clipboard";
import { QuickPanelNotes } from "@/components/quick-panel/quick-panel-notes";
import { QuickPanelProjects } from "@/components/quick-panel/quick-panel-projects";
import { QuickPanelSearch } from "@/components/quick-panel/quick-panel-search";
import { WorkPanel } from "@/components/work/work-panel";
import { ThemeProvider } from "@/components/theme-provider";
import { Button } from "@/components/ui/button";
import { Toaster } from "@/components/ui/sonner";
import { useAppVersion } from "@/hooks/use-app-version";
import { useConfig } from "@/hooks/use-config";
import { ipc } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/** 收起组合窗口（Rust 侧统一处理，避免各页各自调窗口 API）。 */
function hideHub() {
  void ipc.hideHub();
}

/** 主视图 id：就是左侧导航那一列，点哪个就把右边换成哪一页。 */
type HubViewId = "clipboard" | "notes" | "work" | "projects" | "search";

/**
 * 每个主视图的三件套：导航短名、页头标题、页头那一句操作提示。
 *
 * 提示语与各页自带的副标题同源（这里多一份是因为 hub 隐藏了页内标题，
 * 见 `hideHeading`），所以改文案时两处都要看一眼。
 */
interface HubViewMeta {
  id: HubViewId;
  label: string;
  title: string;
  icon: LucideIcon;
  hint: (context: { focused: boolean; pasteOnClick: boolean }) => string;
}

const HUB_VIEWS: HubViewMeta[] = [
  {
    id: "clipboard",
    label: "剪贴板",
    title: "剪贴板",
    icon: ClipboardList,
    hint: ({ focused, pasteOnClick }) =>
      focused
        ? pasteOnClick
          ? "↑↓ 选择 · Enter 粘贴 · PgUp/PgDn 翻页 · Esc 收起"
          : "↑↓ 选择 · Enter 复制 · PgUp/PgDn 翻页 · Esc 收起"
        : pasteOnClick
          ? "点一条记录即可粘到刚才那个窗口；点输入框可搜索"
          : "点一条记录即复制到剪贴板；点输入框可搜索",
  },
  {
    id: "notes",
    label: "速记",
    title: "速记",
    icon: NotebookPen,
    hint: ({ focused }) =>
      focused
        ? "输入即自动保存 · 点保存开始下一条 · Esc 退出"
        : "点输入框开始记；点下方条目把它读上来接着改",
  },
  {
    id: "work",
    label: "工作",
    title: "工作记录",
    icon: ListChecks,
    hint: () => "记下每天完成了什么；周报由记录自动汇总，不用再回忆",
  },
  {
    id: "projects",
    label: "项目",
    title: "项目",
    icon: FolderGit2,
    hint: () => "在应用内启动 / 停止命令（不弹命令行窗口），下方实时显示输出",
  },
  {
    id: "search",
    label: "搜索",
    title: "文件搜索",
    icon: Search,
    hint: ({ focused }) =>
      focused
        ? "↑↓ 选择 · Enter 打开 · Shift+Enter 定位 · Esc 收起"
        : "点击输入框开始打字（会临时接管键盘焦点）",
  },
];

/** 每次呼出组合窗口都固定停在这一页（用户最常用的剪贴板）。 */
const DEFAULT_VIEW: HubViewId = "clipboard";

/** 组合窗口里剪贴板一页放多少条：主视图比右缘面板高得多，一屏能放二十来条。 */
const CLIPBOARD_PAGE_SIZE = 20;

/**
 * 左右方向键是否该交给控件自己处理。
 *
 * 单行输入框里有文本（或选中了文本）时，左右键是在移动光标，不能拿来切页；
 * 空输入框里左右键本来无事可做，才允许切页——否则搜索框一挂载就自动聚焦，
 * 「方向键切菜单」会完全失效。多行编辑器 / 下拉框一律不抢。
 */
function arrowKeyBelongsToTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.closest("[contenteditable='true']")) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return (
      target.value.length > 0 ||
      (target.selectionStart !== null && target.selectionStart !== target.selectionEnd)
    );
  }
  return false;
}

/**
 * 组合窗口（窗口 label = `hub`）：铺满整屏的透明悬浮窗，唤起时整块桌面压暗，
 * 中间浮一块「命令中心」玻璃板。
 *
 * 布局（自左向右三块，全部装在同一块板里）：
 * - 顶栏：品牌标记 + 版本，右侧是快捷键提示与收起；不再放导航；
 * - 左栏：**竖直导航**（剪贴板 / 速记 / 工作 / 项目 / 搜索）+ 「现在 / 系统资源 / 设备」三个常驻小视图，
 *   导航移到左侧是为了让主视图整块留给内容，标题不再和顶部标签条重复；
 * - 右栏：统一的页头（图标 + 标题 + 一句操作提示）+ 主视图内容。
 *
 * 主视图五页**全部常驻挂载**、只把非当前页 `display:none`（见 `HubMainView`），
 * 否则切页会把速记页没写完的草稿丢掉。窗口失焦时 Rust 会隐藏窗口，
 * 这里同时把各页切回 `active=false` 以停止轮询。
 */
export function HubPanel() {
  // 本窗口没有主题引导脚本（见 quick-panel.html），配置也是异步读回来的：
  // 主动拉一次交给 ThemeProvider，并让它**只应用不落盘**——写 localStorage 的职责
  // 归主窗口，否则本窗口挂载瞬间的默认值会覆盖掉主窗口存下的外观偏好。
  useConfig();
  const shortcut = useAppStore((state) => state.config?.hubShortcut ?? "");
  const pasteOnClick = useAppStore((state) => state.config?.clipboardPasteOnClick ?? true);

  const [focused, setFocused] = useState(true);
  /** 当前主视图：**每次呼出都会重置为 `DEFAULT_VIEW`（剪贴板）**，不跨次记忆。 */
  const [view, setView] = useState<HubViewId>(DEFAULT_VIEW);
  /** 每次「呼出组合窗口」自增：剪贴板页据此清空上次的关键词并重新聚焦搜索框。 */
  const [focusToken, setFocusToken] = useState(0);
  const wasFocusedRef = useRef(focused);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onFocusChanged(({ payload }) => {
        if (!disposed) setFocused(payload);
      })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // 组合窗口失焦即隐藏，再次唤起才会重新拿到焦点：只有「由无焦点变有焦点」才算一次呼出。
  // 每次呼出都强制回到剪贴板页，并让该页清空上次的关键词、重新聚焦搜索框。
  useEffect(() => {
    const gained = focused && !wasFocusedRef.current;
    wasFocusedRef.current = focused;
    if (!gained) return;
    setView(DEFAULT_VIEW);
    setFocusToken((token) => token + 1);
  }, [focused]);

  const selectView = useCallback((id: HubViewId) => {
    setView(id);
  }, []);

  /** 当前视图的镜像：键盘监听只挂一次，不能闭包里拿着首次渲染的 `view`。 */
  const viewRef = useRef(view);
  viewRef.current = view;

  /** 在主视图序列里前后移动一格（首尾循环）。 */
  const moveView = useCallback(
    (delta: number) => {
      const index = HUB_VIEWS.findIndex((item) => item.id === viewRef.current);
      const next = HUB_VIEWS[(index + delta + HUB_VIEWS.length) % HUB_VIEWS.length];
      // 先同步镜像再提交：按住方向键连续触发时，不能等下一帧渲染才看到新值。
      viewRef.current = next.id;
      selectView(next.id);
    },
    [selectView],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        // 有弹窗时 Esc 先关弹窗（Radix 自己处理），不要让整个组合窗口一起消失。
        if (document.querySelector('[role="dialog"][data-state="open"]')) return;
        event.preventDefault();
        hideHub();
        return;
      }
      // 只接左右键：上下键在搜索 / 剪贴板页是「选择结果」，抢了会破坏原有手感。
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      // 弹窗打开时不切页；页面自己的处理器已拦截；带修饰键 / 正在编辑文本时让给控件。
      if (
        event.defaultPrevented ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        document.querySelector('[role="dialog"][data-state="open"]') ||
        arrowKeyBelongsToTarget(event.target)
      ) {
        return;
      }
      event.preventDefault();
      moveView(event.key === "ArrowLeft" ? -1 : 1);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [moveView]);

  // `active` = 窗口有焦点 **且** 当前显示的就是这一页：只有当前页该轮询 / 该发起查询。
  const isActive = (id: HubViewId) => focused && view === id;
  const commonProps = {
    focused,
    onRequestFocus: async () => true,
    onReleaseFocus: async () => {},
    onDismiss: hideHub,
  };

  return (
    <ThemeProvider persist={false}>
      <div
        // 整屏蒙版：窗口本身铺满光标所在的屏幕（见 `hub.rs`），
        // 三层分别负责「压暗」「品牌光晕」「四周暗角」，玻璃板再浮在中间。
        className="relative flex h-full w-full items-center justify-center overflow-hidden p-4"
        onMouseDown={(event) => {
          // 点**玻璃板之外**的地方才收起：板内空白、弹窗 / 下拉浮层都算「内部」。
          // 弹窗 / 下拉挂在 `body` 下（Radix Portal），事件仍会经 React 树冒泡到这里，
          // 必须一并放过，否则点一下编辑框的按钮就把 hub 收了。
          const target = event.target as Element | null;
          if (
            !target?.closest("[data-hub-surface]") &&
            !target?.closest('[role="dialog"]') &&
            !target?.closest('[data-slot="dialog-overlay"]') &&
            !target?.closest("[data-radix-popper-content-wrapper]") &&
            !target?.closest("[data-sonner-toaster]")
          ) {
            hideHub();
          }
        }}
      >
        <div aria-hidden className="absolute inset-0 bg-[#08090f]/55 dark:bg-black/65" />
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 bg-[radial-gradient(70%_46%_at_50%_-8%,var(--brand-glow),transparent_62%)] opacity-80"
        />
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 [background:radial-gradient(120%_92%_at_50%_50%,transparent_30%,rgba(0,0,0,0.42)_100%)]"
        />

        <div
          data-hub-surface=""
          className="hub-slab hub-enter relative flex h-full max-h-[900px] w-full max-w-[1340px] min-h-0 flex-col overflow-hidden rounded-[26px]"
        >
          {/* 顶边一道品牌色发丝光：玻璃板与蒙版之间唯一的「亮边」。 */}
          <span
            aria-hidden
            className="pointer-events-none absolute inset-x-24 top-0 z-10 h-px bg-gradient-to-r from-transparent via-brand/45 to-transparent"
          />

          <HubTopBar shortcut={shortcut} />

          <div className="flex min-h-0 flex-1">
            <HubAside active={view} focused={focused} onSelect={selectView} />

            <section className="flex min-w-0 flex-1 flex-col">
              <HubPaneHeader
                view={view}
                focused={focused}
                pasteOnClick={pasteOnClick}
              />

              <div className="flex min-h-0 flex-1 flex-col px-6 pb-5">
                <HubMainView visible={view === "search"} scroll>
                  <QuickPanelSearch active={isActive("search")} hideHeading {...commonProps} />
                </HubMainView>
                <HubMainView visible={view === "notes"} scroll>
                  <QuickPanelNotes active={isActive("notes")} hideHeading {...commonProps} />
                </HubMainView>
                {/* 剪贴板页自己滚列表（翻页条要钉在底部），不让外层再套一层滚动。 */}
                <HubMainView visible={view === "clipboard"} scroll={false}>
                  <QuickPanelClipboard
                    active={isActive("clipboard")}
                    pageSize={CLIPBOARD_PAGE_SIZE}
                    focusToken={focusToken}
                    hideHeading
                    {...commonProps}
                  />
                </HubMainView>
                {/* 项目页自己排版（列表 + 就地看输出），交给外层滚动会把控制台挤没。 */}
                <HubMainView visible={view === "projects"} scroll={false}>
                  <QuickPanelProjects active={isActive("projects")} withOutput />
                </HubMainView>
                {/* 工作页直接用主窗口同款完整台账（项目筛选 / 编辑 / 周报全都在），
                    不依赖主窗口；页头由 hub 统一提供，报告入口挪进录入卡首行。 */}
                <HubMainView visible={view === "work"} scroll={false}>
                  <WorkPanel active={isActive("work")} hideHeader />
                </HubMainView>
              </div>
            </section>
          </div>
        </div>
      </div>
      <Toaster />
    </ThemeProvider>
  );
}

/** 顶栏：左侧品牌（标记 + 字标 + 版本 + 「命令中心」），右侧快捷键提示与收起。 */
function HubTopBar({ shortcut }: { shortcut: string }) {
  const version = useAppVersion();

  return (
    <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border/45 px-5">
      <div className="flex min-w-0 items-center gap-2.5">
        <span className="flex size-7 shrink-0 items-center justify-center rounded-[9px] bg-brand-soft ring-1 ring-brand/20">
          <PrismMark className="size-[15px]" />
        </span>
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="font-heading text-[0.72rem] leading-none font-semibold tracking-[0.18em] uppercase">
            Prism3
          </span>
          {version && (
            <span className="truncate text-[0.6rem] font-medium text-muted-foreground tabular-nums">
              v{version}
            </span>
          )}
        </span>
        <span className="hidden h-4 w-px shrink-0 bg-border/60 sm:block" aria-hidden />
        <span className="hidden text-[0.65rem] tracking-[0.16em] whitespace-nowrap text-muted-foreground sm:block">
          命令中心
        </span>
      </div>

      <div className="flex shrink-0 items-center gap-2">
        {shortcut && (
          <span className="hidden items-center gap-1.5 text-[0.65rem] text-muted-foreground md:flex">
            <kbd className="rounded-md border border-border/65 bg-muted/45 px-1.5 py-0.5 leading-none">
              {shortcut}
            </kbd>
            唤出 / 收起
          </span>
        )}
        <span className="hidden items-center gap-1 text-[0.65rem] text-muted-foreground lg:flex">
          <kbd className="rounded-md border border-border/65 bg-muted/45 px-1.5 py-0.5 leading-none">
            Esc
          </kbd>
          收起
        </span>
        <Button variant="ghost" size="icon-sm" aria-label="收起" title="收起（Esc）" onClick={hideHub}>
          <X className="size-4" />
        </Button>
      </div>
    </header>
  );
}

/**
 * 左栏：竖直导航 + 三个常驻小视图。
 *
 * 导航挪到左侧后不再与主视图标题重复；小视图（现在 / 系统 / 设备）不随主视图切换，
 * 系统卡按**真实窗口焦点**停表。
 */
function HubAside({
  active,
  focused,
  onSelect,
}: {
  active: HubViewId;
  focused: boolean;
  onSelect: (id: HubViewId) => void;
}) {
  return (
    <aside className="flex w-[268px] shrink-0 flex-col border-r border-border/45 bg-black/[0.02] dark:bg-black/25">
      <HubNav active={active} onSelect={onSelect} />

      <div className="hub-aside-scroll flex min-h-0 flex-1 flex-col overflow-y-auto">
        <HubSection icon={Clock} label="现在">
          <ClockCard />
        </HubSection>
        <HubSection icon={Activity} label="系统资源">
          <MonitorCard enabled={focused} />
        </HubSection>
        <HubSection icon={MonitorSmartphone} label="设备">
          <HubDevicesWidget active={focused} />
        </HubSection>
      </div>

      <footer className="flex shrink-0 items-center gap-1.5 border-t border-border/45 px-4 py-2.5 text-[0.65rem] text-muted-foreground">
        <kbd className="rounded border border-border/60 bg-muted/40 px-1 py-0.5 leading-none">←</kbd>
        <kbd className="rounded border border-border/60 bg-muted/40 px-1 py-0.5 leading-none">→</kbd>
        切换视图 · 点空白处返回桌面
      </footer>
    </aside>
  );
}

/** 竖直导航：选中项 = 强调色软底 + 左侧 2px 光束（与主窗口侧栏同一套视觉语言）。 */
function HubNav({
  active,
  onSelect,
}: {
  active: HubViewId;
  onSelect: (id: HubViewId) => void;
}) {
  /** 连续按键时不能等下一帧才拿到新的 active。 */
  const activeRef = useRef(active);
  activeRef.current = active;

  /** 导航内的方向键切换（四个方向都接，左右是本排的主方向）。 */
  const moveTab = (delta: number) => {
    const index = HUB_VIEWS.findIndex((item) => item.id === activeRef.current);
    const next = HUB_VIEWS[(index + delta + HUB_VIEWS.length) % HUB_VIEWS.length];
    activeRef.current = next.id;
    onSelect(next.id);
    // 让焦点跟到新项：连续按键时焦点不会留在旧按钮上。
    window.setTimeout(() => document.getElementById(`hub-tab-${next.id}`)?.focus(), 0);
  };

  return (
    <div className="shrink-0 px-2.5 pt-3.5 pb-1">
      <div className="mb-1.5 flex items-center gap-2 px-2">
        <span className="text-[0.6rem] font-medium tracking-[0.2em] text-muted-foreground">
          视图
        </span>
        <span className="h-px flex-1 bg-border/45" aria-hidden />
      </div>

      <nav
        role="tablist"
        aria-orientation="vertical"
        aria-label="主视图（← → 切换）"
        className="flex flex-col gap-0.5"
        onKeyDown={(event) => {
          const delta =
            event.key === "ArrowLeft" || event.key === "ArrowUp"
              ? -1
              : event.key === "ArrowRight" || event.key === "ArrowDown"
                ? 1
                : 0;
          if (delta === 0) return;
          event.preventDefault();
          event.stopPropagation();
          moveTab(delta);
        }}
      >
        {HUB_VIEWS.map(({ id, label, icon: Icon }) => {
          const on = id === active;
          return (
            <button
              key={id}
              id={`hub-tab-${id}`}
              type="button"
              role="tab"
              aria-selected={on}
              tabIndex={on ? 0 : -1}
              title={`${label}（← → 切换）`}
              onClick={() => onSelect(id)}
              className={cn(
                "group relative flex h-9 items-center gap-2.5 rounded-lg pr-2.5 pl-3 text-left text-[0.8125rem] transition-colors focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none",
                on
                  ? "bg-brand-soft font-medium text-foreground"
                  : "text-muted-foreground hover:bg-muted/40 hover:text-foreground",
              )}
            >
              <span
                aria-hidden
                className={cn(
                  "absolute inset-y-2 left-0 w-0.5 rounded-full bg-brand transition-opacity duration-200",
                  on ? "opacity-100" : "opacity-0",
                )}
              />
              <Icon
                className={cn(
                  "size-4 shrink-0 transition-colors",
                  on ? "text-brand" : "text-muted-foreground",
                )}
                aria-hidden
              />
              <span className="truncate">{label}</span>
            </button>
          );
        })}
      </nav>
    </div>
  );
}

/** 主视图页头：图标软底方块 + 标题 + 一句操作提示；右侧是方向键提示。 */
function HubPaneHeader({
  view,
  focused,
  pasteOnClick,
}: {
  view: HubViewId;
  focused: boolean;
  pasteOnClick: boolean;
}) {
  const meta = HUB_VIEWS.find((item) => item.id === view) ?? HUB_VIEWS[0];
  const Icon = meta.icon;

  return (
    <header className="flex shrink-0 items-start justify-between gap-6 px-6 pt-5 pb-4">
      <div className="flex min-w-0 items-center gap-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-brand-soft text-brand ring-1 ring-brand/15">
          <Icon className="size-4" aria-hidden />
        </span>
        <div className="min-w-0">
          <h1 className="font-heading text-[1.0625rem] leading-tight font-semibold tracking-[-0.02em]">
            {meta.title}
          </h1>
          <p className="mt-1 truncate text-[0.75rem] text-muted-foreground">
            {meta.hint({ focused, pasteOnClick })}
          </p>
        </div>
      </div>

      <span className="hidden shrink-0 items-center gap-1 pt-1.5 text-[0.65rem] text-muted-foreground md:flex">
        <kbd className="rounded border border-border/60 bg-muted/40 px-1 py-0.5 leading-none">←</kbd>
        <kbd className="rounded border border-border/60 bg-muted/40 px-1 py-0.5 leading-none">→</kbd>
        切换
      </span>
    </header>
  );
}

/** 左栏的一段小视图：小标题（图标 + 文字 + 发丝线）+ 内容。 */
function HubSection({
  icon: Icon,
  label,
  children,
}: {
  icon: LucideIcon;
  label: string;
  children: ReactNode;
}) {
  return (
    <section className="flex shrink-0 flex-col gap-2.5 border-t border-border/45 px-4 py-3.5 first:border-t-0">
      <header className="flex items-center gap-2">
        <Icon className="size-3 shrink-0 text-brand/80" aria-hidden />
        <h2 className="text-[0.625rem] font-medium tracking-[0.16em] text-muted-foreground">
          {label}
        </h2>
        <span className="h-px flex-1 bg-border/40" aria-hidden />
      </header>
      {children}
    </section>
  );
}

/**
 * 主视图里的一页容器。
 *
 * 非当前页只 `display:none`、**不卸载**：速记页靠 `active` 由 true 变 false 落盘草稿
 * （见 `QuickPanelNotes` 的 `flush`），卸载会让最后一次输入直接消失；同理，
 * 搜索 / 剪贴板页的输入内容也会随卸载清空。
 *
 * `scroll = false` 交给页面自己排版（项目页要把控制台放在列表下方的剩余高度里，
 * 外层再套一层滚动会把它的高度吃掉）。
 */
function HubMainView({
  visible,
  scroll = true,
  children,
}: {
  visible: boolean;
  scroll?: boolean;
  children: ReactNode;
}) {
  return (
    <div
      aria-hidden={!visible}
      className={cn(
        "min-h-0 flex-1",
        scroll ? "overflow-x-hidden overflow-y-auto" : "flex flex-col overflow-hidden",
        !visible && "hidden",
      )}
    >
      {children}
    </div>
  );
}
