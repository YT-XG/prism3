import {
  Activity,
  Bluetooth,
  ClipboardList,
  Search,
  Settings,
  type LucideIcon,
} from "lucide-react";

import { HomeCardLink } from "@/components/home/home-card";
import { useUpdate } from "@/hooks/use-update";
import type { AppView } from "@/stores/app-store";
import { useAppStore } from "@/stores/app-store";

const LINKS: { view: AppView; label: string; icon: LucideIcon }[] = [
  { view: "search", label: "文件搜索", icon: Search },
  { view: "monitor", label: "系统监控", icon: Activity },
  { view: "bluetooth", label: "蓝牙耳机", icon: Bluetooth },
  { view: "clipboard", label: "剪贴板", icon: ClipboardList },
  { view: "settings", label: "设置", icon: Settings },
];

/** 更新状态 → 一句话 + 是否可点「检查更新」。 */
function describeUpdate(status: string | undefined): { text: string; checkable: boolean } {
  switch (status) {
    case "checking":
      return { text: "检查中…", checkable: false };
    case "upToDate":
      return { text: "已是最新", checkable: true };
    case "available":
    case "downloading":
      return { text: "正在下载新版本", checkable: false };
    case "paused":
      return { text: "下载已暂停", checkable: false };
    case "downloaded":
      return { text: "更新已就绪", checkable: false };
    case "error":
      return { text: "检查失败", checkable: true };
    default:
      return { text: "检查更新", checkable: true };
  }
}

/** 首页「常用入口」卡片：五个页面快捷入口 + 版本 / 更新状态。 */
export function ShortcutsCard() {
  const setActiveView = useAppStore((state) => state.setActiveView);
  const { state, check } = useUpdate();

  const update = describeUpdate(state?.status);
  const version = state?.currentVersion ? `v${state.currentVersion}` : "—";

  const onUpdateClick = () => {
    if (state?.status === "downloaded" || !update.checkable) {
      setActiveView("settings");
      return;
    }
    void check();
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="grid grid-cols-2 gap-1.5">
        {LINKS.map(({ view, label, icon: Icon }, index) => (
          <button
            key={view}
            type="button"
            onClick={() => setActiveView(view)}
            className={
              "flex items-center gap-2 rounded-lg border border-border/60 bg-muted/30 px-2 py-1.5 text-left text-xs transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none" +
              (index === LINKS.length - 1 ? " col-span-2" : "")
            }
          >
            <Icon className="size-3.5 shrink-0 text-brand" aria-hidden="true" />
            <span className="truncate">{label}</span>
          </button>
        ))}
      </div>

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2">
        <span className="text-[0.65rem] text-muted-foreground tabular-nums">{version}</span>
        <HomeCardLink onClick={onUpdateClick}>{update.text}</HomeCardLink>
      </div>
    </div>
  );
}
