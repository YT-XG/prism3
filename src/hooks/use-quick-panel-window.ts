import { useCallback, useEffect, useRef, useState, type PointerEvent } from "react";

import { listen } from "@tauri-apps/api/event";
import { currentMonitor, getCurrentWindow, primaryMonitor } from "@tauri-apps/api/window";
import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type DockEdge } from "@/lib/ipc";

/** 灰条尺寸；灰条窗口的尺寸即该值（吸附 top 时宽高互换）。 */
export const LINE_WIDTH = 6;
export const LINE_HEIGHT = 64;
/** 面板常规宽度（概览 / 关于页）。 */
export const PANEL_WIDTH = 360;
/** 面板加宽宽度（帮助页）：只改宽度，高度不变。 */
export const PANEL_WIDTH_WIDE = 520;
/** 面板宽度过渡时长（毫秒）；面板窗口与内容动画共用，收窄时窗口延后该时长再缩。 */
export const PANEL_WIDTH_TRANSITION = 200;
/** 面板高度；面板窗口高度恒为该值。 */
export const PANEL_HEIGHT = 520;
/** 灰条与面板之间的间距。 */
export const BAR_GAP = 12;
/** 灰条到面板之间占用的总厚度（垂直边缘方向）。 */
const BAR_STRIP = LINE_WIDTH + BAR_GAP;
/** 距工作区边缘的留白。 */
const SCREEN_MARGIN = 16;
/** 鼠标移开后收起的延迟。 */
const COLLAPSE_DELAY = 1000;
/** 收起后强制隐藏面板窗口的兜底延迟（动画事件丢失时）。 */
const BODY_HIDE_FALLBACK = 1500;
/** 拖拽时判定「吸附到某条边缘」的触发半径（逻辑像素）。 */
const SNAP_RADIUS = 72;
/** 面板窗口上报悬停状态的事件名，与 `use-quick-panel-body.ts` 一致。 */
const BODY_HOVER_EVENT = "quick-panel:body-hover";
/** 面板窗口就绪（挂载 / 热重载后）事件名，与 `use-quick-panel-body.ts` 一致。 */
const BODY_READY_EVENT = "quick-panel:body-ready";
/** 面板窗口上报期望宽度（随页面变化）的事件名，与 `use-quick-panel-body.ts` 一致。 */
const BODY_WIDTH_EVENT = "quick-panel:body-width";

const win = getCurrentWindow();

