import { useEffect, useState } from "react";

import { DesktopIcons } from "@/components/desktop/desktop-icons";
import { DesktopSearch } from "@/components/desktop/desktop-search";
import { DesktopWallpaper } from "@/components/desktop/desktop-wallpaper";
import { DesktopWidgets } from "@/components/desktop/desktop-widgets";
import { StartMenu } from "@/components/desktop/start-menu";
import { Taskbar } from "@/components/desktop/taskbar";
import { TopBar } from "@/components/desktop/top-bar";
import { WindowLayer } from "@/components/desktop/window-layer";
import { useWindowStore } from "@/stores/window-store";

/**
 * 桌面外壳：壁纸 + 图标坞 + 桌面搜索 + 小组件 + 浮层窗口 + 任务栏。
 *
 * 与旧外壳（标题栏 + 侧栏 + 内容区）的分工变化：
 * - 导航从「左侧 160px 常驻侧轨」改为「桌面图标 + 任务栏 + 开始菜单」；
 * - 页面不再整页替换，而是装进可拖动/缩放的浮层窗口（`components/desktop/`）；
 * - 顶部只留 34px 原生窗口把手（拖动 + 三键），任务栏 56px。
 *
 * 窗口的开合由 `window-store` 管理；`app-store.setActiveView` 会顺带打开对应窗口，
 * 所以组合窗口、首页卡片、设置页里的跨页跳转都不需要改接线。
 */
export function AppShell() {
  const [menuOpen, setMenuOpen] = useState(false);
  const clampAll = useWindowStore((state) => state.clampAll);

  // 窗口尺寸（拖边、最大化、系统缩放）变化后把窗口收回桌面范围。
  useEffect(() => {
    const onResize = () => clampAll();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [clampAll]);

  return (
    <div className="relative isolate h-screen w-screen overflow-hidden text-foreground">
      <DesktopWallpaper />
      <TopBar />

      {/* 桌面层：壁纸上的图标、搜索条与小组件；点空白处收起开始菜单。 */}
      <div
        className="absolute inset-x-0 z-0"
        style={{ top: "var(--topbar-h)", bottom: "var(--taskbar-h)" }}
        onPointerDown={() => setMenuOpen(false)}
      >
        <DesktopIcons />
        <DesktopSearch />
        <DesktopWidgets />
      </div>

      <WindowLayer />

      {menuOpen ? <StartMenu onClose={() => setMenuOpen(false)} /> : null}

      <Taskbar menuOpen={menuOpen} onToggleMenu={() => setMenuOpen((open) => !open)} />
    </div>
  );
}
