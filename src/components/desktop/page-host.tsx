import { ClipboardPanel } from "@/components/clipboard/clipboard-panel";
import { DevicesPanel } from "@/components/devices/devices-panel";
import { HomeDashboard } from "@/components/home/home-dashboard";
import { MonitorPanel } from "@/components/monitor/monitor-panel";
import { NotesPanel } from "@/components/notes/notes-panel";
import { PhonePanel } from "@/components/phone/phone-panel";
import { ShutdownPanel } from "@/components/power/shutdown-panel";
import { ProjectsPanel } from "@/components/projects/projects-panel";
import { SearchPanel } from "@/components/search/search-panel";
import { ScrollArea } from "@/components/ui/scroll-area";
import { WorkPanel } from "@/components/work/work-panel";
import { SettingsPanel } from "@/components/app/settings-panel";
import { CustomAppView } from "@/components/apps/custom-app-view";
import { useCustomAppById } from "@/hooks/use-app-catalog";
import { isCustomWindowId } from "@/lib/apps";
import { cn } from "@/lib/utils";

/**
 * 窗口内容宿主：把各个页面原样装进浮层窗口。
 *
 * 两套滚动语义必须保留（与旧 `app-shell` 一致）：
 * - 速记 / 剪贴板 / 项目 / 工作 **不套 ScrollArea**：Radix 的 Viewport 内部是
 *   `display: table` 包装层，子元素 `h-full` 撑不满，卡片会被内容撑成一条；
 * - 其余页面用 ScrollArea 包一层，滚动条由组件自己画。
 */
export function PageHost({ windowId }: { windowId: string }) {
  const customApp = useCustomAppById(windowId);
  const view = isCustomWindowId(windowId) ? null : windowId;

  if (customApp) {
    return (
      <div className="h-full min-h-0">
        <CustomAppView app={customApp} />
      </div>
    );
  }

  // 窗口还开着、应用已被删：给个说明，别渲染一片空白
  if (view === null) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
        <p className="text-sm font-medium text-foreground">这个应用已被删除</p>
        <p className="text-xs text-muted-foreground">
          关掉窗口即可；也可以从图标坞的「创建应用」重新建一个。
        </p>
      </div>
    );
  }

  if (view === "home") {
    return (
      <div className="h-full min-h-0">
        <HomeDashboard />
      </div>
    );
  }

  if (view === "phone") {
    // 手机页整页自己排布（镜像占满剩余高度），不能套 ScrollArea——
    // 否则画面会被内容高度撑成一条，只能靠滚动看。
    return (
      <div className="flex h-full min-h-0 flex-col px-5 pt-4 pb-5">
        <PhonePanel />
      </div>
    );
  }

  if (view === "notes" || view === "clipboard" || view === "projects" || view === "work") {
    return (
      <div className="flex h-full min-h-0 flex-col px-6 pt-5 pb-6">
        {view === "notes" ? (
          <NotesPanel />
        ) : view === "clipboard" ? (
          <ClipboardPanel />
        ) : view === "work" ? (
          <WorkPanel />
        ) : (
          <ProjectsPanel />
        )}
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div
        className={cn(
          "mx-auto w-full px-6 pt-6 pb-8",
          view === "settings" || view === "shutdown" ? "max-w-3xl" : "max-w-none",
        )}
      >
        {view === "search" && <SearchPanel />}
        {view === "settings" && <SettingsPanel />}
        {view === "shutdown" && <ShutdownPanel />}
        {view === "monitor" && <MonitorPanel />}
        {view === "devices" && <DevicesPanel />}
      </div>
    </ScrollArea>
  );
}
