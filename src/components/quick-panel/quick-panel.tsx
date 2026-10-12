import { useQuickPanelWindow } from "@/hooks/use-quick-panel-window";
import { cn } from "@/lib/utils";

/**
 * 快捷小窗灰条：跑在独立窗口 `quick-panel` 中，**窗口本身就等于灰条**。
 * 悬停灰条展开面板（另一个窗口），按住拖动换边（左 / 右 / 上）。
 *
 * 展开/收起面板不改变本窗口几何，因此除拖动外灰条始终静止在屏幕边缘。
 */
export function QuickPanel() {
  const { enabled, dragging, edge, rootProps, dragProps } = useQuickPanelWindow();

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
        className={cn(
          "h-full w-full rounded-full bg-neutral-400/70 transition-colors hover:bg-neutral-400",
          dragging ? "cursor-grabbing" : "cursor-grab",
        )}
      />
    </div>
  );
}
