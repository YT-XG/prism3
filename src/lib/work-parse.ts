import type { WorkLogKind, WorkProject } from "@/lib/ipc";
import { WORK_KIND_ALIASES } from "@/lib/work-kinds";

/** 快捷录入解析结果；`title` 之外的字段都可能有默认值。 */
export interface QuickLogDraft {
  title: string;
  projectId: number | null;
  projectName: string;
  hours: number;
  kind: WorkLogKind;
  finishedAt: number;
}

/**
 * 解析快捷录入的一行文本：
 *
 * - `#项目名` → 归属项目（大小写不敏感；匹配不到就保留原文当标题的一部分）；
 * - `1.5h` / `90m` / `30分钟` → 工时（只取第一个）；
 * - `开发 / bug / 缺陷 / 支持 / 文档 / 会议 / 运维 / 其他` → 类型（只取第一个）；
 * - `昨天` / `前天` → 完成时间往前挪一天 / 两天。
 *
 * 解析失败的 token 一律留在标题里——**永远要能存进去**，不能因为语法不对就拒绝记录。
 */
export function parseQuickLog(
  input: string,
  projects: WorkProject[],
  now = new Date(),
): QuickLogDraft {
  const raw = input.trim();
  const kept: string[] = [];
  let projectId: number | null = null;
  let projectName = "";
  let hours = 0;
  let kind: WorkLogKind = "dev";
  let kindSet = false;
  let dayOffset = 0;

  for (const token of raw.split(/\s+/)) {
    if (token.startsWith("#") && token.length > 1 && projectId === null) {
      const name = token.slice(1).toLowerCase();
      const found = projects.find((project) => project.name.toLowerCase() === name);
      if (found) {
        projectId = found.id;
        projectName = found.name;
        continue;
      }
    }

    if (hours === 0) {
      const hourMatch = /^(\d+(?:\.\d+)?)(h|小时)$/i.exec(token);
      const minuteMatch = /^(\d+(?:\.\d+)?)(m|分钟)$/i.exec(token);
      if (hourMatch) {
        hours = Number(hourMatch[1]);
        continue;
      }
      if (minuteMatch) {
        hours = Math.round((Number(minuteMatch[1]) / 60) * 100) / 100;
        continue;
      }
    }

    const alias = WORK_KIND_ALIASES[token.toLowerCase()];
    if (!kindSet && alias) {
      kind = alias;
      kindSet = true;
      continue;
    }

    if (token === "昨天") {
      dayOffset = 1;
      continue;
    }
    if (token === "前天") {
      dayOffset = 2;
      continue;
    }

    kept.push(token);
  }

  const title = kept.join(" ").trim() || raw;
  const finishedAt = Math.floor(now.getTime() / 1000) - dayOffset * 86_400;

  return { title, projectId, projectName, hours, kind, finishedAt };
}
