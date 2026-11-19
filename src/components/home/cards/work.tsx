import { useMemo } from "react";

import { HomeCardHint, HomeCardLink } from "@/components/home/home-card";
import { useWorkLogs } from "@/hooks/use-work-logs";
import { WORK_KIND_LABELS } from "@/lib/work-kinds";
import {
  endOfDay,
  endOfWeek,
  formatClock,
  formatHours,
  startOfDay,
  startOfWeek,
} from "@/lib/work-time";
import { useAppStore } from "@/stores/app-store";

/** 首页「工作记录」卡片：今天完成多少、本周工时、最近两条完成记录。 */
export function WorkCard() {
  const setActiveView = useAppStore((state) => state.setActiveView);
  const today = useMemo(() => ({ from: startOfDay(), to: endOfDay() }), []);
  const week = useMemo(() => ({ from: startOfWeek(), to: endOfWeek() }), []);
  const { logs: todayLogs } = useWorkLogs(today.from, today.to);
  const { logs: weekLogs } = useWorkLogs(week.from, week.to);

  const weekHours = weekLogs.reduce((sum, log) => sum + Math.max(0, log.hours), 0);
  const recent = todayLogs.slice(0, 3);

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="grid grid-cols-2 gap-2">
        <div className="flex flex-col rounded-lg bg-muted/50 px-2.5 py-2">
          <span className="font-heading text-lg font-semibold tabular-nums">
            {todayLogs.length}
          </span>
          <span className="text-[0.65rem] text-muted-foreground">今日完成</span>
        </div>
        <div className="flex flex-col rounded-lg bg-muted/50 px-2.5 py-2">
          <span className="font-heading text-lg font-semibold tabular-nums">
            {weekHours > 0 ? formatHours(weekHours) : "0h"}
          </span>
          <span className="text-[0.65rem] text-muted-foreground">本周工时</span>
        </div>
      </div>

      {recent.length === 0 ? (
        <HomeCardHint>
          今天还没有记录。在快捷小窗的「记录」页写一行，或进「工作」页补记。
        </HomeCardHint>
      ) : (
        <ul className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto">
          {recent.map((log) => (
            <li key={log.id} className="flex items-start gap-2 rounded-lg bg-muted/40 px-2 py-1.5">
              <span className="shrink-0 pt-0.5 text-[0.65rem] text-muted-foreground tabular-nums">
                {formatClock(log.finishedAt)}
              </span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-xs font-medium">{log.title}</p>
                <p className="truncate text-[0.65rem] text-muted-foreground">
                  {[
                    log.projectName,
                    WORK_KIND_LABELS[log.kind],
                    log.hours > 0 ? formatHours(log.hours) : "",
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </p>
              </div>
            </li>
          ))}
        </ul>
      )}

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2">
        <span className="text-[0.65rem] text-muted-foreground">本周 {weekLogs.length} 项</span>
        <HomeCardLink onClick={() => setActiveView("work")}>去记录</HomeCardLink>
      </div>
    </div>
  );
}
