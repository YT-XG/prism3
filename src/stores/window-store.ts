import { create } from "zustand";

import { appDef } from "@/lib/apps";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface AppWindowState {
  /** 窗口 id：内置应用是 `AppView`，自建应用是 `custom:<短 id>`。 */
  id: string;
  rect: Rect;
  /** 层级：越大越靠前。 */
  z: number;
  minimized: boolean;
  maximized: boolean;
  /** 最大化之前的矩形，用于还原。 */
  restore: Rect | null;
}

/** 顶部条与任务栏的高度必须与 `index.css` 的 `--topbar-h` / `--taskbar-h` 一致。 */
const TOPBAR_H = 34;
const TASKBAR_H = 56;
const MARGIN = 10;

/**
 * 窗口可摆放的区域。坐标是**相对窗口层**的（窗口层本身已经让开顶栏与任务栏），
 * 所以 y 从 10 起算，高度是「桌面高度 - 上下边距」。
 */
export function desktopBounds(): Rect {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const desktopH = vh - TOPBAR_H - TASKBAR_H;
  return {
    x: MARGIN,
    y: MARGIN,
    w: Math.max(480, vw - MARGIN * 2),
    h: Math.max(360, desktopH - MARGIN * 2),
  };
}

/** 最大化：铺满整个桌面层（不保留 10px 边距），与「还原」有明显差别。 */
export function fullBounds(): Rect {
  return {
    x: 0,
    y: 0,
    w: window.innerWidth,
    h: window.innerHeight - TOPBAR_H - TASKBAR_H,
  };
}

function clampRect(rect: Rect, bounds = desktopBounds()): Rect {
  const w = Math.min(Math.max(rect.w, 320), bounds.w);
  const h = Math.min(Math.max(rect.h, 240), bounds.h);
  return {
    w,
    h,
    x: Math.min(Math.max(rect.x, bounds.x), bounds.x + bounds.w - w),
    y: Math.min(Math.max(rect.y, bounds.y), bounds.y + bounds.h - h),
  };
}

/** 居中 + 级联：同一批窗口依次错开 26px，避免完全叠死。 */
function defaultRect(
  size: { w: number; h: number },
  openCount: number,
): Rect {
  const bounds = desktopBounds();
  const w = Math.min(size.w, bounds.w);
  const h = Math.min(size.h, bounds.h);
  const step = (openCount % 5) * 26;
  return clampRect(
    {
      w,
      h,
      x: bounds.x + Math.max(0, (bounds.w - w) / 2) + step,
      y: bounds.y + Math.max(0, (bounds.h - h) / 2) + step,
    },
    bounds,
  );
}

/** 内置应用从 `appDef` 拿默认尺寸；自建应用必须由调用方传 `size`。 */
function buildinSize(id: string): { w: number; h: number } {
  try {
    return appDef(id as Parameters<typeof appDef>[0]).size;
  } catch {
    return { w: 720, h: 520 };
  }
}

/** 把某个窗口提到最前，并把 z 重新编号成 1..n（避免 z 无限增长盖过任务栏）。 */
function raise(list: AppWindowState[], id: string): AppWindowState[] {
  const sorted = [...list].sort((a, b) => a.z - b.z);
  const target = sorted.find((item) => item.id === id);
  if (!target) return list;
  return [...sorted.filter((item) => item.id !== id), target].map((item, index) => ({
    ...item,
    z: index + 1,
  }));
}

interface WindowStore {
  windows: AppWindowState[];
  focused: string | null;
  /** 打开（或聚焦）一个应用窗口；`size` 只在首次打开时用于定初始矩形。 */
  open: (id: string, size?: { w: number; h: number }) => void;
  close: (id: string) => void;
  focus: (id: string) => void;
  minimize: (id: string) => void;
  toggleMaximize: (id: string) => void;
  /** 拖动 / 缩放结束时提交几何。 */
  setRect: (id: string, rect: Rect) => void;
  /** 视口变化后把窗口收回桌面范围。 */
  clampAll: () => void;
  isOpen: (id: string) => boolean;
}

/**
 * 浮层窗口管理器（纯前端状态，不落盘）。
 *
 * 与 `app-store.activeView` 的分工：`activeView` 是「用户想看的那个应用」，
 * 这里管「哪些窗口开着、谁在最前」。两者由 `DesktopShell` 单向同步，
 * 这样组合窗口 / 快捷小窗的 `main:navigate` 仍然只需要写 `activeView`。
 */
export const useWindowStore = create<WindowStore>((set, get) => ({
  windows: [],
  focused: null,

  open: (id, size) => {
    const { windows } = get();
    const existing = windows.find((item) => item.id === id);

    if (existing) {
      set({
        focused: id,
        windows: raise(
          windows.map((item) =>
            item.id === id ? { ...item, minimized: false } : item,
          ),
          id,
        ),
      });
      return;
    }

    set({
      focused: id,
      windows: [
        ...windows.map((item, index) => ({ ...item, z: index + 1 })),
        {
          id,
          rect: defaultRect(
            size ?? buildinSize(id),
            windows.length,
          ),
          z: windows.length + 1,
          minimized: false,
          maximized: false,
          restore: null,
        },
      ],
    });
  },

  close: (id) => {
    const remaining = get().windows.filter((item) => item.id !== id);
    const nextFocus =
      [...remaining].filter((item) => !item.minimized).sort((a, b) => b.z - a.z)[0]?.id ?? null;
    set({ windows: remaining, focused: nextFocus });
  },

  focus: (id) => {
    const { windows } = get();
    const target = windows.find((item) => item.id === id);
    if (!target) return;
    if (get().focused === id && !target.minimized) return;
    set({
      focused: id,
      windows: raise(
        windows.map((item) => (item.id === id ? { ...item, minimized: false } : item)),
        id,
      ),
    });
  },

  minimize: (id) => {
    const { windows } = get();
    const remaining = windows.filter((item) => item.id !== id && !item.minimized);
    set({
      focused: remaining.sort((a, b) => b.z - a.z)[0]?.id ?? null,
      windows: windows.map((item) => (item.id === id ? { ...item, minimized: true } : item)),
    });
  },

  toggleMaximize: (id) => {
    const bounds = desktopBounds();
    const full = fullBounds();
    set({
      windows: get().windows.map((item) => {
        if (item.id !== id) return item;
        if (item.maximized) {
          return {
            ...item,
            maximized: false,
            rect: clampRect(item.restore ?? item.rect, bounds),
            restore: null,
          };
        }
        return { ...item, maximized: true, restore: item.rect, rect: full };
      }),
    });
  },

  setRect: (id, rect) => {
    set({
      windows: get().windows.map((item) =>
        item.id === id ? { ...item, rect: clampRect(rect), maximized: false } : item,
      ),
    });
  },

  clampAll: () => {
    set({
      windows: get().windows.map((item) =>
        item.maximized
          ? { ...item, rect: desktopBounds() }
          : { ...item, rect: clampRect(item.rect) },
      ),
    });
  },

  isOpen: (id) => get().windows.some((item) => item.id === id),
}));
