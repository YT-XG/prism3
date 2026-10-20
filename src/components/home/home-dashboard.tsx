import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type PointerEvent as ReactPointerEvent,
} from "react";

import {
  Activity,
  Check,
  ClipboardList,
  Clock,
  Bluetooth,
  LayoutGrid,
  PencilLine,
  Plus,
  RotateCcw,
  Search,
  type LucideIcon,
} from "lucide-react";
import { toast } from "sonner";

import { BluetoothCard } from "@/components/home/cards/bluetooth";
import { ClipboardCard } from "@/components/home/cards/clipboard";
import { ClockCard } from "@/components/home/cards/clock";
import { MonitorCard } from "@/components/home/cards/monitor";
import { SearchCard } from "@/components/home/cards/search";
import { ShortcutsCard } from "@/components/home/cards/shortcuts";
import { HomeCard } from "@/components/home/home-card";
import {
  HOME_COLS,
  HOME_GAP_PX,
  HOME_MIN_ROWS,
  HOME_MODULES,
  HOME_ROW_MIN_PX,
  clampHomePlacement,
  homeModuleDef,
  homePlacementOverlaps,
  type HomeModuleId,
  type HomePlacement,
} from "@/components/home/home-layout";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useHomeLayout } from "@/hooks/use-home-layout";
import { cn } from "@/lib/utils";

const MODULE_ICONS: Record<HomeModuleId, LucideIcon> = {
  clock: Clock,
  monitor: Activity,
  bluetooth: Bluetooth,
  clipboard: ClipboardList,
  search: Search,
  shortcuts: LayoutGrid,
};

const MODULE_BODIES: Record<HomeModuleId, ComponentType> = {
  clock: ClockCard,
  monitor: MonitorCard,
  bluetooth: BluetoothCard,
  clipboard: ClipboardCard,
  search: SearchCard,
  shortcuts: ShortcutsCard,
};

type DragMode = "move" | "resize";

interface DragState {
  mode: DragMode;
  id: HomeModuleId;
  pointerId: number;
  startX: number;
  startY: number;
  origin: HomePlacement;
  cellW: number;
  cellH: number;
}

/**
 * 首页 Bento 工作台。
 *
 * 布局模型在 `home-layout.ts`，持久化在 `use-home-layout.ts`；这里只负责：
 * 12 列网格渲染、编辑态拖拽 / 缩放、添加与移除模块、恢复默认。
 *
 * 拖拽期间不移动真实卡片，而是画一块虚线预览（与 x-hub 同一做法）：
 * 真实卡片只在松手时重排一次，避免每帧触发布局抖动。
 */
