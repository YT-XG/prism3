import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * 主窗口页面头部：标题 + 一句说明 + 右侧操作。
 *
 * 所有页面共用一份，保证标题字号、行高与上下留白一致——此前各页自己写 `h1`，
 * 出现了 16px / 18px 两种字号、说明文字或粗或细的问题，扫视时页与页之间对不上。
 * 结构固定为 `h1` 紧跟 `p`，`index.css` 里「壁纸下的标题轮廓光」规则依赖这个顺序。
 */
export function PageHeader({
  title,
  description,
  badge,
  actions,
  className,
}: {
  title: string;
  description?: ReactNode;
  /** 标题右侧的状态标记（如「采集预热中」）。 */
  badge?: ReactNode;
  /** 右侧操作区：按钮、开关等。 */
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <header
      className={cn(
        "flex shrink-0 flex-wrap items-start justify-between gap-x-5 gap-y-3",
        className,
      )}
    >
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="font-heading text-[1.34rem] leading-[1.2] font-semibold tracking-[-0.028em] [text-wrap:balance]">
            {title}
          </h1>
          {badge}
        </div>
        {description ? (
          <p className="mt-1.5 max-w-[70ch] text-[0.8125rem] leading-[1.6] text-muted-foreground">
            {description}
          </p>
        ) : null}
      </div>

      {actions ? (
        <div className="flex shrink-0 items-center gap-2">{actions}</div>
      ) : null}
    </header>
  );
}
