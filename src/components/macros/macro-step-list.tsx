import { useState, type DragEvent } from "react";

import {
  GripVertical,
  Keyboard,
  MousePointerClick,
  TextCursorInput,
  Timer,
  Trash2,
  type LucideIcon,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import type { MacroAction, MacroMouseButton, MacroStep } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** 拖拽数据类型：从动作面板拖进列表（新增）与拖动已有步骤（排序）用不同的 type。 */
const DRAG_ACTION = "application/x-prism3-macro-action";
const DRAG_STEP = "application/x-prism3-macro-step";

/** 动作面板提供一次性动作；按住类会一次性插入「按下 + 松开」两步。 */
type PaletteKind =
  | "mouseClick"
  | "mouseHold"
  | "key"
  | "keyHold"
  | "text"
  | "wait";

const PALETTE: {
  kind: PaletteKind;
  label: string;
  hint: string;
  icon: LucideIcon;
}[] = [
  { kind: "mouseClick", label: "鼠标点击", hint: "拖入后填坐标", icon: MousePointerClick },
  { kind: "mouseHold", label: "按住鼠标", hint: "插入按下 + 松开两步", icon: MousePointerClick },
  { kind: "key", label: "按键", hint: "Ctrl+S / Enter", icon: Keyboard },
  { kind: "keyHold", label: "按住按键", hint: "插入按下 + 松开两步", icon: Keyboard },
  { kind: "text", label: "文本", hint: "输入一段文字", icon: TextCursorInput },
  { kind: "wait", label: "等待", hint: "暂停若干毫秒", icon: Timer },
];

/** 新建一步动作时的默认值（坐标 / 文本 / 按住时长由用户再编辑）。 */
export function createMacroStep(kind: PaletteKind): MacroStep[] {
  switch (kind) {
    case "mouseClick":
      return [
        {
          delayMs: 0,
          action: { type: "mouseClick", x: 0, y: 0, button: "left" },
        },
      ];
    case "mouseHold":
      return [
        {
          delayMs: 0,
          action: { type: "mouseDown", x: 0, y: 0, button: "left" },
        },
        {
          delayMs: 1000,
          action: { type: "mouseUp", button: "left" },
        },
      ];
    case "key":
      return [{ delayMs: 0, action: { type: "key", combo: "Enter" } }];
    case "keyHold":
      return [
        { delayMs: 0, action: { type: "keyDown", key: "S" } },
        { delayMs: 1000, action: { type: "keyUp", key: "S" } },
      ];
    case "text":
      return [{ delayMs: 0, action: { type: "text", text: "" } }];
    case "wait":
      return [{ delayMs: 0, action: { type: "wait", ms: 500 } }];
  }
}

interface MacroStepsEditorProps {
  steps: MacroStep[];
  onChange: (steps: MacroStep[]) => void;
  /** 录制 / 回放期间禁止改步骤，避免边跑边改。 */
  disabled?: boolean;
}

/**
 * 宏步骤编辑器：顶部是按动作类型划分的「积木」，拖进下方列表新增一步；
 * 列表里的步骤也能拖动调整顺序。点击积木是拖拽的无障碍兜底（直接追加到末尾）。
 */
export function MacroStepsEditor({ steps, onChange, disabled = false }: MacroStepsEditorProps) {
  const [dropIndex, setDropIndex] = useState<number | null>(null);

  const replaceStep = (index: number, patch: Partial<MacroStep>) => {
    onChange(steps.map((step, i) => (i === index ? { ...step, ...patch } : step)));
  };

  const updateAction = (index: number, action: MacroAction) => {
    const step = steps[index];
    if (step) replaceStep(index, { action });
  };

  const removeStep = (index: number) => {
    onChange(steps.filter((_, i) => i !== index));
  };

  const moveStep = (from: number, to: number) => {
    if (from === to) return;
    const next = [...steps];
    const [moved] = next.splice(from, 1);
    if (!moved) return;
    next.splice(from < to ? to - 1 : to, 0, moved);
    onChange(next);
  };

  const insertStep = (kind: PaletteKind, index: number) => {
    const next = [...steps];
    next.splice(
      Math.max(0, Math.min(index, next.length)),
      0,
      ...createMacroStep(kind),
    );
    onChange(next);
  };

  const acceptsDrag = (event: DragEvent) =>
    event.dataTransfer.types.includes(DRAG_ACTION) ||
    event.dataTransfer.types.includes(DRAG_STEP);

  const handleDrop = (event: DragEvent) => {
    if (!acceptsDrag(event)) return;
    event.preventDefault();
    const target = dropIndex ?? steps.length;
    const stepIndex = event.dataTransfer.getData(DRAG_STEP);
    const kind = event.dataTransfer.getData(DRAG_ACTION) as PaletteKind;

    if (stepIndex !== "") {
      moveStep(Number(stepIndex), target);
    } else if (kind) {
      insertStep(kind, target);
    }
    setDropIndex(null);
  };

  /** 行内拖拽时按鼠标在行的上 / 下半区决定插在它前面还是后面。 */
  const handleRowDragOver = (index: number, event: DragEvent) => {
    if (!acceptsDrag(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    const rect = event.currentTarget.getBoundingClientRect();
    const before = event.clientY < rect.top + rect.height / 2;
    setDropIndex(index + (before ? 0 : 1));
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="mr-0.5 text-[0.7rem] text-muted-foreground">拖入积木：</span>
        {PALETTE.map(({ kind, label, hint, icon: Icon }) => (
          <button
            key={kind}
            type="button"
            draggable={!disabled}
            title={`${hint}（拖到步骤列表的指定位置）`}
            onDragStart={(event) => {
              event.dataTransfer.setData(DRAG_ACTION, kind);
              event.dataTransfer.effectAllowed = "copy";
            }}
            onDragEnd={() => setDropIndex(null)}
            onClick={() => !disabled && insertStep(kind, steps.length)}
            className={cn(
              "flex cursor-grab items-center gap-1.5 rounded-lg border border-border/60 bg-muted/30 px-2 py-1 text-xs transition-colors select-none hover:bg-muted",
              disabled && "pointer-events-none opacity-50",
            )}
          >
            <Icon className="size-3.5 shrink-0 text-brand" aria-hidden="true" />
            <span>{label}</span>
          </button>
        ))}
        <span className="ml-auto text-[0.65rem] text-muted-foreground">
          拖动步骤左侧手柄可调整顺序
        </span>
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto rounded-xl border border-border/60 bg-background/40 p-1.5"
        onDragOver={(event) => {
          if (!acceptsDrag(event)) return;
          event.preventDefault();
          if (steps.length === 0) setDropIndex(0);
        }}
        onDrop={handleDrop}
        onDragLeave={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
            setDropIndex(null);
          }
        }}
      >
        {steps.length === 0 ? (
          <div className="flex h-full min-h-32 flex-col items-center justify-center gap-1.5 px-6 text-center">
            <p className="text-sm font-medium">把上面的积木拖到这里</p>
            <p className="text-xs text-muted-foreground">
              也可以先「开始录制」，录完再拖动调整顺序；点积木也会追加一步。
            </p>
          </div>
        ) : (
          <div className="flex flex-col gap-1.5">
            {steps.map((step, index) => (
              <div key={index}>
                {dropIndex === index && (
                  <div className="mb-1 h-0.5 rounded-full bg-brand" aria-hidden="true" />
                )}
                <div
                  onDragOver={(event) => handleRowDragOver(index, event)}
                  className="flex items-start gap-2 rounded-lg border border-border/50 bg-muted/20 px-2 py-1.5"
                >
                  <span
                    draggable={!disabled}
                    title="拖动调整顺序"
                    onDragStart={(event) => {
                      event.dataTransfer.setData(DRAG_STEP, String(index));
                      event.dataTransfer.effectAllowed = "move";
                    }}
                    onDragEnd={() => setDropIndex(null)}
                    className={cn(
                      "mt-0.5 flex cursor-grab items-center gap-1 text-muted-foreground select-none",
                      disabled && "pointer-events-none opacity-50",
                    )}
                  >
                    <GripVertical className="size-3.5" aria-hidden="true" />
                    <span className="w-4 text-center text-[0.65rem] tabular-nums">
                      {index + 1}
                    </span>
                  </span>

                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <ActionFields
                      action={step.action}
                      disabled={disabled}
                      onChange={(action) => updateAction(index, action)}
                    />
                    {step.action.type !== "wait" && (
                      <div className="flex items-center gap-1.5 text-[0.68rem] text-muted-foreground">
                        <span>
                          {step.action.type === "keyUp" || step.action.type === "mouseUp"
                            ? "按住"
                            : "执行前等待"}
                        </span>
                        <Input
                          type="number"
                          min={0}
                          step={50}
                          value={step.delayMs}
                          disabled={disabled}
                          title="执行这一步之前等待多久；松开步骤里是松开前的等待，简单的长按即按住时长"
                          aria-label={`第 ${index + 1} 步执行前的等待毫秒数`}
                          className="h-6 w-20 px-1.5 text-[0.68rem] tabular-nums"
                          onChange={(event) =>
                            replaceStep(index, {
                              delayMs: Math.max(0, Number(event.target.value) || 0),
                            })
                          }
                        />
                        <span>ms</span>
                      </div>
                    )}
                  </div>

                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    disabled={disabled}
                    title="删除这一步"
                    aria-label={`删除第 ${index + 1} 步`}
                    onClick={() => removeStep(index)}
                  >
                    <Trash2 />
                  </Button>
                </div>
              </div>
            ))}
            {dropIndex === steps.length && (
              <div className="mt-0.5 h-0.5 rounded-full bg-brand" aria-hidden="true" />
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function ActionFields({
  action,
  disabled,
  onChange,
}: {
  action: MacroAction;
  disabled: boolean;
  onChange: (action: MacroAction) => void;
}) {
  if (action.type === "mouseClick" || action.type === "mouseDown") {
    return (
      <div className="flex flex-wrap items-center gap-1.5">
        <MousePointerClick className="size-3.5 text-brand" aria-hidden="true" />
        {action.type === "mouseDown" && (
          <span className="text-[0.7rem] text-muted-foreground">按下</span>
        )}
        <MouseButtonPicker
          button={action.button}
          disabled={disabled}
          onChange={(button) => onChange({ ...action, button })}
        />
        <MousePositionFields
          x={action.x}
          y={action.y}
          disabled={disabled}
          onChange={(x, y) => onChange({ ...action, x, y })}
        />
      </div>
    );
  }

  if (action.type === "mouseUp") {
    return (
      <div className="flex flex-wrap items-center gap-1.5">
        <MousePointerClick className="size-3.5 text-brand" aria-hidden="true" />
        <span className="text-[0.7rem] text-muted-foreground">松开</span>
        <MouseButtonPicker
          button={action.button}
          disabled={disabled}
          onChange={(button) => onChange({ ...action, button })}
        />
        <span className="text-[0.68rem] text-muted-foreground">
          按住时长在下方「按住」里调整
        </span>
      </div>
    );
  }

  if (action.type === "key" || action.type === "keyDown" || action.type === "keyUp") {
    const isAtomic = action.type === "key";
    return (
      <div className="flex flex-wrap items-center gap-1.5">
        <Keyboard className="size-3.5 text-brand" aria-hidden="true" />
        {!isAtomic && (
          <span className="text-[0.7rem] text-muted-foreground">
            {action.type === "keyDown" ? "按下" : "松开"}
          </span>
        )}
        <Input
          value={isAtomic ? action.combo : action.key}
          disabled={disabled}
          spellCheck={false}
          placeholder={isAtomic ? "Ctrl+S" : "S / Ctrl"}
          aria-label={isAtomic ? "按键组合" : "单个物理按键"}
          className="h-6 max-w-44 px-1.5 font-mono text-[0.68rem]"
          onChange={(event) => {
            if (isAtomic) onChange({ ...action, combo: event.target.value });
            else onChange({ ...action, key: event.target.value });
          }}
        />
        <span className="text-[0.68rem] text-muted-foreground">
          {isAtomic
            ? "支持 Ctrl / Alt / Shift / Win + 字母、F1-F24、Enter、方向键"
            : "录制出的物理键（修饰键也单独成步）；按住时长在下方「按住」里调整"}
        </span>
      </div>
    );
  }

  if (action.type === "text") {
    return (
      <div className="flex items-center gap-1.5">
        <TextCursorInput className="size-3.5 shrink-0 text-brand" aria-hidden="true" />
        <Textarea
          value={action.text}
          disabled={disabled}
          rows={1}
          spellCheck={false}
          placeholder="要输入的文字（支持中文）"
          aria-label="要输入的文本"
          className="min-h-6 resize-y px-1.5 py-0.5 text-[0.68rem] leading-relaxed"
          onChange={(event) => onChange({ ...action, text: event.target.value })}
        />
      </div>
    );
  }

  return (
    <div className="flex items-center gap-1.5">
      <Timer className="size-3.5 text-brand" aria-hidden="true" />
      <span className="text-[0.7rem] text-muted-foreground">等待</span>
      <Input
        type="number"
        min={0}
        step={100}
        value={action.ms}
        disabled={disabled}
        aria-label="等待的毫秒数"
        className="h-6 w-20 px-1.5 text-[0.68rem] tabular-nums"
        onChange={(event) =>
          onChange({ ...action, ms: Math.max(0, Number(event.target.value) || 0) })
        }
      />
      <span className="text-[0.68rem] text-muted-foreground">ms</span>
    </div>
  );
}

function MouseButtonPicker({
  button,
  disabled,
  onChange,
}: {
  button: MacroMouseButton;
  disabled: boolean;
  onChange: (button: MacroMouseButton) => void;
}) {
  return (
    <>
      {(
        [
          ["left", "左键"],
          ["right", "右键"],
          ["middle", "中键"],
        ] as const
      ).map(([value, label]) => (
        <Button
          key={value}
          type="button"
          size="xs"
          variant="outline"
          disabled={disabled}
          onClick={() => onChange(value)}
          className={cn(
            "h-6 min-w-0 px-1.5 text-[0.65rem]",
            button === value && "bg-brand-soft text-brand",
          )}
        >
          {label}
        </Button>
      ))}
    </>
  );
}

function MousePositionFields({
  x,
  y,
  disabled,
  onChange,
}: {
  x: number;
  y: number;
  disabled: boolean;
  onChange: (x: number, y: number) => void;
}) {
  return (
    <>
      <span className="ml-1 text-[0.68rem] text-muted-foreground">坐标</span>
      <Input
        type="number"
        value={x}
        disabled={disabled}
        aria-label="鼠标 x 坐标"
        className="h-6 w-20 px-1.5 text-[0.68rem] tabular-nums"
        onChange={(event) => onChange(Number(event.target.value) || 0, y)}
      />
      <span className="text-[0.68rem] text-muted-foreground">,</span>
      <Input
        type="number"
        value={y}
        disabled={disabled}
        aria-label="鼠标 y 坐标"
        className="h-6 w-20 px-1.5 text-[0.68rem] tabular-nums"
        onChange={(event) => onChange(x, Number(event.target.value) || 0)}
      />
    </>
  );
}

/** 动作的中文短标签（宏列表 / 卡片摘要用）。 */
export function actionLabel(action: MacroAction): string {
  switch (action.type) {
    case "mouseClick":
      return `鼠标${mouseButtonLabel(action.button)}点击`;
    case "mouseDown":
      return `鼠标${mouseButtonLabel(action.button)}按下`;
    case "mouseUp":
      return `鼠标${mouseButtonLabel(action.button)}松开`;
    case "key":
      return `按键 ${action.combo}`;
    case "keyDown":
      return `按下 ${action.key}`;
    case "keyUp":
      return `松开 ${action.key}`;
    case "text":
      return `文本「${action.text.slice(0, 16)}${action.text.length > 16 ? "…" : ""}」`;
    case "wait":
      return `等待 ${action.ms}ms`;
  }
}

/** 鼠标按键中文名（供后续扩展 / 摘要复用）。 */
export function mouseButtonLabel(button: MacroMouseButton): string {
  return button === "left" ? "左键" : button === "right" ? "右键" : "中键";
}
