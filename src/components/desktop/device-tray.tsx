import { Headphones, Keyboard, Mouse } from "lucide-react";
import { Fragment, useState } from "react";
import { createPortal } from "react-dom";

import { DeviceSummaryList, BatteryBar } from "@/components/devices/device-summary-list";
import { useEarbuds } from "@/hooks/use-earbuds";
import { useMouseBattery } from "@/hooks/use-mouse-battery";
import { useWirelessBatteries } from "@/hooks/use-wireless-batteries";
import { connectedDevices } from "@/lib/devices";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";

/** 托盘里一台设备的一段电量：耳机会有「左 / 右 / 仓」三行，其余只有一行。 */
interface TrayRow {
  label: string | null;
  percent: number | null;
  approx?: boolean;
}

interface TrayDevice {
  id: string;
  name: string;
  icon: typeof Headphones;
  rows: TrayRow[];
}

const fmt = (percent: number | null, approx?: boolean) =>
  percent === null ? "—" : `${approx ? "≈" : ""}${percent}`;

/**
 * 任务栏托盘里的「设备电量」：每台设备一列，列内是若干行「标签 + 电量条 + 数值」。
 *
 * 为什么不只报一个数：电量最有用的信息是**趋势与短板**（左右耳差 20% 就该换边用），
 * 一条细进度条比一个百分比好扫。耳机天然是三行（左 / 右 / 仓），
 * 鼠标、键盘各一行——所以这里按「设备列」排，而不是压成一行。
 *
 * 只塞得下 3 台（56px 高的任务栏）；更多的进浮层（点任意一列打开）。
 * 列表本体与 hub 左栏、设备页同一份 `DeviceSummaryList`。
 */
export function DeviceTray() {
  const [open, setOpen] = useState(false);
  const earbuds = useEarbuds();
  const mouse = useMouseBattery(true);
  const wireless = useWirelessBatteries(true);
  const setActiveView = useAppStore((state) => state.setActiveView);
  // 显示方式由设置页决定（默认「进度条 + 百分比」）；未知值回退默认。
  const style = useAppStore((state) => state.config?.batteryTrayStyle ?? "barPercent");
  const showBar = style !== "percent";
  const showPercent = style !== "bar";

  const devices = connectedDevices(earbuds.state, mouse, wireless);

  const columns: TrayDevice[] = devices.slice(0, 3).map((device) => {
    if (device.kind === "earbuds") {
      return {
        id: device.id,
        name: device.name,
        icon: Headphones,
        rows: [
          { label: "左", percent: device.battery?.left ?? null },
          { label: "右", percent: device.battery?.right ?? null },
          { label: "仓", percent: device.battery?.case ?? null },
        ],
      };
    }

    const icon =
      device.kind === "mouse"
        ? Mouse
        : device.category === "keyboard"
          ? Keyboard
          : device.category === "headset"
            ? Headphones
            : Mouse;

    return {
      id: device.id,
      name: device.name,
      icon,
      rows: [
        {
          label: null,
          percent: device.percent,
          approx: device.kind === "wireless" && device.approx,
        },
      ],
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

  if (columns.length === 0) {
    return (
      <button
        type="button"
        aria-label="设备电量"
        title="没有已连接的设备"
        onClick={() => setOpen((value) => !value)}
        className={cn(
          "flex h-8 items-center gap-1.5 rounded-full px-2 transition-colors",
          open ? "bg-brand/10" : "hover:bg-black/6 dark:hover:bg-white/10",
        )}
      >
        <Headphones className="size-3.5 text-muted-foreground" strokeWidth={2} />
        <span className="text-[10px] font-medium text-muted-foreground">—</span>
      </button>
    );
  }

  return (
    <>
      <div className="flex items-center">
        {columns.map((column, index) => {
          const Icon = column.icon;
          return (
            <Fragment key={column.id}>
              {index > 0 ? (
                <span
                  className="mx-1 h-7 w-px bg-[color:var(--taskbar-border)]"
                  aria-hidden
                />
              ) : null}
              <button
                type="button"
                aria-label={`设备电量：${column.name}`}
                aria-expanded={open}
                title={column.name}
                onClick={() => setOpen((value) => !value)}
                className={cn(
                  "flex items-center gap-1.5 rounded-lg px-2 py-1 transition-colors",
                  open ? "bg-brand/10" : "hover:bg-black/6 dark:hover:bg-white/10",
                )}
              >
                <Icon className="size-[15px] shrink-0 text-muted-foreground" strokeWidth={2} />
                <span className="flex flex-col gap-[5px]">
                  {column.rows.map((row) => (
                    <span
                      key={row.label ?? "value"}
                      className="flex items-center gap-1.5 text-[11px] leading-none"
                    >
                      {row.label ? (
                        <span className="w-[12px] text-muted-foreground">{row.label}</span>
                      ) : null}
                      {showBar ? (
                        <BatteryBar
                          percent={row.percent}
                          className={cn(
                            "h-[5px] shrink-0 bg-black/10 dark:bg-white/15",
                            showPercent ? "w-[46px]" : "w-[68px]",
                          )}
                        />
                      ) : null}
                      {showPercent ? (
                        <span
                          className={cn(
                            "w-[26px] text-right font-medium tabular-nums",
                            row.percent !== null && row.percent <= 20
                              ? "text-amber-600 dark:text-amber-400"
                              : "text-foreground",
                          )}
                        >
                          {fmt(row.percent, row.approx)}
                        </span>
                      ) : null}
                    </span>
                  ))}
                </span>
              </button>
            </Fragment>
          );
        })}

        {devices.length > 3 ? (
          <span className="pl-1 text-[10px] text-muted-foreground tabular-nums">
            +{devices.length - 3}
          </span>
        ) : null}
      </div>

      {open ? createPortal(popover, document.body) : null}
    </>
  );
}
