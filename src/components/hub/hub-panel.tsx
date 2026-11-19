import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Activity,
  MonitorSmartphone,
  ClipboardList,
  Clock,
  FolderGit2,
  ListChecks,
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

/** 主视图 id：就是顶部按钮条里那排按钮，点哪个就把右边的大区域换成哪一页。 */
type HubViewId = "search" | "notes" | "clipboard" | "projects" | "work";

const HUB_VIEWS: { id: HubViewId; label: string; icon: LucideIcon }[] = [
  { id: "clipboard", label: "剪贴板", icon: ClipboardList },
  { id: "notes", label: "速记", icon: NotebookPen },
  { id: "work", label: "工作", icon: ListChecks },
  { id: "projects", label: "项目", icon: FolderGit2 },
  { id: "search", label: "搜索", icon: Search },
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
 * 组合窗口（窗口 label = `hub`）：铺满整屏的透明悬浮窗 + 一层变灰蒙版，
 * 结构参照 Windows Game Bar / Win+G —— **整屏压暗，蒙版上放顶部按钮条 + 左侧小视图 + 主视图**。
 *
 * - 变灰蒙版：根节点整屏半透明黑底（窗口本身铺满光标所在屏幕，见 `hub.rs`），
 *   点空白处即收起——和 Win+G 一样用「整屏变灰」把注意力压到悬浮内容上；
 * - 顶部按钮条：左侧品牌，中间五个按钮切换主视图（**左右方向键也能切**），右侧快捷键提示与「收起」；
 * - 左侧小视图：常驻的系统资源 / 时钟 / 耳机摘要，直接复用首页卡片的数据源；
 * - 右侧主视图：复用快捷小窗的搜索 / 速记 / 剪贴板 / 项目 / 工作五页（它们只依赖
 *   `active` / `focused`，不关心自己跑在哪个窗口；项目页额外内嵌命令输出）。
 *
 * 组合窗口是正常可聚焦窗口，因此 `focused` 就是真实的窗口焦点：失焦时 Rust 会隐藏
 * 窗口，这里同时把各页切回 `active=false` 以停止轮询。主视图五页**全部常驻挂载**、
 * 只把非当前页 `display:none`（见 `HubMainView`），否则切页会把速记页没写完的草稿丢掉。
 */
export function HubPanel() {
  // 本窗口没有主题引导脚本（见 quick-panel.html），配置也是异步读回来的：
  // 主动拉一次交给 ThemeProvider，并让它**只应用不落盘**——写 localStorage 的职责
  // 归主窗口，否则本窗口挂载瞬间的默认值会覆盖掉主窗口存下的外观偏好。
  useConfig();
  const shortcut = useAppStore((state) => state.config?.hubShortcut ?? "");

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
        // 整屏变灰蒙版（参照 Win+G）：窗口本身铺满光标所在的屏幕（见 `hub.rs`），
        // 这一层把桌面压暗，卡片再浮在蒙版上居中排布。
        className="relative flex h-full w-full items-center justify-center overflow-hidden bg-black/45 p-5 dark:bg-black/60"
        onMouseDown={(event) => {
          // 点**任何一张卡片之外**的地方都收起：整屏蒙版、卡片之间的空隙。
          // 只判断 `event.target === currentTarget` 不够——那些空白处命中的是中间的
          // 布局容器，点下去毫无反应。左侧小视图现在是一整张卡片，卡内空白不再收起。
          //
          // 弹窗 / 下拉浮层挂在 `body` 下（Radix Portal），事件仍会经 React 树冒泡到这里；
          // 它们在视觉上属于「卡片内部」，必须一并放过，否则点一下编辑框的按钮就把 hub 收了。
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
        {/* 顶部一层极淡的品牌光晕：让整块悬浮面板像从桌面里「透」出来，而不是贴上去的。 */}
        <div
          aria-hidden
          className="pointer-events-none absolute inset-x-0 top-0 h-[420px] bg-[radial-gradient(60%_100%_at_50%_0%,var(--brand-glow),transparent_70%)] opacity-50"
        />

        <div className="relative flex h-full max-h-[900px] w-full max-w-[1440px] min-h-0 flex-col gap-3">
          <HubTopBar active={view} shortcut={shortcut} onSelect={selectView} />

          <div className="flex min-h-0 flex-1 gap-3">
            <aside
              data-hub-surface=""
              className="flex w-64 shrink-0 flex-col overflow-x-hidden overflow-y-auto rounded-xl bg-background ring-1 ring-border/50"
            >
              <HubSmallView icon={Activity} title="系统资源">
                {/* 小视图常驻可见（不随主视图切换），所以按真实窗口焦点停表。 */}
                <MonitorCard enabled={focused} />
              </HubSmallView>
              <HubSmallView icon={Clock} title="时钟">
                <ClockCard />
              </HubSmallView>
              <HubSmallView icon={MonitorSmartphone} title="设备">
                <HubDevicesWidget active={focused} />
              </HubSmallView>
            </aside>

            <main className="min-w-0 flex-1">
              {/*
                hub 是悬浮层：允许一圈极细 ring + 一层柔和投影把面板从变灰桌面里托起来；
                面板内部仍不逐卡加边框 / 模糊，避免「框里套框」。
              */}
              <div
                data-hub-surface=""
                className="relative flex h-full min-h-0 flex-col overflow-hidden rounded-xl bg-background shadow-[0_30px_80px_-44px_rgba(0,0,0,0.7)] ring-1 ring-border/50"
              >
                <span
                  aria-hidden
                  className="pointer-events-none absolute inset-x-10 top-0 h-px bg-gradient-to-r from-transparent via-brand/50 to-transparent"
                />
                <HubMainView visible={view === "search"} scroll>
                  <QuickPanelSearch active={isActive("search")} {...commonProps} />
                </HubMainView>
                <HubMainView visible={view === "notes"} scroll>
                  <QuickPanelNotes active={isActive("notes")} {...commonProps} />
                </HubMainView>
                {/* 剪贴板页自己滚列表（翻页条要钉在底部），不让外层再套一层滚动。 */}
                <HubMainView visible={view === "clipboard"} scroll={false}>
                  <QuickPanelClipboard
                    active={isActive("clipboard")}
                    pageSize={CLIPBOARD_PAGE_SIZE}
                    focusToken={focusToken}
                    {...commonProps}
                  />
                </HubMainView>
                {/* 项目页自己排版（列表 + 就地看输出），交给外层滚动会把控制台挤没。 */}
                <HubMainView visible={view === "projects"} scroll={false}>
                  <QuickPanelProjects active={isActive("projects")} withOutput />
                </HubMainView>
                {/* 工作页直接用主窗口同款完整台账（项目筛选 / 编辑 / 周报全都在），
                    不依赖主窗口；它自己管理滚动，交给外层会把时间线压没。 */}
                <HubMainView visible={view === "work"} scroll={false}>
                  <WorkPanel active={isActive("work")} />
                </HubMainView>
              </div>
            </main>
          </div>
        </div>
      </div>
      <Toaster />
    </ThemeProvider>
  );
}

/**
 * 顶部按钮条：左侧品牌，中间切换主视图的按钮组，右侧快捷键提示与收起。
 * 按钮组做成「内凹底 + 选中项浮起」的胶囊，与快捷小窗分页器的选中样式同源。
 */
function HubTopBar({
  active,
  shortcut,
  onSelect,
}: {
  active: HubViewId;
  shortcut: string;
  onSelect: (id: HubViewId) => void;
}) {
  const version = useAppVersion();

  /** 同 `HubPanel::viewRef`：连续按键时不能等下一帧才拿到新的 active。 */
  const activeRef = useRef(active);
  activeRef.current = active;

  /** 顶部菜单内的方向键切换（四个方向都接，左右是本排的主方向）。 */
  const moveTab = (delta: number) => {
    const index = HUB_VIEWS.findIndex((item) => item.id === activeRef.current);
    const next = HUB_VIEWS[(index + delta + HUB_VIEWS.length) % HUB_VIEWS.length];
    activeRef.current = next.id;
    onSelect(next.id);
    // 让焦点跟到新 tab：连续按键时焦点不会留在旧按钮上。
    window.setTimeout(() => document.getElementById(`hub-tab-${next.id}`)?.focus(), 0);
  };

  return (
    <header
      data-hub-surface=""
      className="relative flex h-12 shrink-0 items-center gap-2 overflow-hidden rounded-xl bg-background px-3 shadow-[0_18px_50px_-34px_rgba(0,0,0,0.65)] ring-1 ring-border/50"
    >
      <span
        aria-hidden
        className="pointer-events-none absolute inset-x-10 top-0 h-px bg-gradient-to-r from-transparent via-brand/50 to-transparent"
      />

      <span className="flex shrink-0 items-center gap-1.5 pl-1">
        <PrismMark className="size-4 shrink-0" />
        <span className="flex min-w-0 items-baseline gap-1.5">
          <span className="font-heading text-[0.7rem] font-semibold tracking-[0.14em] uppercase">
            Prism3
          </span>
          {version && (
            <span className="truncate text-[0.65rem] font-medium text-muted-foreground tabular-nums">
              v{version}
            </span>
          )}
        </span>
      </span>

      <span className="mx-0.5 h-5 w-px shrink-0 bg-border" aria-hidden />

      <nav
        role="tablist"
        aria-orientation="horizontal"
        aria-label="主视图（← → 切换）"
        className="flex items-center gap-0.5 rounded-lg bg-muted/50 p-0.5 ring-1 ring-border/40"
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
                "relative flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
                on
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              <Icon
                className={cn("size-3.5 shrink-0", on && "text-brand")}
                aria-hidden
              />
              {label}
              {on && (
                <span
                  aria-hidden
                  className="absolute inset-x-3 bottom-0 h-px rounded-full bg-brand"
                />
              )}
            </button>
          );
        })}
      </nav>

      <div className="ml-auto flex shrink-0 items-center gap-1.5 pl-3">
        <span className="hidden items-center gap-1 text-[0.65rem] text-muted-foreground lg:flex">
          <kbd className="rounded border border-border/70 bg-muted/60 px-1 py-0.5 leading-none">
            ←
          </kbd>
          <kbd className="rounded border border-border/70 bg-muted/60 px-1 py-0.5 leading-none">
            →
          </kbd>
          切换
        </span>
        {shortcut && (
          <span className="rounded-md border border-border/50 bg-muted/50 px-2 py-1 text-[0.65rem] text-muted-foreground">
            {shortcut} 唤起 / 收起
          </span>
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="收起"
          title="收起（Esc）"
          onClick={hideHub}
        >
          <X className="size-3.5" />
        </Button>
      </div>
    </header>
  );
}

/** 左侧小视图：三段内容共用一张卡片，中间用发丝分隔线，不再一卡套一卡。 */
function HubSmallView({
  icon: Icon,
  title,
  children,
}: {
  icon: LucideIcon;
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="flex shrink-0 flex-col gap-2 border-t border-border/40 px-3.5 py-3 first:border-t-0">
      <header className="flex items-center gap-2">
        <Icon className="size-3.5 shrink-0 text-brand" aria-hidden />
        <h2 className="min-w-0 truncate font-heading text-[0.8125rem] font-semibold">{title}</h2>
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
        "min-h-0 flex-1 px-3.5 py-3",
        scroll ? "overflow-x-hidden overflow-y-auto" : "overflow-hidden",
        !visible && "hidden",
      )}
    >
      {children}
    </div>
  );
}