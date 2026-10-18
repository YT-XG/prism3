import { useEffect, useState } from "react";

import {
  Bluetooth,
  Check,
  ChevronLeft,
  ChevronRight,
  RefreshCw,
  Trash2,
  Zap,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useEarbuds } from "@/hooks/use-earbuds";
import { displayDeviceName, oneBudUnavailable } from "@/lib/ble-format";
import { formatTimeAgo } from "@/lib/format";
import type { AncGradient, BatteryReading, BleState, EqPreset, NoiseMode } from "@/lib/ipc";
import { cn } from "@/lib/utils";

/** `state.devices` 的元素类型（不另行命名设备结构，直接用状态里的定义）。 */
type DeviceEntry = BleState["devices"][number];

/** 分档选项：与主窗口「蓝牙」页同一套取值，只是这里排成等宽小胶囊。 */
const NOISE_MODES: { value: NoiseMode; label: string }[] = [
  { value: "anc", label: "降噪" },
  { value: "normal", label: "正常" },
  { value: "transparency", label: "通透" },
  { value: "wind", label: "风噪" },
];

const ANC_GRADIENTS: { value: AncGradient; label: string }[] = [
  { value: "light", label: "轻度" },
  { value: "medium", label: "中度" },
  { value: "deep", label: "深度" },
];

const EQ_PRESETS: { value: EqPreset; label: string }[] = [
  { value: "hifi", label: "HIFI" },
  { value: "pop", label: "POP" },
  { value: "rock", label: "ROCK" },
];

interface QuickPanelBluetoothProps {
  /** 面板已展开且当前停留的就是本页：切入本页时补一次状态读取。 */
  active: boolean;
}

/**
 * 快捷小窗的「蓝牙」页，分两级：
 *
 * - 一级 = 已知设备列表：已连接标记（绿勾，仅 `phase === "ready"` 时出现）+ 自动连接的目标设备
 *   （「当前设备」小标，与是否连上无关）+ 该设备的实时电量、自动连接开关、逐行移除，
 *   点击行内正文即可连接 / 切换；点已连接的那一行进入二级。
 * - 二级 = 该设备的功能设置（噪声控制 + ANC 子块 / 音效 / 游戏模式 / 双设备 / 断开），
 *   与主窗口「蓝牙」页同一套控件与取值域，只是排得更紧凑，靠页容器滚动。
 *
 * 显示值一律来自后端广播的 `ble:state`（本页不做乐观更新）：设备侧会隐式改状态
 * （例如写降噪梯度会顺带重新选中降噪），控件只「发起意图」，最终值以广播为准。
 * `state.battery` 只描述**当前已连接**的那台设备，故只有它的行 / 二级页显示电量；
 * 设备报告不可用的字段是 `null`，必须渲染成「–」，**不是 0%**。
 *
 * 二级页只在 `phase === "ready"` 时存在：连接一断（或设备被移除）立即退回列表，绝不展示
 * 过期控件。两级都留在本页内，不动分页器、不新增页。
 *
 * 本页用**默认页宽**（不声明 `panelWidth`）：面板窗口恒为 `PANEL_WINDOW_WIDTH` 且可见期间
 * 绝不缩放，页宽差异只体现为面板盒子的 CSS 宽度（见 docs/quick-panel-flash-postmortem.md 第 8 节）。
 */
