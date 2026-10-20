import { useEffect, useState } from "react";

import { ScrollArea } from "@/components/ui/scroll-area";
import { BluetoothPanel } from "@/components/bluetooth/bluetooth-panel";
import { ClipboardPanel } from "@/components/clipboard/clipboard-panel";
import { HomeDashboard } from "@/components/home/home-dashboard";
import { MonitorPanel } from "@/components/monitor/monitor-panel";
import { NotesPanel } from "@/components/notes/notes-panel";
import { SearchPanel } from "@/components/search/search-panel";
import { cn } from "@/lib/utils";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";

import { AppSidebar } from "./sidebar-nav";
import { SettingsPanel } from "./settings-panel";
import { TitleBar } from "./title-bar";

/**
 * 一体式外壳（参照 x-hub）：页面底色是固定的柔和渐变（`body`），
 * 标题栏与侧栏保持透明，工作区不再铺一层实色面板 —— 内容以玻璃卡片直接浮在渐变上。
 */
export function AppShell() {
  const activeView = useAppStore((state) => state.activeView);

  // 数据型页面（监控 / 搜索 / 剪贴板 / 蓝牙）充分利用窗口宽度，表单型页面保持窄栏更易读。
  const contentWidth = {
    home: "max-w-3xl",
    settings: "max-w-3xl",
    monitor: "max-w-none",
    search: "max-w-none",
    bluetooth: "max-w-none",
    clipboard: "max-w-none",
    notes: "max-w-none",
  }[activeView];

  return (
    <div className="relative isolate flex h-screen w-screen flex-col overflow-hidden text-foreground">
      <WallpaperLayer />
      <TitleBar />

      <div className="flex min-h-0 flex-1">
        <AppSidebar />

        <main className="min-w-0 flex-1 overflow-hidden">
          {activeView === "home" ? (
            <HomeDashboard />
          ) : (
            <ScrollArea className="h-full">
              <div
                className={cn(
                  "mx-auto w-full px-6 py-5 pb-8",
                  contentWidth,
                  // 剪贴板 / 速记是整页数据视图：撑满视口高度，让内容自己滚动。
                  (activeView === "clipboard" || activeView === "notes") &&
                    "flex h-full flex-col",
                )}
              >
                {activeView === "search" && <SearchPanel />}
                {activeView === "settings" && <SettingsPanel />}
                {activeView === "monitor" && <MonitorPanel />}
                {activeView === "bluetooth" && <BluetoothPanel />}
                {activeView === "clipboard" && <ClipboardPanel />}
                {activeView === "notes" && <NotesPanel />}
              </div>
            </ScrollArea>
          )}
        </main>
      </div>
    </div>
  );
}

function WallpaperLayer() {
  const path = useAppStore((state) => state.config?.wallpaper ?? "");
  const blur = useAppStore((state) => state.config?.wallpaperBlur ?? 18);
  const veil = useAppStore((state) => state.config?.wallpaperVeil ?? 35);
  const [src, setSrc] = useState<string | null>(null);

  // 只读一次：配置里存路径，字节由 Rust 读成 data URL（不经过 fs 插件 scope）。
  useEffect(() => {
    let active = true;
    if (!path) {
      setSrc(null);
      return;
    }
    void ipc
      .readWallpaper(path)
      .then((data) => {
        if (active) setSrc(data);
      })
      .catch(() => {
        if (active) setSrc(null);
      });
    return () => {
      active = false;
    };
  }, [path]);

  // 壁纸真的加载出来后才把 `data-wallpaper` 挂到 <html>：
  // CSS 用 `html[data-wallpaper]` 把「亮色实心白卡」切到半透明玻璃，
  // 并给暗色卡片换成深色基底（亮壁纸下白字才有对比度）。读图失败时保持实心卡。
  useEffect(() => {
    if (!src) return;
    const root = document.documentElement;
    root.dataset.wallpaper = "1";
    return () => {
      delete root.dataset.wallpaper;
    };
  }, [src]);

  if (!src) return null;

  return (
    <div className="wallpaper-layer" aria-hidden="true">
      <div
        className="wallpaper-image"
        style={{ backgroundImage: `url("${src}")`, filter: `blur(${blur}px)` }}
      />
      <div className="wallpaper-veil" style={{ opacity: veil / 100 }} />
    </div>
  );
}
