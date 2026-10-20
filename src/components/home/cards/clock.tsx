import { useEffect, useState } from "react";

import { HomeBar } from "@/components/home/home-card";

const WEEKDAYS = ["日", "一", "二", "三", "四", "五", "六"] as const;

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

/** 对齐到下一秒边界刷新，避免 `setInterval` 的相位漂移导致跳秒。 */
function useClock(): Date {
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    let timer: number | undefined;
    const tick = () => {
      setNow(new Date());
      timer = window.setTimeout(tick, 1005 - (Date.now() % 1000));
    };
    tick();
    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, []);

  return now;
}

function greeting(hour: number): string {
  if (hour < 5) return "凌晨好";
  if (hour < 9) return "早上好";
  if (hour < 12) return "上午好";
  if (hour < 14) return "中午好";
  if (hour < 18) return "下午好";
  return "晚上好";
}

/** 首页「时钟」卡片：大号时间 + 日期 + 问候，纯前端，不依赖后端。 */
export function ClockCard() {
  const now = useClock();
  const dateText = `${now.getFullYear()}年${now.getMonth() + 1}月${now.getDate()}日 周${WEEKDAYS[now.getDay()]}`;
  const offsetMinutes = now.getTimezoneOffset();
  const offsetText = `UTC${offsetMinutes <= 0 ? "+" : "-"}${pad(
    Math.floor(Math.abs(offsetMinutes) / 60),
  )}:${pad(Math.abs(offsetMinutes) % 60)}`;
  const secondsToday =
    now.getHours() * 3600 + now.getMinutes() * 60 + now.getSeconds();
  const dayPercent = (secondsToday / 86400) * 100;

  return (
    <div className="flex h-full min-h-0 flex-col justify-between gap-3">
      <div>
        <p className="text-xs text-muted-foreground">{greeting(now.getHours())}</p>
        <div className="mt-1 flex items-baseline gap-1.5">
          <span className="font-heading text-[2.75rem] leading-none font-semibold tracking-[-0.04em] tabular-nums">
            {pad(now.getHours())}:{pad(now.getMinutes())}
          </span>
          <span className="text-sm font-medium tabular-nums text-muted-foreground">
            {pad(now.getSeconds())}
          </span>
        </div>
        <p className="mt-2 text-xs text-muted-foreground">{dateText}</p>
      </div>

      <div className="flex flex-col gap-1.5">
        <HomeBar percent={dayPercent} />
        <div className="flex items-center justify-between text-[0.65rem] text-muted-foreground">
          <span>今天已过 {Math.round(dayPercent)}%</span>
          <span className="tabular-nums">{offsetText}</span>
        </div>
      </div>
    </div>
  );
}
