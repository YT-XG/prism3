import { appDef, DOCK_ORDER } from "@/lib/apps";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useWindowStore } from "@/stores/window-store";

/** 桌面图标坞：两列九宫格，直接浮在壁纸上（没有托盘底板）。 */
export function DesktopIcons() {
  const windows = useWindowStore((state) => state.windows);
  const openWindow = useWindowStore((state) => state.open);
  const setActiveView = useAppStore((state) => state.setActiveView);

  const launch = (id: (typeof DOCK_ORDER)[number]) => {
    setActiveView(id);
    openWindow(id);
  };

  return (
    <div
      className="absolute left-4 grid w-[266px] grid-cols-3 gap-x-0.5 gap-y-0.5"
      style={{ top: "calc(var(--topbar-h) + 16px)" }}
      aria-label="桌面应用"
    >
      {DOCK_ORDER.map((id, index) => {
        const app = appDef(id);
        const Icon = app.icon;
        const running = windows.some((item) => item.id === id);

        return (
          <button
            key={id}
            type="button"
            onClick={() => launch(id)}
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
                <Icon className="size-[22px]" strokeWidth={1.85} />
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
    </div>
  );
}
