import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Moon, Square, Sun, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

import { PrismMark } from "@/components/app/prism-mark";
import { useAppVersion } from "@/hooks/use-app-version";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/**
 * 桌面顶栏（34px）：唯一一块「原生窗口」的把手。
 *
 * 无边框窗口必须留一处可拖动的区域和三个窗口键，参考站的窗口没有这一层，
 * 所以这里压到最小：左侧只有品牌标记与版本，右侧只有主题与三键，
 * 整条铺在壁纸上，靠 `data-tauri-drag-region="deep"` 让空白处可拖动。
 */
export function TopBar() {
  const theme = useAppStore((state) => state.config?.theme);
  const saveConfig = useAppStore((state) => state.saveConfig);
  const version = useAppVersion();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let unlisten: (() => void) | undefined;

    void appWindow.isMaximized().then(setMaximized).catch(() => undefined);
    void appWindow
      .onResized(() => {
        void appWindow.isMaximized().then(setMaximized).catch(() => undefined);
      })
      .then((off) => {
        unlisten = off;
      })
      .catch(() => undefined);

    return () => unlisten?.();
  }, []);

  const appWindow = getCurrentWindow();
  const isDark = theme === "dark";

  const toggleTheme = useCallback(() => {
    void saveConfig({ theme: isDark ? "light" : "dark" }).catch((cause) =>
      toast.error(`切换主题失败：${String(cause)}`),
    );
  }, [isDark, saveConfig]);

  return (
    <header
      data-tauri-drag-region="deep"
      className="absolute inset-x-0 top-0 z-40 flex h-[var(--topbar-h)] items-center gap-2 px-3 select-none"
    >
      <span className="flex size-[21px] shrink-0 items-center justify-center rounded-[6px] text-brand">
        <PrismMark className="size-[15px]" />
      </span>
      <span className="text-[10px] font-semibold tracking-[0.2em] text-[color:var(--dock-label)] uppercase">
        Prism3
      </span>
      {version ? (
        <span className="text-[10px] font-medium text-muted-foreground tabular-nums">v{version}</span>
      ) : null}

      <div className="ml-auto flex items-center gap-0.5">
        <BarButton label="切换主题" onClick={toggleTheme}>
          {isDark ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
        </BarButton>

        <span className="mx-1 h-3.5 w-px bg-[color:var(--taskbar-border)]" aria-hidden />

        <BarButton label="最小化" onClick={() => void appWindow.minimize()}>
          <Minus className="size-3.5" />
        </BarButton>
        <BarButton
          label={maximized ? "还原" : "最大化"}
          onClick={() => void appWindow.toggleMaximize()}
        >
          {maximized ? <Copy className="size-3.5" /> : <Square className="size-3" />}
        </BarButton>
        <BarButton label="关闭" danger onClick={() => void appWindow.close()}>
          <X className="size-3.5" />
        </BarButton>
      </div>
    </header>
  );
}

function BarButton({
  label,
  onClick,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors",
        "hover:bg-black/6 hover:text-foreground dark:hover:bg-white/10",
        danger && "hover:bg-[#e81123] hover:text-white dark:hover:bg-[#e81123]",
      )}
    >
      {children}
    </button>
  );
}
