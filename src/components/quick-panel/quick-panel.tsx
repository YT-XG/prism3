import { useAppStatus, useAppStatusReporter } from "@/hooks/use-app-status";
import { useQuickPanelWindow } from "@/hooks/use-quick-panel-window";
import { cn } from "@/lib/utils";

/**
 * 快捷小窗灰条：跑在独立窗口 `quick-panel` 中，**窗口本身就等于灰条**。
 * 悬停灰条展开面板（另一个窗口），按住拖动换边（左 / 右 / 上）。
 *
 * 展开/收起面板不改变本窗口几何，因此除拖动外灰条始终静止在屏幕边缘。
 *
 * 外观全部由 `quick-panel.css` 的 `.quick-panel-bar` 承担，这里只挂三个数据属性：
 * `data-status`（色相 + 呼吸/闪烁）、`data-edge`（渐变方向，上吸附时窗口横放）、
 * `data-dragging`（拖动中停表并提亮）。
 */
export function QuickPanel() {
  const { enabled, dragging, edge, rootProps, dragProps } = useQuickPanelWindow();
  // 两个 hook 必须在提前 return 之前调用。窗口被禁用时 Rust 会 hide 本窗口，
  // reporter 靠 visibilitychange 自动停表；彻底禁用时窗口被销毁，零采样。
  const status = useAppStatus();
  useAppStatusReporter();

  if (!enabled) {
    return <div className="h-full w-full" />;
  }

  return (
    <div className="h-full w-full" {...rootProps}>
      <div
        {...dragProps}
        role="separator"
        aria-label="快捷小窗把手"
        aria-orientation={edge === "top" ? "horizontal" : "vertical"}
        data-status={status}
        data-edge={edge}
        data-dragging={dragging}
        className={cn(
          "quick-panel-bar h-full w-full rounded-full",
          dragging ? "cursor-grabbing" : "cursor-grab",
        )}
      />
    </div>
  );
}