export function HomeDashboard() {
  const { placements, addModule, removeModule, moveModule, resizeModule, resetLayout } =
    useHomeLayout();

  const [editing, setEditing] = useState(false);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [preview, setPreview] = useState<HomePlacement | null>(null);
  const [previewBad, setPreviewBad] = useState(false);

  const gridRef = useRef<HTMLDivElement>(null);
  const placementsRef = useRef(placements);
  const previewRef = useRef<HomePlacement | null>(null);

  useEffect(() => {
    placementsRef.current = placements;
  }, [placements]);

  const rowCount = useMemo(() => {
    let rows = HOME_MIN_ROWS;
    for (const placement of placements) {
      rows = Math.max(rows, placement.y + placement.h);
    }
    if (preview) rows = Math.max(rows, preview.y + preview.h);
    return rows;
  }, [placements, preview]);

  const gridMetrics = useCallback(() => {    const element = gridRef.current;
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    const cellW = (rect.width - HOME_GAP_PX * (HOME_COLS - 1)) / HOME_COLS;
    const cellH = (rect.height - HOME_GAP_PX * (rowCount - 1)) / rowCount;
    return cellW > 4 && cellH > 4 ? { cellW, cellH } : null;
  }, [rowCount]);

  const beginDrag = (
    mode: DragMode,
    event: ReactPointerEvent<HTMLElement>,
    placement: HomePlacement,
  ) => {
    if (!editing || (event.pointerType === "mouse" && event.button !== 0)) return;
    event.preventDefault();
    event.stopPropagation();

    const metrics = gridMetrics();
    if (!metrics) return;

    event.currentTarget.setPointerCapture?.(event.pointerId);
    previewRef.current = placement;
    setPreview(placement);
    setPreviewBad(false);
    setDrag({
      mode,
      id: placement.id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      origin: placement,
      cellW: metrics.cellW,
      cellH: metrics.cellH,
    });
  };

  // 拖拽期间统一挂 window 监听：指针移出卡片 / 滚出网格也能收到事件。
  useEffect(() => {
    if (!drag) return;

    const previousUserSelect = document.body.style.userSelect;
    document.body.style.userSelect = "none";

    const handleMove = (event: PointerEvent) => {
      if (event.pointerId !== drag.pointerId) return;
      const def = homeModuleDef(drag.id);
      if (!def) return;

      const deltaX = Math.round(
        (event.clientX - drag.startX) / (drag.cellW + HOME_GAP_PX),
      );
      const deltaY = Math.round(
        (event.clientY - drag.startY) / (drag.cellH + HOME_GAP_PX),
      );

      const next =
        drag.mode === "move"
          ? clampHomePlacement(
              {
                ...drag.origin,
                x: drag.origin.x + deltaX,
                y: drag.origin.y + deltaY,
              },
              def,
            )
          : clampHomePlacement(
              {
                ...drag.origin,
                w: drag.origin.w + deltaX,
                h: drag.origin.h + deltaY,
              },
              def,
            );

      previewRef.current = next;
      setPreview(next);
      setPreviewBad(
        homePlacementOverlaps(next, placementsRef.current, drag.id),
      );
    };

    const finish = () => {
      const target = previewRef.current;
      if (target) {
        if (drag.mode === "move") {
          // 只是点了一下标题（没有移动）不写配置。
          if (target.x !== drag.origin.x || target.y !== drag.origin.y) {
            // 目标被占时由 hook 向下找空位；这里不阻止，松手总能落位。
            moveModule(drag.id, target.x, target.y);
          }
        } else if (
          (target.w !== drag.origin.w || target.h !== drag.origin.h) &&
          !homePlacementOverlaps(target, placementsRef.current, drag.id)
        ) {
          resizeModule(drag.id, target.w, target.h);
        }
      }
      previewRef.current = null;
      setDrag(null);
      setPreview(null);
      setPreviewBad(false);
    };

    window.addEventListener("pointermove", handleMove);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);

    return () => {
      document.body.style.userSelect = previousUserSelect;
      window.removeEventListener("pointermove", handleMove);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  }, [drag, moveModule, resizeModule]);

  const availableModules = HOME_MODULES.filter(
    (module) => !placements.some((placement) => placement.id === module.id),
  );

  const handleReset = () => {
    resetLayout();
    toast.success("已恢复默认布局");
  };

  return (
    <div className="flex h-full min-h-0 flex-col px-6 py-5 pb-4">
      <div className="mb-3 flex shrink-0 flex-wrap items-center gap-2">
        <div className="min-w-0">
          <h1 className="font-heading text-base font-semibold">首页</h1>
          <p className="text-[0.65rem] text-muted-foreground">
            {editing
              ? "拖动卡片标题移动 · 右下角改尺寸 · 改动即时保存"
              : "常用模块一览，布局会自动保存"}
          </p>
        </div>

        <div className="ml-auto flex items-center gap-1">
          {editing && (
            <>
              <AddModuleMenu
                available={availableModules}
                onAdd={(id) => {
                  if (addModule(id)) toast.success("已添加模块");
                }}
              />
              <Button variant="ghost" size="sm" onClick={handleReset}>
                <RotateCcw data-icon="inline-start" />
                恢复默认
              </Button>
            </>
          )}

          <Button
            variant={editing ? "default" : "outline"}
            size="sm"
            onClick={() => setEditing((value) => !value)}
          >
            {editing ? (
              <>
                <Check data-icon="inline-start" />
                完成
              </>
            ) : (
              <>
                <PencilLine data-icon="inline-start" />
                编辑布局
              </>
            )}
          </Button>
        </div>
      </div>

      <div
        className={cn(
          "min-h-0 flex-1",
          placements.length === 0 ? "overflow-hidden" : "overflow-y-auto",
        )}
      >
        {placements.length === 0 ? (
          <div className="flex h-full min-h-64 flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border/70 text-center">
            <LayoutGrid className="size-7 text-muted-foreground" aria-hidden="true" />
            <div>
              <p className="text-sm font-medium">工作台是空的</p>
              <p className="mt-1 text-xs text-muted-foreground">
                从模块库添加卡片，或恢复默认布局。
              </p>
            </div>
            <div className="flex items-center gap-2">
              {editing ? (
                <AddModuleMenu
                  available={availableModules}
                  onAdd={(id) => {
                    if (addModule(id)) toast.success("已添加模块");
                  }}
                />
              ) : (
                <Button size="sm" onClick={() => setEditing(true)}>
                  <Plus data-icon="inline-start" />
                  编辑布局
                </Button>
              )}
              <Button variant="outline" size="sm" onClick={handleReset}>
                恢复默认
              </Button>
            </div>
          </div>
        ) : (
          <div
            ref={gridRef}
            className={cn(
              "relative grid h-full grid-cols-12 gap-4",
              drag && "cursor-grabbing",
            )}
            style={{
              gridTemplateRows: `repeat(${rowCount}, minmax(${HOME_ROW_MIN_PX}px, 1fr))`,
              minHeight:
                rowCount * HOME_ROW_MIN_PX + (rowCount - 1) * HOME_GAP_PX,
            }}
          >
            {placements.map((placement) => {
              const def = homeModuleDef(placement.id);
              const Icon = MODULE_ICONS[placement.id];
              const Body = MODULE_BODIES[placement.id];
              if (!def) return null;

              return (
                <HomeCard
                  key={placement.id}
                  placement={placement}
                  icon={Icon}
                  title={def.title}
                  editing={editing}
                  dragging={preview?.id === placement.id}
                  onRemove={() => removeModule(placement.id)}
                  onDragStart={(event) => beginDrag("move", event, placement)}
                  onResizeStart={(event) => beginDrag("resize", event, placement)}
                >
                  <Body />
                </HomeCard>
              );
            })}

            {preview && (
              <div
                aria-hidden="true"
                className={cn(
                  "pointer-events-none z-20 rounded-xl border-2 border-dashed",
                  previewBad
                    ? "border-destructive/70 bg-destructive/5"
                    : "border-brand/70 bg-brand-soft",
                )}
                style={{
                  gridColumn: `${preview.x + 1} / span ${preview.w}`,
                  gridRow: `${preview.y + 1} / span ${preview.h}`,
                }}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function AddModuleMenu({
  available,
  onAdd,
}: {
  available: typeof HOME_MODULES;
  onAdd: (id: HomeModuleId) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" disabled={available.length === 0}>
          <Plus data-icon="inline-start" />
          添加模块
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-44">
        {available.map((module) => {
          const Icon = MODULE_ICONS[module.id];
          return (
            <DropdownMenuItem
              key={module.id}
              onSelect={() => onAdd(module.id)}
            >
              <Icon aria-hidden="true" />
              {module.title}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