export function QuickPanelBluetooth({ active }: QuickPanelBluetoothProps) {
  // 控件袋整体下传给二级页，避免把十几个回调逐个穿参。
  const controls = useEarbuds();
  const { state, refresh, setActive, setAutoConnect, removeDevice } = controls;

  /** 二级页选中的设备地址；`null` = 停留在一级设备列表。 */
  const [selectedAddress, setSelectedAddress] = useState<string | null>(null);
  const phase = state?.phase ?? null;

  // 状态由 `ble:state` 广播驱动，但面板窗口收起期间不会有新广播；每次切入本页补读一次，
  // 免得看到过期的电量 / 连接状态。失败由 hook 自己弹 toast，这里不管。
  useEffect(() => {
    if (active) void refresh();
  }, [active, refresh]);

  // 连接断开或设备被移除时清掉选中项，回到列表（渲染侧还会再按 `phase` 兜一层）。
  useEffect(() => {
    if (selectedAddress === null) return;
    const stillThere = state?.devices.some((entry) => entry.address === selectedAddress) ?? false;
    if (phase !== "ready" || !stillThere) setSelectedAddress(null);
  }, [phase, selectedAddress, state]);

  if (!state) {
    return (
      <div className="flex flex-col gap-0.5">
        <h2 className="font-heading text-sm font-medium">蓝牙耳机</h2>
        <p className="text-xs text-muted-foreground">正在读取耳机状态…</p>
      </div>
    );
  }

  // 真正有活连接的设备：电量与各项设置都属于它。`activeAddress` 只是自动连接的目标，
  // 两者通常一致，展示以活连接为准；未连接时退回选中项（仅用于「当前设备」标记）。
  const liveAddress = state.phase === "ready" ? (state.address ?? state.activeAddress) : null;
  const currentAddress = liveAddress ?? state.activeAddress;

  // 二级页的前提是「这台设备此刻就是活连接」：`phase` 一变就不认，避免渲染过期控件。
  const selectedEntry =
    selectedAddress !== null && selectedAddress === liveAddress
      ? (state.devices.find((entry) => entry.address === selectedAddress) ?? null)
      : null;

  if (selectedEntry) {
    return (
      <DeviceSettings
        entry={selectedEntry}
        state={state}
        controls={controls}
        onBack={() => setSelectedAddress(null)}
      />
    );
  }

  const connected = state.phase === "ready";
  const busy = state.phase === "connecting" || state.phase === "scanning";
  const connectedEntry = state.devices.find((entry) => entry.address === currentAddress) ?? null;

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-start justify-between gap-2">
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="font-heading text-sm font-medium">蓝牙耳机</h2>
          <p className="truncate text-xs text-muted-foreground">
            {connected
              ? `已连接 ${
                  connectedEntry
                    ? displayDeviceName(connectedEntry.name, connectedEntry.address)
                    : "未保存的设备"
                }`
              : describePhase(state.phase)}
          </p>
        </div>
        <Button
          variant="ghost"
          size="icon-xs"
          title="重新读取状态"
          aria-label="重新读取状态"
          disabled={busy}
          onClick={() => void refresh()}
          className="shrink-0"
        >
          <RefreshCw className={cn(busy && "animate-spin")} />
        </Button>
      </div>

      {state.error && <p className="text-[11px] text-destructive">{state.error}</p>}

      <div className="flex items-center justify-between gap-2 rounded-lg bg-muted/50 px-2.5 py-1.5">
        <span className="flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
          <span
            aria-hidden
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              state.watching ? "bg-emerald-500" : "bg-muted-foreground/40",
            )}
          />
          <span className="truncate">{autoConnectText(state)}</span>
        </span>
        <Switch
          size="sm"
          checked={state.autoConnect}
          aria-label="自动连接已保存的耳机"
          onCheckedChange={(checked) => void setAutoConnect(checked)}
          className="shrink-0"
        />
      </div>

      {state.devices.length === 0 ? (
        <div className="flex flex-col items-center gap-1 rounded-lg bg-muted/50 px-3 py-4 text-center">
          <Bluetooth className="size-4 text-muted-foreground" aria-hidden />
          <span className="text-xs text-muted-foreground">还没有已知设备</span>
          <span className="text-[11px] text-muted-foreground">
            在主窗口的「蓝牙」页里添加并连接一次，设备就会记到这里
          </span>
        </div>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {state.devices.map((entry) => (
            <DeviceRow
              key={entry.address}
              entry={entry}
              isActive={entry.address === currentAddress}
              phase={state.phase}
              battery={state.battery}
              busy={busy}
              onOpen={() => setSelectedAddress(entry.address)}
              onSelect={setActive}
              onRemove={removeDevice}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * 一级页的一台设备：点行内正文 = 连接（已连接则进入设置），行尾两个按钮分别是
 * 显式的连接 / 切换与移除——移除必须留在列表里，否则未连接的设备再也删不掉。
 */
function DeviceRow({
  entry,
  isActive,
  phase,
  battery,
  busy,
  onOpen,
  onSelect,
  onRemove,
}: {
  entry: DeviceEntry;
  isActive: boolean;
  phase: BleState["phase"];
  battery: BatteryReading | null;
  busy: boolean;
  onOpen: () => void;
  onSelect: (address: string) => Promise<void>;
  onRemove: (address: string) => Promise<void>;
}) {
  const name = displayDeviceName(entry.name, entry.address);
  const connected = isActive && phase === "ready";
  const pending = isActive && (phase === "connecting" || phase === "scanning");

  const handleRowClick = () => {
    if (busy) return;
    if (connected) onOpen();
    else void onSelect(entry.address);
  };

  return (
    <li className="flex items-center gap-1 rounded-lg bg-muted/50 py-1.5 pr-1 pl-2.5">
      <button
        type="button"
        onClick={handleRowClick}
        disabled={busy}
        title={connected ? "打开设备设置" : `连接 ${name}`}
        className={cn(
          "flex min-w-0 flex-1 items-center justify-between gap-2 rounded-md text-left focus-visible:outline-none",
          !busy && "cursor-pointer",
        )}
      >
        <span className="flex min-w-0 flex-col gap-0.5">
          <span className="flex min-w-0 items-center gap-1.5 text-xs font-medium">
            <span className="truncate">{name}</span>
            {/* 绿勾只表示「此刻就连着」（`connected`），不能拿「自动连接的目标」顶替：
                未连上的目标若也画勾，界面会与标题的「未连接」自相矛盾。目标本身另用文字标出。 */}
            {connected && (
              <span className="flex shrink-0 items-center text-emerald-600 dark:text-emerald-500">
                <Check className="size-3" aria-hidden />
                <span className="sr-only">已连接</span>
              </span>
            )}
            {isActive && !connected && (
              <span className="shrink-0 rounded bg-muted-foreground/15 px-1 text-[10px] font-normal text-muted-foreground">
                当前设备
              </span>
            )}
          </span>
          <span className="truncate text-[11px] text-muted-foreground tabular-nums">
            {detailText({ entry, isActive, phase, battery })}
          </span>
        </span>

        {/* 只有已连接的那一行可以进设置页，用右向箭头表示「可进入」。 */}
        {connected && (
          <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        )}
      </button>

      {pending ? (
        <span className="shrink-0 px-1 text-[11px] text-muted-foreground">连接中…</span>
      ) : (
        !connected && (
          <Button
            variant="outline"
            size="xs"
            disabled={busy}
            title={`连接 ${name}`}
            onClick={() => void onSelect(entry.address)}
            className="shrink-0"
          >
            {phase === "ready" ? "切换" : "连接"}
          </Button>
        )
      )}

      <Button
        variant="ghost"
        size="icon-xs"
        title="移除设备"
        aria-label={`移除 ${name}`}
        onClick={() => void onRemove(entry.address)}
        className="shrink-0"
      >
        <Trash2 />
      </Button>
    </li>
  );
}

/** 二级页：当前设备的功能设置（与主窗口「蓝牙」页同一套控件，排得更紧凑）。 */
function DeviceSettings({
  entry,
  state,
  controls,
  onBack,
}: {
  entry: DeviceEntry;
  state: BleState;
  controls: ReturnType<typeof useEarbuds>;
  onBack: () => void;
}) {
  const {
    disconnect,
    setNoiseMode,
    setAncGradient,
    setAdaptiveAnc,
    setEqPreset,
    setGameMode,
    setDualDevice,
  } = controls;

  // 二级页只在已连接时存在，这里的 `ready` 仍作为控件启用条件显式带上。
  const ready = state.phase === "ready";
  const battery = state.battery;
  const batteryUnknown =
    battery === null ||
    [battery.left, battery.right, battery.case].some((value) => value === null);

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-center gap-1">
        <Button
          variant="ghost"
          size="icon-xs"
          title="返回设备列表"
          aria-label="返回设备列表"
          onClick={onBack}
          className="shrink-0"
        >
          <ChevronLeft />
        </Button>
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-xs font-medium">
            {displayDeviceName(entry.name, entry.address)}
          </span>
          <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
            <span
              className={cn(
                "size-1.5 shrink-0 rounded-full",
                ready ? "bg-emerald-500" : "bg-muted-foreground/40",
              )}
              aria-hidden
            />
            已连接
          </span>
        </div>
      </div>

      <div className="flex flex-col gap-1 rounded-lg bg-muted/50 p-2.5">
        <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1">
          <BatteryCell label="左" value={battery?.left ?? null} charging={battery?.leftCharging ?? null} />
          <BatteryCell label="右" value={battery?.right ?? null} charging={battery?.rightCharging ?? null} />
          <BatteryCell label="仓" value={battery?.case ?? null} />
        </div>
        {batteryUnknown && (
          <p className="text-[11px] text-muted-foreground">「–」表示设备未报告该项电量，不是 0%。</p>
        )}
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-medium text-muted-foreground">噪声控制</span>
        <OptionRow
          options={NOISE_MODES}
          value={state.noiseMode}
          disabled={!ready}
          onSelect={(mode) => void setNoiseMode(mode)}
        />

        {/* ANC 子块与主窗口同一条件：只有当前模式就是降噪时才出现。 */}
        {state.noiseMode === "anc" && (
          <div className="flex flex-col gap-2 border-l border-border/60 pt-0.5 pl-3">
            {/* 只提示、不禁用：单只耳机在线时设备会响一声但不改梯度（协议笔记 §14.3 第 4 条）。 */}
            {oneBudUnavailable(battery) && (
              <p className="text-[11px] text-muted-foreground">需两只耳机都在线</p>
            )}

            <div className="flex flex-col gap-1">
              <span className="text-[11px] text-muted-foreground">降噪梯度</span>
              <OptionRow
                options={ANC_GRADIENTS}
                value={state.ancGradient}
                disabled={!ready}
                onSelect={(level) => void setAncGradient(level)}
              />
            </div>
            <SwitchRow
              label="自适应降噪"
              checked={state.adaptiveAnc ?? false}
              disabled={!ready}
              onChange={setAdaptiveAnc}
            />
          </div>
        )}
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-xs font-medium text-muted-foreground">音效调节</span>
        <OptionRow
          options={EQ_PRESETS}
          value={state.eqPreset}
          disabled={!ready}
          onSelect={(preset) => void setEqPreset(preset)}
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <SwitchRow
          label="游戏模式"
          checked={state.gameMode ?? false}
          disabled={!ready}
          onChange={setGameMode}
        />
        <SwitchRow
          label="双设备连接"
          checked={state.dualDevice ?? false}
          disabled={!ready}
          onChange={setDualDevice}
        />
      </div>

      <Button
        variant="outline"
        size="xs"
        disabled={!ready}
        onClick={() => void disconnect()}
        className="w-full"
      >
        断开连接
      </Button>
    </div>
  );
}

/**
 * 分档选择：等宽小胶囊，样式与图标栏的选中胶囊一致（面板窄，不用主窗口的 `Tabs`）。
 * 选中态由外部 `value`（即广播状态）决定，不在本地记忆。
 */
function OptionRow<T extends string>({
  options,
  value,
  disabled,
  onSelect,
}: {
  options: { value: T; label: string }[];
  value: T | null;
  disabled: boolean;
  onSelect: (value: T) => void;
}) {
  return (
    <div className="flex gap-1 rounded-lg bg-muted/50 p-1">
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={active}
            disabled={disabled}
            onClick={() => onSelect(option.value)}
            className={cn(
              "flex-1 rounded-md px-1 py-1 text-[11px] transition-colors focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50",
              active
                ? "bg-background font-medium text-foreground shadow-sm ring-1 ring-black/5 dark:ring-white/10"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

/** 开关行（游戏模式 / 双设备连接 / 自适应降噪）：文字 + 小号开关。 */
function SwitchRow({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (enabled: boolean) => Promise<void>;
}) {
  return (
    <div className="flex items-center justify-between gap-2 rounded-lg bg-muted/50 px-2.5 py-1.5">
      <span className="text-xs">{label}</span>
      <Switch
        size="sm"
        checked={checked}
        disabled={disabled}
        aria-label={label}
        onCheckedChange={(next) => void onChange(next)}
        className="shrink-0"
      />
    </div>
  );
}

/** 剩余电量：设备未报告时为「–」，绝不显示成 0%；`charging` 为真时补一个充电标志。 */
function BatteryCell({
  label,
  value,
  charging,
}: {
  label: string;
  value: number | null;
  charging?: boolean | null;
}) {
  return (
    <span className="flex items-baseline gap-1">
      <span className="font-heading text-base font-medium tabular-nums">
        {value === null ? "–" : `${value}%`}
      </span>
      <span className="text-[11px] text-muted-foreground">{label}</span>
      {charging === true && (
        <span
          className="flex items-center self-center text-emerald-600 dark:text-emerald-500"
          title="充电中"
        >
          <Zap className="size-3" aria-hidden />
          <span className="sr-only">充电中</span>
        </span>
      )}
    </span>
  );
}

/** 当前设备那一行的细节：已连接显示电量，连接中显示进度，其余显示上次连接时间。 */
function detailText({
  entry,
  isActive,
  phase,
  battery,
}: {
  entry: DeviceEntry;
  isActive: boolean;
  phase: BleState["phase"];
  battery: BatteryReading | null;
}): string {
  if (isActive) {
    if (phase === "ready") return batteryText(battery);
    if (phase === "connecting") return "正在连接…";
    if (phase === "scanning") return "正在搜索设备…";
    return "未连接";
  }
  return entry.lastConnectedAt === null
    ? "未连接过"
    : `上次连接 ${formatTimeAgo(entry.lastConnectedAt)}`;
}

/**
 * 三路电量一行：设备未报告（`null`）时是「–」，**不是 0%**；整个读数缺失时三路都是「–」。
 */
function batteryText(battery: BatteryReading | null): string {
  const percent = (value: number | null) => (value === null ? "–" : `${value}%`);
  const parts = [
    `左 ${percent(battery?.left ?? null)}`,
    `右 ${percent(battery?.right ?? null)}`,
    `仓 ${percent(battery?.case ?? null)}`,
  ];
  return parts.join(" · ");
}

/** 自动连接 / 监听状态：`watching` 表示后台正在盯着设备出现，`autoConnect` 是开关本身。 */
function autoConnectText(state: BleState): string {
  if (state.watching) return state.autoConnect ? "监听中 · 自动连接" : "监听中";
  return state.autoConnect ? "自动连接已开启" : "自动连接已关闭";
}

function describePhase(phase: BleState["phase"]): string {
  switch (phase) {
    case "scanning":
      return "正在搜索设备…";
    case "connecting":
      return "正在连接…";
    case "ready":
      return "已连接";
    case "error":
      return "连接失败";
    default:
      return "未连接";
  }
}
