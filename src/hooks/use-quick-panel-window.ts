import { useCallback, useEffect, useRef, useState, type PointerEvent } from "react";

import { listen } from "@tauri-apps/api/event";
import { currentMonitor, getCurrentWindow, primaryMonitor } from "@tauri-apps/api/window";
import { error as logError } from "@tauri-apps/plugin-log";

import { ipc, type DockEdge } from "@/lib/ipc";

/** 灰条尺寸；灰条窗口的尺寸即该值（吸附 top 时宽高互换）。与 Rust `QUICK_PANEL_BAR_*` 保持一致。 */
export const LINE_WIDTH = 8;
export const LINE_HEIGHT = 64;
/** 面板常规宽度（概览 / 关于页）。 */
export const PANEL_WIDTH = 360;
/** 面板加宽宽度（帮助页）：只改宽度，高度不变。 */
export const PANEL_WIDTH_WIDE = 520;
/**
 * 面板**窗口**的固定宽度：恒为最大页宽，可见期间从不缩放。
 * WebView 内容跟不上窗口缩放——缩放瞬间旧帧会贴着新视口左上角被画出一两帧
 * （表现为面板整体平移 / 瘦成一条再弹回，见 docs/quick-panel-flash-postmortem.md），
 * 因此换页调宽调窄只改面板盒子的 CSS 宽度，窗口几何不动。
 * **约束**：新增页面时 `panelWidth` 不得超过该值，否则内容会被窗口裁掉。
 */
export const PANEL_WINDOW_WIDTH = PANEL_WIDTH_WIDE;
/** 面板宽度过渡时长（毫秒）：面板盒子 CSS 过渡用（窗口不参与，纯视觉）。 */
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
/** 面板进入/退出「输入聚焦」模式的事件名：聚焦期间不自动收起。 */
const BODY_PIN_EVENT = "quick-panel:body-pinned";
/** 面板主动请求收起（Esc）的事件名：收起动作仍由灰条窗口执行。 */
const BODY_DISMISS_EVENT = "quick-panel:body-dismiss";

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
 * 面板窗口沿边起点：以灰条为锚点、按**窗口沿边长度**（恒定，top 吸附时为
 * `PANEL_WINDOW_WIDTH`，否则 `PANEL_HEIGHT`）夹取到工作区内。窗口尺寸不随
 * 页面变化，故该值也与当前页无关；面板盒子在窗口内贴灰条一侧对齐。
 */
function panelStart(bounds: MonitorBounds, edge: DockEdge, barPos: number) {
  const panelLen = edge === "top" ? PANEL_WINDOW_WIDTH : PANEL_HEIGHT;
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
 * 面板窗口几何：**尺寸恒为 `PANEL_WINDOW_WIDTH × PANEL_HEIGHT`**（最大页宽，
 * 换页不再缩放窗口），位置按灰条锚定侧重算，与灰条隔 `BAR_GAP`。
 * 窄页时窗口比面板盒子宽出的部分是透明死区（贴远侧），不参与悬停。
 */
function panelGeometry(bounds: MonitorBounds, edge: DockEdge, barPos: number): Geometry {
  const start = panelStart(bounds, edge, barPos);
  if (edge === "top") {
    return {
      x: Math.round(start),
      y: bounds.top + BAR_STRIP,
      width: PANEL_WINDOW_WIDTH,
      height: PANEL_HEIGHT,
    };
  }
  return {
    x:
      edge === "left"
        ? bounds.left + BAR_STRIP
        : bounds.right - BAR_STRIP - PANEL_WINDOW_WIDTH,
    y: Math.round(start),
    width: PANEL_WINDOW_WIDTH,
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
 * 关键不变式：**灰条窗口的几何只在拖动灰条与开关/换边时变化；面板窗口的
 * 尺寸永远不变（位置只在展开与拖动时变）**。展开/收起与换页调宽全部只做
 * 面板盒子自身的 CSS 动画，任何窗口都不缩放——WebView 内容跟不上窗口缩放，
 * 可见期间缩放必然闪帧（见 docs/quick-panel-flash-postmortem.md）。
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
  /** 灰条 / 面板盒子的悬停状态：两者都不悬停才允许自动收起。 */
  const barHoverRef = useRef(false);
  const bodyHoverRef = useRef(false);
  /**
   * 面板是否处于「输入聚焦」模式（用户在搜索框里打字）。
   * 打字期间鼠标很容易移出面板，此时**必须**暂停自动收起，否则面板会在输入中途消失。
   */
  const bodyPinnedRef = useRef(false);
  /** 拖动中待刷新的几何（raf 节流）。 */
  const rafRef = useRef<number | null>(null);
  const pendingRef = useRef<{ bar: Geometry; body: Geometry | null } | null>(null);

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

  /** 定位并显示面板窗口（只定位不缩放：尺寸恒定），同时让面板播放展开动画。 */
  const placeAndShowBody = useCallback(async () => {
    const bounds = boundsRef.current ?? (await getBounds());
    const barPos = currentBarPos(bounds);
    offsetRef.current = barPos;
    const geometry = panelGeometry(bounds, edgeRef.current, barPos);
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
    // 新展开的面板一定处于非聚焦态（上一次的 pin 不能带过来）。
    bodyPinnedRef.current = false;
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
    // 收起前先退出「输入聚焦」模式：Rust 侧会把焦点还给用户原来的应用，
    // 并保证「面板不可见 ⇒ 不可聚焦」这条不变量在所有收起路径上都成立。
    bodyPinnedRef.current = false;
    clearCollapseTimer();
    reportFailure("退出面板输入聚焦模式", ipc.blurQuickPanelBody());
    reportFailure("收起快捷小窗面板", ipc.closeQuickPanelBody(edgeRef.current));

    // 兜底：窗口被挂起等导致动画事件丢失时强制隐藏，避免残留可点透明窗口。
    clearHideBodyTimer();
    hideBodyTimerRef.current = window.setTimeout(() => {
      hideBodyTimerRef.current = null;
      reportFailure("隐藏快捷小窗面板", ipc.hideQuickPanelBody());
    }, BODY_HIDE_FALLBACK);
  }, [clearCollapseTimer, clearHideBodyTimer]);

  const scheduleCollapse = useCallback(() => {
    // 输入聚焦期间不自动收起：用户正在打字，鼠标移到别处是常态。
    if (draggingRef.current || !openRef.current || bodyPinnedRef.current) return;
    clearCollapseTimer();
    collapseTimerRef.current = window.setTimeout(() => {
      collapseTimerRef.current = null;
      if (barHoverRef.current || bodyHoverRef.current || bodyPinnedRef.current) return;
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

  // 面板的「输入聚焦」状态：聚焦期间暂停自动收起（用户正在打字）。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen<boolean>(BODY_PIN_EVENT, (event) => {
      if (!active) return;
      bodyPinnedRef.current = event.payload;
      if (event.payload) clearCollapseTimer();
      else scheduleCollapse();
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅面板聚焦状态事件失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [clearCollapseTimer, scheduleCollapse]);

  // 面板主动请求收起（Esc）：收起逻辑与兜底定时器仍由灰条窗口统一执行。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;

    void listen(BODY_DISMISS_EVENT, () => {
      if (active) closeBody();
    })
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch((cause) => {
        void logError(`订阅面板收起请求失败：${String(cause)}`);
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [closeBody]);

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
      body: openRef.current ? panelGeometry(bounds, nextEdge, barPos) : null,
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
        const body = panelGeometry(bounds, edge, barPos);
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
