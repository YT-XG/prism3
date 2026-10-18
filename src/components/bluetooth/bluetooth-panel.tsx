import { useEffect, useState } from "react";

import {
  ArrowLeft,
  Bluetooth,
  ChevronRight,
  Loader2,
  Radar,
  RefreshCw,
  X,
  Zap,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useEarbuds } from "@/hooks/use-earbuds";
import { displayDeviceName, oneBudUnavailable } from "@/lib/ble-format";
import { formatTimeAgo } from "@/lib/format";
import type {
  AncGradient,
  BatteryReading,
  BleDeviceSnapshotEntry,
  BleState,
  EqPreset,
  NoiseMode,
} from "@/lib/ipc";

const noiseModes: { value: NoiseMode; label: string }[] = [
  { value: "anc", label: "降噪" },
  { value: "normal", label: "正常" },
  { value: "transparency", label: "通透" },
  { value: "wind", label: "风噪" },
];

const ancGradients: { value: AncGradient; label: string }[] = [
  { value: "light", label: "轻度" },
  { value: "medium", label: "中度" },
  { value: "deep", label: "深度" },
];

const eqPresets: { value: EqPreset; label: string }[] = [
  { value: "hifi", label: "HIFI" },
  { value: "pop", label: "POP" },
  { value: "rock", label: "ROCK" },
];

/**
 * 「蓝牙」页：主窗口唯一的耳机管理入口，分两级。
 *
 * 一级是设备管理（已记住的耳机 + 自动连接 + 扫描添加）；点**已连接**的那一行进入二级，
 * 二级是那台设备的功能设置（电量 / 降噪 / 音效 / 开关 / 断开），它不是独立的菜单入口。
 * 二级完全由广播状态推导：链路一断（或设备被移除）当帧就退回一级，绝不留过期控件。
 *
 * 控件只「发起意图」，显示值一律来自后端广播的 `ble:state`——设备侧会隐式改状态
 * （例如设置降噪梯度会顺带重新选中降噪），前端不做乐观更新；需要活动链路的控件在未连接时
 * 禁用，设备列表与自动连接不依赖链路，始终可用。
 */
