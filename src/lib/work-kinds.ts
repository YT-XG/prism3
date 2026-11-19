import type { WorkLogKind } from "@/lib/ipc";

/** 工作类型的中文名（与 Rust `WORK_KINDS` 一一对应）。 */
export const WORK_KIND_LABELS: Record<WorkLogKind, string> = {
  dev: "开发",
  bug: "缺陷",
  support: "支持",
  doc: "文档",
  meeting: "会议",
  ops: "运维",
  other: "其他",
};

export const WORK_KIND_OPTIONS: { value: WorkLogKind; label: string }[] = (
  Object.keys(WORK_KIND_LABELS) as WorkLogKind[]
).map((value) => ({ value, label: WORK_KIND_LABELS[value] }));

/** 类型徽标底色：浅色系，暗色下也不刺眼。 */
export const WORK_KIND_CLASS: Record<WorkLogKind, string> = {
  dev: "bg-blue-500/12 text-blue-700 dark:text-blue-300",
  bug: "bg-rose-500/12 text-rose-700 dark:text-rose-300",
  support: "bg-amber-500/14 text-amber-700 dark:text-amber-300",
  doc: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-300",
  meeting: "bg-violet-500/12 text-violet-700 dark:text-violet-300",
  ops: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-300",
  other: "bg-muted text-muted-foreground",
};

/** 快捷录入里的类型别名（小写比较）。 */
export const WORK_KIND_ALIASES: Record<string, WorkLogKind> = {
  开发: "dev",
  dev: "dev",
  feature: "dev",
  bug: "bug",
  缺陷: "bug",
  修复: "bug",
  支持: "support",
  support: "support",
  文档: "doc",
  doc: "doc",
  docs: "doc",
  会议: "meeting",
  meeting: "meeting",
  运维: "ops",
  ops: "ops",
  其他: "other",
  other: "other",
};

export function isWorkLogKind(value: string): value is WorkLogKind {
  return value in WORK_KIND_LABELS;
}
