import { useEffect, useState, type ReactNode } from "react";

import { getCurrentWindow } from "@tauri-apps/api/window";

import { QuickPanelClipboard } from "@/components/quick-panel/quick-panel-clipboard";
import { QuickPanelNotes } from "@/components/quick-panel/quick-panel-notes";
import { QuickPanelProjects } from "@/components/quick-panel/quick-panel-projects";
import { QuickPanelSearch } from "@/components/quick-panel/quick-panel-search";
import { ipc } from "@/lib/ipc";

/** 收起组合窗口（Rust 侧统一处理，避免各页各自调窗口 API）。 */
function hideHub() {
  void ipc.hideHub();
}

/**
 * 组合窗口（窗口 label = `hub`）：一个透明大窗口里并排放四张卡片，
 * 看起来像四个独立小窗，实际共用同一个 WebView。
 *
 * 内容直接复用快捷小窗的四页组件：它们本身只依赖 `active` / `focused`，
 * 不关心自己跑在哪个窗口。组合窗口是正常可聚焦窗口，因此 `focused` 就是
 * 真实的窗口焦点：失焦时 Rust 会隐藏窗口，这里同时把各页切回 `active=false`
 * 以停止轮询。
 */
export function HubPanel() {
  const [focused, setFocused] = useState(true);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWindow()
      .onFocusChanged(({ payload }) => {
        if (!disposed) setFocused(payload);
      })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        hideHub();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // 四页都按「窗口有焦点」理解：组合窗口没有临时聚焦模式。
  const pageProps = {
    active: focused,
    focused,
    onRequestFocus: async () => true,
    onReleaseFocus: async () => {},
    onDismiss: hideHub,
  };

  return (
    <div
      className="h-full w-full p-4"
      onMouseDown={(event) => {
        // 点透明背景 = 收起；点卡片内部的空白区域不触发。
        if (event.target === event.currentTarget) hideHub();
      }}
    >
      <div className="grid h-full min-h-0 grid-cols-4 gap-3">
        <HubPane>
          <QuickPanelSearch {...pageProps} />
        </HubPane>
        <HubPane>
          <QuickPanelNotes {...pageProps} />
        </HubPane>
        <HubPane>
          <QuickPanelClipboard {...pageProps} />
        </HubPane>
        <HubPane>
          <QuickPanelProjects active={focused} />
        </HubPane>
      </div>
    </div>
  );
}

/** 单张卡片外壳：透明窗口里靠圆角与边框让它看起来是独立小窗。 */
function HubPane({ children }: { children: ReactNode }) {
  return (
    <section className="flex h-full min-h-0 min-w-0 flex-col overflow-y-auto rounded-xl border border-border/70 bg-background px-3.5 py-3 text-sm">
      {children}
    </section>
  );
}
