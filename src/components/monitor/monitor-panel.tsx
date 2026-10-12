import { useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useTheme } from "@/components/theme-provider";
import { useSystemMonitor } from "@/hooks/use-system-monitor";
import { formatBytes, formatPercent, formatRate, formatUptime } from "@/lib/format";
import type { DiskInfo } from "@/lib/ipc";

import { MetricChart, type ChartSeries } from "./metric-chart";

/**
 * 图表配色：canvas 取不到 CSS 变量，按亮/暗主题各给一套具体色值。
 * 用十六进制而非 oklch，确保 canvas fillStyle 一定能解析。
 */
const PALETTE = {
  light: {
    cpu: { color: "#3b82f6", fill: "#3b82f624" },
    mem: { color: "#8b5cf6", fill: "#8b5cf624" },
    rx: { color: "#10b981", fill: "#10b98124" },
    tx: { color: "#f59e0b", fill: "#f59e0b24" },
  },
  dark: {
    cpu: { color: "#60a5fa", fill: "#60a5fa2e" },
    mem: { color: "#a78bfa", fill: "#a78bfa2e" },
    rx: { color: "#34d399", fill: "#34d3992e" },
    tx: { color: "#fbbf24", fill: "#fbbf242e" },
  },
} as const;

/** 「监控」页：系统状态折线图 + 系统信息 + 磁盘用量。 */
export function MonitorPanel() {
  const { snapshot, history } = useSystemMonitor();
  const palette = useIsDarkTheme() ? PALETTE.dark : PALETTE.light;

  const cpuUsage = snapshot?.cpuUsage ?? null;
  const memPercent =
    snapshot && snapshot.memTotal > 0 ? (snapshot.memUsed / snapshot.memTotal) * 100 : 0;
  const osLabel = snapshot
    ? snapshot.osLongName || `${snapshot.osName} ${snapshot.osVersion}`.trim()
    : "—";

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <h1 className="font-heading text-lg font-medium">监控</h1>
        {cpuUsage === null && <Badge variant="secondary">采集预热中</Badge>}
      </div>
      <p className="text-sm text-muted-foreground">
        每 1 秒采集一次，展示最近 60 秒的趋势；离开本页即停止采集。
      </p>

      <div className="grid gap-4 sm:grid-cols-2">
        <MetricCard
          title="CPU"
          hint="全局占用率"
          value={cpuUsage === null ? "—" : formatPercent(cpuUsage)}
          series={[{ label: "CPU", ...palette.cpu }]}
          xs={history.xs}
          ys={[history.cpu]}
          max={100}
        />

        <MetricCard
          title="内存"
          hint={
            snapshot
              ? `${formatBytes(snapshot.memUsed)} / ${formatBytes(snapshot.memTotal)}`
              : "占用率"
          }
          value={snapshot ? formatPercent(memPercent) : "—"}
          series={[{ label: "内存", ...palette.mem }]}
          xs={history.xs}
          ys={[history.mem]}
          max={100}
        />
      </div>

      <Card>
        <CardHeader>
          <CardTitle>网络</CardTitle>
          <CardDescription>非虚拟网卡合计（不含环回与虚拟机网卡）。</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-2">
          <div className="flex flex-wrap gap-x-6 gap-y-1">
            <RateValue label="↓ 下载" value={snapshot?.netRxPerSec ?? 0} />
            <RateValue label="↑ 上传" value={snapshot?.netTxPerSec ?? 0} />
          </div>
          <MetricChart
            xs={history.xs}
            ys={[history.rx, history.tx]}
            series={[
              { label: "下载", ...palette.rx },
              { label: "上传", ...palette.tx },
            ]}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>硬件</CardTitle>
          <CardDescription>处理器、主板、显卡与内存型号。</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <InfoRow label="处理器" value={snapshot?.cpuBrand || "—"} />
            <InfoRow label="主板" value={snapshot?.motherboard || "—"} />
            <InfoRow label="整机" value={snapshot?.systemModel || "—"} />
          </div>

          <div className="flex flex-col gap-2">
            <span className="text-xs font-medium text-muted-foreground">显卡</span>
            {snapshot && snapshot.gpus.length > 0 ? (
              snapshot.gpus.map((gpu) => (
                <div
                  key={`${gpu.name}-${gpu.memory}`}
                  className="flex items-baseline justify-between gap-3 text-sm"
                >
                  <span className="truncate font-medium">{gpu.name}</span>
                  {gpu.memory > 0 && (
                    <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                      {formatBytes(gpu.memory)} 显存
                    </span>
                  )}
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">正在读取显卡信息…</p>
            )}
          </div>

          <div className="flex flex-col gap-2">
            <span className="text-xs font-medium text-muted-foreground">内存</span>
            {snapshot && snapshot.memoryModules.length > 0 ? (
              snapshot.memoryModules.map((memory) => (
                <div
                  key={`${memory.partNumber}-${memory.capacity}`}
                  className="flex items-baseline justify-between gap-3 text-sm"
                >
                  <span className="truncate font-medium">
                    {[memory.manufacturer, memory.partNumber].filter(Boolean).join(" ") || "内存条"}
                  </span>
                  <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                    {[
                      memory.memoryType,
                      memory.speed > 0 ? `${memory.speed} MHz` : "",
                      formatBytes(memory.capacity),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">正在读取内存信息…</p>
            )}
          </div>
        </CardContent>
      </Card>

      <div className="grid gap-4 sm:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>系统信息</CardTitle>
            <CardDescription>主机与运行状态。</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-2">
            <InfoRow label="主机名" value={snapshot?.hostname ?? "—"} />
            <InfoRow label="操作系统" value={osLabel || "—"} />
            <InfoRow label="本机 IP" value={snapshot?.ip ?? "—"} />
            <InfoRow
              label="开机时长"
              value={snapshot ? formatUptime(snapshot.uptimeSecs) : "—"}
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>磁盘</CardTitle>
            <CardDescription>各分区已用 / 总量。</CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            {snapshot && snapshot.disks.length > 0 ? (
              snapshot.disks.map((disk) => <DiskRow key={disk.mountPoint} disk={disk} />)
            ) : (
              <p className="text-sm text-muted-foreground">正在读取磁盘信息…</p>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

interface MetricCardProps {
  title: string;
  hint: string;
  value: string;
  series: ChartSeries[];
  xs: number[];
  ys: (number | null)[][];
  max?: number;
}

function MetricCard({ title, hint, value, series, xs, ys, max }: MetricCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{hint}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <span className="font-heading text-2xl font-medium tabular-nums">{value}</span>
        <MetricChart xs={xs} ys={ys} series={series} max={max} />
      </CardContent>
    </Card>
  );
}

function RateValue({ label, value }: { label: string; value: number }) {
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="font-heading text-xl font-medium tabular-nums">{formatRate(value)}</span>
      <span className="text-xs text-muted-foreground">{label}</span>
    </span>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-4 text-sm">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="truncate text-right font-medium">{value}</span>
    </div>
  );
}

function DiskRow({ disk }: { disk: DiskInfo }) {
  const used = Math.max(0, disk.total - disk.available);
  const percent = disk.total > 0 ? (used / disk.total) * 100 : 0;
  const label = disk.mountPoint || disk.name || "磁盘";

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 text-sm">
        <span className="flex items-center gap-2">
          <span className="font-medium">{label}</span>
          {disk.name && <span className="text-xs text-muted-foreground">{disk.name}</span>}
          <Badge variant="secondary">{disk.kind}</Badge>
          {disk.isRemovable && <Badge variant="outline">可移动</Badge>}
        </span>
        <span className="text-xs tabular-nums text-muted-foreground">
          {formatBytes(used)} / {formatBytes(disk.total)}（{formatPercent(percent)}）
        </span>
      </div>
      <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
        <div
          className="h-full rounded-full bg-primary"
          style={{ width: `${Math.min(100, percent)}%` }}
        />
      </div>
    </div>
  );
}

/** 解析出实际生效的暗色状态（`system` 时跟随系统）。 */
function useIsDarkTheme() {
  const { theme } = useTheme();
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches,
  );

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const handleChange = () => setSystemDark(media.matches);
    media.addEventListener("change", handleChange);
    return () => media.removeEventListener("change", handleChange);
  }, []);

  return theme === "dark" || (theme === "system" && systemDark);
}