interface MonitorBounds {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** 窗口在屏幕坐标系下的几何（逻辑像素）。 */
interface Geometry {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface QuickPanelStatePayload {
  enabled: boolean;
  edge: DockEdge;
  offset: number | null;
}

interface DragState {
  /** 按下点相对灰条沿边起点（顶端/左端）的偏移，拖动中保持握持不变。 */
  grabAlong: number;
  pointerId: number;
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

/** 后台 IPC 失败只记日志：窗口操作失败不应打断交互。 */
function reportFailure(what: string, promise: Promise<unknown>) {
  void promise.catch((cause) => logError(`${what}失败：${String(cause)}`));
}

/**
 * 以逻辑像素返回当前显示器工作区（排除任务栏/停靠栏，避免面板与其重叠）。
 */
async function resolveWorkArea(): Promise<MonitorBounds> {
  const monitor =
    (await currentMonitor().catch(() => null)) ??
    (await primaryMonitor().catch(() => null));

  if (monitor) {
    const scale = monitor.scaleFactor || 1;
    const rect = monitor.workArea ?? { position: monitor.position, size: monitor.size };
    const left = rect.position.x / scale;
    const top = rect.position.y / scale;
    return {
      left,
      top,
      right: left + rect.size.width / scale,
      bottom: top + rect.size.height / scale,
    };
  }

  // 兜底：拿不到显示器信息时退化为浏览器报告的屏幕尺寸。
  return { left: 0, top: 0, right: window.screen.width, bottom: window.screen.height };
}

/** 沿吸附方向（边缘走向）的可用区间与默认位置，长度指元素在该轴上的尺寸。 */
function alongAxis(bounds: MonitorBounds, edge: DockEdge, length: number) {
  if (edge === "top") {
    return {
      min: bounds.left + SCREEN_MARGIN,
      max: bounds.right - length - SCREEN_MARGIN,
      fallback: bounds.left + (bounds.right - bounds.left - length) / 2,
    };
  }
  return {
    min: bounds.top + SCREEN_MARGIN,
    max: bounds.bottom - length - SCREEN_MARGIN,
    fallback: bounds.bottom - length - SCREEN_MARGIN,
  };
}

/** 灰条沿边方向（顶端/左端）的可用区间。 */
function barAxis(bounds: MonitorBounds, edge: DockEdge) {
  return alongAxis(bounds, edge, LINE_HEIGHT);
}

/** 灰条沿边坐标（顶端/左端在屏幕坐标系下的值）。 */
function barPosition(bounds: MonitorBounds, edge: DockEdge, offset: number | null) {
  const { min, max, fallback } = barAxis(bounds, edge);
  return clamp(offset ?? fallback, min, max);
}

/**
 * 面板沿边起点：以灰条为锚点居中展开，贴到工作区边界时夹取。
 * 保证灰条始终落在面板范围内（`panelStart <= barPos <= panelStart + panelLen - LINE_HEIGHT`）。
 * 宽度变化时（帮助页加宽）沿边长度随之变化，起点重算以保持灰条锚定侧不变。
 */
function panelStart(bounds: MonitorBounds, edge: DockEdge, barPos: number, width: number) {
  const panelLen = edge === "top" ? width : PANEL_HEIGHT;
  const { min, max } = alongAxis(bounds, edge, panelLen);
  return clamp(barPos + LINE_HEIGHT - panelLen, min, max);
}

/** 灰条几何：紧贴指定边缘的细条（灰条窗口本身就是它，不含面板）。 */
function barGeometry(bounds: MonitorBounds, edge: DockEdge, barPos: number): Geometry {
  if (edge === "top") {
    return { x: Math.round(barPos), y: bounds.top, width: LINE_HEIGHT, height: LINE_WIDTH };
  }
  return {
    x: edge === "left" ? bounds.left : bounds.right - LINE_WIDTH,
    y: Math.round(barPos),
    width: LINE_WIDTH,
    height: LINE_HEIGHT,
  };
}

/**
 * 面板几何：面板窗口**只装面板本体**（灰条由独立窗口承担），高度恒为 `PANEL_HEIGHT`，
 * 宽度随页面变化（帮助页加宽）；位置按灰条锚定侧重算，与灰条隔 `BAR_GAP`。
 */
function panelGeometry(
  bounds: MonitorBounds,
  edge: DockEdge,
  barPos: number,
  width: number,
): Geometry {
  const start = panelStart(bounds, edge, barPos, width);
  if (edge === "top") {
    return {
      x: Math.round(start),
      y: bounds.top + BAR_STRIP,
      width,
      height: PANEL_HEIGHT,
    };
  }
  return {
    x: edge === "left" ? bounds.left + BAR_STRIP : bounds.right - BAR_STRIP - width,
    y: Math.round(start),
    width,
    height: PANEL_HEIGHT,
  };
}

/** 光标距每条可吸附边缘的距离，取最近且处于触发半径内的边缘。 */
function nearestDockEdge(
  bounds: MonitorBounds,
  cursor: { x: number; y: number },
  current: DockEdge,
): DockEdge {
  const candidates = [
    { edge: "left", distance: Math.abs(cursor.x - bounds.left) },
    { edge: "right", distance: Math.abs(bounds.right - cursor.x) },
    { edge: "top", distance: Math.abs(cursor.y - bounds.top) },
  ] as const;

  let best: DockEdge = current;
  let bestDistance = Infinity;
  for (const { edge, distance } of candidates) {
    if (distance < bestDistance) {
      best = edge;
      bestDistance = distance;
    }
  }
  return bestDistance <= SNAP_RADIUS ? best : current;
}

/**
 * 灰条窗口（`quick-panel`）的几何与交互，并驱动面板窗口（`quick-panel-body`）。
 *
 * 关键不变式：**灰条窗口的几何只在拖动灰条与开关/换边时变化**。
 * 展开/收起面板完全不碰灰条窗口（面板是另一个窗口，用 CSS 动画展开），
 * 所以「灰条只在被拖动时移动」是结构保证，不依赖逐帧对齐窗口位移与 CSS 补偿。
 *
 * 该 hook 只在灰条窗口内使用；窗口自身尺寸即灰条尺寸，故无需在窗口内做偏移补偿。
 */
export function useQuickPanelWindow() {
  const [enabled, setEnabled] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [edge, setEdge] = useState<DockEdge>("right");

  const boundsRef = useRef<MonitorBounds | null>(null);
  const edgeRef = useRef<DockEdge>("right");
  const offsetRef = useRef<number | null>(null);
  const collapseTimerRef = useRef<number | null>(null);
  const hideBodyTimerRef = useRef<number | null>(null);
  const draggingRef = useRef(false);
  const dragStateRef = useRef<DragState | null>(null);
  /** 面板是否已请求展开。 */
  const openRef = useRef(false);
  /** 灰条 / 面板窗口的悬停状态：两者都不悬停才允许自动收起。 */
  const barHoverRef = useRef(false);
  const bodyHoverRef = useRef(false);
  /** 拖动中待刷新的几何（raf 节流）。 */
  const rafRef = useRef<number | null>(null);
  const pendingRef = useRef<{ bar: Geometry; body: Geometry | null } | null>(null);
  /** 面板窗口当前宽度（由面板窗口按页面上报，帮助页会加宽）。 */
  const bodyWidthRef = useRef<number>(PANEL_WIDTH);

  const getBounds = useCallback(async () => {
    const bounds = await resolveWorkArea();
    boundsRef.current = bounds;
    return bounds;
  }, []);

  const currentBarPos = useCallback((bounds: MonitorBounds) => {
    return barPosition(bounds, edgeRef.current, offsetRef.current);
  }, []);

  const clearCollapseTimer = useCallback(() => {
    if (collapseTimerRef.current !== null) {
      window.clearTimeout(collapseTimerRef.current);
      collapseTimerRef.current = null;
    }
  }, []);

  const clearHideBodyTimer = useCallback(() => {
    if (hideBodyTimerRef.current !== null) {
      window.clearTimeout(hideBodyTimerRef.current);
      hideBodyTimerRef.current = null;
    }
  }, []);

  /** 立即把灰条窗口摆到给定几何（位置 + 尺寸，一次 IPC）。 */
  const applyBarGeometry = useCallback((geometry: Geometry) => {
    return ipc.setQuickPanelGeometry(geometry.x, geometry.y, geometry.width, geometry.height);
  }, []);

  /**
   * 按面板窗口上报的宽度重定位并缩放面板窗口（一次 IPC，不逐帧改几何）。
   * 面板窗口在展开/收窄时自行按同一时长播放内容宽度过渡，收窄会延后上报，
   * 因此这里始终按目标宽度直接落定即可。
   */
  const applyBodyWidth = useCallback(
    (width: number) => {
      bodyWidthRef.current = width;
      if (!openRef.current) return;
      const bounds = boundsRef.current;
      if (!bounds) return;
      const geometry = panelGeometry(bounds, edgeRef.current, currentBarPos(bounds), width);
      reportFailure(
        "调整快捷小窗面板宽度",
        ipc.setQuickPanelBodyGeometry(geometry.x, geometry.y, geometry.width, geometry.height),
      );
    },
    [currentBarPos],
  );

  /** 贴到当前边缘显示灰条窗口（面板窗口此时应是隐藏的）。 */
  const showBar = useCallback(async () => {
    const bounds = boundsRef.current ?? (await getBounds());
    const barPos = currentBarPos(bounds);
    offsetRef.current = barPos;
    const geometry = barGeometry(bounds, edgeRef.current, barPos);

    // 隐藏窗口上的 setSize 可能不生效，导致首次 show 沿用旧尺寸。
    // 故在 show 前后各落实一次，并再等一帧兜底。
    await applyBarGeometry(geometry);
    await win.show();
    await applyBarGeometry(geometry);
    await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
    await applyBarGeometry(geometry);
  }, [applyBarGeometry, currentBarPos, getBounds]);

  /** 定位并显示面板窗口，同时让面板播放展开动画。 */
  const placeAndShowBody = useCallback(async () => {
    const bounds = boundsRef.current ?? (await getBounds());
    const barPos = currentBarPos(bounds);
    offsetRef.current = barPos;
    const geometry = panelGeometry(bounds, edgeRef.current, barPos, bodyWidthRef.current);
    await ipc.openQuickPanelBody(
      geometry.x,
      geometry.y,
      geometry.width,
      geometry.height,
      edgeRef.current,
    );
  }, [currentBarPos, getBounds]);

  /** 展开面板：定位 → 显示 → 面板窗口播放展开动画。灰条窗口几何始终不变。 */
  const openBody = useCallback(async () => {
    if (openRef.current) return;
    clearCollapseTimer();
    clearHideBodyTimer();

    // 先置位再 await：鼠标在灰条上抖动时不会重复展开。
    openRef.current = true;
    try {
      await placeAndShowBody();
    } catch (cause) {
      openRef.current = false;
      void logError(`快捷小窗展开失败：${String(cause)}`);
    }
  }, [clearCollapseTimer, clearHideBodyTimer, placeAndShowBody]);

  /**
   * 面板窗口就绪（挂载 / 热重载）后按灰条侧的真实状态重推一次：
   * 已展开则重新定位显示，收起则重播收起动画（保证窗口不会残留可见）。
   */
  const resyncBody = useCallback(() => {
    if (openRef.current) {
      reportFailure("同步快捷小窗面板", placeAndShowBody());
    } else {
      reportFailure("重置快捷小窗面板", ipc.closeQuickPanelBody(edgeRef.current));
    }
  }, [placeAndShowBody]);

  /** 收起面板：面板窗口播完收起动画后自行隐藏；灰条窗口不动。 */
  const closeBody = useCallback(() => {
    if (!openRef.current) return;
    openRef.current = false;
    clearCollapseTimer();
    reportFailure("收起快捷小窗面板", ipc.closeQuickPanelBody(edgeRef.current));

    // 兜底：窗口被挂起等导致动画事件丢失时强制隐藏，避免残留可点透明窗口。
    clearHideBodyTimer();
    hideBodyTimerRef.current = window.setTimeout(() => {
      hideBodyTimerRef.current = null;
      reportFailure("隐藏快捷小窗面板", ipc.hideQuickPanelBody());
    }, BODY_HIDE_FALLBACK);
  }, [clearCollapseTimer, clearHideBodyTimer]);

  const scheduleCollapse = useCallback(() => {
    if (draggingRef.current || !openRef.current) return;
    clearCollapseTimer();
    collapseTimerRef.current = window.setTimeout(() => {
      collapseTimerRef.current = null;
      if (barHoverRef.current || bodyHoverRef.current) return;
      closeBody();
    }, COLLAPSE_DELAY);
  }, [clearCollapseTimer, closeBody]);

  /** 应用开关状态：开启则贴到记忆边缘显示灰条，关闭则两个窗口都收起。 */
  const applyState = useCallback(
    async (nextEnabled: boolean, nextEdge: DockEdge, offset: number | null) => {
      setEnabled(nextEnabled);
      const resolvedEdge = nextEdge ?? "right";
      edgeRef.current = resolvedEdge;
      setEdge(resolvedEdge);
      offsetRef.current = offset;
      await getBounds();

      // 开关 / 边缘 / 偏移变化后旧面板位置已失效，先收起。
      closeBody();

      if (nextEnabled) {
        await showBar();
      } else {
        clearCollapseTimer();
        await win.hide();
      }
    },
    [clearCollapseTimer, closeBody, getBounds, showBar],
  );

  // 初始化：先订阅事件（后续变更），再主动拉一次配置（事件不缓存，需要兜底）。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void (async () => {
      const off = await listen<QuickPanelStatePayload>("quick-panel:state", (event) => {
        if (!active) return;
        void applyState(event.payload.enabled, event.payload.edge, event.payload.offset);
      });

      if (!active) {
        off();
        return;
      }
      unlisten = off;

      const config = await ipc.getConfig();
      if (!active) return;
      await applyState(config.quickPanel, config.quickPanelEdge, config.quickPanelY);
    })().catch((cause) => {
      void logError(`快捷小窗初始化失败：${String(cause)}`);
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [applyState]);

  // 面板窗口的悬停状态：它自己不会收起面板，统一由灰条窗口调度，避免两边计时器打架。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<boolean>(BODY_HOVER_EVENT, (event) => {
      if (!active) return;
      bodyHoverRef.current = event.payload;
      if (event.payload) clearCollapseTimer();
      else scheduleCollapse();
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅面板悬停事件失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [clearCollapseTimer, scheduleCollapse]);

