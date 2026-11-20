import { ArrowDown, ArrowUp, ClipboardList, Cpu, HardDrive, MonitorSmartphone } from "lucide-react";
import { useEffect, useState } from "react";

import { DeviceSummaryList } from "@/components/devices/device-summary-list";
import { useClipboardList } from "@/hooks/use-clipboard-list";
import { useEarbuds } from "@/hooks/use-earbuds";
import { useMouseBattery } from "@/hooks/use-mouse-battery";
import { useSystemMonitor } from "@/hooks/use-system-monitor";
import { useWirelessBatteries } from "@/hooks/use-wireless-batteries";
import { connectedDevices } from "@/lib/devices";
import { cn } from "@/lib/utils";

/** 桌面右侧的小组件列（参考站的「桌面小组件」位置：右上，纵向堆叠）。 */
export function DesktopWidgets() {
  return (
    <div
      className="absolute right-5 hidden w-[292px] flex-col gap-3 overflow-y-auto pr-0.5 min-[1080px]:flex [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      style={{
        top: "calc(var(--topbar-h) + 16px)",
        bottom: "calc(var(--taskbar-h) + 16px)",
      }}
    >
      <ClockWidget />
      <DevicesWidget />
      <SystemWidget />
      <ClipboardWidget />
    </div>
  );
}

/** 毛玻璃小卡：桌面层级用真模糊（数量只有三张，GPU 成本可接受）。 */
function WidgetCard({
  index,
  title,
  icon: Icon,
  action,
  children,
}: {
  index: number;
  title: string;
  icon: React.ComponentType<{ className?: string; strokeWidth?: number }>;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section
      style={{ "--i": index } as React.CSSProperties}
      className={cn(
        "rise-in overflow-hidden rounded-[18px] border p-3.5",
        "border-white/70 bg-white/62 shadow-[0_22px_48px_-32px_rgba(16,40,80,.6)] backdrop-blur-2xl",
        "dark:border-white/10 dark:bg-white/8",
      )}
    >
      <header className="mb-2.5 flex items-center gap-1.5">
        <Icon className="size-3.5 text-muted-foreground" strokeWidth={2} />
        <span className="text-[10.5px] font-semibold tracking-[0.12em] text-muted-foreground uppercase">
          {title}
        </span>
        {action ? <span className="ml-auto">{action}</span> : null}
      </header>
      {children}
    </section>
  );
}

const GREETING = (hour: number) => {
  if (hour < 5) return "夜深了";
  if (hour < 9) return "早上好";
  if (hour < 12) return "上午好";
  if (hour < 14) return "中午好";
  if (hour < 18) return "下午好";
  return "晚上好";
};

function ClockWidget() {
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1000 * 30);
    return () => window.clearInterval(timer);
  }, []);

  const time = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  const weekday = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"][now.getDay()];
  const date = `${now.getMonth() + 1} 月 ${now.getDate()} 日`;
  const dayProgress =
    (now.getHours() * 3600 + now.getMinutes() * 60 + now.getSeconds()) / 86400;

  return (
    <WidgetCard index={0} title="时钟" icon={ClockIcon}>
      <div className="flex items-end justify-between gap-2">
        <span className="font-heading text-[40px] leading-none font-semibold tracking-[-0.04em] tabular-nums text-foreground">
          {time}
        </span>
        <span className="pb-1 text-right text-[11px] leading-tight text-muted-foreground">
          {date}
          <br />
          {weekday}
        </span>
      </div>
      <div className="mt-3 h-[3px] overflow-hidden rounded-full bg-black/8 dark:bg-white/12">
        <span
          className="block h-full rounded-full bg-brand"
          style={{ width: `${(dayProgress * 100).toFixed(1)}%` }}
        />
      </div>
      <p className="mt-1.5 text-[10.5px] text-muted-foreground">
        {GREETING(now.getHours())} · 今天已过 {Math.round(dayProgress * 100)}%
      </p>
    </WidgetCard>
  );
}

function ClockIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      className={className}
      aria-hidden
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7.5V12l3 1.8" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

