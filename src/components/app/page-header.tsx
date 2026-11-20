import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * 页面顶部的**操作条**：只放右侧的状态标记与按钮，不再画大标题。
 *
 * 应用跑在桌面窗口里，标题栏（`app-window.tsx`，图标 + 应用名）已经说清「这是哪个页」，
 * 页内再写一遍「剪贴板」「首页」只是把同一句话说两次、还占掉一行高度。
 * 所以这里只保留真正需要放在顶部的东西：
 *
 * - `actions`：编辑布局 / 生成周报 / 添加项目这类页级动作；
 * - `badge`：「采集预热中」这类状态。
 *
 * 两者都没有时**返回 `null`**，页面直接从内容开始（不留空白条）。
 * `title` 仍然必填，但只作为屏幕阅读器的页面标题（`sr-only h1`），不参与视觉层级。
 */
export function PageHeader({
  title,
  badge,
  actions,
  className,
}: {
  /** 页面名：仅用于无障碍标题，视觉上不渲染。 */
  title: string;
  /** 右侧的状态标记（如「采集预热中」）。 */
  badge?: ReactNode;
  /** 右侧操作区：按钮、开关等。 */
  actions?: ReactNode;
  className?: string;
}) {
  if (!badge && !actions) return null;

  return (
    <header
      className={cn(
        "mb-4 flex shrink-0 flex-wrap items-center justify-end gap-2",
        className,
      )}
    >
      <h1 className="sr-only">{title}</h1>
      {badge}
      {actions}
    </header>
  );
}
