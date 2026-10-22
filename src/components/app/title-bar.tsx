import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Moon, Settings, Square, Sun, X } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/**
 * 自定义标题栏（无边框窗口）。
 *
 * `data-tauri-drag-region="deep"` 让整条标题栏的空白区域都可拖动，
 * 且 Tauri 内置脚本已处理「双击最大化」；按钮等可点击元素会自动阻断拖动。
 */
export function TitleBar() {
  const activeView = useAppStore((state) => state.activeView);
  const setActiveView = useAppStore((state) => state.setActiveView);
  const theme = useAppStore((state) => state.config?.theme);
  const saveConfig = useAppStore((state) => state.saveConfig);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let unlisten: (() => void) | undefined;

    void appWindow.isMaximized().then(setMaximized);
    void appWindow
      .onResized(() => {
        void appWindow.isMaximized().then(setMaximized);
      })
      .then((off) => {
        unlisten = off;
      });

    return () => unlisten?.();
  }, []);

  const appWindow = getCurrentWindow();
  const isDark = theme === "dark";

  const toggleTheme = () =>
    saveConfig({ theme: isDark ? "light" : "dark" }).catch((cause) =>
      toast.error(`切换主题失败：${String(cause)}`),
    );

  return (
    <header
      data-tauri-drag-region="deep"
      className="flex h-12 shrink-0 items-center select-none"
    >
      <div className="w-40 shrink-0 px-3 font-heading text-xs font-semibold tracking-[0.14em] uppercase">
        Prism3
      </div>

      <div className="flex flex-1 items-center justify-end gap-0.5 pr-1">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="设置"
          title="设置"
          className={cn(activeView === "settings" && "bg-muted")}
          onClick={() => setActiveView("settings")}
        >
          <Settings className="size-3.5" />
        </Button>

        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="切换主题"
          title="切换主题"
          onClick={() => void toggleTheme()}
        >
          {isDark ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
        </Button>

        <div className="mx-1 h-5 w-px bg-border" />

        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="最小化"
          title="最小化"
          onClick={() => void appWindow.minimize()}
        >
          <Minus className="size-3.5" />
        </Button>

        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={maximized ? "还原" : "最大化"}
          title={maximized ? "还原" : "最大化"}
          onClick={() => void appWindow.toggleMaximize()}
        >
          {maximized ? (
            <Copy className="size-3.5" />
          ) : (
            <Square className="size-3.5" />
          )}
        </Button>

        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="关闭"
          title="关闭"
          className="hover:bg-destructive hover:text-white"
          onClick={() => void appWindow.close()}
        >
          <X className="size-3.5" />
        </Button>
      </div>
    </header>
  );
}
