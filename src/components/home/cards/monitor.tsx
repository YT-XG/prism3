import { Cpu, MemoryStick } from "lucide-react";

import { HomeBar, HomeCardHint } from "@/components/home/home-card";
import { useSystemMonitor } from "@/hooks/use-system-monitor";
import { formatBytes, formatRate } from "@/lib/format";

function percentOf(used: number, total: number): number {
  return total > 0 ? (used / total) * 100 : 0;
}

/** 首页「系统资源」卡片：CPU / 内存 / 网络 / 首块磁盘，1s 轮询，离开首页即停。 */
export function MonitorCard() {
  const { snapshot } = useSystemMonitor({ trackHistory: false });

  if (!snapshot) {
    return <HomeCardHint>正在读取系统状态…</HomeCardHint>;
  }

  const cpu = snapshot.cpuUsage;
  const cpuPercent = cpu ?? 0;
  const memPercent = percentOf(snapshot.memUsed, snapshot.memTotal);
  const disk = snapshot.disks[0];
  const diskPercent = disk ? percentOf(disk.total - disk.available, disk.total) : 0;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex flex-col gap-1.5">
        <div className="flex items-baseline justify-between gap-2">
          <span className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground">
            <Cpu className="size-3" aria-hidden="true" />
            CPU
          </span>
          <span className="font-heading text-lg leading-none font-semibold tabular-nums">
            {cpu === null ? "—" : `${Math.round(cpuPercent)}%`}
          </span>
        </div>
        <HomeBar percent={cpuPercent} warn={cpuPercent >= 85} />
      </div>

      <div className="flex flex-col gap-1.5">
        <div className="flex items-baseline justify-between gap-2">
          <span className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground">
            <MemoryStick className="size-3" aria-hidden="true" />
            内存
          </span>
          <span className="font-heading text-lg leading-none font-semibold tabular-nums">
            {Math.round(memPercent)}%
          </span>
        </div>
        <HomeBar percent={memPercent} warn={memPercent >= 85} />
        <span className="text-[0.65rem] text-muted-foreground tabular-nums">
          {formatBytes(snapshot.memUsed)} / {formatBytes(snapshot.memTotal)}
        </span>
      </div>

      <div className="mt-auto flex flex-col gap-1 text-[0.65rem] text-muted-foreground">
        <div className="flex items-center justify-between gap-2 tabular-nums">
          <span>↓ {formatRate(snapshot.netRxPerSec)}</span>
          <span>↑ {formatRate(snapshot.netTxPerSec)}</span>
        </div>
        {disk && (
          <div className="flex items-center justify-between gap-2">
            <span className="truncate">
              {disk.mountPoint || disk.name || "磁盘"}
              <span className="ml-1 opacity-70">{Math.round(diskPercent)}%</span>
            </span>
            <span className="shrink-0 tabular-nums">{formatBytes(disk.available)} 可用</span>
          </div>
        )}
      </div>
    </div>
  );
}
