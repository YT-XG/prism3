import type { LucideIcon } from "lucide-react";
import { useMemo } from "react";

import {
  APPS,
  appDef,
  customIdOf,
  customSize,
  customWindowId,
  DOCK_ORDER,
  isCustomWindowId,
  type AppDef,
  type CustomApp,
} from "@/lib/apps";
import { useAppStore } from "@/stores/app-store";

/** 外壳与窗口共用的应用元信息：内置应用与自建应用在界面上没有区别。 */
export interface AppMeta {
  /** 窗口 id：内置是 `AppView`，自建是 `custom:<短 id>`。 */
  id: string;
  label: string;
  title: string;
  description: string;
  tint: [string, string];
  icon?: LucideIcon;
  /** 自建应用用 emoji 当图标。 */
  emoji?: string;
  kind: "builtin" | "link" | "html";
  size: { w: number; h: number };
  /** 允许缩到的最小尺寸。 */
  min: { w: number; h: number };
}

function fromBuiltin(def: AppDef): AppMeta {
  return {
    id: def.id,
    label: def.label,
    title: def.title,
    description: def.description,
    tint: def.tint,
    icon: def.icon,
    kind: "builtin",
    size: def.size,
    min: def.min,
  };
}

function fromCustom(app: CustomApp): AppMeta {
  return {
    id: customWindowId(app.id),
    label: app.name,
    title: app.name,
    description: app.kind === "link" ? app.url : "自定义 HTML",
    tint: app.tint,
    emoji: app.icon,
    kind: app.kind,
    size: customSize(app.kind),
    min: { w: 420, h: 320 },
  };
}

/** 配置里的自建应用（未加载配置时是空数组，别在这里造默认值）。 */
export function useCustomApps(): CustomApp[] {
  const list = useAppStore((state) => state.config?.customApps);
  return useMemo(() => list ?? [], [list]);
}

/**
 * 一个窗口 id 对应的应用元信息。
 *
 * 自建应用被删掉后，开着的窗口仍要能画出来（不能因为查不到就抛错），
 * 所以这里给一个「已删除的应用」占位，标题栏与任务栏都还能显示。
 */
export function useAppMeta(id: string): AppMeta {
  const custom = useCustomApps();

  return useMemo(() => {
    if (isCustomWindowId(id)) {
      const found = custom.find((item) => item.id === customIdOf(id));
      if (found) return fromCustom(found);
      return {
        id,
        label: "已删除的应用",
        title: "已删除的应用",
        description: "这个自建应用已被删除",
        tint: ["#94a3b8", "#5b6675"] as [string, string],
        emoji: "🗑",
        kind: "html" as const,
        min: { w: 420, h: 320 },
        size: customSize("html"),
      };
    }

    return fromBuiltin(appDef(id as AppDef["id"]));
  }, [custom, id]);
}

/** 桌面图标坞 / 开始菜单 / 桌面搜索共用的完整应用列表：内置（按坞顺序）+ 自建。 */
export function useAppCatalog(): AppMeta[] {
  const custom = useCustomApps();

  return useMemo(() => {
    const builtins = DOCK_ORDER.map((id) => fromBuiltin(appDef(id)));
    return [...builtins, ...custom.map(fromCustom)];
  }, [custom]);
}

/** 兼容用：不是自建 id 时返回 null，调用方据此决定要不要渲染自定义内容。 */
export function useCustomAppById(id: string): CustomApp | null {
  const custom = useCustomApps();
  return useMemo(
    () => (isCustomWindowId(id) ? custom.find((item) => item.id === customIdOf(id)) ?? null : null),
    [custom, id],
  );
}

export { APPS };
