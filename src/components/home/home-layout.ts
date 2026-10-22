/**
 * 首页 Bento 布局的纯数据层：模块目录、默认摆位、配置解析与碰撞计算。
 *
 * 与 `x-hub` 的 12 列自由网格同一套语义：
 * - 坐标是整数格（`x` 从 0 起、宽 ≤ 12；`y` 从 0 起），不是像素；
 * - 每个模块声明最小尺寸，摆位永远钳制到最小尺寸以上；
 * - 目标格被占用时向下找最近空位（移动 / 新增都不会「弹回」原来的位置）；
 * - 配置里损坏 / 已删除的模块条目直接忽略，整份失效时回退默认布局。
 *
 * 这一层不碰 React、不写配置，便于单独推演和复用；持久化在
 * `hooks/use-home-layout.ts`，交互在 `home-dashboard.tsx`。
 */

/** 网格列数（固定 12 列，行高由渲染层按可用高度均分）。 */
export const HOME_COLS = 12;

/** 画布最少行数：窗口再矮也保留 8 行，避免卡片被压成一条。 */
export const HOME_MIN_ROWS = 8;

/** 行高下限（px）：可用高度不够时页面改为纵向滚动，而不是继续压扁卡片。 */
export const HOME_ROW_MIN_PX = 64;

/** 网格间距（px），必须与渲染层的 `gap-4` 一致（拖拽换算用）。 */
export const HOME_GAP_PX = 16;

export type HomeModuleId =
  | "clock"
  | "monitor"
  | "search"
  | "clipboard"
  | "notes"
  | "bluetooth"
  | "projects"
  | "shortcuts";

export interface HomeModuleDef {
  id: HomeModuleId;
  title: string;
  /** 内容完整展示的最小格子。 */
  minW: number;
  minH: number;
  /** 内容正好铺满的推荐格子（默认布局 / 新增模块按它落位）。 */
  idealW: number;
  idealH: number;
}

export interface HomePlacement {
  id: HomeModuleId;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** 首页可放置的模块目录。新增模块时这里、卡片渲染与 `MODULE_ICONS` 三处一起加。 */
export const HOME_MODULES: HomeModuleDef[] = [
  { id: "clock", title: "时钟", minW: 3, minH: 2, idealW: 4, idealH: 3 },
  { id: "monitor", title: "系统资源", minW: 3, minH: 2, idealW: 4, idealH: 3 },
  { id: "bluetooth", title: "蓝牙耳机", minW: 3, minH: 2, idealW: 4, idealH: 3 },
  { id: "clipboard", title: "剪贴板", minW: 3, minH: 3, idealW: 4, idealH: 5 },
  { id: "notes", title: "速记", minW: 3, minH: 3, idealW: 4, idealH: 5 },
  { id: "search", title: "快速搜索", minW: 3, minH: 3, idealW: 4, idealH: 5 },
  { id: "projects", title: "项目", minW: 3, minH: 3, idealW: 4, idealH: 5 },
  { id: "shortcuts", title: "常用入口", minW: 2, minH: 2, idealW: 4, idealH: 5 },
];

const MODULE_BY_ID = new Map<string, HomeModuleDef>(
  HOME_MODULES.map((module) => [module.id, module]),
);

/** 默认摆位：上行三张 4×3，下行三张 4×5，共 8 行（正好一个屏幕）。 */
export const HOME_DEFAULT_LAYOUT: HomePlacement[] = [
  { id: "clock", x: 0, y: 0, w: 4, h: 3 },
  { id: "monitor", x: 4, y: 0, w: 4, h: 3 },
  { id: "bluetooth", x: 8, y: 0, w: 4, h: 3 },
  { id: "clipboard", x: 0, y: 3, w: 4, h: 5 },
  { id: "search", x: 4, y: 3, w: 4, h: 5 },
  { id: "shortcuts", x: 8, y: 3, w: 4, h: 5 },
];

export function homeModuleDef(id: string): HomeModuleDef | undefined {
  return MODULE_BY_ID.get(id);
}

export function homeModuleTitle(id: HomeModuleId): string {
  return MODULE_BY_ID.get(id)?.title ?? id;
}

export function defaultHomeLayout(): HomePlacement[] {
  return HOME_DEFAULT_LAYOUT.map((placement) => ({ ...placement }));
}

/** 两个格子是否重叠（同模块也按重叠算，由调用方用 `ignoreId` 排除自身）。 */
export function homePlacementsCollide(a: HomePlacement, b: HomePlacement): boolean {
  return (
    a.x < b.x + b.w &&
    a.x + a.w > b.x &&
    a.y < b.y + b.h &&
    a.y + a.h > b.y
  );
}

/** 目标格是否与除 `ignoreId` 之外的任意模块重叠。 */
export function homePlacementOverlaps(
  rect: HomePlacement,
  placements: HomePlacement[],
  ignoreId?: HomeModuleId,
): boolean {
  return placements.some(
    (other) => other.id !== ignoreId && homePlacementsCollide(rect, other),
  );
}

/**
 * 在 `x` 所在的列带内，从 `y` 向下找第一个能容纳 `w×h` 的空位。
 * 找不到时落在布局最下方（宁可滚动，也不把卡片弹回原处）。
 */
export function findFreeHomeSpot(
  placements: HomePlacement[],
  w: number,
  h: number,
  x: number,
  y: number,
): { x: number; y: number } {
  const cx = Math.min(Math.max(Math.round(x), 0), HOME_COLS - w);
  let maxY = 0;
  for (const placement of placements) {
    maxY = Math.max(maxY, placement.y + placement.h);
  }
  const startY = Math.max(Math.round(y), 0);
  const limit = maxY + h;

  for (let yy = startY; yy <= limit; yy += 1) {
    const rect: HomePlacement = { id: "clock", x: cx, y: yy, w, h };
    if (!homePlacementOverlaps(rect, placements)) return { x: cx, y: yy };
  }
  return { x: cx, y: startY };
}

/** 把任意矩形钳制进网格与模块最小尺寸。 */
export function clampHomePlacement(
  placement: HomePlacement,
  def: HomeModuleDef,
): HomePlacement {
  const w = Math.min(Math.max(Math.round(placement.w), def.minW), HOME_COLS);
  const h = Math.max(Math.round(placement.h), def.minH);
  const x = Math.min(Math.max(Math.round(placement.x), 0), HOME_COLS - w);
  const y = Math.max(Math.round(placement.y), 0);
  return { id: placement.id, x, y, w, h };
}

function toInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? Math.round(value) : null;
}

