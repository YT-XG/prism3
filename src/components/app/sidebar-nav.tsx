import {
  Activity,
  Bluetooth,
  ClipboardList,
  House,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Settings,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useAppStore, type AppView } from "@/stores/app-store";

const items: { id: AppView; label: string; icon: typeof House }[] = [
  { id: "home", label: "首页", icon: House },
  { id: "search", label: "搜索", icon: Search },
  { id: "monitor", label: "监控", icon: Activity },
  { id: "bluetooth", label: "蓝牙", icon: Bluetooth },
  { id: "clipboard", label: "剪贴板", icon: ClipboardList },
  { id: "settings", label: "设置", icon: Settings },
];

/**
 * 左侧导航（参照 x-hub 的极简侧轨）：展开 224px / 收起 56px，宽度过渡 200ms。
 * 收起态只留图标 + hover 气泡（`title`），当前项用强调色软底而不是下划线。
 */
export function AppSidebar() {
  const activeView = useAppStore((state) => state.activeView);
  const setActiveView = useAppStore((state) => state.setActiveView);
  const collapsed = useAppStore((state) => state.sidebarCollapsed);
  const toggleSidebar = useAppStore((state) => state.toggleSidebar);

  return (
    <aside
      className={cn(
        "flex shrink-0 flex-col gap-1 border-r border-sidebar-border bg-sidebar p-2 pt-1 transition-[width] duration-200 ease-out",
        collapsed ? "w-14" : "w-56",
      )}
    >
      {items.map(({ id, label, icon: Icon }) => {
        const active = activeView === id;
        return (
          <Button
            key={id}
            variant="ghost"
            aria-current={active ? "page" : undefined}
            aria-label={label}
            title={collapsed ? label : undefined}
            className={cn(
              "h-8 gap-2",
              collapsed ? "justify-center px-0" : "justify-start",
              active && "bg-brand-soft text-foreground hover:bg-brand-soft",
            )}
            onClick={() => setActiveView(id)}
          >
            <Icon className="size-4 shrink-0" />
            {!collapsed && <span className="truncate">{label}</span>}
          </Button>
        );
      })}

      <Button
        variant="ghost"
        aria-label={collapsed ? "展开侧栏" : "收起侧栏"}
        title={collapsed ? "展开侧栏" : undefined}
        className={cn(
          "mt-auto h-8 gap-2 text-muted-foreground",
          collapsed ? "justify-center px-0" : "justify-start",
        )}
        onClick={toggleSidebar}
      >
        {collapsed ? (
          <PanelLeftOpen className="size-4 shrink-0" />
        ) : (
          <PanelLeftClose className="size-4 shrink-0" />
        )}
        {!collapsed && <span className="truncate">收起侧栏</span>}
      </Button>
    </aside>
  );
}
