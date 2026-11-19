import type { WorkLog } from "@/lib/ipc";
import { formatDate, formatHours } from "@/lib/work-time";

/**
 * 把一段工作记录拼成可直接粘贴进日报 / 周报的 Markdown。
 *
 * 口径固定为「按项目分组、组内按完成时间正序」：汇报时同一条线的事聚在一起，
 * 读的人不用自己跳着找。没有归属的记录统一落在「未归属项目」。
 */
export function buildWorkReport(logs: WorkLog[], from: number, to: number): string {
  const sorted = [...logs].sort((a, b) => a.finishedAt - b.finishedAt);
  const totalHours = sorted.reduce((sum, log) => sum + Math.max(0, log.hours), 0);

  const groups = new Map<string, WorkLog[]>();
  for (const log of sorted) {
    const name = log.projectName.trim() || "未归属项目";
    const bucket = groups.get(name);
    if (bucket) bucket.push(log);
    else groups.set(name, [log]);
  }

  const lines: string[] = [
    `# 工作周报（${formatDate(from)} ~ ${formatDate(to)}）`,
    "",
    `本周完成 **${sorted.length}** 项，共 **${formatHours(totalHours) === "—" ? "0h" : formatHours(totalHours)}**。`,
    "",
  ];

  if (sorted.length === 0) {
    lines.push("_这段时间还没有记录。_");
    return lines.join("\n");
  }

  for (const [project, entries] of groups) {
    lines.push(`## ${project}`, "");
    for (const entry of entries) {
      const hours = entry.hours > 0 ? `（${formatHours(entry.hours)}）` : "";
      const date = new Date(entry.finishedAt * 1000);
      const dateLabel = `${date.getMonth() + 1}/${date.getDate()} `;
      lines.push(`- ${dateLabel}${entry.title}${hours}`);
      if (entry.detail.trim()) {
        lines.push(`  - ${entry.detail.trim().replace(/\n/g, " ")}`);
      }
    }
    lines.push("");
  }

  return lines.join("\n").trimEnd();
}
