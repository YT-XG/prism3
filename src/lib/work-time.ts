/**
 * 工作记录用的本地时间换算。
 *
 * 后端一律存秒级 unix 时间戳；「今天 / 本周 / 本月」按**本地时区**取边界，
 * 因此这里统一用 `Date` 的年月日构造，而不是直接对时间戳做固定偏移。
 */

const DAY_SECS = 86_400;
const WEEKDAY_LABELS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

function toSecs(date: Date): number {
  return Math.floor(date.getTime() / 1000);
}

/** 当天 00:00:00。 */
export function startOfDay(date = new Date()): number {
  return toSecs(new Date(date.getFullYear(), date.getMonth(), date.getDate()));
}

/** 当天 23:59:59。 */
export function endOfDay(date = new Date()): number {
  return toSecs(new Date(date.getFullYear(), date.getMonth(), date.getDate() + 1)) - 1;
}

/** 本周一 00:00:00（周一为一周起点）。 */
export function startOfWeek(date = new Date()): number {
  const start = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const offset = (start.getDay() + 6) % 7; // 周日 = 0 → 偏移 6
  start.setDate(start.getDate() - offset);
  return toSecs(start);
}

/** 本周日 23:59:59。 */
export function endOfWeek(date = new Date()): number {
  return startOfWeek(date) + 7 * DAY_SECS - 1;
}

/** 本月 1 日 00:00:00。 */
export function startOfMonth(date = new Date()): number {
  return toSecs(new Date(date.getFullYear(), date.getMonth(), 1));
}

/** 本月最后一天 23:59:59。 */
export function endOfMonth(date = new Date()): number {
  return toSecs(new Date(date.getFullYear(), date.getMonth() + 1, 1)) - 1;
}

/** 本地日期键（`YYYY-MM-DD`），用于按天分组。 */
export function dayKey(secs: number): string {
  const date = new Date(secs * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(
    date.getDate(),
  ).padStart(2, "0")}`;
}

/** `YYYY-MM-DD`（周报标题用，不能出现「今天 / 昨天」这类相对词）。 */
export function formatDate(secs: number): string {
  return dayKey(secs);
}

/** 「今天 / 昨天 / 11月18日 周二」。 */
export function formatDayLabel(secs: number): string {
  const key = dayKey(secs);
  if (key === dayKey(startOfDay())) return "今天";
  if (key === dayKey(startOfDay() - DAY_SECS)) return "昨天";

  const date = new Date(secs * 1000);
  return `${date.getMonth() + 1}月${date.getDate()}日 ${WEEKDAY_LABELS[date.getDay()]}`;
}

/** `HH:mm`。 */
export function formatClock(secs: number): string {
  const date = new Date(secs * 1000);
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

/** 工时展示：`0` = 未填。 */
export function formatHours(hours: number): string {
  if (!Number.isFinite(hours) || hours <= 0) return "—";
  const rounded = Math.round(hours * 100) / 100;
  return `${rounded}h`;
}

/** `<input type="date">` 需要的 `YYYY-MM-DD`。 */
export function dateInputValue(secs: number): string {
  return dayKey(secs);
}

/** 把日期输入值合成为时间戳：保留 `baseSecs` 的时分秒，换成所选日期。 */
export function timestampFromDateInput(value: string, baseSecs: number): number {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!match) return baseSecs;
  const [, year, month, day] = match;
  const base = new Date(baseSecs * 1000);
  return toSecs(
    new Date(Number(year), Number(month) - 1, Number(day), base.getHours(), base.getMinutes()),
  );
}
