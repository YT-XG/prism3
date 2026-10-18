/** 字节数格式化为易读单位（1024 进制）。 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  return `${value.toFixed(exponent === 0 ? 0 : 1)} ${units[exponent]}`;
}

/** 速率（字节/秒）统一以 MB/s 展示。 */
export function formatRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return "0.00 MB/s";
  return `${(bytesPerSec / 1024 / 1024).toFixed(2)} MB/s`;
}

/** 秒数格式化为「3天4小时5分」。 */
export function formatUptime(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);

  if (days > 0) return `${days}天${hours}小时${minutes}分`;
  if (hours > 0) return `${hours}小时${minutes}分`;
  return `${minutes}分`;
}

/** 百分比格式化为一位小数。 */
export function formatPercent(value: number): string {
  return `${value.toFixed(1)}%`;
}

/**
 * 秒级时间戳格式化为「刚刚 / 14:03 / 昨天 14:03 / 3 天前 / 2025-03-02」。
 *
 * 文件搜索结果里大量出现近期修改的文件，相对时间比完整日期更好扫读。
 */
export function formatTimeAgo(modifiedSecs: number): string {
  if (!Number.isFinite(modifiedSecs) || modifiedSecs <= 0) return "—";

  const then = new Date(modifiedSecs * 1000);
  const now = new Date();
  const clock = `${String(then.getHours()).padStart(2, "0")}:${String(then.getMinutes()).padStart(2, "0")}`;

  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const elapsedMs = now.getTime() - then.getTime();
  if (elapsedMs < 60_000) return "刚刚";
  if (elapsedMs < 3_600_000) return `${Math.floor(elapsedMs / 60_000)} 分钟前`;
  if (then.getTime() >= startOfToday) return clock;
  if (then.getTime() >= startOfToday - 86_400_000) return `昨天 ${clock}`;

  const days = Math.floor((startOfToday - then.getTime()) / 86_400_000) + 1;
  if (days <= 30) return `${days} 天前`;

  const month = String(then.getMonth() + 1).padStart(2, "0");
  const day = String(then.getDate()).padStart(2, "0");
  if (then.getFullYear() === now.getFullYear()) return `${month}-${day}`;
  return `${then.getFullYear()}-${month}-${day}`;
}