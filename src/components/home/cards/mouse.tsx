import { BatteryCharging, BatteryLow, BatteryMedium, BatteryFull, Usb } from "lucide-react";

import { HomeBar } from "@/components/home/home-card";
import { useMouseBattery } from "@/hooks/use-mouse-battery";
import { cn } from "@/lib/utils";

/** 电量分档：与官方驱动一致（≤20 / ≤40 / ≤60 / ≤80 / 更高）。 */
function batteryIcon(percent: number) {
  if (percent <= 20) return BatteryLow;
  if (percent <= 60) return BatteryMedium;
  return BatteryFull;
}

/**
 * 首页「鼠标电量」卡片：雷神 ML903 接收器的电量与充电状态。
 *
 * 数据来自 HID feature report 私有协议（`src-tauri/src/mouse.rs`）。有线模式下设备
 * 回 0，官方驱动也把它当作「没有电量信息」，因此这里显示提示而不是 `0%`。
 */
export function MouseCard() {
  const battery = useMouseBattery();
  const percent = battery.percent;
  const Icon = battery.charging
    ? BatteryCharging
    : percent === null
      ? Usb
      : batteryIcon(percent);

  return (
    <div className="flex h-full min-h-0 flex-col justify-between gap-3">
      <div className="flex items-start justify-between gap-2">
        <div>
          <p className="text-xs text-muted-foreground">ML903</p>
          <div className="mt-1 flex items-baseline gap-1.5">
            <span className="font-heading text-[2.5rem] leading-none font-semibold tracking-[-0.04em] tabular-nums">
              {percent === null ? "--" : percent}
            </span>
            <span className="text-sm font-medium text-muted-foreground">%</span>
          </div>
        </div>
        <Icon
          className={cn(
            "mt-1 size-7 shrink-0",
            battery.charging ? "text-brand" : "text-muted-foreground",
          )}
          aria-hidden
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <HomeBar percent={percent ?? 0} />
        <div className="flex items-center justify-between gap-2 text-[0.65rem] text-muted-foreground">
          <span className="truncate">{statusText(battery.connected, battery.wireless, percent)}</span>
          {battery.charging ? (
            <span className="flex shrink-0 items-center gap-1 text-brand">
              <BatteryCharging className="size-3" aria-hidden />
              充电中
            </span>
          ) : (
            <span className="shrink-0 tabular-nums">
              {battery.wireless && percent !== null ? "2.4G" : "有线"}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

function statusText(connected: boolean, wireless: boolean, percent: number | null): string {
  if (!connected) return "未检测到接收器";
  if (percent === null) {
    return wireless ? "未上报电量（可能已休眠）" : "有线模式不读电量";
  }
  if (percent <= 20) return "电量偏低";
  return "电量正常";
}
