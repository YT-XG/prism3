import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type AnimationEvent,
} from "react";

import { emitTo, listen } from "@tauri-apps/api/event";
import { error as logError } from "@tauri-apps/plugin-log";

import { PANEL_WIDTH, PANEL_WIDTH_TRANSITION } from "@/hooks/use-quick-panel-window";
import { ipc, type DockEdge } from "@/lib/ipc";

/** 灰条窗口标签，与 `tauri.conf.json` 一致。 */
const BAR_LABEL = "quick-panel";
/** 面板窗口向灰条窗口上报悬停的事件名，与 `use-quick-panel-window.ts` 一致。 */
const BODY_HOVER_EVENT = "quick-panel:body-hover";
/** 面板窗口就绪（挂载 / 热重载后）事件名：让灰条按真实状态重推一次。 */
const BODY_READY_EVENT = "quick-panel:body-ready";
/** 面板窗口向灰条窗口上报期望宽度（随页面变化）的事件名，与 `use-quick-panel-window.ts` 一致。 */
const BODY_WIDTH_EVENT = "quick-panel:body-width";
/** 收窄时窗口延后于内容过渡再缩的缓冲（毫秒），避免内容被提前裁掉。 */
const SHRINK_WINDOW_BUFFER = 60;
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
 * 上报悬停、收起动画结束后隐藏自己。何时展开/收起完全由灰条窗口决定，
 * 因此面板窗口永不移动窗口几何，也就不会牵连灰条。
 *
 * 该 hook 只在面板窗口内使用。
 */
export function useQuickPanelBody() {
  const [phase, setPhase] = useState<BodyPhase>("closed");
  const [edge, setEdge] = useState<DockEdge>("right");
  /** 面板内容当前宽度：随页面在常规 / 加宽间过渡，驱动 CSS 动画。 */
  const [contentWidth, setContentWidth] = useState(PANEL_WIDTH);

  const hideTimerRef = useRef<number | null>(null);
  const contentWidthRef = useRef<number>(PANEL_WIDTH);
  const widthTimerRef = useRef<number | null>(null);

  const cancelWidthTimer = useCallback(() => {
    if (widthTimerRef.current !== null) {
      window.clearTimeout(widthTimerRef.current);
      widthTimerRef.current = null;
    }
  }, []);

  /** 向灰条窗口上报面板窗口应有的宽度（由其重定位缩放窗口）。 */
  const emitWidth = useCallback((width: number) => {
    void emitTo(BAR_LABEL, BODY_WIDTH_EVENT, width).catch((cause) =>
      logError(`上报面板宽度失败：${String(cause)}`),
    );
  }, []);

  /**
   * 请求切换到目标宽度：内容立即开始 CSS 过渡；窗口宽度则分方向处理——
   * 加宽时先让窗口变大（避免内容溢出被裁），收窄时等内容过渡结束再缩窗口。
   */
  const requestWidth = useCallback(
    (width: number) => {
      if (width === contentWidthRef.current) return;
      const expanding = width > contentWidthRef.current;
      contentWidthRef.current = width;

      cancelWidthTimer();
      if (expanding) {
        // 先把窗口放大（内容已按目标宽度布局，窗口须足够大才不被裁），再落定内容。
        emitWidth(width);
        setContentWidth(width);
      } else {
        // 内容先过渡收窄，窗口等过渡结束再缩；否则内容会被提前裁掉。
        setContentWidth(width);
        widthTimerRef.current = window.setTimeout(() => {
          widthTimerRef.current = null;
          emitWidth(width);
        }, PANEL_WIDTH_TRANSITION + SHRINK_WINDOW_BUFFER);
      }
    },
    [cancelWidthTimer, emitWidth],
  );

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

        // 订阅就绪后上报当前宽度再报就绪：热重载时灰条也能拿到正确宽度重推展开。
        emitWidth(contentWidthRef.current);
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
      cancelWidthTimer();
    };
  }, [cancelHide, cancelWidthTimer, emitWidth, scheduleHide]);

  const reportHover = useCallback((hovering: boolean) => {
    void emitTo(BAR_LABEL, BODY_HOVER_EVENT, hovering).catch((cause) =>
      logError(`上报面板悬停状态失败：${String(cause)}`),
    );
  }, []);

  const handleMouseEnter = useCallback(() => reportHover(true), [reportHover]);
  const handleMouseLeave = useCallback(() => reportHover(false), [reportHover]);

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
    /** 挂在窗口根节点：整块面板区域都算「悬停」，避免缝隙里误收起。 */
    rootProps: { onMouseEnter: handleMouseEnter, onMouseLeave: handleMouseLeave },
    bodyProps: {
      "data-state": phase,
      onAnimationEnd: handleAnimationEnd,
    },
  };
}
