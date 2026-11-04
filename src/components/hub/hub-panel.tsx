import { useCallback, useEffect, useState, type ReactNode } from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Activity,
  MonitorSmartphone,
  ClipboardList,
  Clock,
  FolderGit2,
  NotebookPen,
  Search,
  Triangle,
  Workflow,
  X,
  type LucideIcon,
} from "lucide-react";

import { ClockCard } from "@/components/home/cards/clock";
import { MonitorCard } from "@/components/home/cards/monitor";
import { HubDevicesWidget } from "@/components/hub/hub-widgets";
import { MacrosPanel } from "@/components/macros/macros-panel";
import { QuickPanelClipboard } from "@/components/quick-panel/quick-panel-clipboard";
import { QuickPanelNotes } from "@/components/quick-panel/quick-panel-notes";
import { QuickPanelProjects } from "@/components/quick-panel/quick-panel-projects";
import { QuickPanelSearch } from "@/components/quick-panel/quick-panel-search";
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
type HubViewId = "search" | "notes" | "clipboard" | "projects" | "macros";

const HUB_VIEWS: { id: HubViewId; label: string; icon: LucideIcon }[] = [
  { id: "search", label: "搜索", icon: Search },
  { id: "notes", label: "速记", icon: NotebookPen },
  { id: "clipboard", label: "剪贴板", icon: ClipboardList },
  { id: "projects", label: "项目", icon: FolderGit2 },
  { id: "macros", label: "宏", icon: Workflow },
];

/** 记忆当前主视图的 localStorage 键（与快捷小窗分页器同一套做法）。 */
const VIEW_STORAGE_KEY = "prism3:hub:view";
/** 组合窗口里剪贴板一页放多少条：主视图比右缘面板高得多，一屏能放二十来条。 */
const CLIPBOARD_PAGE_SIZE = 20;

/** 读取上次停留的主视图；没写过或值已失效时回到第一页。 */
function readStoredView(): HubViewId {
  try {
    const stored = window.localStorage.getItem(VIEW_STORAGE_KEY);
    if (HUB_VIEWS.some((view) => view.id === stored)) return stored as HubViewId;
  } catch {
    // 隐私模式等场景下读不到，用默认页。
  }
  return HUB_VIEWS[0].id;
}

/**
 * 组合窗口（窗口 label = `hub`）：铺满整屏的透明悬浮窗 + 一层变灰蒙版，
 * 结构参照 Windows Game Bar / Win+G —— **整屏压暗，蒙版上放顶部按钮条 + 左侧小视图 + 主视图**。
 *
 * - 变灰蒙版：根节点整屏半透明黑底（窗口本身铺满光标所在屏幕，见 `hub.rs`），
 *   点空白处即收起——和 Win+G 一样用「整屏变灰」把注意力压到悬浮内容上；
 * - 顶部按钮条：左侧品牌，中间五个按钮切换主视图，右侧快捷键提示与「收起」；
 * - 左侧小视图：常驻的系统资源 / 时钟 / 耳机摘要，直接复用首页卡片的数据源；
 * - 右侧主视图：复用快捷小窗的搜索 / 速记 / 剪贴板 / 项目四页，外加宏页（它们只依赖
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
  const [view, setView] = useState<HubViewId>(readStoredView);

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

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        hideHub();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const selectView = useCallback((id: HubViewId) => {
    setView(id);
    try {
      window.localStorage.setItem(VIEW_STORAGE_KEY, id);
    } catch {
      // 写不进去也不影响本次切换。
    }
  }, []);

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
        className="flex h-full w-full items-center justify-center bg-black/45 p-6 dark:bg-black/55"
        onMouseDown={(event) => {
          // 点**任何一张卡片之外**的地方都收起：整屏蒙版、卡片之间的空隙，以及左侧小视图
          // 下方那片空白（它露出的就是蒙版，视觉上等同空白）。只判断 `event.target ===
          // currentTarget` 不够——那些空白处命中的是中间的布局容器，点下去毫无反应。
          const target = event.target as Element | null;
          if (!target?.closest("[data-hub-surface]")) hideHub();
        }}
      >
        <div className="flex h-full max-h-[900px] w-full max-w-[1440px] min-h-0 flex-col gap-3">
          <HubTopBar active={view} shortcut={shortcut} onSelect={selectView} />

          <div className="flex min-h-0 flex-1 gap-3">
            <aside className="flex w-64 shrink-0 flex-col gap-3 overflow-y-auto">
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
                卡片一律「无边框、无投影」：整屏已经压了一层变灰蒙版，靠底色 + 圆角就够
                分层，再套边线 / 阴影反而像框里套框。
              */}
              <div
                data-hub-surface=""
                className="flex h-full min-h-0 flex-col overflow-hidden rounded-xl bg-background"
              >
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
                    {...commonProps}
                  />
                </HubMainView>
                {/* 项目页自己排版（列表 + 就地看输出），交给外层滚动会把控制台挤没。 */}
                <HubMainView visible={view === "projects"} scroll={false}>
                  <QuickPanelProjects active={isActive("projects")} withOutput />
                </HubMainView>
                {/* 宏页两列自己排版；开始录制 / 从组合窗口回放时本窗口会被自动隐藏。 */}
                <HubMainView visible={view === "macros"} scroll={false}>
                  <MacrosPanel />
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

  return (
    <header
      data-hub-surface=""
      className="flex h-11 shrink-0 items-center gap-2 rounded-xl bg-background px-2.5"
    >
      <span className="flex shrink-0 items-baseline gap-1.5 pl-1">
        <Triangle className="size-3.5 shrink-0 self-center text-brand" strokeWidth={1.5} aria-hidden />
        <span className="font-heading text-[0.7rem] font-semibold tracking-[0.14em] uppercase">
          Prism3
        </span>
        {version && (
          <span className="text-[0.65rem] font-medium text-muted-foreground tabular-nums">
            v{version}
          </span>
        )}
      </span>

      <span className="mx-0.5 h-5 w-px shrink-0 bg-border" aria-hidden />

      <nav
        role="tablist"
        aria-orientation="horizontal"
        aria-label="主视图"
        className="flex items-center gap-0.5 rounded-lg bg-muted/60 p-0.5"
      >
        {HUB_VIEWS.map(({ id, label, icon: Icon }) => {
          const on = id === active;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={on}
              onClick={() => onSelect(id)}
              className={cn(
                "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs transition-colors focus-visible:outline-none",
                on ? "bg-background text-foreground" : "text-muted-foreground hover:text-foreground",
              )}
            >
              <Icon className="size-3.5 shrink-0" aria-hidden />
              {label}
            </button>
          );
        })}
      </nav>

      <div className="ml-auto flex shrink-0 items-center gap-1.5 pl-3">
        {shortcut && (
          <span className="rounded-md bg-muted/60 px-2 py-1 text-[0.65rem] text-muted-foreground">
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

/** 左侧小视图外壳：与主视图同一套卡片外观，标题行带图标。 */
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
    <section
      data-hub-surface=""
      className="flex shrink-0 flex-col gap-2 rounded-xl bg-background px-3.5 py-3 text-sm"
    >
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