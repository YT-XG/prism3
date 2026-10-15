import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type AnimationEvent,
} from "react";

import { emitTo, listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";

import { PANEL_WIDTH } from "@/hooks/use-quick-panel-window";
import { ipc, type DockEdge } from "@/lib/ipc";

/** 灰条窗口标签，与 `tauri.conf.json` 一致。 */
const BAR_LABEL = "quick-panel";
/** 面板窗口向灰条窗口上报悬停的事件名，与 `use-quick-panel-window.ts` 一致。 */
const BODY_HOVER_EVENT = "quick-panel:body-hover";
/** 面板窗口就绪（挂载 / 热重载后）事件名：让灰条按真实状态重推一次。 */
const BODY_READY_EVENT = "quick-panel:body-ready";
/** 灰条窗口（Rust 命令）驱动面板展开/收起的事件名。 */
const BODY_STATE_EVENT = "quick-panel:body-state";
/** 收起动画名：播完即隐藏窗口。 */
const EXIT_ANIMATION = "quick-panel-exit";
/** 收起动画后隐藏窗口的兜底延迟（动画事件丢失时）。 */
const HIDE_FALLBACK = 600;

/** `open` = 播放展开动画，`closed` = 播放收起动画并最终隐藏窗口。 */
export type BodyPhase = "open" | "closed";

interface BodyStatePayload {
  open: boolean;
  edge: DockEdge;
}

/** 展开/收起动画的擦除起点：始终从灰条所在的一侧擦出（`--qp-clip`）。 */
export const CLIP_BY_EDGE: Record<DockEdge, string> = {
  right: "inset(0 0 0 100%)",
  left: "inset(0 100% 0 0)",
  top: "inset(0 0 100% 0)",
};

/**
 * 面板窗口（`quick-panel-body`）的交互状态：只负责播放展开/收起动画、向灰条窗口
 * 上报悬停、收起动画结束后隐藏自己，以及按当前页驱动面板盒子的宽度过渡。
 * 何时展开/收起完全由灰条窗口决定。
 *
 * 面板窗口的**窗口尺寸恒为最大页宽**（见 `use-quick-panel-window.ts` 的
 * `PANEL_WINDOW_WIDTH`），换页调宽调窄都只改 CSS 宽度、完全不碰窗口几何——
 * WebView 内容跟不上窗口缩放（缩放瞬间会以旧帧贴新视口左上角绘制一两帧），
 * 可见期间缩放窗口必然闪动，故从结构上杜绝。
 *
 * 该 hook 只在面板窗口内使用。
 */
export function useQuickPanelBody() {
  const [phase, setPhase] = useState<BodyPhase>("closed");
  const [edge, setEdge] = useState<DockEdge>("right");
  /** 面板内容当前宽度：随页面在常规 / 加宽间过渡，驱动 CSS 动画。 */
  const [contentWidth, setContentWidth] = useState(PANEL_WIDTH);

  const hideTimerRef = useRef<number | null>(null);

  const cancelHide = useCallback(() => {
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
  }, []);

  const hideNow = useCallback(() => {
    cancelHide();
    void ipc
      .hideQuickPanelBody()
      .catch((cause) => logError(`隐藏快捷小窗面板失败：${String(cause)}`));
  }, [cancelHide]);

  /** 收起动画播完隐藏窗口；同时挂兜底定时器，避免动画事件丢失后留下透明可点窗口。 */
  const scheduleHide = useCallback(() => {
    cancelHide();
    hideTimerRef.current = window.setTimeout(() => {
      hideTimerRef.current = null;
      hideNow();
    }, HIDE_FALLBACK);
  }, [cancelHide, hideNow]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<BodyStatePayload>(BODY_STATE_EVENT, (event) => {
      if (!active) return;
      setEdge(event.payload.edge);
      setPhase(event.payload.open ? "open" : "closed");
      if (event.payload.open) cancelHide();
      else scheduleHide();
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();

        // 订阅就绪后上报一次：热重载时灰条也能拿到真实状态重推展开。
        void emitTo(BAR_LABEL, BODY_READY_EVENT).catch((cause) =>
          logError(`上报面板就绪失败：${String(cause)}`),
        );
      })
      .catch((cause) => {
        void logError(`订阅面板状态事件失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
      cancelHide();
    };
  }, [cancelHide, scheduleHide]);

  const reportHover = useCallback((hovering: boolean) => {
    void emitTo(BAR_LABEL, BODY_HOVER_EVENT, hovering).catch((cause) =>
      logError(`上报面板悬停状态失败：${String(cause)}`),
    );
  }, []);

  const handleMouseEnter = useCallback(() => reportHover(true), [reportHover]);
  const handleMouseLeave = useCallback(() => reportHover(false), [reportHover]);

  /**
   * 切换面板内容宽度：只改 CSS（面板盒子自身带宽度过渡），不碰窗口几何。
   * 窗口恒为最大页宽，加宽不会被裁、收窄也不需要等窗口；过渡被打断时 CSS 自动反向。
   */
  const requestWidth = useCallback((width: number) => {
    setContentWidth(width);
  }, []);

  /** 收起动画自然结束才隐藏；中途反向（重新展开）会触发 animationcancel，不会误隐藏。 */
  const handleAnimationEnd = useCallback(
    (event: AnimationEvent<HTMLElement>) => {
      if (event.animationName !== EXIT_ANIMATION) return;
      hideNow();
    },
    [hideNow],
  );

  return {
    phase,
    edge,
    contentWidth,
    requestWidth,
    /** 挂在面板盒子（可视卡片）上：透明死区不算悬停，移出卡片即开始收起计时。 */
    rootProps: { onMouseEnter: handleMouseEnter, onMouseLeave: handleMouseLeave },
    bodyProps: {
      "data-state": phase,
      onAnimationEnd: handleAnimationEnd,
    },
  };
}
