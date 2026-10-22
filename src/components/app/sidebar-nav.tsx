import {
  Activity,
  Bluetooth,
  ClipboardList,
  FolderGit2,
  House,
  NotebookPen,
  Search,
  Settings,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useAppStore, type AppView } from "@/stores/app-store";

const items: { id: AppView; label: string; icon: typeof House }[] = [
  { id: "home", label: "首页", icon: House },
  { id: "search", label: "搜索", icon: Search },
  { id: "projects", label: "项目", icon: FolderGit2 },
  { id: "monitor", label: "监控", icon: Activity },
  { id: "bluetooth", label: "蓝牙", icon: Bluetooth },
  { id: "clipboard", label: "剪贴板", icon: ClipboardList },
  { id: "notes", label: "速记", icon: NotebookPen },
  { id: "settings", label: "设置", icon: Settings },
];

/**
 * 左侧导航（参照 x-hub 的极简侧轨）：固定 160px，当前项用强调色软底而不是下划线。
 */
export function AppSidebar() {
  const activeView = useAppStore((state) => state.activeView);
  const setActiveView = useAppStore((state) => state.setActiveView);

  return (
    <aside className="flex w-40 shrink-0 flex-col gap-1 p-2 pt-1">
      {items.map(({ id, label, icon: Icon }) => {
        const active = activeView === id;
        return (
          <Button
            key={id}
            variant="ghost"
            aria-current={active ? "page" : undefined}
            aria-label={label}
            className={cn(
              "h-8 justify-start gap-2",
              active && "bg-brand-soft text-foreground hover:bg-brand-soft",
            )}
            onClick={() => setActiveView(id)}
          >
            <Icon className="size-4 shrink-0" />
            <span className="truncate">{label}</span>
          </Button>
        );
      })}
    </aside>
  );
}
