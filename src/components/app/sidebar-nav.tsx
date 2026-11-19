import {
  Activity,
  MonitorSmartphone,
  ClipboardList,
  FolderGit2,
  House,
  ListChecks,
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
  { id: "work", label: "工作", icon: ListChecks },
  { id: "monitor", label: "监控", icon: Activity },
  { id: "devices", label: "设备", icon: MonitorSmartphone },
  { id: "clipboard", label: "剪贴板", icon: ClipboardList },
  { id: "notes", label: "速记", icon: NotebookPen },
];

/** 设置固定在侧栏底部，和上面的日常导航分开。 */
const settingsItem: { id: AppView; label: string; icon: typeof House } = {
  id: "settings",
  label: "设置",
  icon: Settings,
};

/**
 * 左侧导航（参照 x-hub 的极简侧轨）：固定 160px，当前项用强调色软底 + 左侧光束，
 * 不用下划线；设置常驻底部，日常入口不会因为它而上下浮动。
 */
export function AppSidebar() {
  const activeView = useAppStore((state) => state.activeView);
  const setActiveView = useAppStore((state) => state.setActiveView);

  return (
    <aside className="flex w-40 shrink-0 flex-col overflow-y-auto p-2 pt-1">
      <nav className="flex flex-col gap-0.5" aria-label="主导航">
        {items.map(({ id, label, icon: Icon }) => {
          const active = activeView === id;
          return (
            <NavButton
              key={id}
              active={active}
              label={label}
              icon={Icon}
              onClick={() => setActiveView(id)}
            />
          );
        })}
      </nav>

      <div className="mt-auto flex flex-col gap-0.5 pt-3">
        <span className="mb-1 h-px bg-border/60" aria-hidden />
        <NavButton
          active={activeView === settingsItem.id}
          label={settingsItem.label}
          icon={settingsItem.icon}
          onClick={() => setActiveView(settingsItem.id)}
        />
      </div>
    </aside>
  );
}

function NavButton({
  active,
  label,
  icon: Icon,
  onClick,
}: {
  active: boolean;
  label: string;
  icon: typeof House;
  onClick: () => void;
}) {
  return (
    <Button
      variant="ghost"
      aria-current={active ? "page" : undefined}
      aria-label={label}
      className={cn(
        "relative h-8 justify-start gap-2 pl-3",
        active && "bg-brand-soft text-foreground hover:bg-brand-soft",
      )}
      onClick={onClick}
    >
      <span
        aria-hidden
        className={cn(
          "absolute inset-y-1.5 left-0 w-0.5 rounded-full bg-brand transition-opacity",
          active ? "opacity-100" : "opacity-0",
        )}
      />
      <Icon className="size-4 shrink-0" />
      <span className="truncate">{label}</span>
    </Button>
  );
}
