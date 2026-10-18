import { Activity, Bluetooth, House, Search, Settings } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useAppStore, type AppView } from "@/stores/app-store";

const items: { id: AppView; label: string; icon: typeof House }[] = [
  { id: "home", label: "首页", icon: House },
  { id: "search", label: "搜索", icon: Search },
  { id: "monitor", label: "监控", icon: Activity },
  { id: "bluetooth", label: "蓝牙", icon: Bluetooth },
  { id: "settings", label: "设置", icon: Settings },
];

export function AppSidebar() {
  const activeView = useAppStore((state) => state.activeView);
  const setActiveView = useAppStore((state) => state.setActiveView);

  return (
    <aside className="flex w-56 shrink-0 flex-col gap-1 p-3 pt-1">
      {items.map(({ id, label, icon: Icon }) => (
        <Button
          key={id}
          variant={activeView === id ? "secondary" : "ghost"}
          className="justify-start"
          onClick={() => setActiveView(id)}
        >
          <Icon data-icon="inline-start" />
          {label}
        </Button>
      ))}
    </aside>
  );
}
