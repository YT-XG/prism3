import { Headphones, Keyboard, Mouse } from "lucide-react";
import { useState } from "react";
import { createPortal } from "react-dom";

import { DeviceSummaryList } from "@/components/devices/device-summary-list";
import { useEarbuds } from "@/hooks/use-earbuds";
import { useMouseBattery } from "@/hooks/use-mouse-battery";
import { useWirelessBatteries } from "@/hooks/use-wireless-batteries";
import { connectedDevices } from "@/lib/devices";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/**
 * 任务栏托盘里的「设备电量」：一个图标 + 主设备最紧张的那格电量，点开是完整电量列表。
 *
 * 为什么放托盘而不是桌面小组件：电量是**系统状态**，和时钟、网络同级；
 * 放在任务栏既不占桌面，也不用为了看电量去开窗口（Windows 托盘也是这个心智）。
 * 列表本体复用 `DeviceSummaryList`——与 hub 左栏、设备页同一份版式。
 */
export function DeviceTray() {
  const [open, setOpen] = useState(false);
  const earbuds = useEarbuds();
  const mouse = useMouseBattery(true);
  const wireless = useWirelessBatteries(true);
  const setActiveView = useAppStore((state) => state.setActiveView);

  const devices = connectedDevices(earbuds.state, mouse, wireless);

  // 托盘上一台设备一段：耳机给「左·右·仓」三格，其余给单格百分比。
  // 全部平铺而不是只报一个最小值——一眼就能看出是哪一格在掉电。
  const chips = devices.slice(0, 4).map((device) => {
    if (device.kind === "earbuds") {
      const values = [device.battery?.left, device.battery?.right, device.battery?.case];
      const text = values.map((value) => (typeof value === "number" ? value : "—")).join("·");
      const percent = Math.min(
        ...values.filter((value): value is number => typeof value === "number"),
      );
      return {
        key: device.id,
        icon: Headphones,
        text,
        low: Number.isFinite(percent) && percent <= 20,
        title: `${device.name} · 左 ${values[0] ?? "—"}% · 右 ${values[1] ?? "—"}% · 仓 ${values[2] ?? "—"}%`,
      };
    }

    const Icon =
      device.kind === "mouse"
        ? Mouse
        : device.category === "keyboard"
          ? Keyboard
          : Headphones;
    const percent = device.percent;
    return {
      key: device.id,
      icon: Icon,
      text:
        percent === null
          ? "—"
          : `${device.kind === "wireless" && device.approx ? "≈" : ""}${percent}%`,
      low: percent !== null && percent <= 20,
      title: `${device.name} · ${percent === null ? "未上报电量" : `${percent}%`}`,
    };
  });

  const popover = (
    <>
      <div
        className="fixed inset-0 z-[55]"
        aria-hidden
        onPointerDown={() => setOpen(false)}
      />
      <div
        role="dialog"
        aria-label="设备电量"
        className="menu-in fixed right-3 z-[60] w-[304px] rounded-2xl border p-3"
        style={{
          bottom: "calc(var(--taskbar-h) + 8px)",
          background: "var(--window-bg)",
          borderColor: "var(--window-border)",
          boxShadow: "0 28px 60px -30px rgba(16, 40, 80, 0.55)",
          backdropFilter: "blur(26px) saturate(1.3)",
          WebkitBackdropFilter: "blur(26px) saturate(1.3)",
        }}
      >
        <header className="mb-2 flex items-center justify-between">
          <span className="text-[10.5px] font-semibold tracking-[0.12em] text-muted-foreground uppercase">
            设备电量
          </span>
          <span className="text-[10px] text-muted-foreground tabular-nums">
            {devices.length > 0 ? `${devices.length} 台在线` : "未连接"}
          </span>
        </header>

        <DeviceSummaryList
          devices={devices}
          earbudError={earbuds.state?.error ?? null}
          onReadEarbuds={() => void earbuds.readBattery()}
        />

        <button
          type="button"
          onClick={() => {
            setActiveView("devices");
            setOpen(false);
          }}
          className="mt-2 w-full rounded-lg py-1.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-black/5 hover:text-foreground dark:hover:bg-white/8"
        >
          打开设备窗口
        </button>
      </div>
    </>
  );

  return (
    <>
      <div className="flex items-center gap-0.5">
        {chips.length === 0 ? (
          <button
            type="button"
            aria-label="设备电量"
            aria-expanded={open}
            title="没有已连接的设备"
            onClick={() => setOpen((value) => !value)}
            className={cn(
              "flex h-8 items-center gap-1.5 rounded-full px-2 transition-colors",
              open ? "bg-brand/10" : "hover:bg-black/6 dark:hover:bg-white/10",
            )}
          >
            <Headphones className="size-3.5 text-muted-foreground" strokeWidth={2} />
            <span className="text-[10.5px] font-medium text-muted-foreground">—</span>
          </button>
        ) : (
          chips.map((chip) => {
            const Icon = chip.icon;
            return (
              <button
                key={chip.key}
                type="button"
                aria-label="设备电量"
                aria-expanded={open}
                title={chip.title}
                onClick={() => setOpen((value) => !value)}
                className={cn(
                  "flex h-8 items-center gap-1 rounded-full px-1.5 transition-colors",
                  open ? "bg-brand/10" : "hover:bg-black/6 dark:hover:bg-white/10",
                )}
              >
                <Icon
                  className={cn("size-3.5", chip.low ? "text-amber-500" : "text-muted-foreground")}
                  strokeWidth={2}
                />
                <span
                  className={cn(
                    "text-[10.5px] font-medium tabular-nums",
                    chip.low ? "text-amber-600 dark:text-amber-400" : "text-foreground",
                  )}
                >
                  {chip.text}
                </span>
              </button>
            );
          })
        )}
        {devices.length > 4 ? (
          <span className="px-1 text-[10px] text-muted-foreground tabular-nums">
            +{devices.length - 4}
          </span>
        ) : null}
      </div>

      {open ? createPortal(popover, document.body) : null}
    </>
  );
}