/**
 * 解析配置里的 JSON（`AppConfig.homeLayout`）。
 *
 * - 空串 / 非数组 / JSON 损坏 → `null`（调用方回退默认布局）；
 * - `[]` → `[]`（用户主动清空了工作台，这是合法状态，不能当成损坏）；
 * - 未知模块、重复模块、非整数坐标 → 忽略对应条目；
 * - 尺寸钳制到最小尺寸，重叠条目按 y/x 排序后向下让位。
 */
export function parseHomeLayout(raw: string | null | undefined): HomePlacement[] | null {
  if (!raw || !raw.trim()) return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!Array.isArray(parsed)) return null;
  if (parsed.length === 0) return [];

  const seen = new Set<HomeModuleId>();
  const accepted: HomePlacement[] = [];

  for (const entry of parsed) {
    if (!entry || typeof entry !== "object") continue;
    const record = entry as Record<string, unknown>;
    const def = typeof record.id === "string" ? homeModuleDef(record.id) : undefined;
    if (!def || seen.has(def.id)) continue;

    const x = toInteger(record.x);
    const y = toInteger(record.y);
    if (x === null || y === null) continue;

    const w = toInteger(record.w) ?? def.idealW;
    const h = toInteger(record.h) ?? def.idealH;

    seen.add(def.id);
    accepted.push(clampHomePlacement({ id: def.id, x, y, w, h }, def));
  }

  if (accepted.length === 0) return null;

  // 尺寸钳制可能放大老数据，逐项消解重叠：从上往下扫，撞到已确认项就向下让位。
  const settled: HomePlacement[] = [];
  for (const placement of [...accepted].sort((a, b) => a.y - b.y || a.x - b.x)) {
    if (!homePlacementOverlaps(placement, settled)) {
      settled.push(placement);
      continue;
    }
    const spot = findFreeHomeSpot(settled, placement.w, placement.h, placement.x, placement.y);
    settled.push({ ...placement, x: spot.x, y: spot.y });
  }
  return settled;
}

/** 序列化为配置字符串（字段顺序固定，便于人工排查与比较）。 */
export function serializeHomeLayout(placements: HomePlacement[]): string {
  return JSON.stringify(
    placements.map(({ id, x, y, w, h }) => ({ id, x, y, w, h })),
  );
}