export function BluetoothPanel() {
  // 控件袋整体下传给二级页，避免把十几个回调逐个穿参。
  const controls = useEarbuds();
  const { state } = controls;

  /** 二级页选中的设备地址；`null` = 停留在一级设备列表。 */
  const [selectedAddress, setSelectedAddress] = useState<string | null>(null);
  const phase = state?.phase ?? null;

  // 连接断开或设备被移除时清掉选中项，回到列表（渲染侧还会再按 phase 兜一层）。
  useEffect(() => {
    if (selectedAddress === null) return;
    const stillThere = state?.devices.some((entry) => entry.address === selectedAddress) ?? false;
    if (phase !== "ready" || !stillThere) setSelectedAddress(null);
  }, [phase, selectedAddress, state]);

  if (!state) {
    return (
      <div className="flex flex-col gap-4">
        <PanelHeader />
        <p className="text-sm text-muted-foreground">正在读取耳机状态…</p>
      </div>
    );
  }

  // 二级页的前提是「这台设备此刻就是活连接」：`phase` 一变就不认。
  const liveAddress = state.phase === "ready" ? state.address : null;
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

  return (
    <div className="flex flex-col gap-4">
      <PanelHeader />

      <ConnectionCard state={state} controls={controls} />

      <Card>
        <CardHeader>
          <CardTitle>设备列表</CardTitle>
          <CardDescription>
            点已连接的那一行进入设备设置；点其它行则是把它切换成当前设备。
          </CardDescription>
        </CardHeader>
        <CardContent>
          {state.devices.length === 0 ? (
            <div className="flex flex-col items-center gap-1 rounded-lg bg-muted/50 px-3 py-4 text-center">
              <Bluetooth className="size-4 text-muted-foreground" aria-hidden />
              <span className="text-xs text-muted-foreground">还没有添加设备</span>
              <span className="text-xs text-muted-foreground">
                用下方「添加设备」扫描附近的耳机并加入列表，之后开仓取出耳机就会自动连上。
              </span>
            </div>
          ) : (
            <ul className="flex flex-col gap-1">
              {state.devices.map((entry) => (
                <DeviceRow
                  key={entry.address}
                  entry={entry}
                  active={entry.address === state.activeAddress}
                  live={entry.address === liveAddress}
                  phase={state.phase}
                  battery={state.battery}
                  busy={busyPhase(state.phase)}
                  onOpen={() => setSelectedAddress(entry.address)}
                  onSelect={() => void controls.setActive(entry.address)}
                  onRemove={() => void controls.removeDevice(entry.address)}
                />
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>自动连接</CardTitle>
          <CardDescription>
            自动连上「设备列表」里选中的那一台；手动「断开」会暂停自动重连。
            {state.watching && <span className="ml-1">监听中…</span>}
          </CardDescription>
        </CardHeader>
        <CardContent className="flex items-center justify-between gap-3">
          <Label htmlFor="ble-auto-connect">开仓取出耳机时自动连上</Label>
          <Switch
            id="ble-auto-connect"
            checked={state.autoConnect}
            onCheckedChange={(checked) => void controls.setAutoConnect(checked)}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>添加设备</CardTitle>
          <CardDescription>
            耳机广播既不带名字也不带服务 UUID，候选多半只显示地址；RSSI 越接近 0 信号越强。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div>
            <Button
              variant="outline"
              size="sm"
              disabled={controls.scanning}
              onClick={() => void controls.scan()}
            >
              {controls.scanning ? (
                <Loader2 className="animate-spin" data-icon="inline-start" />
              ) : (
                <Radar data-icon="inline-start" />
              )}
              {controls.scanning ? "扫描中…" : "扫描"}
            </Button>
          </div>

          {controls.candidates.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              {controls.scanning ? "正在扫描附近的 BLE 设备…" : "还没有扫描结果。"}
            </p>
          ) : (
            <ul className="flex flex-col gap-1">
              {controls.candidates.map((candidate) => (
                <li
                  key={candidate.address}
                  className="flex items-center justify-between gap-3 rounded-md bg-muted/50 px-2.5 py-1.5"
                >
                  <div className="flex min-w-0 flex-col gap-0.5">
                    <span className="flex items-center gap-2">
                      <span className="truncate text-xs">
                        {displayDeviceName(candidate.name, candidate.address)}
                      </span>
                      {candidate.known && <Badge variant="secondary">已在列表</Badge>}
                    </span>
                    <span className="truncate text-xs text-muted-foreground">
                      {candidate.address} · {candidate.rssi} dBm
                    </span>
                  </div>

                  <Button
                    variant="outline"
                    size="xs"
                    onClick={() => void controls.addDevice(candidate)}
                  >
                    添加
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

/** 页面标题（一级与「正在读取」共用）。 */
function PanelHeader() {
  return (
    <div>
      <h1 className="font-heading text-lg font-medium">蓝牙</h1>
      <p className="text-sm text-muted-foreground">
        直连 BLE 耳机：管理已记住的设备并按设备调整降噪 / 音效；设备列表与自动连接不需要先连上。
      </p>
    </div>
  );
}

/** 一级页的「当前连接」：连接状态、地址与刷新 / 连接 / 断开。 */
function ConnectionCard({
  state,
  controls,
}: {
  state: BleState;
  controls: ReturnType<typeof useEarbuds>;
}) {
  const ready = state.phase === "ready";

  return (
    <Card>
      <CardHeader>
        <CardTitle>当前连接</CardTitle>
        <CardDescription>
          耳机同一时刻只接受一个连接，厂商手机 App 占线时连接会失败。
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex min-w-0 flex-col gap-1">
            <span className="flex items-center gap-2 text-sm font-medium">
              <span className="truncate">{headerTitle(state)}</span>
              <Badge variant={phaseBadgeVariant(state.phase)}>{describePhase(state.phase)}</Badge>
            </span>
            <span className="truncate text-xs text-muted-foreground">
              {state.address ?? "尚未获取设备地址"}
            </span>
          </div>

          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={!ready}
              onClick={() => void controls.refresh()}
            >
              <RefreshCw data-icon="inline-start" />
              刷新
            </Button>
            {ready ? (
              <Button variant="outline" size="sm" onClick={() => void controls.disconnect()}>
                断开
              </Button>
            ) : (
              <Button
                size="sm"
                disabled={busyPhase(state.phase)}
                onClick={() => void controls.connect()}
              >
                连接
              </Button>
            )}
          </div>
        </div>

        {state.error && <p className="text-xs text-destructive">{state.error}</p>}
      </CardContent>
    </Card>
  );
}

/**
 * 设备列表的一行：正文区点一下 = 已连接则进入设备设置、否则切换过去；行尾是显式按钮。
 *
 * 「移除」必须留在行内——未连接的设备否则再也删不掉。只有已连接的那一行可进入二级，
 * 因此右向箭头也只挂在它上面。
 */
function DeviceRow({
  entry,
  active,
  live,
  phase,
  battery,
  busy,
  onOpen,
  onSelect,
  onRemove,
}: {
  entry: BleDeviceSnapshotEntry;
  /** 是否是「当前设备」（自动连接的目标）。 */
  active: boolean;
  /** 是否就是此刻连上的那一台。 */
  live: boolean;
  phase: BleState["phase"];
  battery: BatteryReading | null;
  busy: boolean;
  onOpen: () => void;
  onSelect: () => void;
  onRemove: () => void;
}) {
  const name = displayDeviceName(entry.name, entry.address);

  return (
    <li className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5 rounded-md bg-muted/50 px-2.5 py-1.5">
      <button
        type="button"
        disabled={busy}
        title={live ? "打开设备设置" : `切换 ${name}`}
        onClick={live ? onOpen : onSelect}
        className="flex min-w-0 flex-1 items-center gap-2 text-left disabled:cursor-default"
      >
        <span className="flex min-w-0 flex-col gap-0.5">
          <span className="flex items-center gap-2">
            <span className="truncate text-xs">{name}</span>
            {active && <Badge variant="outline">当前设备</Badge>}
            {live && <Badge variant="secondary">已连接</Badge>}
          </span>
          <span className="truncate text-xs text-muted-foreground">
            {entry.address}
            {entry.lastConnectedAt !== null &&
              ` · 上次连接 ${formatTimeAgo(entry.lastConnectedAt)}`}
          </span>
        </span>

        {live && (
          <span className="flex items-center gap-1 text-xs tabular-nums text-muted-foreground">
            {(battery?.leftCharging === true || battery?.rightCharging === true) && (
              <Zap className="size-3 text-emerald-600 dark:text-emerald-500" aria-hidden />
            )}
            {formatBatterySummary(battery)}
          </span>
        )}

        {live && <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />}
      </button>

      <div className="flex flex-wrap items-center gap-2">
        {!live && (
          <Button variant="outline" size="xs" disabled={busy} onClick={onSelect}>
            {phase === "ready" ? "切换" : "连接"}
          </Button>
        )}
        <Button variant="ghost" size="icon-xs" aria-label={`移除 ${name}`} onClick={onRemove}>
          <X />
        </Button>
      </div>
    </li>
  );
}

/**
 * 二级页：当前设备的功能设置（进入前提就是这条链路活着，控件仍显式带上 `ready` 判定）。
 *
 * 显示值全部来自 `ble:state`，不做乐观更新；「–」表示设备报告该项电量不可用，不是 0%。
 */
function DeviceSettings({
  entry,
  state,
  controls,
  onBack,
}: {
  entry: BleDeviceSnapshotEntry;
  state: BleState;
  controls: ReturnType<typeof useEarbuds>;
  onBack: () => void;
}) {
  const {
    disconnect,
    refresh,
    readBattery,
    setNoiseMode,
    setAncGradient,
    setAdaptiveAnc,
    setEqPreset,
    setGameMode,
    setDualDevice,
  } = controls;
  const ready = state.phase === "ready";

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-1">
          <Button
            variant="ghost"
            size="icon-sm"
            title="返回设备列表"
            aria-label="返回设备列表"
            onClick={onBack}
          >
            <ArrowLeft />
          </Button>
          <div className="flex min-w-0 flex-col gap-1">
            <span className="flex items-center gap-2 text-sm font-medium">
              <span className="truncate">{displayDeviceName(entry.name, entry.address)}</span>
              <Badge variant={phaseBadgeVariant(state.phase)}>{describePhase(state.phase)}</Badge>
            </span>
            <span className="truncate text-xs text-muted-foreground">{entry.address}</span>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" disabled={!ready} onClick={() => void refresh()}>
            <RefreshCw data-icon="inline-start" />
            刷新
          </Button>
          <Button variant="outline" size="sm" disabled={!ready} onClick={() => void disconnect()}>
            断开连接
          </Button>
        </div>
      </div>

      {state.error && <p className="text-xs text-destructive">{state.error}</p>}

      <Card>
        <CardHeader>
          <CardTitle>电量</CardTitle>
          <CardDescription>
            {state.battery
              ? `读取于 ${formatTimeAgo(state.battery.readAtSecs)}`
              : "尚未读取到电量。"}
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex flex-wrap gap-x-6 gap-y-2">
            <BatteryValue
              label="左"
              value={state.battery?.left ?? null}
              charging={state.battery?.leftCharging ?? null}
            />
            <BatteryValue
              label="右"
              value={state.battery?.right ?? null}
              charging={state.battery?.rightCharging ?? null}
            />
            <BatteryValue label="仓" value={state.battery?.case ?? null} />
          </div>

          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">
              「–」表示设备报告该项电量不可用（充电仓入耳时常见），不是 0%。
            </p>
            <Button variant="ghost" size="xs" disabled={!ready} onClick={() => void readBattery()}>
              读取电量
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>噪声控制</CardTitle>
          <CardDescription>
            设备侧即时生效；它会自行改状态，因此这里只显示它报告的当前值。
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <Tabs
            value={state.noiseMode ?? ""}
            onValueChange={(value) => void setNoiseMode(value as NoiseMode)}
          >
            <TabsList>
              {noiseModes.map(({ value, label }) => (
                <TabsTrigger key={value} value={value} disabled={!ready}>
                  {label}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>

          {state.noiseMode === "anc" && (
            <div className="flex flex-col gap-3 border-l border-border/60 pt-1 pl-3">
              {/* 只提示、不禁用：耳机没有两只都在线时，它会响一声但不会改梯度（协议笔记 §14.3 第 4 条）。
                  文案对「缺一只」与「两只都不可用」都成立，别写成「只有一只在线」。 */}
              {oneBudUnavailable(state.battery) && (
                <p className="text-xs text-muted-foreground">
                  需要两只耳机都在线，耳机才会切换降噪梯度。
                </p>
              )}

              <div className="flex flex-col gap-2">
                <span className="text-xs text-muted-foreground">降噪梯度</span>
                <Tabs
                  value={state.ancGradient ?? ""}
                  onValueChange={(value) => void setAncGradient(value as AncGradient)}
                >
                  <TabsList>
                    {ancGradients.map(({ value, label }) => (
                      <TabsTrigger key={value} value={value} disabled={!ready}>
                        {label}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                </Tabs>
              </div>

              <div className="flex items-center justify-between">
                <Label htmlFor="adaptive-anc">自适应降噪</Label>
                <Switch
                  id="adaptive-anc"
                  checked={state.adaptiveAnc ?? false}
                  disabled={!ready}
                  onCheckedChange={(checked) => void setAdaptiveAnc(checked)}
                />
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>音效调节</CardTitle>
          <CardDescription>按曲风挑一档；同样以设备报告的状态为准。</CardDescription>
        </CardHeader>
        <CardContent>
          <Tabs
            value={state.eqPreset ?? ""}
            onValueChange={(value) => void setEqPreset(value as EqPreset)}
          >
            <TabsList>
              {eqPresets.map(({ value, label }) => (
                <TabsTrigger key={value} value={value} disabled={!ready}>
                  {label}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>其他设置</CardTitle>
          <CardDescription>游戏模式压低延迟；双设备连接允许同时保持两台设备。</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center justify-between">
            <Label htmlFor="game-mode">游戏模式</Label>
            <Switch
              id="game-mode"
              checked={state.gameMode ?? false}
              disabled={!ready}
              onCheckedChange={(checked) => void setGameMode(checked)}
            />
          </div>

          <div className="flex items-center justify-between">
            <Label htmlFor="dual-device">双设备连接</Label>
            <Switch
              id="dual-device"
              checked={state.dualDevice ?? false}
              disabled={!ready}
              onCheckedChange={(checked) => void setDualDevice(checked)}
            />
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

/** 列表行内的一行电量：与电量块同一口径——设备未报告的项显示「–」，绝不落成 0%。 */
function formatBatterySummary(battery: BatteryReading | null): string {
  if (!battery) return "电量未知";

  const percent = (value: number | null) => (value === null ? "–" : `${value}%`);
  return `左 ${percent(battery.left)} · 右 ${percent(battery.right)} · 仓 ${percent(battery.case)}`;
}

/** 剩余电量：设备未报告时为「–」，绝不显示成 0%；`charging` 为真时补一个充电标志。 */
function BatteryValue({
  label,
  value,
  charging,
}: {
  label: string;
  value: number | null;
  charging?: boolean | null;
}) {
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="font-heading text-xl font-medium tabular-nums">
        {value === null ? "–" : `${value}%`}
      </span>
      <span className="text-xs text-muted-foreground">{label}</span>
      {charging === true && (
        <span
          className="flex items-center gap-1 self-center text-emerald-600 dark:text-emerald-500"
          title="充电中"
        >
          <Zap className="size-3" aria-hidden />
          <span className="sr-only">充电中</span>
        </span>
      )}
    </span>
  );
}

/**
 * 当前连接的展示名：与设备列表共用同一套口径（去掉末尾的 `_BLE` 后缀），
 * 没有名字时沿用占位文案——耳机不广播名字，`name` 为 null 是正常现象。
 */
function headerTitle(state: BleState): string {
  if (!state.name) return state.address ? "已连接耳机" : "未连接设备";
  return displayDeviceName(state.name, state.address ?? state.name);
}

/** 正在建立链路（扫描 / 连接中）：此时不该再发起新的连接或进入二级页。 */
function busyPhase(phase: BleState["phase"]): boolean {
  return phase === "scanning" || phase === "connecting";
}

function describePhase(phase: BleState["phase"]): string {
  switch (phase) {
    case "scanning":
      return "搜索中";
    case "connecting":
      return "连接中";
    case "ready":
      return "已连接";
    case "error":
      return "连接失败";
    default:
      return "未连接";
  }
}

function phaseBadgeVariant(phase: BleState["phase"]): "secondary" | "outline" | "destructive" {
  switch (phase) {
    case "ready":
      return "secondary";
    case "error":
      return "destructive";
    default:
      return "outline";
  }
}
