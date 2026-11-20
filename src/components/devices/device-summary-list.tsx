import type { ReactNode } from "react";

import {
  BatteryCharging,
  Headphones,
  Keyboard,
  MonitorSmartphone,
  Mouse,
  Zap,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { oneBudUnavailable } from "@/lib/ble-format";
import type { ConnectedDevice } from "@/lib/devices";
import {
  earbudsBatterySummary,
  mouseBatterySummary,
  wirelessBatterySummary,
} from "@/lib/devices";
import { cn } from "@/lib/utils";

/**
 * 「已连接设备」摘要列表：**组合窗口左栏与桌面小组件共用这一份**。
 *
 * 与「设备」页同源——两边都调 `connectedDevices()` 这个纯函数，只是这里排得更紧、
 * 不提供跳转。版式：每台设备一张软底小卡（图标方块 + 名称 + 电量条），电量用条而非徽标，
 * 一眼扫过去就能比较；未上报的项显示「—」并把原因写在下一行，绝不落成 0%。
 */
export function DeviceSummaryList({
  devices,
  onReadEarbuds,
  earbudError,
  compact = false,
  className,
}: {
  devices: ConnectedDevice[];
  /** 耳机没上报电量时「读取电量」按钮的动作；不传则不渲染按钮（桌面小组件只读）。 */
  onReadEarbuds?: () => void;
  /** 会话错误（连接失败等），只有主窗口之外的两处摘要需要显示。 */
  earbudError?: string | null;
  /** 紧凑模式（桌面小组件）：去掉传输方式副标题、收紧内边距，只保留「名字 + 电量条」。 */
  compact?: boolean;
  className?: string;
}) {
  if (devices.length === 0) {
    return (
      <div
        className={cn(
          "flex flex-col items-center gap-2 rounded-xl border border-dashed border-border/50 px-3 py-4 text-center",
          className,
        )}
      >
        <span className="flex size-8 items-center justify-center rounded-lg bg-muted/45 text-muted-foreground">
          <MonitorSmartphone className="size-4" aria-hidden />
        </span>
        <p className="text-[0.6875rem] font-medium">没有已连接的设备</p>
        <p className="text-[0.625rem] leading-relaxed text-muted-foreground">
          取出耳机或插上 2.4G 接收器后会自动出现在这里。
        </p>
      </div>
    );
  }

  return (
    <ul className={cn("flex flex-col gap-2", className)}>
      {devices.map((device) => {
        if (device.kind === "earbuds") {
          return (
            <DeviceCard key={device.id} compact={compact}>
              <DeviceHeader
                icon={Headphones}
                name={device.name}
                title={device.address}
                trailing={<OnlineDot />}
              />

              {device.battery ? (
                <div className="grid grid-cols-3 gap-2">
                  {(
                    [
                      ["左", device.battery.left],
                      ["右", device.battery.right],
                      ["仓", device.battery.case],
                    ] as const
                  ).map(([label, value]) => (
                    <div key={label} className="flex flex-col gap-1">
                      <div className="flex items-baseline justify-between text-[0.625rem]">
                        <span className="text-muted-foreground">{label}</span>
                        <span className="font-medium tabular-nums text-foreground/85">
                          {value === null ? "—" : `${value}%`}
                        </span>
                      </div>
                      <BatteryBar percent={value} />
                    </div>
                  ))}
                </div>
              ) : (
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[0.625rem] text-muted-foreground">
                    {earbudsBatterySummary(null)}
                  </span>
                  {onReadEarbuds ? (
                    <Button variant="outline" size="xs" onClick={onReadEarbuds}>
                      <BatteryCharging data-icon="inline-start" />
                      读取电量
                    </Button>
                  ) : null}
                </div>
              )}

              {device.tags.length > 0 && (
                <div className="flex flex-wrap gap-1">
                  {device.tags.map((tag) => (
                    <span
                      key={tag}
                      className="rounded-md bg-background/55 px-1.5 py-0.5 text-[0.625rem] text-muted-foreground ring-1 ring-border/40"
                    >
                      {tag}
                    </span>
                  ))}
                </div>
              )}

              {earbudError ? (
                <p className="text-[0.625rem] leading-relaxed break-words text-destructive">
                  {earbudError}
                </p>
              ) : null}

              {!compact && oneBudUnavailable(device.battery) ? (
                <p className="text-[0.625rem] leading-relaxed text-muted-foreground">
                  有一只耳机没有上报电量；只有一只在线时，降噪梯度可能切换不动。
                </p>
              ) : null}
            </DeviceCard>
          );
        }

        const icon =
          device.kind === "mouse"
            ? Mouse
            : device.category === "keyboard"
              ? Keyboard
              : device.category === "headset"
                ? Headphones
                : Mouse;
        const percent = device.percent;
        const charging = device.charging === true;
        const approx = device.kind === "wireless" && device.approx;
        const subtitle =
          device.kind === "mouse"
            ? percent === null
              ? mouseBatterySummary(device)
              : device.subtitle
            : percent === null
              ? wirelessBatterySummary(device)
              : device.subtitle;

        return (
          <DeviceCard key={device.id} compact={compact}>
            <DeviceHeader
              icon={icon}
              name={device.name}
              trailing={
                <span className="flex items-center gap-1">
                  <span className="font-medium tabular-nums text-foreground/85">
                    {percent === null ? "—" : `${approx ? "约 " : ""}${percent}%`}
                  </span>
                  {charging && (
                    <Zap className="size-3 text-emerald-600 dark:text-emerald-500" aria-hidden />
                  )}
                </span>
              }
            />
            <BatteryBar percent={percent} charging={charging} />
            {compact ? null : (
              <p className="truncate text-[0.625rem] text-muted-foreground">{subtitle}</p>
            )}
          </DeviceCard>
        );
      })}
    </ul>
  );
}

function DeviceCard({ children, compact }: { children: ReactNode; compact?: boolean }) {
  return (
    <li
      className={cn(
        "flex flex-col rounded-xl border border-border/45 bg-muted/20",
        compact ? "gap-1.5 p-2" : "gap-2 p-2.5",
      )}
    >
      {children}
    </li>
  );
}

function DeviceHeader({
  icon: Icon,
  name,
  title,
  trailing,
}: {
  icon: typeof Mouse;
  name: string;
  title?: string;
  trailing: ReactNode;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="flex size-6 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-brand">
        <Icon className="size-3.5" aria-hidden />
      </span>
      <span className="min-w-0 flex-1 truncate text-xs font-medium" title={title}>
        {name}
      </span>
      <span className="shrink-0 text-[0.6875rem]">{trailing}</span>
    </div>
  );
}

/** 在线标记：一个很小的绿点，不给每台设备再加一枚「已连接」徽标。 */
function OnlineDot() {
  return (
    <span className="flex items-center gap-1 text-[0.625rem] text-muted-foreground">
      <span className="size-1.5 rounded-full bg-emerald-500/90 ring-2 ring-emerald-500/15" />
      在线
    </span>
  );
}

/** 细电量条：走 `transform` 合成，轮询刷新不触发布局重排；低电量转暖色。 */
export function BatteryBar({
  percent,
  charging = false,
  className,
}: {
  percent: number | null;
  charging?: boolean;
  className?: string;
}) {
  const value = percent === null ? 0 : Math.min(100, Math.max(0, percent));
  return (
    <div className={cn("h-1 w-full overflow-hidden rounded-full bg-muted", className)}>
      <div
        className={cn(
          "h-full origin-left rounded-full transition-transform duration-500 ease-out",
          charging
            ? "bg-[linear-gradient(90deg,#059669,#34d399)]"
            : value <= 20
              ? "bg-[linear-gradient(90deg,#f59e0b,#f87171)]"
              : "bg-[linear-gradient(90deg,var(--brand-600),var(--brand))]",
        )}
        style={{ transform: `scaleX(${value / 100})` }}
      />
    </div>
  );
}
