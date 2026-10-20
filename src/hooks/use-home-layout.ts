import { useCallback, useEffect, useRef, useState } from "react";

import { toast } from "sonner";

import {
  clampHomePlacement,
  defaultHomeLayout,
  findFreeHomeSpot,
  homeModuleDef,
  homePlacementOverlaps,
  parseHomeLayout,
  serializeHomeLayout,
  type HomeModuleId,
  type HomePlacement,
} from "@/components/home/home-layout";
import { useAppStore } from "@/stores/app-store";

/**
 * 首页 Bento 布局的状态与持久化。
 *
 * 布局存在 `AppConfig.homeLayout`（JSON 字符串），由 Rust 侧随手写盘：
 * - 配置加载完成 / 设置页「恢复默认」把 `homeLayout` 清空时，这里重新解析；
 * - 拖拽 / 改尺寸只在**松手时**提交一次，避免每帧写盘；
 * - 写入用整份 `saveConfig`（与现有配置项同一路径），失败会提示；配置随后
 *   回退到上一次成功写入的值，布局也跟着回到那份（界面与磁盘保持一致）。
 */
export function useHomeLayout() {
  const config = useAppStore((state) => state.config);
  const saveConfig = useAppStore((state) => state.saveConfig);

  const [placements, setPlacements] = useState<HomePlacement[]>(defaultHomeLayout);

  /** 最新布局的同步副本：事件回调（拖拽松手）里读它，不依赖闭包里的旧 state。 */
  const placementsRef = useRef(placements);
  /** 已经应用过的配置原文：避免「本地保存 → 配置回声」把布局再解析一遍。 */
  const appliedRawRef = useRef<string | null>(null);

  useEffect(() => {
    placementsRef.current = placements;
  }, [placements]);

  useEffect(() => {
    if (!config) return;
    const raw = config.homeLayout;
    if (appliedRawRef.current === raw) return;
    appliedRawRef.current = raw;

    // 自己刚写进去的值（或内容等价）不需要重新解析，保持引用稳定。
    if (serializeHomeLayout(placementsRef.current) === raw) return;

    // `null` 同时覆盖「空串」与「损坏数据」，都回退到默认布局。
    setPlacements(parseHomeLayout(raw) ?? defaultHomeLayout());
  }, [config]);

  const persist = useCallback(
    (next: HomePlacement[]) => {
      setPlacements(next);
      placementsRef.current = next;

      const raw = serializeHomeLayout(next);
      appliedRawRef.current = raw;
      void saveConfig({ homeLayout: raw }).catch((cause) => {
        toast.error(`保存首页布局失败：${String(cause)}`);
      });
    },
    [saveConfig],
  );

  const addModule = useCallback(
    (id: HomeModuleId): boolean => {
      const def = homeModuleDef(id);
      const current = placementsRef.current;
      if (!def || current.some((placement) => placement.id === id)) return false;

      let maxY = 0;
      for (const placement of current) {
        maxY = Math.max(maxY, placement.y + placement.h);
      }
      const spot = findFreeHomeSpot(current, def.idealW, def.idealH, 0, maxY);
      persist([
        ...current,
        { id, x: spot.x, y: spot.y, w: def.idealW, h: def.idealH },
      ]);
      return true;
    },
    [persist],
  );

  const removeModule = useCallback(
    (id: HomeModuleId) => {
      persist(placementsRef.current.filter((placement) => placement.id !== id));
    },
    [persist],
  );

  /** 移动：目标被占时向下找最近空位（与新增一致，不弹回）。 */
  const moveModule = useCallback(
    (id: HomeModuleId, x: number, y: number) => {
      const current = placementsRef.current;
      const placement = current.find((item) => item.id === id);
      const def = homeModuleDef(id);
      if (!placement || !def) return;

      const target = clampHomePlacement({ ...placement, x, y }, def);
      const others = current.filter((item) => item.id !== id);
      const spot = homePlacementOverlaps(target, others)
        ? findFreeHomeSpot(others, placement.w, placement.h, target.x, target.y)
        : { x: target.x, y: target.y };

      persist(
        current.map((item) =>
          item.id === id ? { ...item, x: spot.x, y: spot.y } : item,
        ),
      );
    },
    [persist],
  );

  /** 改尺寸：撞到其他卡片时拒绝（返回 false），由调用方保留原尺寸。 */
  const resizeModule = useCallback(
    (id: HomeModuleId, w: number, h: number): boolean => {
      const current = placementsRef.current;
      const placement = current.find((item) => item.id === id);
      const def = homeModuleDef(id);
      if (!placement || !def) return false;

      const target = clampHomePlacement({ ...placement, w, h }, def);
      const others = current.filter((item) => item.id !== id);
      if (homePlacementOverlaps(target, others)) return false;

      persist(
        current.map((item) =>
          item.id === id
            ? { ...item, x: target.x, y: target.y, w: target.w, h: target.h }
            : item,
        ),
      );
      return true;
    },
    [persist],
  );

  const resetLayout = useCallback(() => {
    persist(defaultHomeLayout());
  }, [persist]);

  return {
    placements,
    addModule,
    removeModule,
    moveModule,
    resizeModule,
    resetLayout,
  };
}
