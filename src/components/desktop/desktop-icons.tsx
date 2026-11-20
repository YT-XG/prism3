import { Plus } from "lucide-react";

import { useAppCatalog, type AppMeta } from "@/hooks/use-app-catalog";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import type { AppView } from "@/stores/app-store";
import { useWindowStore } from "@/stores/window-store";

/** 桌面图标坞：三列九宫格（自建应用会往后排），直接浮在壁纸上。 */
export function DesktopIcons() {
  const windows = useWindowStore((state) => state.windows);
  const openWindow = useWindowStore((state) => state.open);
  const setActiveView = useAppStore((state) => state.setActiveView);
  const openCustomAppDialog = useAppStore((state) => state.openCustomAppDialog);
  const apps = useAppCatalog();

  const launch = (meta: AppMeta) => {
    if (meta.kind === "builtin") {
      setActiveView(meta.id as AppView);
      return;
    }
    openWindow(meta.id, meta.size);
  };

  return (
    <div
      className="absolute left-4 grid w-[266px] grid-cols-3 gap-x-0.5 gap-y-0.5"
      style={{ top: "calc(var(--topbar-h) + 16px)" }}
      aria-label="桌面应用"
    >
      {apps.map((app, index) => {
        const Icon = app.icon;
        const running = windows.some((item) => item.id === app.id);

        return (
          <button
            key={app.id}
            type="button"
            onClick={() => launch(app)}
            title={`${app.title} · ${app.description}`}
            style={{ "--i": index } as React.CSSProperties}
            className={cn(
              "dock-button rise-in group flex w-[86px] flex-col items-center gap-[7px] rounded-xl px-1 pt-2 pb-1.5",
              "outline-none transition-colors hover:bg-white/55 focus-visible:ring-2 focus-visible:ring-brand/50",
              "active:bg-white/70 dark:hover:bg-white/10 dark:active:bg-white/14",
            )}
          >
            <span className="relative">
              <span
                className="dock-tile flex size-[46px] items-center justify-center rounded-[13px] text-white"
                style={
                  {
                    backgroundImage: `linear-gradient(150deg, ${app.tint[0]} 0%, ${app.tint[1]} 100%)`,
                    "--tile-color": app.tint[1],
                  } as React.CSSProperties
                }
              >
                {Icon ? (
                  <Icon className="size-[22px]" strokeWidth={1.85} />
                ) : (
                  <span className="text-[21px] leading-none">{app.emoji ?? "📦"}</span>
                )}
              </span>
              {running ? (
                <span className="absolute -right-0.5 -bottom-0.5 size-[7px] rounded-full bg-brand ring-2 ring-white/90 dark:ring-[#0d1017]" />
              ) : null}
            </span>
            <span
              className="desktop-label text-[11px] leading-none font-medium"
              style={{ color: "var(--dock-label)", textShadow: "var(--dock-label-shadow)" }}
            >
              {app.label}
            </span>
          </button>
        );
      })}

      {/* 创建应用：放在坞的最后，和普通图标同样大小，点开弹「创建应用」对话框 */}
      <button
        type="button"
        onClick={() => openCustomAppDialog()}
        title="创建应用 · 一个网址或一段自定义 HTML"
        style={{ "--i": apps.length } as React.CSSProperties}
        className={cn(
          "dock-button rise-in group flex w-[86px] flex-col items-center gap-[7px] rounded-xl px-1 pt-2 pb-1.5",
          "outline-none transition-colors hover:bg-white/55 focus-visible:ring-2 focus-visible:ring-brand/50",
          "active:bg-white/70 dark:hover:bg-white/10 dark:active:bg-white/14",
        )}
      >
        <span
          className="flex size-[46px] items-center justify-center rounded-[13px] border border-dashed text-muted-foreground"
          style={{
            borderColor: "color-mix(in srgb, var(--foreground) 22%, transparent)",
            background: "color-mix(in srgb, var(--card) 45%, transparent)",
          }}
        >
          <Plus className="size-[20px]" strokeWidth={1.9} />
        </span>
        <span
          className="desktop-label text-[11px] leading-none font-medium"
          style={{ color: "var(--dock-label)", textShadow: "var(--dock-label-shadow)" }}
        >
          创建应用
        </span>
      </button>
    </div>
  );
}
