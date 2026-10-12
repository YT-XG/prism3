import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type WheelEvent as ReactWheelEvent,
} from "react";

/** 累计滚动量达到该值才翻一页，避免触控板惯性误触发。 */
const SWITCH_THRESHOLD = 48;
/** 两次翻页的最小间隔（毫秒），与滑页动画时长接近。 */
const SWITCH_COOLDOWN = 320;

export interface UsePanelPagerOptions {
  /** 页面 id 列表，顺序即排序；定位与记忆都基于 id（增删页不影响既定位置）。 */
  ids: string[];
  /** 记忆当前页 id 的 localStorage 键。 */
  storageKey: string;
}

function clampIndex(value: number, count: number) {
  return Math.min(Math.max(value, 0), Math.max(0, count - 1));
}

/**
 * 面板分页状态：按 id 定位与记忆，加上滚轮/触控板手势翻页。
 * 与具体 UI 解耦，`PanelPager` 只负责渲染。
 */
export function usePanelPager({ ids, storageKey }: UsePanelPagerOptions) {
  const count = ids.length;

  const [index, setIndex] = useState(() => {
    try {
      const stored = window.localStorage.getItem(storageKey);
      const found = stored === null ? -1 : ids.indexOf(stored);
      return found === -1 ? 0 : found;
    } catch {
      return 0;
    }
  });

  // 记忆停留页，展开时直接回到原处（同一 id 不变，页面增删不漂移）。
  useEffect(() => {
    const id = ids[index];
    if (id !== undefined) {
      try {
        window.localStorage.setItem(storageKey, id);
      } catch {
        // 隐私模式等场景下写入失败，忽略即可。
      }
    }
  }, [ids, index, storageKey]);

  const goTo = useCallback((target: number) => setIndex(clampIndex(target, count)), [count]);

  const step = useCallback(
    (delta: 1 | -1) => setIndex((prev) => clampIndex(prev + delta, count)),
    [count],
  );

  const wheelAccumRef = useRef(0);
  const lastSwitchRef = useRef(0);

  /**
   * 滚轮/触控板翻页：横滑直接翻页；竖向滚动优先留给当前页内容，
   * 内容滚到边缘后才继续翻页，避免与页面内滚动冲突。
   */
  const handleWheel = useCallback(
    (event: ReactWheelEvent<HTMLElement>, scrollHost: HTMLElement | null) => {
      const { deltaX, deltaY, shiftKey } = event;
      const horizontal = Math.abs(deltaX) > Math.abs(deltaY) || shiftKey;
      const delta = horizontal ? deltaX || deltaY : deltaY;
      if (delta === 0) return;

      if (!horizontal && scrollHost) {
        const overflowing = scrollHost.scrollHeight - scrollHost.clientHeight > 1;
        const atTop = scrollHost.scrollTop <= 0;
        const atBottom =
          scrollHost.scrollTop + scrollHost.clientHeight >= scrollHost.scrollHeight - 1;
        const atEdge = !overflowing || (delta < 0 ? atTop : atBottom);
        if (!atEdge) {
          wheelAccumRef.current = 0;
          return;
        }
      }

      const now = performance.now();
      if (now - lastSwitchRef.current < SWITCH_COOLDOWN) return;

      wheelAccumRef.current += delta;
      if (Math.abs(wheelAccumRef.current) < SWITCH_THRESHOLD) return;

      const direction: 1 | -1 = wheelAccumRef.current > 0 ? 1 : -1;
      wheelAccumRef.current = 0;
      lastSwitchRef.current = now;
      step(direction);
    },
    [step],
  );

  return { index, goTo, handleWheel };
}