const gb = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(1)} GB`;

/**
 * 设备电量：与 hub 左栏、设备页同源（都走 `connectedDevices` 纯投影），
 * 这里只把同一份 `DeviceSummaryList` 排进桌面小组件——电量条比一串百分比好扫。
 */
function DevicesWidget() {
  const earbuds = useEarbuds();
  const mouse = useMouseBattery(true);
  const wireless = useWirelessBatteries(true);
  const devices = connectedDevices(earbuds.state, mouse, wireless);

  return (
    <WidgetCard
      index={1}
      title="设备"
      icon={MonitorSmartphone}
      action={
        <span className="text-[10px] text-muted-foreground tabular-nums">
          {devices.length > 0 ? `${devices.length} 台在线` : "未连接"}
        </span>
      }
    >
      <DeviceSummaryList devices={devices.slice(0, 2)} compact />
      {devices.length > 2 ? (
        <p className="mt-1 text-[10px] text-muted-foreground">还有 {devices.length - 2} 台设备</p>
      ) : null}
    </WidgetCard>
  );
}

function SystemWidget() {
  const { snapshot } = useSystemMonitor({ trackHistory: false });
  const cpu = snapshot?.cpuUsage ?? null;
  const memUsed = snapshot?.memUsed ?? 0;
  const memTotal = snapshot?.memTotal ?? 0;
  const memPercent = memTotal > 0 ? (memUsed / memTotal) * 100 : 0;
  const disk = snapshot?.disks?.[0];

  return (
    <WidgetCard index={2} title="系统资源" icon={Cpu}>
      <Metric label="CPU" value={cpu === null ? "—" : `${Math.round(cpu)}%`} percent={cpu ?? 0} />
      <Metric label="内存" value={`${Math.round(memPercent)}%`} percent={memPercent} hint={`${gb(memUsed)} / ${gb(memTotal)}`} />

      <div className="mt-2.5 flex items-center justify-between border-t border-black/6 pt-2.5 text-[10.5px] text-muted-foreground dark:border-white/8">
        <span className="flex items-center gap-1">
          <HardDrive className="size-3" />
          {disk
            ? `${disk.mountPoint} ${Math.round(((disk.total - disk.available) / disk.total) * 100)}%`
            : "—"}
        </span>
        <span className="flex items-center gap-2 tabular-nums">
          <span className="flex items-center gap-0.5">
            <ArrowDown className="size-3" />
            {snapshot ? `${(snapshot.netRxPerSec / 1024 ** 2).toFixed(2)} MB/s` : "—"}
          </span>
          <span className="flex items-center gap-0.5">
            <ArrowUp className="size-3" />
            {snapshot ? `${(snapshot.netTxPerSec / 1024 ** 2).toFixed(2)} MB/s` : "—"}
          </span>
        </span>
      </div>
    </WidgetCard>
  );
}

function Metric({
  label,
  value,
  percent,
  hint,
}: {
  label: string;
  value: string;
  percent: number;
  hint?: string;
}) {
  return (
    <div className="mt-2 first:mt-0">
      <div className="flex items-baseline justify-between text-[11px]">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-medium tabular-nums text-foreground">{value}</span>
      </div>
      <div className="mt-1 h-[5px] overflow-hidden rounded-full bg-black/8 dark:bg-white/12">
        <span
          className="block h-full rounded-full bg-brand transition-[width] duration-500"
          style={{ width: `${Math.min(100, Math.max(0, percent))}%` }}
        />
      </div>
      {hint ? <p className="mt-1 text-[10px] text-muted-foreground tabular-nums">{hint}</p> : null}
    </div>
  );
}

function ago(secs: number) {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - secs);
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

function ClipboardWidget() {
  const { entries, total, copy, state } = useClipboardList({
    enabled: true,
    query: "",
    kind: "all",
    limit: 3,
  });

  return (
    <WidgetCard
      index={3}
      title="剪贴板"
      icon={ClipboardList}
      action={
        <span className="text-[10px] text-muted-foreground tabular-nums">
          {state?.enabled ? `${total} 条` : "未开启"}
        </span>
      }
    >
      {entries.length === 0 ? (
        <p className="py-2 text-[11.5px] text-muted-foreground">复制点什么，这里就会出现。</p>
      ) : (
        <ul className="-mx-1 flex flex-col">
          {entries.slice(0, 2).map((entry) => (
            <li key={entry.id}>
              <button
                type="button"
                onClick={() => void copy(entry.id)}
                title={entry.preview}
                className="flex w-full items-start gap-2 rounded-lg px-1 py-1.5 text-left transition-colors hover:bg-black/5 dark:hover:bg-white/8"
              >
                <span className="mt-[3px] flex size-4 shrink-0 items-center justify-center rounded-[5px] bg-brand/12 text-[9px] font-semibold text-brand">
                  {entry.kind === "image" ? "图" : "T"}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[11.5px] leading-[1.35] text-foreground">
                    {entry.preview}
                  </span>
                  <span className="block text-[10px] text-muted-foreground">
                    {ago(entry.createdAt)}
                    {entry.pinned ? " · 收藏" : ""}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </WidgetCard>
  );
}

