import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Moon, Square, Sun, X } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { useAppVersion } from "@/hooks/use-app-version";
import { useAppStore } from "@/stores/app-store";

import { PrismMark } from "./prism-mark";

/**
 * 自定义标题栏（无边框窗口）。
 *
 * `data-tauri-drag-region="deep"` 让整条标题栏的空白区域都可拖动，
 * 且 Tauri 内置脚本已处理「双击最大化」；按钮等可点击元素会自动阻断拖动。
 *
 * 左侧品牌列宽与侧栏一致（160px），品牌标记装在一个强调色软底方块里——
 * 这是整窗唯一的「物」：其余全是文字与细线。设置入口只在侧栏底部保留一处，
 * 这里不再放重复的齿轮按钮。
 */
export function TitleBar() {
  const theme = useAppStore((state) => state.config?.theme);
  const saveConfig = useAppStore((state) => state.saveConfig);
  const version = useAppVersion();
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
      <div className="flex w-40 shrink-0 items-center gap-2.5 px-3">
        <span className="flex size-6 shrink-0 items-center justify-center rounded-[7px] bg-brand-soft text-brand ring-1 ring-[color:color-mix(in_srgb,var(--brand)_16%,transparent)]">
          <PrismMark className="size-3.5" />
        </span>
        <span className="flex min-w-0 flex-col justify-center leading-none">
          <span className="font-heading text-[0.7rem] font-semibold tracking-[0.16em] uppercase">
            Prism3
          </span>
          {version && (
            <span className="mt-1 truncate text-[0.625rem] font-medium text-muted-foreground tabular-nums">
              v{version}
            </span>
          )}
        </span>
      </div>

      <div className="flex flex-1 items-center justify-end gap-0.5 pr-2">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="切换主题"
          title="切换主题"
          className="text-muted-foreground hover:text-foreground"
          onClick={() => void toggleTheme()}
        >
          {isDark ? <Sun className="size-3.5" /> : <Moon className="size-3.5" />}
        </Button>

        <div className="mx-1.5 h-4 w-px bg-border/70" />

        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="最小化"
          title="最小化"
          className="text-muted-foreground hover:text-foreground"
          onClick={() => void appWindow.minimize()}
        >
          <Minus className="size-3.5" />
        </Button>

        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={maximized ? "还原" : "最大化"}
          title={maximized ? "还原" : "最大化"}
          className="text-muted-foreground hover:text-foreground"
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
          className="text-muted-foreground hover:bg-destructive hover:text-white"
          onClick={() => void appWindow.close()}
        >
          <X className="size-3.5" />
        </Button>
      </div>
    </header>
  );
}
