import { useWindowStore } from "@/stores/window-store";

import { AppWindow } from "./app-window";

/**
 * 窗口层：只负责把开着的窗口按 z 序铺到桌面上。
 *
 * `pointer-events-none` 让窗口之间的空档仍能点到桌面（关开始菜单 / 失焦），
 * 窗口自己再打开 `pointer-events-auto`。
 *
 * 最小化的窗口**不卸载**、只 `display:none`：速记草稿、工作记录的表单状态
 * 都要在收起再打开后还在（与真实桌面的窗口语义一致）。
 */
export function WindowLayer() {
  const windows = useWindowStore((state) => state.windows);
  const focused = useWindowStore((state) => state.focused);

  return (
    <div
      className="pointer-events-none absolute inset-x-0 z-20"
      style={{ top: "var(--topbar-h)", bottom: "var(--taskbar-h)" }}
    >
      {windows.map((item) => (
        <AppWindow key={item.id} state={item} focused={focused === item.id} />
      ))}
    </div>
  );
}