  // 面板窗口就绪（首帧 / 热重载）：把灰条侧的真实状态重推一次，避免状态事件被错过。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen(BODY_READY_EVENT, () => {
      if (active) resyncBody();
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅面板就绪事件失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [resyncBody]);

  // 面板窗口上报宽度（随页面变化）：展开时立即重定位缩放，收窄时面板延后上报至过渡结束。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<number>(BODY_WIDTH_EVENT, (event) => {
      if (active) applyBodyWidth(event.payload);
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅面板宽度事件失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [applyBodyWidth]);

  // 卸载时清理定时器与待刷新的帧。
  useEffect(() => {
    return () => {
      if (rafRef.current !== null) window.cancelAnimationFrame(rafRef.current);
      if (collapseTimerRef.current !== null) window.clearTimeout(collapseTimerRef.current);
      if (hideBodyTimerRef.current !== null) window.clearTimeout(hideBodyTimerRef.current);
    };
  }, []);

  const handleBarEnter = useCallback(() => {
    barHoverRef.current = true;
    if (openRef.current) clearCollapseTimer();
    else void openBody();
  }, [clearCollapseTimer, openBody]);

  const handleBarLeave = useCallback(() => {
    barHoverRef.current = false;
    scheduleCollapse();
  }, [scheduleCollapse]);

  const handleDragStart = useCallback(
    (event: PointerEvent<HTMLElement>) => {
      // 只在面板展开时允许拖动：收起态灰条极小，误触概率高于收益。
      if (!openRef.current) return;
      const bounds = boundsRef.current;
      if (!bounds) return;

      event.currentTarget.setPointerCapture(event.pointerId);
      clearCollapseTimer();
      draggingRef.current = true;
      setDragging(true);

      const barPos = currentBarPos(bounds);
      offsetRef.current = barPos;
      dragStateRef.current = {
        grabAlong: (edgeRef.current === "top" ? event.screenX : event.screenY) - barPos,
        pointerId: event.pointerId,
      };
    },
    [clearCollapseTimer, currentBarPos],
  );

  const handleDragMove = useCallback((event: PointerEvent<HTMLElement>) => {
    const drag = dragStateRef.current;
    const bounds = boundsRef.current;
    if (!drag || !bounds || drag.pointerId !== event.pointerId) return;

    // 光标贴近某条可吸附边缘时切换到该边缘（左/右/上，不含底部）。
    const nextEdge = nearestDockEdge(
      bounds,
      { x: event.screenX, y: event.screenY },
      edgeRef.current,
    );

    const axis = barAxis(bounds, nextEdge);
    const cursorAlong = nextEdge === "top" ? event.screenX : event.screenY;
    const barPos = clamp(cursorAlong - drag.grabAlong, axis.min, axis.max);

    edgeRef.current = nextEdge;
    offsetRef.current = barPos;
    setEdge(nextEdge);

    pendingRef.current = {
      bar: barGeometry(bounds, nextEdge, barPos),
      body: openRef.current
        ? panelGeometry(bounds, nextEdge, barPos, bodyWidthRef.current)
        : null,
    };
    if (rafRef.current !== null) return;
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = null;
      const pending = pendingRef.current;
      if (!pending) return;
      reportFailure("移动快捷小窗灰条", applyBarGeometry(pending.bar));
      if (pending.body) {
        reportFailure(
          "跟随移动快捷小窗面板",
          ipc.setQuickPanelBodyPosition(pending.body.x, pending.body.y),
        );
      }
    });
  }, [applyBarGeometry]);

  const handleDragEnd = useCallback(
    (event: PointerEvent<HTMLElement>) => {
      const drag = dragStateRef.current;
      if (!drag) return;

      if (event.currentTarget.hasPointerCapture(drag.pointerId)) {
        event.currentTarget.releasePointerCapture(drag.pointerId);
      }
      if (rafRef.current !== null) {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      pendingRef.current = null;

      dragStateRef.current = null;
      draggingRef.current = false;
      setDragging(false);

      const bounds = boundsRef.current;
      if (!bounds) return;

      const edge = edgeRef.current;
      const barPos = offsetRef.current ?? currentBarPos(bounds);
      offsetRef.current = barPos;

      // 拖动中已实时定位，落点直接对齐最终几何并持久化。
      reportFailure("落定快捷小窗灰条", applyBarGeometry(barGeometry(bounds, edge, barPos)));
      if (openRef.current) {
        const body = panelGeometry(bounds, edge, barPos, bodyWidthRef.current);
        reportFailure("落定快捷小窗面板", ipc.setQuickPanelBodyPosition(body.x, body.y));
      }
      reportFailure("保存快捷小窗位置", ipc.setQuickPanelDock(edge, Math.round(barPos)));
    },
    [applyBarGeometry, currentBarPos],
  );

  return {
    enabled,
    dragging,
    edge,
    rootProps: { onMouseEnter: handleBarEnter, onMouseLeave: handleBarLeave },
    dragProps: {
      onPointerDown: handleDragStart,
      onPointerMove: handleDragMove,
      onPointerUp: handleDragEnd,
      onPointerCancel: handleDragEnd,
    },
  };
}
