import { useEffect, useState } from "react";

import { PrismMark } from "@/components/app/prism-mark";
import { useAppStatus } from "@/hooks/use-app-status";
import { useAppMeta } from "@/hooks/use-app-catalog";
import { DOCK_ORDER } from "@/lib/apps";
import { cn } from "@/lib/utils";
import { useWindowStore } from "@/stores/window-store";

import { DeviceTray } from "./device-tray";

const STATUS_TEXT = {
  normal: "运行正常",
  busy: "负载偏高",
  warn: "资源紧张",
} as const;

const STATUS_COLOR = {
  normal: "#22c55e",
  busy: "#f59e0b",
  warn: "#ef4444",
} as const;

/**
 * 底部任务栏（56px）：开始按钮 + 运行中的应用 + 系统托盘。
 *
 * 与参考站一致——半透明底板、顶部发丝线、应用按钮用「图标 + 下划线」表示运行态；
 * 多出来的只有托盘里的运行状态点，它把原来快捷小窗灰条的呼吸灯搬进了主窗口。
 */
export function Taskbar({
  menuOpen,
  onToggleMenu,
}: {
  menuOpen: boolean;
  onToggleMenu: () => void;
}) {
  const windows = useWindowStore((state) => state.windows);
  const focused = useWindowStore((state) => state.focused);
  const openWindow = useWindowStore((state) => state.open);
  const focusWindow = useWindowStore((state) => state.focus);
  const minimizeWindow = useWindowStore((state) => state.minimize);
  const status = useAppStatus();

  const running = DOCK_ORDER.map((id) => windows.find((item) => item.id === id)).filter(
    (item): item is NonNullable<typeof item> => Boolean(item),
  );

  return (
    <footer
      className="taskbar-in absolute inset-x-0 bottom-0 z-50 flex h-[var(--taskbar-h)] items-center gap-1 px-2"
      style={{
        background: "var(--taskbar-bg)",
        borderTop: "1px solid var(--taskbar-border)",
        backdropFilter: "blur(26px) saturate(1.35)",
        WebkitBackdropFilter: "blur(26px) saturate(1.35)",
      }}
    >
      <button
        type="button"
        aria-label="开始"
        aria-expanded={menuOpen}
        title="开始"
        onClick={onToggleMenu}
        className={cn(
          "flex size-10 items-center justify-center rounded-xl transition-colors",
          menuOpen ? "bg-brand/10" : "hover:bg-black/6 dark:hover:bg-white/10",
        )}
      >
        <span className="flex size-[30px] items-center justify-center rounded-[9px] bg-brand/10 text-brand"><PrismMark className="size-[17px]" /></span>
      </button>

      <span className="mx-1 h-6 w-px bg-[color:var(--taskbar-border)]" aria-hidden />

      <div className="flex min-w-0 flex-1 items-center gap-1 overflow-hidden">
        {running.length === 0 ? (
          <span className="px-2 text-[11px] text-muted-foreground">
            点左侧图标或开始菜单打开应用
          </span>
        ) : null}

        {running.map((item) => {
          const active = focused === item.id && !item.minimized;
          return (
            <TaskbarApp
              key={item.id}
              id={item.id}
              minimized={item.minimized}
              active={active}
              onActivate={() => {
                if (item.minimized) openWindow(item.id);
                else if (active) minimizeWindow(item.id);
                else focusWindow(item.id);
              }}
            />
          );
        })}
      </div>

      <div className="ml-auto flex items-center gap-2 pr-1">
        <DeviceTray />

        <span
          className="flex items-center gap-1.5 rounded-full bg-black/4 px-2 py-1 dark:bg-white/8"
          title={STATUS_TEXT[status]}
        >
          <span
            className="size-[6px] rounded-full"
            style={{ backgroundColor: STATUS_COLOR[status] }}
          />
          <span className="text-[10px] font-medium text-muted-foreground">
            {STATUS_TEXT[status]}
          </span>
        </span>

        <TrayClock />
      </div>
    </footer>
  );
}

/** 托盘时钟：时间在上、日期在下（与参考站同构）。 */
function TrayClock() {
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1000 * 15);
    return () => window.clearInterval(timer);
  }, []);

  const time = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  const date = `${now.getFullYear()}/${now.getMonth() + 1}/${now.getDate()}`;

  return (
    <div className="flex min-w-[64px] flex-col items-end leading-none">
      <span className="text-[12px] font-semibold tabular-nums text-foreground">{time}</span>
      <span className="mt-1 text-[10px] tabular-nums text-muted-foreground">{date}</span>
    </div>
  );
}


/** 任务栏上的一个「运行中的应用」：图标 + 名称 + 当前态下划线（支持自建应用的 emoji 图标）。 */
function TaskbarApp({
  id,
  minimized,
  active,
  onActivate,
}: {
  id: string;
  minimized: boolean;
  active: boolean;
  onActivate: () => void;
}) {
  const app = useAppMeta(id);
  const Icon = app.icon;

  return (
    <button
      type="button"
      title={app.title}
      onClick={onActivate}
      className={cn(
        "relative flex h-9 shrink-0 items-center gap-2 rounded-lg px-2.5 transition-colors",
        active
          ? "bg-white/70 shadow-[0_1px_3px_rgba(16,40,80,.12)] dark:bg-white/12"
          : "hover:bg-white/45 dark:hover:bg-white/8",
        minimized && "opacity-70",
      )}
    >
      <span
        className="flex size-[22px] items-center justify-center rounded-[7px] text-white"
        style={{ backgroundImage: `linear-gradient(150deg, ${app.tint[0]}, ${app.tint[1]})` }}
      >
        {Icon ? (
          <Icon className="size-[13px]" strokeWidth={2} />
        ) : (
          <span className="text-[11px] leading-none">{app.emoji ?? "📦"}</span>
        )}
      </span>
      <span className="hidden max-w-28 truncate text-[11.5px] font-medium text-foreground min-[1180px]:inline">
        {app.label}
      </span>
      {active ? (
        <span className="absolute inset-x-2 -bottom-[3px] h-[2px] rounded-full bg-brand" />
      ) : null}
    </button>
  );
}
