import { Copy, Minus, Square, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { useAppMeta } from "@/hooks/use-app-catalog";
import { cn } from "@/lib/utils";
import { useWindowStore, type AppWindowState } from "@/stores/window-store";

import { PageHost } from "./page-host";

interface DragState {
  mode: "move" | "resize";
  startX: number;
  startY: number;
  rect: { x: number; y: number; w: number; h: number };
}

/** 关闭动画时长，必须与 `index.css` 的 `window-close` 一致。 */
const CLOSE_MS = 130;

/**
 * 一个浮层窗口：标题栏（图标 + 标题 + 三个窗口键）、可拖动、右下角可缩放。
 *
 * 拖动与缩放走「pointerdown 记录起点 → window 上监听 move → 直接改内联样式」，
 * 松手才提交到 store：这样 60fps 拖动不会触发 React 重渲染，
 * 也避免 store 抖动导致任务栏、开始菜单跟着重画。
 */
export function AppWindow({ state, focused }: { state: AppWindowState; focused: boolean }) {
  const app = useAppMeta(state.id);
  const Icon = app.icon;

  const setRect = useWindowStore((store) => store.setRect);
  const focusWindow = useWindowStore((store) => store.focus);
  const minimizeWindow = useWindowStore((store) => store.minimize);
  const toggleMaximize = useWindowStore((store) => store.toggleMaximize);
  const closeWindow = useWindowStore((store) => store.close);

  const elRef = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [closing, setClosing] = useState(false);

  const beginDrag = useCallback(
    (mode: "move" | "resize") => (event: React.PointerEvent) => {
      if (event.button !== 0) return;
      // 不能 preventDefault：它会连带取消兼容的 mouse 事件，标题栏的双击最大化就没了。
      // 文字选择已经由 `select-none` 挡住，拖动本身也不依赖默认行为。
      event.stopPropagation();
      focusWindow(state.id);
      setDrag({
        mode,
        startX: event.clientX,
        startY: event.clientY,
        rect: { ...state.rect },
      });
    },
    [focusWindow, state.id, state.rect],
  );

  useEffect(() => {
    if (!drag) return;
    const el = elRef.current;
    if (!el) return;

    const onMove = (event: PointerEvent) => {
      const dx = event.clientX - drag.startX;
      const dy = event.clientY - drag.startY;
      if (drag.mode === "move") {
        el.style.transform = `translate(${dx}px, ${dy}px)`;
        return;
      }
      const w = Math.max(app.min.w, drag.rect.w + dx);
      const h = Math.max(app.min.h, drag.rect.h + dy);
      el.style.width = `${w}px`;
      el.style.height = `${h}px`;
    };

    const onUp = (event: PointerEvent) => {
      const dx = event.clientX - drag.startX;
      const dy = event.clientY - drag.startY;
      if (drag.mode === "move") {
        el.style.transform = "";
        setRect(state.id, {
          ...drag.rect,
          x: drag.rect.x + dx,
          y: drag.rect.y + dy,
        });
      } else {
        el.style.width = "";
        el.style.height = "";
        setRect(state.id, {
          ...drag.rect,
          w: Math.max(app.min.w, drag.rect.w + dx),
          h: Math.max(app.min.h, drag.rect.h + dy),
        });
      }
      setDrag(null);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp, { once: true });
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [app.min.h, app.min.w, drag, setRect, state.id]);

  const requestClose = useCallback(() => {
    setClosing(true);
    window.setTimeout(() => closeWindow(state.id), CLOSE_MS);
  }, [closeWindow, state.id]);

  return (
    <div
      ref={elRef}
      className={`app-window pointer-events-auto ${state.maximized ? "" : "rounded-[13px]"}`}
      data-state={closing ? "closing" : "open"}
      data-dragging={drag ? "1" : undefined}
      style={{
        left: state.rect.x,
        top: state.rect.y,
        width: state.rect.w,
        height: state.rect.h,
        zIndex: 10 + state.z,
        display: state.minimized ? "none" : undefined,
      }}
      onPointerDownCapture={() => focusWindow(state.id)}
      role="dialog"
      aria-label={app.title}
    >
      <div
        className={`window-frame flex h-full w-full flex-col overflow-hidden border ${state.maximized ? "" : "rounded-[13px]"}`}
        style={{
          background: "var(--window-bg)",
          borderColor: "var(--window-border)",
          boxShadow: focused ? "var(--window-shadow)" : "var(--window-shadow-inactive)",
          backdropFilter: "blur(26px) saturate(1.25)",
          WebkitBackdropFilter: "blur(26px) saturate(1.25)",
        }}
      >
        <div
          className="flex h-10 shrink-0 items-center gap-2 border-b pl-3 select-none"
          style={{
            background: "var(--window-titlebar)",
            borderColor: "var(--window-border)",
            cursor: drag?.mode === "move" ? "grabbing" : "default",
          }}
          onPointerDown={beginDrag("move")}
          onDoubleClick={() => toggleMaximize(state.id)}
        >
          <span
            className="flex size-[19px] shrink-0 items-center justify-center rounded-[6px] text-white"
            style={{ backgroundImage: `linear-gradient(150deg, ${app.tint[0]}, ${app.tint[1]})` }}
            aria-hidden
          >
            {Icon ? (
              <Icon className="size-[11px]" strokeWidth={2.2} />
            ) : (
              <span className="text-[10px] leading-none">{app.emoji ?? "📦"}</span>
            )}
          </span>
          <span className="truncate text-[12px] font-medium text-foreground">{app.title}</span>

          <div
            className="ml-auto flex items-center gap-0.5 pr-1"
            onPointerDown={(event) => event.stopPropagation()}
            onDoubleClick={(event) => event.stopPropagation()}
          >
            <WindowButton label="最小化" onClick={() => minimizeWindow(state.id)}>
              <Minus className="size-3.5" />
            </WindowButton>
            <WindowButton
              label={state.maximized ? "还原" : "最大化"}
              onClick={() => toggleMaximize(state.id)}
            >
              {state.maximized ? <Copy className="size-3" /> : <Square className="size-2.5" />}
            </WindowButton>
            <WindowButton label="关闭" danger onClick={requestClose}>
              <X className="size-3.5" />
            </WindowButton>
          </div>
        </div>

        <div
          className="min-h-0 flex-1 overflow-hidden"
          style={{ background: "var(--window-body)" }}
        >
          <PageHost windowId={state.id} />
        </div>
      </div>

      {state.maximized ? null : (
        <div
          role="presentation"
          aria-hidden
          onPointerDown={beginDrag("resize")}
          className="absolute right-0 bottom-0 z-10 size-4 cursor-nwse-resize"
          style={{
            background:
              "linear-gradient(135deg, transparent 0 46%, color-mix(in srgb, var(--muted-foreground) 26%, transparent) 46% 52%, transparent 52% 62%, color-mix(in srgb, var(--muted-foreground) 26%, transparent) 62% 68%, transparent 68%)",
          }}
        />
      )}
    </div>
  );
}

function WindowButton({
  label,
  onClick,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={cn(
        "flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors",
        "hover:bg-black/6 hover:text-foreground dark:hover:bg-white/10",
        danger && "hover:bg-[#e81123] hover:text-white dark:hover:bg-[#e81123]",
      )}
    >
      {children}
    </button>
  );
}

