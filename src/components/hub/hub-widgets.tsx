import { BatteryCharging, Headphones, Mouse, Zap } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useEarbuds } from "@/hooks/use-earbuds";
import { useMouseBattery } from "@/hooks/use-mouse-battery";
import {
  connectedDevices,
  earbudsBatterySummary,
  mouseBatterySummary,
} from "@/lib/devices";
import { oneBudUnavailable } from "@/lib/ble-format";

/**
 * 组合窗口左侧「设备」小视图：已连接设备的只读摘要（耳机 + 2.4G 鼠标）。
 *
 * 与「设备」页同源——两边都调 `connectedDevices()` 这个纯函数，只是这边排得更紧、
 * 不提供跳转。**不渲染任何跳转入口**：组合窗口是独立 WebView，
 * `useAppStore.setActiveView` 改的是本窗口那份没人看的视图状态，点下去毫无反馈。
 * 完整操作请回主窗口的「设备」页。
 */
export function HubDevicesWidget({ active }: { active: boolean }) {
  const earbuds = useEarbuds();
  // 组合窗口是隐藏而非卸载：失焦时停表，别让后台窗口还在按 30s 读 HID。
  const mouse = useMouseBattery(active);
  const devices = connectedDevices(earbuds.state, mouse);

  if (devices.length === 0) {
    return (
      <div className="flex flex-col gap-1.5">
        <p className="text-xs text-muted-foreground">还没有已连接的设备</p>
        <p className="text-[0.65rem] leading-relaxed text-muted-foreground">
          取出耳机或插上鼠标的 2.4G 接收器后会自动出现在这里。
        </p>
      </div>
    );
  }

  return (
    <ul className="flex flex-col gap-2.5">
      {devices.map((device) =>
        device.kind === "earbuds" ? (
          <li key={device.id} className="flex flex-col gap-1.5">
            <div className="flex items-start justify-between gap-2">
              <span className="flex min-w-0 items-center gap-1.5">
                <Headphones className="size-3.5 shrink-0 text-brand" aria-hidden />
                <span className="truncate text-xs font-medium">{device.name}</span>
              </span>
              <Badge variant="secondary" className="shrink-0">
                已连接
              </Badge>
            </div>

            <p className="truncate text-[0.65rem] text-muted-foreground">
              {device.address} · {earbudsBatterySummary(device.battery)}
            </p>

            {device.tags.length > 0 && (
              <div className="flex flex-wrap gap-1">
                {device.tags.map((tag) => (
                  <Badge key={tag} variant="outline">
                    {tag}
                  </Badge>
                ))}
              </div>
            )}

            {!device.battery && (
              <Button
                variant="outline"
                size="xs"
                className="self-start"
                onClick={() => void earbuds.readBattery()}
              >
                <BatteryCharging data-icon="inline-start" />
                读取电量
              </Button>
            )}

            {earbuds.state?.error ? (
              <p className="text-[0.65rem] leading-relaxed break-words text-destructive">
                {earbuds.state.error}
              </p>
            ) : null}

            {oneBudUnavailable(device.battery) ? (
              <p className="text-[0.65rem] text-muted-foreground">
                有一只耳机没有上报电量；只有一只在线时，降噪梯度可能切换不动。
              </p>
            ) : null}
          </li>
        ) : (
          <li key={device.id} className="flex flex-col gap-1">
            <div className="flex items-start justify-between gap-2">
              <span className="flex min-w-0 items-center gap-1.5">
                <Mouse className="size-3.5 shrink-0 text-brand" aria-hidden />
                <span className="truncate text-xs font-medium">{device.name}</span>
              </span>
              <Badge variant="secondary" className="shrink-0">
                {device.subtitle}
              </Badge>
            </div>
            <p className="flex items-center gap-1 truncate text-[0.65rem] text-muted-foreground">
              {mouseBatterySummary(device)}
              {device.charging && (
                <Zap className="size-3 text-emerald-600 dark:text-emerald-500" aria-hidden />
              )}
            </p>
          </li>
        ),
      )}
    </ul>
  );
}